//! `POST /v1/login` and `DELETE /v1/login` (openspec `hub-client-read-scope`,
//! R2-H): a browser exchanges a read token once for an `HttpOnly` session
//! cookie, so the token need not rest in the page's local storage.
//!
//! The cookie grants exactly the token's read scope, for 8 hours, and nothing
//! else: every write still needs its bearer token. Sessions live in the hub's
//! memory; a restart signs everyone out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::auth::{Authenticated, ReadScope};
use crate::error::HubError;
use crate::state::AppState;

pub const COOKIE_NAME: &str = "cchv_session";
const LIFETIME: Duration = Duration::from_secs(8 * 3600);

/// One browser session: (principal, scope, expiry).
type Session = (String, ReadScope, Instant);

/// Live browser sessions by cookie value.
#[derive(Clone, Default)]
pub struct Logins(Arc<Mutex<HashMap<String, Session>>>);

impl Logins {
    /// The principal behind a cookie value, if it is live.
    pub fn lookup(&self, value: &str) -> Option<Authenticated> {
        let mut map = self.0.lock().expect("logins lock");
        let now = Instant::now();
        map.retain(|_, (_, _, until)| *until > now);
        map.get(value).map(|(p, s, _)| Authenticated(p.clone(), *s))
    }

    fn insert(&self, principal: String, scope: ReadScope) -> String {
        let value = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.0
            .lock()
            .expect("logins lock")
            .insert(value.clone(), (principal, scope, Instant::now() + LIFETIME));
        value
    }

    fn remove(&self, value: &str) {
        self.0.lock().expect("logins lock").remove(value);
    }
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

fn cookie(value: &str, max_age: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={value}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={max_age}"
    ))
    .expect("cookie header")
}

/// Exchange the bearer token for a session cookie. Only a bearer token logs
/// in: neither the Tailscale identity nor an existing cookie can mint one.
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, HubError> {
    let Authenticated(principal, scope) = crate::auth::bearer_read_principal(&headers, &state)?;
    // Marked as a cookie session: it may read, and nothing may write on it
    // (alias writes refuse it, security's small item).
    let value = state.logins.insert(format!("cookie:{principal}"), scope);
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut()
        .insert(SET_COOKIE, cookie(&value, LIFETIME.as_secs()));
    Ok(resp)
}

/// Sign out: forget the session and expire the cookie.
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(v) = cookie_value(&headers) {
        state.logins.remove(v);
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(SET_COOKIE, cookie("", 0));
    resp
}
