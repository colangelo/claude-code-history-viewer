//! Sync daemon binary entry point.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut once = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--once" => once = true,
            other => anyhow::bail!("unknown argument: {other} (supported: --once)"),
        }
    }

    if once {
        sync_daemon::run_once().await
    } else {
        sync_daemon::run().await
    }
}
