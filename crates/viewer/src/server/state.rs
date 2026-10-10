//! Shared application state for the Axum web server.
//!
//! This state is shared between all Axum request handlers.

use crate::commands::metadata::MetadataState;
use crate::commands::watcher::FileWatchEvent;
use crate::server::auth::AuthState;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast;

/// Shared state accessible by all Axum route handlers.
#[derive(Clone)]
pub struct AppState {
    /// Metadata state (wrapped in Arc for Axum Clone requirement)
    pub metadata: Arc<MetadataState>,
    /// Server start time for uptime calculation.
    pub start_time: Instant,
    /// `WebUI` authentication mode.
    pub auth: AuthState,
    /// Whether mutating `WebUI` API endpoints should be rejected.
    pub read_only: bool,
    /// Broadcast channel for file-change events (SSE consumers subscribe here).
    pub event_tx: broadcast::Sender<FileWatchEvent>,
}
