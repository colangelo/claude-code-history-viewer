//! Identity + alias management: `GET /v1/identities`,
//! `POST /v1/identities/aliases`, `DELETE /v1/identities/aliases/{id}`.
//!
//! An identity is the equivalence class of project rows sharing an
//! `identity_key` (derived at ingest) — nothing here is materialized. The only
//! persisted state is the explicit alias table, and alias writes accept the
//! read principal (machine token OR trusted Tailscale identity): aliases are
//! reversible view metadata, so the write bar matches read access, with the
//! principal recorded for audit.
//!
//! Runtime `sqlx::query*` on purpose: the CI gate builds with `SQLX_OFFLINE`
//! (see the note at the top of `journal.rs`).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeMap, HashSet};

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::state::AppState;

/// One member path of an identity, folded across machines and providers.
#[derive(Debug, Serialize)]
pub struct IdentityMember {
    pub project_path: String,
    pub providers: Vec<String>,
    pub machines: Vec<String>,
    /// True when EVERY project row binding this path to the identity is a
    /// linked worktree (the unit `include_worktrees=false` excludes).
    pub worktree: bool,
    /// For worktrees: the main checkout's path.
    pub main_path: Option<String>,
    pub last_active: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct IdentityAlias {
    pub id: i64,
    pub project_path: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

/// Advisory link suggestion — the hub never acts on these itself.
#[derive(Debug, Serialize)]
pub struct IdentitySuggestion {
    /// `orphan_path` (fingerprint-less path, basename matches a member) or
    /// `related_identity` (shares a root commit — fork or remote drift).
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Identity {
    pub identity_key: String,
    /// Basename of the most recently active member path.
    pub display_name: String,
    pub members: Vec<IdentityMember>,
    pub aliases: Vec<IdentityAlias>,
    pub suggestions: Vec<IdentitySuggestion>,
}

#[derive(Debug, Deserialize)]
pub struct ListIdentityParams {
    /// `false` skips suggestion computation.
    pub suggestions: Option<bool>,
}

/// Windows-tolerant basename (the webapp applies the same rule for display).
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
        .unwrap_or(path)
}

/// Root-commit component of a `g:<root>[|<remote>]` key (`r:` keys have none).
fn key_root(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("g:")?;
    Some(rest.split('|').next().unwrap_or(rest))
}

#[derive(FromRow)]
struct MemberRow {
    identity_key: String,
    project_path: String,
    worktree: bool,
    main_path: Option<String>,
    providers: Vec<String>,
    machines: Vec<String>,
    last_active: Option<DateTime<Utc>>,
}

#[derive(FromRow)]
struct AliasRow {
    id: i64,
    project_path: String,
    identity_key: String,
    created_by: String,
    created_at: DateTime<Utc>,
}

pub async fn list(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ListIdentityParams>,
) -> Result<Json<Vec<Identity>>, HubError> {
    let member_rows = sqlx::query_as::<_, MemberRow>(
        r"
        SELECT p.identity_key,
               p.project_path,
               bool_and(p.git_worktree)          AS worktree,
               max(p.git_main_path)              AS main_path,
               array_agg(DISTINCT p.provider)    AS providers,
               array_agg(DISTINCT mac.hostname)  AS machines,
               max(p.last_modified)              AS last_active
        FROM projects p
        JOIN machines mac ON p.machine_id = mac.machine_id
        WHERE p.identity_key IS NOT NULL
        GROUP BY p.identity_key, p.project_path
        ",
    )
    .fetch_all(&state.pool)
    .await?;

    let alias_rows = sqlx::query_as::<_, AliasRow>(
        "SELECT id, project_path, identity_key, created_by, created_at
         FROM project_identity_aliases ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;

    // Fold rows into identities (BTreeMap for a stable listing order).
    let mut identities: BTreeMap<String, Identity> = BTreeMap::new();
    let entry = |map: &mut BTreeMap<String, Identity>, key: &str| {
        map.entry(key.to_string()).or_insert_with(|| Identity {
            identity_key: key.to_string(),
            display_name: String::new(),
            members: Vec::new(),
            aliases: Vec::new(),
            suggestions: Vec::new(),
        });
    };
    for row in member_rows {
        entry(&mut identities, &row.identity_key);
        let id = identities
            .get_mut(&row.identity_key)
            .expect("just inserted");
        id.members.push(IdentityMember {
            project_path: row.project_path,
            providers: row.providers,
            machines: row.machines,
            worktree: row.worktree,
            main_path: row.main_path,
            last_active: row.last_active,
        });
    }
    // Aliases attach even when no live member carries the key any more
    // (identity with only dead paths stays visible and manageable).
    for row in alias_rows {
        entry(&mut identities, &row.identity_key);
        let id = identities
            .get_mut(&row.identity_key)
            .expect("just inserted");
        id.aliases.push(IdentityAlias {
            id: row.id,
            project_path: row.project_path,
            created_by: row.created_by,
            created_at: row.created_at,
        });
    }

    for id in identities.values_mut() {
        id.members.sort_by_key(|m| std::cmp::Reverse(m.last_active));
        id.display_name = id
            .members
            .first()
            .map(|m| basename(&m.project_path))
            .or_else(|| id.aliases.first().map(|a| basename(&a.project_path)))
            .unwrap_or("")
            .to_string();
    }

    if params.suggestions.unwrap_or(true) {
        // Orphans: fingerprint-less paths not already claimed by an alias.
        // A path fingerprinted under ANY key is excluded even when stale
        // rows for it (other provider, pre-identity archive) carry a NULL
        // key: it is already a member somewhere, and an alias for it would
        // be redundant at best, conflicting at worst.
        let orphan_paths = sqlx::query_scalar::<_, String>(
            r"
            SELECT DISTINCT project_path FROM projects
            WHERE identity_key IS NULL
              AND project_path NOT IN (SELECT project_path FROM project_identity_aliases)
              AND project_path NOT IN (
                  SELECT project_path FROM projects WHERE identity_key IS NOT NULL
              )
            ",
        )
        .fetch_all(&state.pool)
        .await?;

        for id in identities.values_mut() {
            let member_basenames: HashSet<&str> = id
                .members
                .iter()
                .map(|m| basename(&m.project_path))
                .collect();
            for orphan in &orphan_paths {
                if member_basenames.contains(basename(orphan)) {
                    id.suggestions.push(IdentitySuggestion {
                        kind: "orphan_path".into(),
                        project_path: Some(orphan.clone()),
                        identity_key: None,
                    });
                }
            }
        }

        // Related identities: same root commit, different key (fork, or a
        // remote that drifted leaving stranded rows on the old key).
        let keys: Vec<String> = identities.keys().cloned().collect();
        for key in &keys {
            let Some(root) = key_root(key) else { continue };
            let related: Vec<String> = keys
                .iter()
                .filter(|k| *k != key && key_root(k) == Some(root))
                .cloned()
                .collect();
            let id = identities.get_mut(key).expect("key exists");
            for rk in related {
                id.suggestions.push(IdentitySuggestion {
                    kind: "related_identity".into(),
                    project_path: None,
                    identity_key: Some(rk),
                });
            }
        }
    }

    Ok(Json(identities.into_values().collect()))
}

#[derive(Debug, Deserialize)]
pub struct CreateAlias {
    pub project_path: String,
    pub identity_key: String,
}

/// Create (or re-point — a path belongs to at most one identity) an alias.
pub async fn create_alias(
    Authenticated(principal): Authenticated,
    State(state): State<AppState>,
    Json(payload): Json<CreateAlias>,
) -> Result<(StatusCode, Json<serde_json::Value>), HubError> {
    let path = payload.project_path.trim();
    let key = payload.identity_key.trim();
    if path.is_empty() || key.is_empty() {
        return Err(HubError::BadRequest(
            "project_path and identity_key must be non-empty".into(),
        ));
    }
    // The key must exist on some project row: rows are never deleted
    // (cumulative archive), so a live identity always has carriers — this
    // only rejects typos and stale clients.
    let known: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE identity_key = $1)")
            .bind(key)
            .fetch_one(&state.pool)
            .await?;
    if !known {
        return Err(HubError::BadRequest(format!(
            "unknown identity_key `{key}` (no project carries it)"
        )));
    }

    let id: i64 = sqlx::query_scalar(
        r"
        INSERT INTO project_identity_aliases (project_path, identity_key, created_by)
        VALUES ($1, $2, $3)
        ON CONFLICT (project_path)
        DO UPDATE SET identity_key = excluded.identity_key,
                      created_by = excluded.created_by,
                      created_at = now()
        RETURNING id
        ",
    )
    .bind(path)
    .bind(key)
    .bind(&principal)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "project_path": path,
            "identity_key": key,
        })),
    ))
}

pub async fn delete_alias(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, HubError> {
    let result = sqlx::query("DELETE FROM project_identity_aliases WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(HubError::NotFound(format!("no alias with id {id}")));
    }
    Ok(StatusCode::NO_CONTENT)
}
