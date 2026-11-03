//! Who reads the hub, and how often a credential fails (openspec
//! `hub-read-needs-credential` D1 and D4).
//!
//! D1: requests are counted by the kind of credential that let them in, so
//! every caller still relying on the Tailscale identity header is found before
//! that trust is dropped. Counts only: never a login, a token or a path.
//!
//! D4: behind Tailscale serve every request comes from loopback, so a limit
//! per source address means nothing. Failed credential checks on every route
//! share one window instead; past the limit a request whose credential fails
//! gets 429. A request with a valid credential never fails its check, so the
//! limiter never refuses it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header::RETRY_AFTER, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};

use crate::state::AppState;

/// The window failed credential checks are counted over.
pub const FAIL_WINDOW: Duration = Duration::from_secs(600);
/// Failed checks allowed in one window before failing requests get 429.
pub const FAIL_LIMIT: usize = 300;

/// The kind of credential that let a request in.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// A `[[tokens]]` or `[[readers]]` bearer token.
    Bearer,
    /// A `[[client_readers]]` bearer token (matched by hash).
    ClientReader,
    /// A `/v1/login` session cookie.
    Cookie,
    /// The `Tailscale-User-Login` header (to be removed, D5).
    Tailscale,
}

#[derive(Debug)]
pub struct AuthStats {
    since: DateTime<Utc>,
    bearer: AtomicU64,
    client_reader: AtomicU64,
    cookie: AtomicU64,
    tailscale: AtomicU64,
    failed: AtomicU64,
    limited: AtomicU64,
    window: Mutex<VecDeque<Instant>>,
}

impl Default for AuthStats {
    fn default() -> Self {
        Self {
            since: Utc::now(),
            bearer: AtomicU64::new(0),
            client_reader: AtomicU64::new(0),
            cookie: AtomicU64::new(0),
            tailscale: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            limited: AtomicU64::new(0),
            window: Mutex::new(VecDeque::new()),
        }
    }
}

impl AuthStats {
    pub fn accepted(&self, kind: Kind) {
        let counter = match kind {
            Kind::Bearer => &self.bearer,
            Kind::ClientReader => &self.client_reader,
            Kind::Cookie => &self.cookie,
            Kind::Tailscale => &self.tailscale,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one failed credential check at `now`; true when the window now
    /// holds more than [`FAIL_LIMIT`] failures.
    pub fn failed_at(&self, now: Instant) -> bool {
        self.failed.fetch_add(1, Ordering::Relaxed);
        let mut window = self.window.lock().expect("auth stats lock");
        while window
            .front()
            .is_some_and(|t| now.duration_since(*t) > FAIL_WINDOW)
        {
            window.pop_front();
        }
        window.push_back(now);
        window.len() > FAIL_LIMIT
    }

    fn snapshot(&self) -> serde_json::Value {
        let in_window = self.window.lock().expect("auth stats lock").len();
        serde_json::json!({
            "since": self.since,
            "accepted": {
                "bearer": self.bearer.load(Ordering::Relaxed),
                "client_reader": self.client_reader.load(Ordering::Relaxed),
                "cookie": self.cookie.load(Ordering::Relaxed),
                "tailscale": self.tailscale.load(Ordering::Relaxed),
            },
            "failed": self.failed.load(Ordering::Relaxed),
            "failed_in_window": in_window,
            "window_secs": FAIL_WINDOW.as_secs(),
            "limit": FAIL_LIMIT,
            "limited": self.limited.load(Ordering::Relaxed),
        })
    }
}

/// `GET /v1/healthz/auth`: counts by credential kind and failures. No login,
/// token or path is ever part of the answer.
pub async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(state.auth_stats.snapshot())
}

/// Router layer: every 401 (a failed credential check, on any route) is
/// counted; past [`FAIL_LIMIT`] in [`FAIL_WINDOW`] it becomes a 429 with
/// `Retry-After`. A valid credential is never answered 401, so it is never
/// limited.
pub async fn limit_failed_credentials(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    if response.status() != StatusCode::UNAUTHORIZED {
        return response;
    }
    if !state.auth_stats.failed_at(Instant::now()) {
        return response;
    }
    state.auth_stats.limited.fetch_add(1, Ordering::Relaxed);
    let mut limited = (
        StatusCode::TOO_MANY_REQUESTS,
        "too many failed credentials; retry later",
    )
        .into_response();
    limited.headers_mut().insert(
        RETRY_AFTER,
        HeaderValue::from_str(&FAIL_WINDOW.as_secs().to_string()).expect("retry-after"),
    );
    limited
}

pub type Shared = Arc<AuthStats>;
