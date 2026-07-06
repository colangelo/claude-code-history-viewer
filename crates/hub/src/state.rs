//! Shared application state: the Postgres pool and the token → machine-id map.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Bearer token → machine id. `Arc` so the state stays cheap to clone per request.
    pub tokens: Arc<HashMap<String, Uuid>>,
    /// Tailscale logins granted READ scope via the `Tailscale-User-Login`
    /// header (see `HubConfig::trust_tailscale_identity`). Empty = disabled.
    pub trusted_identities: Arc<Vec<String>>,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        tokens: HashMap<String, Uuid>,
        trusted_identities: Vec<String>,
    ) -> Self {
        Self {
            pool,
            tokens: Arc::new(tokens),
            trusted_identities: Arc::new(trusted_identities),
        }
    }
}
