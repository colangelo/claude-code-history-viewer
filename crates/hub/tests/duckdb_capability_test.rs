//! Which SQL the *bundled* `DuckDB` actually provides — change
//! `hub-stats-duckdb-mirror`, design D7.
//!
//! This test exists because a probe run against a developer's machine produced
//! a wrong design decision. The `duckdb` CLI, and any process whose
//! `~/.duckdb/extensions` happens to hold a cached extension, silently autoloads
//! things the statically bundled library does not have. SQL verified that way
//! can still fail inside the hub with a bare `Binder Error`. `AT TIME ZONE` is
//! exactly that: it was recorded as working "in core", and it is not.
//!
//! So **every probe here runs against an empty extension directory with
//! autoload and autoinstall switched off**. That is the deploy reality — a hub
//! binary on a machine with no `DuckDB` extension cache and no reason to have
//! one — and it makes the result independent of whoever runs the test.
//!
//! Needs no database.

use duckdb::Connection;

/// A connection that cannot reach for an extension, however convenient one
/// would be. Without this the test passes or fails depending on what the
/// developer happened to run earlier.
fn sealed() -> Connection {
    let dir = std::env::temp_dir().join(format!(
        "cchv-duckdb-sealed-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("create empty extension dir");
    let conn = Connection::open_in_memory().expect("open in-memory duckdb");
    conn.execute_batch(&format!(
        "SET extension_directory='{}';
         SET autoinstall_known_extensions=false;
         SET autoload_known_extensions=false;",
        dir.display()
    ))
    .expect("seal the connection off from extensions");
    conn
}

fn probe(sql: &str) -> Result<String, String> {
    sealed()
        .query_row(&format!("SELECT ({sql})::VARCHAR"), [], |r| {
            r.get::<_, Option<String>>(0)
        })
        .map(|v| v.unwrap_or_else(|| "NULL".into()))
        .map_err(|e| e.to_string())
}

fn expect(sql: &str, want: &str) {
    match probe(sql) {
        Ok(got) => assert_eq!(got, want, "`{sql}` produced the wrong value"),
        Err(e) => panic!("`{sql}` is not available in the bundled DuckDB build: {e}"),
    }
}

/// **The finding that redirected this design.** `AT TIME ZONE` needs `icu` for
/// every zone, UTC included, and `icu` is neither bundled nor statically
/// linkable from the published crate. Asserted as a *failure* so that if a
/// future `DuckDB` brings it into core, this test says so loudly and D7 can be
/// revisited deliberately rather than by accident.
#[test]
fn at_time_zone_is_not_available_which_is_why_offsets_are_computed_in_rust() {
    for sql in [
        "CAST(TIMESTAMPTZ '2026-01-15 10:00:00+00' AT TIME ZONE 'Europe/Rome' AS VARCHAR)",
        "CAST(TIMESTAMPTZ '2026-01-15 10:00:00+00' AT TIME ZONE 'UTC' AS VARCHAR)",
    ] {
        assert!(
            probe(sql).is_err(),
            "`{sql}` succeeded — AT TIME ZONE is now usable without an extension. \
             D7 chose Rust-side offsets because it was not; revisit that with a \
             measurement, do not just start using this."
        );
    }
}

/// Likewise the extension itself, which must never be loaded: it is absent from
/// the bundled build, so a `LOAD` would try to fetch it over the network at
/// runtime on a service that has to start without one.
#[test]
fn the_icu_extension_cannot_be_loaded() {
    assert!(
        sealed().execute_batch("LOAD icu").is_err(),
        "icu loaded from a sealed connection — the seal is not working, and \
         every other assertion in this file is therefore untrustworthy"
    );
}

/// The substitute for `AT TIME ZONE`: shift a UTC timestamp by an offset the
/// Rust side computed. This is the arithmetic the whole design rests on.
#[test]
fn offset_shifting_works_without_any_extension() {
    // +02:00 — Rome in July.
    expect(
        "CAST(TIMESTAMP '2026-07-15 10:00:00' + to_seconds(7200) AS VARCHAR)",
        "2026-07-15 12:00:00",
    );
    // +05:30 — a half-hour zone, which whole-hour re-bucketing cannot express.
    expect(
        "CAST(TIMESTAMP '2026-07-15 10:00:00' + to_seconds(19800) AS VARCHAR)",
        "2026-07-15 15:30:00",
    );
    // Negative offsets cross the day boundary backwards.
    expect(
        "CAST(CAST(TIMESTAMP '2026-07-15 02:00:00' + to_seconds(-18000) AS DATE) AS VARCHAR)",
        "2026-07-14",
    );
}

#[test]
fn timestamp_formatting_is_available_without_an_extension() {
    // The exact shape `DateRange` carries. `strftime` exists in the sealed
    // build but is shadowed once icu loads, so the rollups use this instead —
    // it depends only on the canonical timestamp cast.
    expect(
        "replace(substr(CAST(TIMESTAMP '2026-01-15 10:20:30' AS VARCHAR), 1, 19), ' ', 'T') || 'Z'",
        "2026-01-15T10:20:30Z",
    );
    // And it must stay correct when the value carries sub-second precision,
    // which real ingested timestamps do.
    expect(
        "replace(substr(CAST(TIMESTAMP '2026-01-15 10:20:30.123456' AS VARCHAR), 1, 19), ' ', 'T') || 'Z'",
        "2026-01-15T10:20:30Z",
    );
    // Day bucket, the `daily` rollup's group key.
    expect(
        "CAST(CAST(TIMESTAMP '2026-01-15 23:20:30' AS DATE) AS VARCHAR)",
        "2026-01-15",
    );
}

#[test]
fn date_part_extraction_matches_postgres() {
    // Sunday is 0 in both engines — the heatmap's day axis depends on it.
    expect("extract(dow FROM TIMESTAMP '2026-07-19 12:00:00')", "0");
    expect("extract(dow FROM TIMESTAMP '2026-07-20 12:00:00')", "1");
    expect("extract(hour FROM TIMESTAMP '2026-07-19 13:00:00')", "13");
    // `days_span` in the date range.
    expect(
        "date_part('day', TIMESTAMP '2026-07-25 01:00:00' - TIMESTAMP '2026-01-01 00:00:00')",
        "205",
    );
    // Gap in seconds, for the active-session-time idle cap.
    expect(
        "EXTRACT(EPOCH FROM (TIMESTAMP '2026-01-01 00:30:00' - TIMESTAMP '2026-01-01 00:00:00'))",
        "1800.0",
    );
}

#[test]
fn aggregate_and_window_features_are_available() {
    let conn = sealed();
    conn.execute_batch(
        "CREATE TABLE t (g INT, v BIGINT, e BOOLEAN);
         INSERT INTO t VALUES (1, 10, true), (1, 20, false), (2, 30, NULL);",
    )
    .expect("seed");

    // FILTER, bool_or, and the BIGINT cast that stops sum() widening to HUGEINT.
    let (filtered, any_err): (i64, bool) = conn
        .query_row(
            "SELECT coalesce(sum(v) FILTER (WHERE g = 1), 0)::BIGINT,
                    coalesce(bool_or(e), false) FROM t",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("filter/bool_or");
    assert_eq!(filtered, 30);
    assert!(any_err);

    // lag() over a partition — the active-session-time gap calculation.
    let lagged: i64 = conn
        .query_row(
            "SELECT coalesce(sum(d), 0)::BIGINT FROM
               (SELECT v - lag(v) OVER (PARTITION BY g ORDER BY v) AS d FROM t)",
            [],
            |r| r.get(0),
        )
        .expect("lag");
    assert_eq!(lagged, 10);

    // CROSS JOIN LATERAL (VALUES …) — the tool/skill/subagent fan-out.
    let fanned: i64 = conn
        .query_row(
            "SELECT count(*)::BIGINT FROM t
               CROSS JOIN LATERAL (VALUES ('a', t.v), ('b', t.v)) AS k(kind, name)",
            [],
            |r| r.get(0),
        )
        .expect("lateral values");
    assert_eq!(fanned, 6);
}

/// The shipped SQL must not contain the two constructs that only work by
/// accident on a machine with a warm extension cache. Grepping the source is
/// crude, but it is the only check that catches a *new* query added later by
/// someone who tested it in the CLI and found it fine.
///
/// **Which files get grepped is decided by content, not by a hardcoded list.**
/// The list version broke the moment `stats_duck.rs` was renamed to `stats.rs`
/// (task 4.6) — it failed loudly, which was lucky; the same edit could as easily
/// have left it grepping a file that no longer held the rollups. So: every source
/// file that mentions `duckdb` is checked, which picks up a new module for free.
/// Postgres-only modules are skipped deliberately — `journal.rs` and `health.rs`
/// use `AT TIME ZONE` legitimately, because in Postgres it needs no extension.
#[test]
fn shipped_sql_contains_no_extension_dependent_constructs() {
    let src_dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut checked: Vec<String> = Vec::new();
    let mut scanned_the_rollups = false;

    for entry in std::fs::read_dir(&src_dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        if !src.contains("duckdb") {
            continue;
        }
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        // The scope materialization is the heart of the rollups; seeing it proves
        // the grep is looking at real query text and not just at type imports.
        scanned_the_rollups |= src.contains("stats_scope");
        checked.push(file.clone());

        for (line_no, line) in src.lines().enumerate() {
            // Prose about the constraint is fine; SQL that uses it is not.
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") {
                continue;
            }
            for banned in ["AT TIME ZONE", "LOAD icu", "strftime("] {
                assert!(
                    !line.contains(banned),
                    "{file}:{} uses `{banned}`, which needs the icu extension \
                     (or is shadowed by it). See design D7.",
                    line_no + 1
                );
            }
        }
    }

    // A grep that silently matched nothing would pass forever. These two say the
    // scan actually reached the DuckDB code.
    assert!(
        checked.len() >= 2,
        "expected the mirror and the rollups to be scanned, found {checked:?}"
    );
    assert!(
        scanned_the_rollups,
        "no scanned file builds `stats_scope` — the rollup SQL was not checked \
         (moved to a file that does not mention duckdb?). Scanned: {checked:?}"
    );
}
