//! `POST /v1/login` and `DELETE /v1/login` (openspec `hub-client-read-scope`,
//! R2-H; `hub-read-needs-credential` D3, security eff460e1): a browser
//! exchanges a READER token once for an `HttpOnly` session cookie, so the token
//! need not rest in the page.
//!
//! - Only reader tokens log in: a lab reader (`[[readers]]`, not `stats_only`)
//!   or a client reader. A machine or ingest token is refused.
//! - Sessions live in `browser_sessions` and survive a hub restart. The table
//!   holds sha256 of the cookie value and a keyed fingerprint of the token,
//!   never either value.
//! - The principal and scope come from the current config at every lookup,
//!   through that fingerprint: a token removed from the config ends its
//!   sessions at the next start.
//! - A session ends after 7 days unused or 30 days in all. A login that carries
//!   a cookie replaces it; a principal keeps at most 5 sessions.
//! - The cookie reads, and nothing else: every write still needs its bearer token.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::auth::{bearer, Authenticated, ReadScope};
use crate::error::HubError;
use crate::state::AppState;

/// `__Host-`: the browser takes it only over https, from the hub's own origin,
/// with `Path=/` and no `Domain` (security B3).
pub const COOKIE_NAME: &str = "__Host-cchv_session";
/// A session unused this long is over.
pub const IDLE_SECS: i64 = 7 * 24 * 3600;
/// A session is over this long after its login, used or not.
pub const LIFETIME_SECS: i64 = 30 * 24 * 3600;
/// `last_seen` is written at most this often per session.
const TOUCH_SECS: i64 = 300;
/// Live sessions per principal; a new login drops the oldest past this.
pub const MAX_PER_PRINCIPAL: i64 = 5;

/// HMAC-SHA256 (RFC 2104) with a key of at most one block.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    assert!(key.len() <= 64, "HMAC key longer than one block");
    let mut block = [0_u8; 64];
    block[..key.len()].copy_from_slice(key);
    let pad = |b: u8| block.map(|k| k ^ b);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(msg)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// sha256 of a token, hex: the form client readers are configured in.
pub fn token_sha256(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// 32 random bytes: a cookie value or a fresh key.
pub fn random_key() -> [u8; 32] {
    // Four v4 UUIDs (122 random bits each, from the OS) folded by sha256.
    let mut h = Sha256::new();
    for _ in 0..4 {
        h.update(uuid::Uuid::new_v4().as_bytes());
    }
    h.finalize().into()
}

/// The hub's fingerprint key: read from `path`, or created there (mode 0600,
/// written to a temporary file and renamed) when it does not exist yet.
pub fn load_or_create_key(path: &Path) -> anyhow::Result<[u8; 32]> {
    let parse = |text: &str| -> anyhow::Result<[u8; 32]> {
        let t = text.trim();
        anyhow::ensure!(
            t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()),
            "login key file {} is not 64 hex characters",
            path.display()
        );
        let mut key = [0_u8; 32];
        for (i, k) in key.iter_mut().enumerate() {
            *k = u8::from_str_radix(&t[2 * i..2 * i + 2], 16)?;
        }
        Ok(key)
    };
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let key = random_key();
            let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(hex(&key).as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&tmp, path)?;
            Ok(key)
        }
        Err(e) => Err(anyhow::anyhow!(
            "reading login key file {}: {e}",
            path.display()
        )),
    }
}

/// A token allowed to log in, by sha256: the principal and scope it reads as.
#[derive(Clone)]
pub struct Eligible {
    pub token_sha256: String,
    pub principal: String,
    pub scope: ReadScope,
}

/// The login side of the state: the key and the fingerprints of the tokens
/// that may log in, from the config the hub started with.
#[derive(Clone)]
pub struct Logins {
    key: Arc<[u8; 32]>,
    eligible: Arc<Vec<Eligible>>,
    /// token_fp → index into `eligible`.
    by_fp: Arc<HashMap<String, usize>>,
}

impl Default for Logins {
    /// A random key and no eligible token: nothing logs in, nothing persists
    /// past the process. `run()` and the tests build the real one.
    fn default() -> Self {
        Self::new(random_key(), Vec::new())
    }
}

impl Logins {
    pub fn new(key: [u8; 32], eligible: Vec<Eligible>) -> Self {
        let key = Arc::new(key);
        let by_fp = eligible
            .iter()
            .enumerate()
            .map(|(i, e)| (hex(&hmac_sha256(&key[..], e.token_sha256.as_bytes())), i))
            .collect();
        Self {
            key,
            eligible: Arc::new(eligible),
            by_fp: Arc::new(by_fp),
        }
    }

    /// The same eligible tokens under another key (the hub's file key).
    #[must_use]
    pub fn with_key(&self, key: [u8; 32]) -> Self {
        Self::new(key, self.eligible.as_ref().clone())
    }

    fn fingerprint(&self, token_sha256: &str) -> String {
        hex(&hmac_sha256(&self.key[..], token_sha256.as_bytes()))
    }

    /// The principal behind a cookie value, if its session is live and the
    /// token that opened it is still configured.
    pub async fn lookup(
        &self,
        pool: &PgPool,
        value: &str,
    ) -> Result<Option<Authenticated>, HubError> {
        let id_hash = Sha256::digest(value.as_bytes()).to_vec();
        let row: Option<(String, bool)> = sqlx::query_as(
            "SELECT token_fp, last_seen < now() - make_interval(secs => $2) FROM browser_sessions
             WHERE id_hash = $1 AND expires_at > now()
               AND last_seen > now() - make_interval(secs => $3)",
        )
        .bind(&id_hash)
        .bind(TOUCH_SECS as f64)
        .bind(IDLE_SECS as f64)
        .fetch_optional(pool)
        .await?;
        let Some((fp, stale)) = row else {
            return Ok(None);
        };
        let Some(e) = self.by_fp.get(&fp).map(|&i| &self.eligible[i]) else {
            return Ok(None);
        };
        if stale {
            sqlx::query("UPDATE browser_sessions SET last_seen = now() WHERE id_hash = $1")
                .bind(&id_hash)
                .execute(pool)
                .await?;
        }
        Ok(Some(Authenticated(
            format!("cookie:{}", e.principal),
            e.scope,
        )))
    }

    /// Open a session for the token with this sha256. `replaces`: the cookie
    /// the browser already had, ended in the same transaction.
    async fn open(
        &self,
        pool: &PgPool,
        e: &Eligible,
        replaces: Option<&str>,
    ) -> Result<String, HubError> {
        let value = hex(&random_key());
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM browser_sessions WHERE expires_at <= now() OR last_seen <= now() - make_interval(secs => $1)")
            .bind(IDLE_SECS as f64)
            .execute(&mut *tx)
            .await?;
        if let Some(old) = replaces {
            sqlx::query("DELETE FROM browser_sessions WHERE id_hash = $1")
                .bind(Sha256::digest(old.as_bytes()).to_vec())
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO browser_sessions (id_hash, principal, token_fp, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
        )
        .bind(Sha256::digest(value.as_bytes()).to_vec())
        .bind(&e.principal)
        .bind(self.fingerprint(&e.token_sha256))
        .bind(LIFETIME_SECS as f64)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM browser_sessions WHERE principal = $1 AND id_hash NOT IN (
                 SELECT id_hash FROM browser_sessions WHERE principal = $1
                 ORDER BY created_at DESC, id_hash LIMIT $2)",
        )
        .bind(&e.principal)
        .bind(MAX_PER_PRINCIPAL)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(value)
    }

    /// The eligible entry for a bearer token, if it may log in.
    fn eligible_for(&self, token: &str) -> Option<&Eligible> {
        let fp = self.fingerprint(&token_sha256(token));
        self.by_fp.get(&fp).map(|&i| &self.eligible[i])
    }
}

/// End one session by its cookie value.
async fn end(pool: &PgPool, value: &str) -> Result<(), HubError> {
    sqlx::query("DELETE FROM browser_sessions WHERE id_hash = $1")
        .bind(Sha256::digest(value.as_bytes()).to_vec())
        .execute(pool)
        .await?;
    Ok(())
}

/// `hub sessions revoke`: end every session of one principal, or all. Returns
/// how many ended.
pub async fn revoke(pool: &PgPool, principal: Option<&str>) -> anyhow::Result<u64> {
    let done = match principal {
        Some(p) => {
            sqlx::query("DELETE FROM browser_sessions WHERE principal = $1")
                .bind(p)
                .execute(pool)
                .await?
        }
        None => {
            sqlx::query("DELETE FROM browser_sessions")
                .execute(pool)
                .await?
        }
    };
    Ok(done.rows_affected())
}

/// The session cookie's value in a request, if any.
pub fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .find_map(|kv| kv.strip_prefix(COOKIE_NAME)?.strip_prefix('='))
}

fn cookie(value: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"
    ))
    .expect("cookie header")
}

/// Exchange a reader token for a session cookie. Only a bearer reader token
/// logs in: neither a machine or ingest token, nor the Tailscale identity,
/// nor an existing cookie can mint one.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, HubError> {
    let e = bearer(&headers)
        .and_then(|t| state.logins.eligible_for(t))
        .ok_or(HubError::Unauthorized)?
        .clone();
    let value = state
        .logins
        .open(&state.pool, &e, cookie_value(&headers))
        .await?;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, cookie(&value, LIFETIME_SECS));
    Ok(resp)
}

/// Sign out: end the session and expire the cookie.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, HubError> {
    if let Some(v) = cookie_value(&headers) {
        end(&state.pool, v).await?;
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(SET_COOKIE, cookie("", 0));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn only_eligible_tokens_have_a_fingerprint_and_the_key_matters() {
        let e = Eligible {
            token_sha256: token_sha256("reader-token"),
            principal: "machine:r".into(),
            scope: ReadScope::Lab,
        };
        let a = Logins::new([1; 32], vec![e]);
        assert!(a.eligible_for("reader-token").is_some());
        assert!(a.eligible_for("machine-token").is_none());
        let b = a.with_key([2; 32]);
        assert!(b.eligible_for("reader-token").is_some());
        assert_ne!(
            a.fingerprint(&token_sha256("reader-token")),
            b.fingerprint(&token_sha256("reader-token")),
            "the fingerprint is keyed, not a bare hash"
        );
    }

    #[test]
    fn the_key_file_is_created_once_private_and_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("hub-login-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub-login.key");
        let k1 = load_or_create_key(&path).unwrap();
        let k2 = load_or_create_key(&path).unwrap();
        assert_eq!(k1, k2);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&path, "short").unwrap();
        assert!(load_or_create_key(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cookie_is_host_prefixed_with_no_domain() {
        let c = cookie("v", 10);
        let c = c.to_str().unwrap();
        assert!(c.starts_with("__Host-cchv_session=v; Path=/; HttpOnly; Secure"));
        assert!(!c.to_ascii_lowercase().contains("domain"));
    }
}
