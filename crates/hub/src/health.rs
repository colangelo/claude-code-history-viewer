//! `GET /v1/healthz` — unauthenticated liveness + database connectivity check.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::state::AppState;

pub async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ok", "db": "up" }))),
        Err(e) => {
            tracing::error!(error = %e, "healthz db check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "degraded", "db": "down" })),
            )
        }
    }
}
