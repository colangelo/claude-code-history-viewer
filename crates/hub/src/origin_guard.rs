//! Where the Tailscale identity header may count (security fe3fd22, F1).
//!
//! tailscale serve adds `Tailscale-User-Login` to every request from ac's
//! devices, so a web page ac opens could otherwise read the archive with a
//! plain cross-origin `fetch`, and a page whose name resolves to the hub's
//! local port could forge the header. The header counts only on a request
//! addressed to the hub's own name (`Host`) that comes from the hub's own page
//! or from no page at all (`Origin` absent). Anything else is 401, counted
//! here by route template: never a path, a body or a login.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::extract::{MatchedPath, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::request::Parts;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

/// `scheme://host[:port]`, lowercased, no trailing slash. `None` if it is not
/// an http(s) origin (a path, a query or another scheme).
pub fn normalize_origin(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches('/').to_ascii_lowercase();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))?;
    (!rest.is_empty() && !rest.contains(['/', '?', '#', '@', ' '])).then_some(s)
}

/// The `host[:port]` part of a normalized origin.
fn authority(origin: &str) -> &str {
    origin.split_once("://").map_or(origin, |(_, a)| a)
}

/// Whether a request's `Host` and `Origin` let the identity header count.
/// `public_origins` are normalized; empty means the header never counts.
pub fn header_context_ok(
    host: Option<&str>,
    origin: Option<&str>,
    public_origins: &[String],
) -> bool {
    let Some(host) = host.map(|h| h.trim().to_ascii_lowercase()) else {
        return false;
    };
    if !public_origins.iter().any(|o| authority(o) == host) {
        return false;
    }
    match origin {
        None => true,
        Some(o) => normalize_origin(o).is_some_and(|o| public_origins.contains(&o)),
    }
}

/// The request's `Host` and `Origin` headers, as text.
pub fn context(parts: &Parts) -> (Option<&str>, Option<&str>) {
    let get = |name| parts.headers.get(name).and_then(|v| v.to_str().ok());
    (get(HOST), get(ORIGIN))
}

/// Refused header requests by route template, since start.
#[derive(Clone)]
pub struct Refusals {
    since: chrono::DateTime<chrono::Utc>,
    by_route: Arc<Mutex<BTreeMap<String, u64>>>,
}

impl Default for Refusals {
    fn default() -> Self {
        Self {
            since: chrono::Utc::now(),
            by_route: Arc::default(),
        }
    }
}

impl Refusals {
    /// Count one refusal under the request's route template.
    pub fn record(&self, parts: &Parts) -> String {
        let route = parts
            .extensions
            .get::<MatchedPath>()
            .map_or("(unmatched)", MatchedPath::as_str)
            .to_string();
        *self
            .by_route
            .lock()
            .expect("refusals lock")
            .entry(route.clone())
            .or_default() += 1;
        route
    }
}

#[derive(Serialize)]
pub struct RefusalsView {
    /// Since the hub started.
    since: chrono::DateTime<chrono::Utc>,
    total: u64,
    /// Route template → refusals. Never a path, a body or a login.
    by_route: BTreeMap<String, u64>,
}

/// `GET /v1/healthz/origin`: how many requests carried the identity header
/// from a foreign page or to a foreign name, by route template. A detector for
/// F1 attempts; counts only.
pub async fn healthz(State(state): State<AppState>) -> Json<RefusalsView> {
    let by_route = state
        .origin_refusals
        .by_route
        .lock()
        .expect("refusals lock")
        .clone();
    Json(RefusalsView {
        since: state.origin_refusals.since,
        total: by_route.values().sum(),
        by_route,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Vec<String> {
        vec![
            "https://hub.example:8788".into(),
            "http://hub.example:8787".into(),
        ]
    }

    #[test]
    fn origins_normalize_and_reject_paths_and_other_schemes() {
        assert_eq!(
            normalize_origin("HTTPS://Hub.Example:8788/").as_deref(),
            Some("https://hub.example:8788")
        );
        for bad in [
            "hub.example",
            "ftp://hub.example",
            "https://hub.example/x",
            "https://",
            "null",
        ] {
            assert_eq!(normalize_origin(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_header_counts_only_for_the_hubs_own_name_and_page() {
        let own = own();
        // No page (curl, a native client) to the hub's own name.
        assert!(header_context_ok(Some("hub.example:8788"), None, &own));
        assert!(header_context_ok(Some("HUB.example:8787"), None, &own));
        // The hub's own page.
        assert!(header_context_ok(
            Some("hub.example:8788"),
            Some("https://hub.example:8788"),
            &own
        ));
        // Any other page.
        assert!(!header_context_ok(
            Some("hub.example:8788"),
            Some("https://evil.example"),
            &own
        ));
        assert!(!header_context_ok(
            Some("hub.example:8788"),
            Some("null"),
            &own
        ));
        // A page at another name that resolves to the hub (DNS rebinding).
        assert!(!header_context_ok(Some("evil.example:8790"), None, &own));
        assert!(!header_context_ok(Some("127.0.0.1:8790"), None, &own));
        // No Host, or no configured origins: never.
        assert!(!header_context_ok(None, None, &own));
        assert!(!header_context_ok(Some("hub.example:8788"), None, &[]));
    }
}
