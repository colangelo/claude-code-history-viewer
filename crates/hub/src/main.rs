//! Hub binary entry point.

use tracing_subscriber::EnvFilter;

const USAGE: &str = "\
cchv-hub — archive hub

USAGE:
    hub                                  serve (default)
    hub migrate                          apply pending migrations, then exit
    hub backfill-analytics [--batch N]   derive analytics fields over stored messages
    hub mirror rebuild                   rebuild the statistics mirror and swap it in
                                         (required after backfill-analytics)
    hub findings dry-run [--since 7d|<RFC3339>] [--rule ID]... [--expect ROW|SESSION:UUID]... [--batch N]
                                         scan stored messages for credential shapes,
                                         read-only; prints locations and shapes, never values
";

/// Every value of a repeatable `--flag V` / `--flag=V`.
fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix(flag) {
            if let Some(v) = v.strip_prefix('=') {
                out.push(v.to_string());
            } else if v.is_empty() {
                if let Some(v) = it.next() {
                    out.push(v.clone());
                }
            }
        }
    }
    out
}

fn usage_error(msg: &str) -> ! {
    eprintln!("{msg}\n\n{USAGE}");
    std::process::exit(2);
}

fn dry_run_options(args: &[String]) -> hub::findings::DryRunOptions {
    use chrono::{DateTime, Utc};
    let since = flag_values(args, "--since").pop().map(|s| {
        DateTime::parse_from_rfc3339(&s)
            .map(|t| t.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                hub::findings::parse_since(&s)
                    .map(|secs| Utc::now() - chrono::Duration::seconds(secs))
            })
            .unwrap_or_else(|| {
                usage_error(&format!(
                    "bad --since {s:?}: use e.g. 7d or an RFC 3339 time"
                ))
            })
    });
    let rules = flag_values(args, "--rule")
        .iter()
        .map(|r| {
            hub::redact::Rule::from_id(r)
                .unwrap_or_else(|| usage_error(&format!("unknown --rule {r:?}")))
        })
        .collect();
    let expect = flag_values(args, "--expect")
        .iter()
        .map(|e| {
            hub::findings::Expect::parse(e).unwrap_or_else(|| {
                usage_error(&format!("bad --expect {e:?}: a row id or SESSION:UUID"))
            })
        })
        .collect();
    hub::findings::DryRunOptions {
        since,
        rules,
        expect,
        batch: flag_i64(args, "--batch").unwrap_or(1000),
    }
}

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
        Some("mirror") => match args.get(1).map(String::as_str) {
            Some("rebuild") => hub::run_mirror_rebuild().await,
            other => {
                eprintln!(
                    "unknown mirror subcommand: {}\n\n{USAGE}",
                    other.unwrap_or("(none)")
                );
                std::process::exit(2);
            }
        },
        Some("findings") => match args.get(1).map(String::as_str) {
            Some("dry-run") => hub::run_findings_dry_run(dry_run_options(&args[2..])).await,
            other => usage_error(&format!(
                "unknown findings subcommand: {}",
                other.unwrap_or("(none)")
            )),
        },
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
