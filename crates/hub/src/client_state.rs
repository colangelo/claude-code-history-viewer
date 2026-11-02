//! The running hub's client-scope state (openspec `hub-client-read-scope`,
//! R2-C, R2-D, R2-I): the policy generation in force, re-validated on a timer
//! and before every ingest batch, the accepted marker set kept in the database,
//! and the enforcement latch.

use std::sync::{Arc, RwLock};

use sqlx::PgPool;

use crate::client_policy::{self, Acceptance, Load, Policy};
use crate::config::ClientScopeConfig;

/// What the hub knows right now.
#[derive(Clone)]
pub enum Snapshot {
    /// A policy every lab decision is made against.
    Ready(Arc<Policy>),
    /// No usable policy: every session is client for a lab reader. The reason
    /// names the problem, never a marker.
    Unknown(String),
}

impl Snapshot {
    /// The generation a session must carry to be lab, if any can be.
    pub fn generation(&self) -> Option<&str> {
        match self {
            Snapshot::Ready(p) => Some(&p.generation),
            Snapshot::Unknown(_) => None,
        }
    }
}

pub struct ClientScope {
    cfg: ClientScopeConfig,
    snapshot: RwLock<Snapshot>,
    /// The latch as last read from `client_scope_state.enforced_at`.
    latched: RwLock<bool>,
}

impl ClientScope {
    pub fn new(cfg: ClientScopeConfig) -> Self {
        Self {
            cfg,
            snapshot: RwLock::new(Snapshot::Unknown("not loaded yet".into())),
            latched: RwLock::new(false),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.read().expect("client scope lock").clone()
    }

    /// Enforcing: configured to, or latched by an earlier enforcing start.
    pub fn enforcing(&self) -> bool {
        self.cfg.enforce || *self.latched.read().expect("client scope lock")
    }

    /// The latch is set but the config says observe: a stale render.
    pub fn latch_overrides_config(&self) -> bool {
        !self.cfg.enforce && *self.latched.read().expect("client scope lock")
    }

    /// Re-read both files and reconcile with the accepted set. A generation
    /// that is not the one in force replaces it before anything else reads it;
    /// an unchanged generation is a no-op.
    pub async fn refresh(&self, pool: &PgPool) -> anyhow::Result<()> {
        self.refresh_latch(pool).await?;
        let load = client_policy::load(&self.cfg);
        let next = match load {
            Load::Unknown(reason) => {
                if !matches!(&*self.snapshot.read().expect("lock"), Snapshot::Unknown(r) if *r == reason)
                {
                    tracing::warn!(%reason, "client_scope policy unknown: every session is client for lab readers");
                    audit(pool, "policy-unknown", None, Some(&reason), None).await?;
                }
                Snapshot::Unknown(reason)
            }
            Load::Ready(policy) => {
                if self.snapshot().generation() == Some(policy.generation.as_str()) {
                    return Ok(());
                }
                self.accept(pool, policy).await?
            }
        };
        *self.snapshot.write().expect("client scope lock") = next;
        Ok(())
    }

    async fn accept(&self, pool: &PgPool, policy: Policy) -> anyhow::Result<Snapshot> {
        let accepted: std::collections::BTreeSet<String> =
            sqlx::query_scalar::<_, String>("SELECT marker_hash FROM client_scope_policy")
                .fetch_all(pool)
                .await?
                .into_iter()
                .collect();
        let gen = policy.generation.clone();
        match client_policy::compare(&accepted, &policy.marker_hashes) {
            Acceptance::Narrowed { removed, added } => {
                let why = format!("routing narrowed: {removed} removed, {added} added; run client-scope accept-routing");
                tracing::warn!(
                    removed,
                    added,
                    "client_scope policy change: narrowing, unknown until accepted"
                );
                audit(pool, "policy-narrowed", Some(&gen), Some(&why), None).await?;
                Ok(Snapshot::Unknown(why))
            }
            outcome => {
                let (action, added) = match outcome {
                    Acceptance::Bootstrap => ("policy-bootstrap", policy.marker_hashes.len()),
                    Acceptance::Widened { added } => ("policy-widened", added),
                    Acceptance::Narrowed { .. } => unreachable!(),
                };
                record_set(pool, &policy, "hub").await?;
                tracing::warn!(
                    added,
                    removed = 0,
                    "client_scope policy change: added {added} removed 0"
                );
                audit(
                    pool,
                    action,
                    Some(&gen),
                    Some(&format!("added {added}")),
                    Some("hub"),
                )
                .await?;
                Ok(Snapshot::Ready(Arc::new(policy)))
            }
        }
    }

    /// Set the latch on the first enforcing start, and read it back.
    async fn refresh_latch(&self, pool: &PgPool) -> anyhow::Result<()> {
        if self.cfg.enforce {
            let set = sqlx::query(
                "UPDATE client_scope_state SET enforced_at = now(), updated_at = now()
                 WHERE enforced_at IS NULL",
            )
            .execute(pool)
            .await?
            .rows_affected();
            if set > 0 {
                audit(pool, "latch-set", None, None, Some("hub")).await?;
            }
        }
        let latched: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT enforced_at FROM client_scope_state")
                .fetch_optional(pool)
                .await?
                .flatten();
        *self.latched.write().expect("client scope lock") = latched.is_some();
        if latched.is_some() && !self.cfg.enforce {
            tracing::error!("client_scope: config says enforce = false after the hub has enforced; enforcing anyway (latch)");
        }
        Ok(())
    }
}

/// Replace the accepted set with this policy's (after an accepted widening, a
/// bootstrap, or the audited `accept-routing`).
pub async fn record_set(pool: &PgPool, policy: &Policy, by: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM client_scope_policy")
        .execute(&mut *tx)
        .await?;
    for h in &policy.marker_hashes {
        sqlx::query(
            "INSERT INTO client_scope_policy (marker_hash, accepted_gen, accepted_by) VALUES ($1, $2, $3)",
        )
        .bind(h)
        .bind(&policy.generation)
        .bind(by)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE client_scope_state SET generation = $1, updated_at = now()")
        .bind(&policy.generation)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn audit(
    pool: &PgPool,
    action: &str,
    generation: Option<&str>,
    detail: Option<&str>,
    approved_by: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO client_scope_audit (action, generation, detail, approved_by) VALUES ($1, $2, $3, $4)",
    )
    .bind(action)
    .bind(generation)
    .bind(detail)
    .bind(approved_by)
    .execute(pool)
    .await?;
    Ok(())
}
