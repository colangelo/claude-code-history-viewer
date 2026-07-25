//! Local-time bucketing without a `DuckDB` extension (design D7).
//!
//! The statistics rollups bucket by day and by hour-of-day in the caller's
//! timezone — "what hours do I work" is meaningless in UTC for someone in
//! `Europe/Rome`, and the webapp sends the browser's IANA zone on every
//! request. `DuckDB`'s `AT TIME ZONE` would do this, but it lives in the `icu`
//! extension, which is not in the bundled build and cannot be statically linked
//! from the published crate. So the offset is resolved here, with IANA data
//! compiled into the binary.
//!
//! A single offset per request would be wrong: a window spanning a DST
//! transition has two. What this module produces is the list of half-open UTC
//! spans over which the offset is constant, which `DuckDB` then joins against.
//!
//! Transitions are found by walking the range hourly and coalescing runs of
//! equal offset, rather than by reading a transition table — `chrono-tz` does
//! not expose one. Every IANA transition in the modern era lands on a whole
//! hour, so hourly sampling is exact for any data this archive can hold; the
//! walk is bounded by the archive's own date range and costs microseconds.

use chrono::{DateTime, Duration, NaiveDateTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;

/// A half-open UTC interval `[from, to)` over which the zone's offset from UTC
/// is constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TzSpan {
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,
    pub offset_secs: i32,
}

/// Sentinel bounds, so every row in the mirror falls inside exactly one span
/// however far outside the sampled range its timestamp sits. A row with a
/// corrupt far-future timestamp gets the last known offset, which is a wrong
/// bucket rather than a silently dropped row.
fn floor() -> NaiveDateTime {
    DateTime::from_timestamp(-62_135_596_800, 0)
        .expect("year 1 is representable")
        .naive_utc()
}

fn ceiling() -> NaiveDateTime {
    DateTime::from_timestamp(253_402_300_799, 0)
        .expect("year 9999 is representable")
        .naive_utc()
}

/// The offset spans for `tz` covering `[start, end]`, in ascending order.
///
/// The first span extends back to the floor and the last forward to the
/// ceiling, so the join in `stats_duck` is total and needs no `LEFT JOIN`
/// fallback that could silently null out a timestamp.
pub fn spans(tz: Tz, start: NaiveDateTime, end: NaiveDateTime) -> Vec<TzSpan> {
    let step = Duration::hours(1);
    // Widen by a step on each side: a transition exactly at `start` must be
    // seen as a change, not inherited as the opening offset.
    let mut at = (start - step).max(floor());
    let last = (end + step).min(ceiling());

    let mut out: Vec<TzSpan> = Vec::new();
    loop {
        let offset = offset_secs_at(tz, at);
        match out.last_mut() {
            Some(prev) if prev.offset_secs == offset => prev.to = at,
            _ => out.push(TzSpan {
                from: at,
                to: at,
                offset_secs: offset,
            }),
        }
        if at >= last {
            break;
        }
        at = (at + step).min(last);
    }

    if let Some(first) = out.first_mut() {
        first.from = floor();
    }
    if let Some(final_span) = out.last_mut() {
        final_span.to = ceiling();
    }
    // Each span's `to` was left at the last *sample* that still had its offset;
    // the boundary is the next sample, which is the following span's `from`.
    for i in 0..out.len().saturating_sub(1) {
        out[i].to = out[i + 1].from;
    }
    out
}

/// Seconds east of UTC in `tz` at the given UTC instant.
fn offset_secs_at(tz: Tz, utc: NaiveDateTime) -> i32 {
    tz.from_utc_datetime(&utc).offset().fix().local_minus_utc()
}

/// Parse an IANA name, or `None` if it is not a zone this build knows.
///
/// Callers turn `None` into a `400`. Rejecting up front matters more here than
/// it did against Postgres: an unknown zone used to surface as a database error
/// naming the zone, whereas silently defaulting would produce plausible
/// statistics bucketed in the wrong day.
pub fn parse(name: &str) -> Option<Tz> {
    name.parse::<Tz>().ok()
}

/// UTC, for the default window and for tests.
pub fn utc() -> Tz {
    chrono_tz::UTC
}

/// Convenience for callers that only have a `DateTime<Utc>` range.
pub fn spans_for_range(tz: Tz, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<TzSpan> {
    spans(tz, start.naive_utc(), end.naive_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        DateTime::parse_from_rfc3339(s).unwrap().naive_utc()
    }

    #[test]
    fn utc_is_one_span_with_no_offset() {
        let s = spans(
            utc(),
            dt("2026-01-01T00:00:00Z"),
            dt("2026-12-31T00:00:00Z"),
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].offset_secs, 0);
        assert_eq!(s[0].from, floor());
        assert_eq!(s[0].to, ceiling());
    }

    #[test]
    fn rome_has_two_spans_across_a_year_with_real_dst() {
        let tz = parse("Europe/Rome").expect("known zone");
        let s = spans(tz, dt("2026-01-01T00:00:00Z"), dt("2026-12-31T00:00:00Z"));
        assert_eq!(s.len(), 3, "CET, CEST, CET");
        assert_eq!(s[0].offset_secs, 3600, "January is +01");
        assert_eq!(s[1].offset_secs, 7200, "July is +02");
        assert_eq!(s[2].offset_secs, 3600, "back to +01 after October");
        // 2026 transitions: 29 March and 25 October, both at 01:00 UTC.
        assert_eq!(s[1].from, dt("2026-03-29T01:00:00Z"));
        assert_eq!(s[1].to, dt("2026-10-25T01:00:00Z"));
    }

    #[test]
    fn spans_are_contiguous_and_total() {
        let tz = parse("America/New_York").expect("known zone");
        let s = spans(tz, dt("2025-01-01T00:00:00Z"), dt("2026-12-31T00:00:00Z"));
        assert_eq!(s.first().unwrap().from, floor());
        assert_eq!(s.last().unwrap().to, ceiling());
        for w in s.windows(2) {
            assert_eq!(w[0].to, w[1].from, "no gap and no overlap between spans");
            assert_ne!(
                w[0].offset_secs, w[1].offset_secs,
                "adjacent spans must differ, or they should have been coalesced"
            );
        }
    }

    #[test]
    fn half_hour_zones_resolve_exactly() {
        let tz = parse("Asia/Kolkata").expect("known zone");
        let s = spans(tz, dt("2026-01-01T00:00:00Z"), dt("2026-07-01T00:00:00Z"));
        assert_eq!(s.len(), 1, "no DST");
        assert_eq!(s[0].offset_secs, 19_800, "+05:30");
    }

    #[test]
    fn a_range_shorter_than_one_step_still_yields_a_span() {
        let tz = parse("Europe/Rome").unwrap();
        let s = spans(tz, dt("2026-07-01T10:00:00Z"), dt("2026-07-01T10:00:00Z"));
        assert!(!s.is_empty());
        assert_eq!(s[0].offset_secs, 7200);
    }

    #[test]
    fn unknown_zones_are_rejected_rather_than_defaulted() {
        assert!(parse("Mars/Olympus_Mons").is_none());
        assert!(parse("").is_none());
        assert!(parse("Europe/Rome").is_some());
    }
}
