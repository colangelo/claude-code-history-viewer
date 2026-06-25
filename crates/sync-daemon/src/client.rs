//! The hub client. A trait so the sync logic can be tested against a mock; the
//! real implementation POSTs to `/v1/ingest` with at-least-once retry/backoff.

use std::time::Duration;

use archive_protocol::{IngestBatch, IngestResponse};

/// Delivers ingest batches to the hub.
#[allow(async_fn_in_trait)] // internal trait, only ever used with concrete types
pub trait HubClient {
    async fn ingest(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse>;
}

pub struct ReqwestHubClient {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    max_retries: u32,
}

impl ReqwestHubClient {
    pub fn new(hub_url: &str, token: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: format!("{}/v1/ingest", hub_url.trim_end_matches('/')),
            token: token.to_string(),
            max_retries: 5,
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    // 0.2s, 0.4s, 0.8s, … capped at 10s.
    let secs = 0.2 * 2f64.powi(i32::try_from(attempt.min(6)).unwrap_or(6));
    Duration::from_secs_f64(secs.min(10.0))
}

impl HubClient for ReqwestHubClient {
    async fn ingest(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let result = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.token)
                .json(batch)
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    return Ok(resp.json::<IngestResponse>().await?);
                }
                // 4xx is a permanent error (bad request / auth) — do not retry.
                Ok(resp) if resp.status().is_client_error() => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("hub rejected batch ({status}): {body}");
                }
                // 5xx or transport error — retry with backoff.
                Ok(resp) => {
                    if attempt > self.max_retries {
                        anyhow::bail!("hub error {} after {attempt} attempts", resp.status());
                    }
                    tracing::warn!(status = %resp.status(), attempt, "ingest retry");
                }
                Err(e) => {
                    if attempt > self.max_retries {
                        return Err(anyhow::anyhow!(e).context("ingest failed after retries"));
                    }
                    tracing::warn!(error = %e, attempt, "ingest retry");
                }
            }
            tokio::time::sleep(backoff(attempt)).await;
        }
    }
}
