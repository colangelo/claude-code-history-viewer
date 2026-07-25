//! Hub binary entry point.

use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
cchv-hub — archive hub

USAGE:
    hub                                  serve (default)
    hub migrate                          apply pending migrations, then exit
    hub backfill-analytics [--batch N]   derive analytics fields over stored messages
";

/// `--flag N` / `--flag=N`, when present and parseable.
fn flag_i64(args: &[String], flag: &str) -> Option<i64> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix(flag) {
            if let Some(v) = v.strip_prefix('=') {
                return v.parse().ok();
            }
            if v.is_empty() {
                return it.next().and_then(|n| n.parse().ok());
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => hub::run().await,
        Some("migrate") => hub::run_migrate().await,
        Some("backfill-analytics") => {
            let batch = flag_i64(&args, "--batch").unwrap_or(hub::backfill::DEFAULT_BATCH);
            hub::run_backfill(batch).await
        }
        Some("-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown subcommand: {other}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}
