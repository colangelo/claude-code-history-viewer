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
    hub findings dry-run [--since 7d|<RFC3339>] [--rule ID]... [--expect ROW|SESSION:UUID]... [--batch N] [--fingerprint-key-file PATH]
                                         scan stored messages for credential shapes,
                                         read-only; prints locations and shapes, never values
    hub findings journal-scan [--rule ID]...
                                         the same detector over the journal entries, read-only;
                                         prints entry, date, model and rule counts, never text
    hub findings redact --rule ID... --max-rows N [--since ...] [--expect ...]... [--batch N] [--no-redirty]
                                         the REAL run: rewrites exactly the rows the same dry
                                         run lists for the named rules; refuses before writing
                                         on a missed --expect or more than N rows; --no-redirty
                                         leaves ingest_xid alone, so no journal day re-distills
    hub findings ack (--message ROW|SESSION:UUID)... [--detected-between FROM,TO] [--rule ID]...
                     --as false_positive|handled --by WHO --note TEXT --max-findings N
                                         acknowledge triaged findings: they stop paging and are
                                         reported as acked_by_rule; refuses before writing on an
                                         empty selection, more than N findings, or a note the
                                         detector flags. FROM and TO are RFC 3339 times
    hub client-scope accept-routing --approved-by WHO
                                         accept the routing file's marker set as it is now,
                                         removals included (a narrowing); audited
    hub client-scope clear --session N --reason TEXT --approved-by WHO
                                         clear one session's client class (a narrowing); audited
    hub client-scope unlatch --reason TEXT --approved-by WHO
                                         lift the enforcement latch (emergency); audited
    hub client-scope protect --session N --reason TEXT --approved-by WHO
    hub client-scope restamp-history --from FILE --dry-run | --approved-by WHO
                                         make pre-routing journal entries lab again where their
                                         text and sessions match nothing under the full router
                                         (markers and terms); the write is audited
    hub client-scope sweep [--dry-run] [--batch N] [--rescan]
                                         classify every session not scanned under the routing
                                         generation the hub accepted; raises classes only and
                                         stamps a session only if no ingest touched it meanwhile;
                                         --dry-run is read-only and prints counts only;
                                         --rescan also reads sessions already stamped lab
                                         (run once after a release that changes the extractor)
    hub client-scope instruction-entry --git REPO REV:PATH
                                         print the routing-file entries for PATH at REV of the git
                                         repo REPO: as Claude Code injects it (trailing whitespace
                                         removed) and in its search_text form (whitespace
                                         collapsed); refuses a file whose edges could carry part
                                         of a live marker. accept-routing re-checks each entry
    hub client-scope census --routing FILE --floor FILE [--days N] [--ids]
                                         read-only: re-read the sessions stamped client under a
                                         candidate routing file with the same markers, and count
                                         them by stamped -> candidate class; --ids lists the
                                         session ids that would be lab (ids and classes only)
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
        fingerprint_key: flag_values(args, "--fingerprint-key-file")
            .pop()
            .map(|path| {
                let hex = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| usage_error(&format!("reading {path}: {e}")));
                let hex = hex.trim();
                if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    usage_error(&format!(
                        "{path}: expected 64 hex characters (a 32-byte key)"
                    ));
                }
                (0..32)
                    .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("checked hex"))
                    .collect()
            }),
    }
}

fn ack_options(args: &[String]) -> hub::findings::AckOptions {
    use chrono::{DateTime, Utc};
    let one = |flag: &str| flag_values(args, flag).pop();
    let time = |s: &str| {
        DateTime::parse_from_rfc3339(s.trim())
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| usage_error(&format!("bad time {s:?}: use RFC 3339")))
    };
    let detected = one("--detected-between").map(|range| {
        let (from, to) = range
            .split_once(',')
            .unwrap_or_else(|| usage_error("--detected-between needs FROM,TO"));
        (time(from), time(to))
    });
    let scan = dry_run_options(args);
    let messages = flag_values(args, "--message")
        .iter()
        .map(|e| {
            hub::findings::Expect::parse(e).unwrap_or_else(|| {
                usage_error(&format!("bad --message {e:?}: a row id or SESSION:UUID"))
            })
        })
        .collect();
    hub::findings::AckOptions {
        messages,
        detected,
        rules: scan.rules,
        classification: one("--as")
            .and_then(|c| hub::findings::Classification::from_id(&c))
            .unwrap_or_else(|| usage_error("findings ack needs --as false_positive|handled")),
        by: one("--by").unwrap_or_else(|| usage_error("findings ack needs --by WHO")),
        note: one("--note").unwrap_or_else(|| usage_error("findings ack needs --note TEXT")),
        max_findings: flag_i64(args, "--max-findings")
            .unwrap_or_else(|| usage_error("findings ack needs --max-findings N")),
    }
}

/// `--approved-by WHO`, required by every audited client-scope action.
fn approver(args: &[String]) -> String {
    flag_values(args, "--approved-by")
        .pop()
        .filter(|w| !w.trim().is_empty())
        .unwrap_or_else(|| usage_error("this client-scope action needs --approved-by WHO"))
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
            Some("journal-scan") => {
                hub::run_findings_journal_scan(dry_run_options(&args[2..]).rules).await
            }
            Some("redact") => {
                let scan = dry_run_options(&args[2..]);
                if scan.rules.is_empty() {
                    usage_error("findings redact needs at least one --rule");
                }
                let max_rows = flag_i64(&args[2..], "--max-rows").unwrap_or_else(|| {
                    usage_error(
                        "findings redact needs --max-rows N (the row count of your dry run)",
                    )
                });
                let keep_ingest_xid = args[2..].iter().any(|a| a == "--no-redirty");
                hub::run_findings_redact(hub::findings::RedactOptions {
                    scan,
                    max_rows,
                    keep_ingest_xid,
                })
                .await
            }
            Some("ack") => hub::run_findings_ack(ack_options(&args[2..])).await,
            other => usage_error(&format!(
                "unknown findings subcommand: {}",
                other.unwrap_or("(none)")
            )),
        },
        Some("client-scope") => match args.get(1).map(String::as_str) {
            Some("accept-routing") => {
                hub::run_client_scope_admin(hub::ClientScopeAdmin::AcceptRouting {
                    approved_by: approver(&args[2..]),
                })
                .await
            }
            Some("clear") => {
                hub::run_client_scope_admin(hub::ClientScopeAdmin::Clear {
                    session: flag_i64(&args[2..], "--session")
                        .unwrap_or_else(|| usage_error("client-scope clear needs --session N")),
                    reason: flag_values(&args[2..], "--reason")
                        .pop()
                        .unwrap_or_else(|| usage_error("client-scope clear needs --reason TEXT")),
                    approved_by: approver(&args[2..]),
                })
                .await
            }
            Some("protect") => {
                hub::run_client_scope_admin(hub::ClientScopeAdmin::Protect {
                    session: flag_i64(&args[2..], "--session")
                        .unwrap_or_else(|| usage_error("client-scope protect needs --session N")),
                    reason: flag_values(&args[2..], "--reason")
                        .pop()
                        .unwrap_or_else(|| usage_error("client-scope protect needs --reason TEXT")),
                    approved_by: approver(&args[2..]),
                })
                .await
            }
            Some("unlatch") => {
                hub::run_client_scope_admin(hub::ClientScopeAdmin::Unlatch {
                    reason: flag_values(&args[2..], "--reason")
                        .pop()
                        .unwrap_or_else(|| usage_error("client-scope unlatch needs --reason TEXT")),
                    approved_by: approver(&args[2..]),
                })
                .await
            }
            Some("restamp-history") => {
                let dry = args[2..].iter().any(|a| a == "--dry-run");
                let by = flag_values(&args[2..], "--approved-by").pop();
                if dry == by.is_some() {
                    usage_error("client-scope restamp-history needs exactly one of --dry-run or --approved-by WHO");
                }
                // The classify step's file (`cchv-distill --restamp-history FILE`):
                // the hub no longer decides on its own (design R3-E8).
                let from = flag_values(&args[2..], "--from").pop().unwrap_or_else(|| {
                    usage_error("client-scope restamp-history needs --from FILE (from cchv-distill --restamp-history)")
                });
                hub::run_client_scope_admin(hub::ClientScopeAdmin::Restamp {
                    from: from.into(),
                    approved_by: by,
                })
                .await
            }
            Some("instruction-entry") => {
                let rest = &args[2..];
                match rest {
                    [flag, repo, rev_path] if flag == "--git" => {
                        hub::run_instruction_entry(repo, rev_path)
                    }
                    _ => usage_error("client-scope instruction-entry needs --git REPO REV:PATH"),
                }
            }
            Some("census") => {
                let need = |flag: &str| {
                    flag_values(&args[2..], flag).pop().unwrap_or_else(|| {
                        usage_error(&format!("client-scope census needs {flag} FILE"))
                    })
                };
                let days = flag_i64(&args[2..], "--days").map(|d| {
                    i32::try_from(d).unwrap_or_else(|_| usage_error("--days is too large"))
                });
                hub::run_client_scope_census(
                    need("--routing").into(),
                    need("--floor").into(),
                    days.or(Some(31)),
                    args[2..].iter().any(|a| a == "--ids"),
                )
                .await
            }
            Some("sweep") => {
                let dry_run = args[2..].iter().any(|a| a == "--dry-run");
                let batch = flag_i64(&args[2..], "--batch").unwrap_or(200);
                let rescan = args[2..].iter().any(|a| a == "--rescan");
                hub::run_client_scope_sweep(dry_run, batch, rescan).await
            }
            other => usage_error(&format!(
                "unknown client-scope subcommand: {}",
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
