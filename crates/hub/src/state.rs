//! Shared application state: the Postgres pool, the token → machine-id map,
//! and the (optional) sentence embedder for semantic journal search.

use std::collections::HashMap;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::embed::Embedder;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Bearer token → machine id. `Arc` so the state stays cheap to clone per request.
    pub tokens: Arc<HashMap<String, Uuid>>,
    /// Tailscale logins granted READ scope via the `Tailscale-User-Login`
    /// header (see `HubConfig::trust_tailscale_identity`). Empty = disabled.
    pub trusted_identities: Arc<Vec<String>>,
    /// Sentence embedder for semantic journal search. `None` (unconfigured)
    /// and a configured-but-failed load both degrade queries to keyword.
    pub embedder: Option<Arc<dyn Embedder>>,
    /// Wakes the embedding sweep early (journal writes nudge it so fresh
    /// entries become semantically searchable without waiting an interval).
    /// Always present; notifying with no sweep listening is a no-op.
    pub embed_nudge: Arc<Notify>,
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
            embedder: None,
            embed_nudge: Arc::new(Notify::new()),
        }
    }

    /// Attach an embedder (builder-style so existing constructions stay valid).
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }
}
