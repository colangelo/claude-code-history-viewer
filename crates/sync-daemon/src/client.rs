//! The hub client. A trait so the sync logic can be tested against a mock; the
//! real implementation POSTs to `/v1/ingest` with at-least-once retry/backoff.

use std::time::Duration;

use archive_protocol::{IngestBatch, IngestResponse};

/// Delivers ingest batches to the hub.
#[allow(async_fn_in_trait)] // internal trait, only ever used with concrete types
pub trait HubClient {
    async fn ingest(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse>;
}

/// Base per-request timeout (overridable via `CCHV_INGEST_TIMEOUT_SECS`) —
/// without a timeout, a request straddling e.g. a laptop sleep cycle can block
/// `send()` forever.
///
/// This was 30 s, and 30 s was simply *wrong*: measured over 313 completed m4m
/// ingests the hub's own latency runs p50 4.6 s / p90 17.8 s / max **39.7 s**,
/// so the old budget sat inside the tail of the distribution. Every session
/// whose batch landed above the line failed deterministically, on every pass,
/// forever — the ~42-session permanent backlog of the 2026-07-19 m4m
/// retry-backlog report. 180 s is ~4.5x the observed worst case; the headroom
/// is deliberate, since hub latency rides a pg1 round trip and drifts with
/// archive size.
const DEFAULT_INGEST_TIMEOUT_SECS: u64 = 180;

/// Extra timeout granted per MiB of serialized payload, on top of the base.
/// Hub ingest latency scales with batch size — the session that pinned the bug
/// carried 434 messages in an 8.4 MB payload — so the budget scales too rather
/// than betting one flat number covers both a 100 KiB batch and one carrying a
/// 40 MiB tool result.
const TIMEOUT_SECS_PER_MIB: u64 = 10;

/// Ceiling on the derived timeout, chosen to stay under the per-batch deadline
/// in `sync::ingest_with_deadline` (600 s), which is the real outer arbiter.
const MAX_INGEST_TIMEOUT_SECS: u64 = 420;

/// Timeout budget for one request: `base + 10 s/MiB`, capped at
/// `MAX_INGEST_TIMEOUT_SECS` — but never below `base`, so an operator who
/// raises `CCHV_INGEST_TIMEOUT_SECS` past the cap gets what they asked for.
fn request_timeout(base: Duration, payload_bytes: usize) -> Duration {
    let mib = u64::try_from(payload_bytes / (1024 * 1024)).unwrap_or(u64::MAX);
    let secs = base
        .as_secs()
        .saturating_add(mib.saturating_mul(TIMEOUT_SECS_PER_MIB))
        .min(MAX_INGEST_TIMEOUT_SECS.max(base.as_secs()));
    Duration::from_secs(secs)
}

/// One-word classification of a transport error, so the log says *why* a send
/// failed instead of only that it did.
fn error_kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else if e.is_redirect() {
        "redirect"
    } else if e.is_request() {
        "request"
    } else {
        "other"
    }
}

/// Flatten the `source` chain. `reqwest::Error`'s `Display` prints only the top
/// frame ("error sending request for url (…)"), which is exactly the string
/// that made 16k log lines unactionable — the real cause (timed out / connection
/// refused / os error N) lives one or two `source()` hops down.
fn error_chain(e: &reqwest::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut source = std::error::Error::source(e);
    while let Some(err) = source {
        parts.push(err.to_string());
        source = err.source();
    }
    parts.join(": ")
}

/// Read a positive-integer-seconds env var, falling back to `default_secs`
/// when unset or invalid.
fn env_duration_secs(var: &str, default_secs: u64) -> Duration {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(default_secs))
}

pub struct ReqwestHubClient {
    client: reqwest::Client,
    endpoint: String,
    token: String,
    max_retries: u32,
    base_timeout: Duration,
}

impl ReqwestHubClient {
    pub fn new(hub_url: &str, token: &str) -> Self {
        let base_timeout =
            env_duration_secs("CCHV_INGEST_TIMEOUT_SECS", DEFAULT_INGEST_TIMEOUT_SECS);
        Self {
            // The real budget is set per request (`request_timeout`, which scales
            // with payload size); this is the floor for anything that doesn't.
            client: reqwest::Client::builder()
                .timeout(base_timeout)
                .build()
                .expect("building reqwest client with a timeout must not fail"),
            endpoint: format!("{}/v1/ingest", hub_url.trim_end_matches('/')),
            token: token.to_string(),
            max_retries: 5,
            base_timeout,
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
        // Named on every retry warning so a session that eventually fails can be
        // traced back through the retries that preceded it.
        let sid = batch
            .sessions
            .first()
            .map(|s| s.session_id.as_str())
            .unwrap_or("");
        let messages = batch.messages.len();
        // Serialize once. `.json(batch)` re-serialized the whole batch on every
        // attempt — up to 6x the work for a batch that was failing precisely
        // because it was big. The byte count also feeds the timeout budget and
        // names the payload in every retry warning.
        let body = serde_json::to_vec(batch)?;
        let payload_bytes = body.len();
        let timeout = request_timeout(self.base_timeout, payload_bytes);
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let result = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .timeout(timeout)
                .body(body.clone())
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
                    anyhow::bail!("hub rejected batch ({status}) [session {sid}]: {body}");
                }
                // 5xx or transport error — retry with backoff.
                Ok(resp) => {
                    if attempt > self.max_retries {
                        anyhow::bail!(
                            "hub error {} after {attempt} attempts [session {sid}]",
                            resp.status()
                        );
                    }
                    tracing::warn!(status = %resp.status(), attempt, session_id = %sid, messages, payload_bytes, "ingest retry");
                }
                Err(e) => {
                    let kind = error_kind(&e);
                    let cause = error_chain(&e);
                    if attempt > self.max_retries {
                        return Err(anyhow::anyhow!(e).context(format!(
                            "ingest failed after {attempt} attempts ({kind}: {cause}) \
                             [session {sid}, {messages} messages, {payload_bytes} bytes, \
                             {timeout:?} per-attempt timeout]"
                        )));
                    }
                    tracing::warn!(
                        error = %e,
                        kind,
                        cause,
                        attempt,
                        session_id = %sid,
                        messages,
                        payload_bytes,
                        timeout_secs = timeout.as_secs(),
                        "ingest retry"
                    );
                }
            }
            tokio::time::sleep(backoff(attempt)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1024 * 1024;

    #[test]
    fn timeout_is_the_base_for_small_payloads() {
        let base = Duration::from_secs(30);
        assert_eq!(request_timeout(base, 0), base);
        assert_eq!(request_timeout(base, 1000), base);
    }

    #[test]
    fn timeout_scales_with_payload_size() {
        let base = Duration::from_secs(30);
        assert_eq!(request_timeout(base, 8 * MIB), Duration::from_secs(110));
    }

    #[test]
    fn timeout_is_capped() {
        let base = Duration::from_secs(30);
        assert_eq!(
            request_timeout(base, 500 * MIB),
            Duration::from_secs(MAX_INGEST_TIMEOUT_SECS)
        );
    }

    #[test]
    fn a_base_above_the_cap_is_never_clamped_down() {
        // An operator who sets CCHV_INGEST_TIMEOUT_SECS high must not have it
        // silently clamped below what they asked for.
        let base = Duration::from_secs(600);
        assert_eq!(request_timeout(base, 0), base);
        assert_eq!(request_timeout(base, 8 * MIB), base);
    }
}
