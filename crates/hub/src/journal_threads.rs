//! Journal open-question threads (Gitea #15,
//! `openspec/changes/journal-open-questions-report`).
//!
//! A thread is the set of open questions that carry the same thread id. The
//! distiller decides the links (it has the day's work in context); the hub
//! assigns new ids, refuses ids that are not this project's, and folds the
//! entries back into threads for `GET /v1/journal/open-questions`.
//!
//! * [`resolve_links`] — validation and id assignment for the entry POST.
//! * [`fold_threads`] — pure: entries → threads with a state.
//! * [`open_questions`] — the read endpoint (read auth).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Query, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::state::AppState;

/// A thread not mentioned within a project's last `QUIET_AFTER_ACTIVE_DAYS`
/// active days (days with an `entry` row) is `quiet`. Active days, not calendar
/// days, so a holiday does not age every thread at once.
pub const QUIET_AFTER_ACTIVE_DAYS: usize = 3;

const DEFAULT_DAYS: i64 = 30;
const MAX_DAYS: i64 = 3650;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

// ---------------------------------------------------------------------------
// POST side: validate links and assign ids
// ---------------------------------------------------------------------------

/// Validate an entry's thread links and resolutions and return them ready to
/// store: one id per open question (fresh ids for new threads) and the
/// deduplicated resolved ids.
///
/// Rejected with `400`, before anything is written: a links array whose length
/// differs from `open_questions`, an id both continued and resolved, and any id
/// that is not an open question of ANOTHER entry of the same project (this
/// entry's own row is about to be replaced, so its ids do not count).
pub async fn resolve_links(
    pool: &PgPool,
    entry_date: NaiveDate,
    project_path: &str,
    open_questions: &[String],
    links: Option<&[Option<i64>]>,
    resolved: &[i64],
) -> Result<(Vec<i64>, Vec<i64>), HubError> {
    let n = open_questions.len();
    let links: Vec<Option<i64>> = match links {
        Some(l) if l.len() != n => {
            return Err(HubError::BadRequest(format!(
                "open_question_threads has {} items but open_questions has {n}",
                l.len()
            )))
        }
        Some(l) => l.to_vec(),
        None => vec![None; n],
    };
    let resolved: BTreeSet<i64> = resolved.iter().copied().collect();
    let continued: BTreeSet<i64> = links.iter().flatten().copied().collect();
    if let Some(id) = continued.intersection(&resolved).next() {
        return Err(HubError::BadRequest(format!(
            "thread {id} is both continued and resolved in one entry"
        )));
    }

    let referenced: Vec<i64> = continued.union(&resolved).copied().collect();
    if !referenced.is_empty() {
        let known: Vec<i64> = sqlx::query_scalar(
            r"
            SELECT DISTINCT t
            FROM journal_entries e, unnest(e.open_question_threads) AS t
            WHERE e.project_path = $1
              AND e.entry_date <> $2
              AND t = ANY($3)
            ",
        )
        .bind(project_path)
        .bind(entry_date)
        .bind(&referenced)
        .fetch_all(pool)
        .await?;
        let unknown: Vec<i64> = referenced
            .iter()
            .filter(|id| !known.contains(id))
            .copied()
            .collect();
        if !unknown.is_empty() {
            return Err(HubError::BadRequest(format!(
                "thread ids {unknown:?} are not open questions of an earlier entry of this project"
            )));
        }
    }

    let fresh_needed = links.iter().filter(|l| l.is_none()).count();
    let fresh: Vec<i64> = if fresh_needed == 0 {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "SELECT nextval('journal_thread_id_seq') FROM generate_series(1, $1::int)",
        )
        .bind(i32::try_from(fresh_needed).unwrap_or(i32::MAX))
        .fetch_all(pool)
        .await?
    };
    let mut fresh = fresh.into_iter();
    let ids = links
        .into_iter()
        .map(|l| l.or_else(|| fresh.next()).unwrap_or_default())
        .collect();
    Ok((ids, resolved.into_iter().collect()))
}

// ---------------------------------------------------------------------------
// The fold (pure)
// ---------------------------------------------------------------------------

/// The thread-relevant columns of one `entry`-status row.
#[derive(Debug, Clone, FromRow)]
pub struct EntryThreads {
    pub entry_date: NaiveDate,
    pub project_path: String,
    pub open_questions: Vec<String>,
    pub open_question_threads: Vec<i64>,
    pub resolved_threads: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadState {
    Open,
    Quiet,
    Resolved,
}

impl ThreadState {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "quiet" => Some(Self::Quiet),
            "resolved" => Some(Self::Resolved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    pub thread_id: i64,
    /// The most recent wording.
    pub question: String,
    pub first_seen: NaiveDate,
    pub last_seen: NaiveDate,
    /// Distinct entry dates that mention it.
    pub days_mentioned: usize,
    pub entry_dates: Vec<NaiveDate>,
    pub state: ThreadState,
    /// Latest resolution, when it is after the last mention.
    pub resolved_on: Option<NaiveDate>,
}

/// Fold entries into threads per project. `rows` may come in any order.
///
/// A thread is `resolved` when its latest resolution is dated after its last
/// mention (so a later mention reopens it); otherwise `quiet` when its last
/// mention is before the project's `quiet_after` most recent active days;
/// otherwise `open`. A project with fewer active days than that has no quiet
/// threads.
pub fn fold_threads(rows: &[EntryThreads], quiet_after: usize) -> BTreeMap<String, Vec<Thread>> {
    #[derive(Default)]
    struct Acc {
        mentions: Vec<(NaiveDate, String)>,
        resolutions: Vec<NaiveDate>,
    }
    let mut sorted: Vec<&EntryThreads> = rows.iter().collect();
    sorted.sort_by_key(|r| r.entry_date);

    let mut per_project: BTreeMap<&str, (BTreeSet<NaiveDate>, HashMap<i64, Acc>)> = BTreeMap::new();
    for row in sorted {
        let (active, threads) = per_project.entry(&row.project_path).or_default();
        active.insert(row.entry_date);
        for (q, id) in row.open_questions.iter().zip(&row.open_question_threads) {
            threads
                .entry(*id)
                .or_default()
                .mentions
                .push((row.entry_date, q.clone()));
        }
        for id in &row.resolved_threads {
            threads
                .entry(*id)
                .or_default()
                .resolutions
                .push(row.entry_date);
        }
    }

    per_project
        .into_iter()
        .map(|(project, (active, threads))| {
            let cutoff = active
                .iter()
                .rev()
                .nth(quiet_after.saturating_sub(1))
                .copied();
            let mut out: Vec<Thread> = threads
                .into_iter()
                .filter_map(|(thread_id, acc)| {
                    let (last_seen, question) = acc.mentions.last()?.clone();
                    let first_seen = acc.mentions.first()?.0;
                    let mut entry_dates: Vec<NaiveDate> =
                        acc.mentions.iter().map(|(d, _)| *d).collect();
                    entry_dates.dedup();
                    let resolved_on = acc
                        .resolutions
                        .iter()
                        .max()
                        .copied()
                        .filter(|r| *r > last_seen);
                    let state = if resolved_on.is_some() {
                        ThreadState::Resolved
                    } else if active.len() >= quiet_after && cutoff.is_some_and(|c| last_seen < c) {
                        ThreadState::Quiet
                    } else {
                        ThreadState::Open
                    };
                    Some(Thread {
                        thread_id,
                        question,
                        first_seen,
                        last_seen,
                        days_mentioned: entry_dates.len(),
                        entry_dates,
                        state,
                        resolved_on,
                    })
                })
                .collect();
            out.sort_by(|a, b| {
                b.last_seen
                    .cmp(&a.last_seen)
                    .then(b.thread_id.cmp(&a.thread_id))
            });
            (project.to_string(), out)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GET /v1/journal/open-questions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpenQuestionsParams {
    /// Exact `project_path`. Without it, per-project counts only.
    pub project: Option<String>,
    /// Window length in days, ending at `before` (default 30).
    pub days: Option<i64>,
    /// Exclusive upper bound (`YYYY-MM-DD`): state as of that day, mentions and
    /// resolutions on or after it ignored. The distiller passes its entry date.
    pub before: Option<String>,
    /// Comma-separated subset of `open,quiet,resolved` (default: all).
    pub state: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ProjectThreads {
    pub project_path: String,
    pub from: NaiveDate,
    pub before: NaiveDate,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Serialize, Default)]
pub struct ProjectCounts {
    pub project_path: String,
    pub open: usize,
    pub quiet: usize,
    pub resolved: usize,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OpenQuestionsResponse {
    Project(ProjectThreads),
    Counts { projects: Vec<ProjectCounts> },
}

pub async fn open_questions(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<OpenQuestionsParams>,
) -> Result<Json<OpenQuestionsResponse>, HubError> {
    let days = params.days.unwrap_or(DEFAULT_DAYS);
    if !(1..=MAX_DAYS).contains(&days) {
        return Err(HubError::BadRequest(format!(
            "days must be between 1 and {MAX_DAYS}"
        )));
    }
    let before = match params.before.as_deref() {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| HubError::BadRequest("before must be YYYY-MM-DD".into()))?,
        None => Utc::now().date_naive() + Duration::days(1),
    };
    let from = before - Duration::days(days);
    let states: Option<BTreeSet<&str>> = match params.state.as_deref() {
        None | Some("") => None,
        Some(s) => {
            let set: BTreeSet<&str> = s.split(',').map(str::trim).collect();
            if let Some(bad) = set.iter().find(|s| ThreadState::parse(s).is_none()) {
                return Err(HubError::BadRequest(format!(
                    "unknown state `{bad}` (expected open, quiet, resolved)"
                )));
            }
            Some(set)
        }
    };
    let wanted = |t: &Thread| {
        t.last_seen >= from
            && match &states {
                None => true,
                Some(s) => s
                    .iter()
                    .any(|name| ThreadState::parse(name) == Some(t.state)),
            }
    };

    // One project's entries, or every project's: a few thousand short rows at
    // most, folded in memory. Earlier than `from` still matters: a thread first
    // seen before the window, and the active days that decide `quiet`.
    let rows = sqlx::query_as::<_, EntryThreads>(
        r"
        SELECT entry_date, project_path, open_questions, open_question_threads,
               resolved_threads
        FROM journal_entries
        WHERE status = 'entry'
          AND ($1::text IS NULL OR project_path = $1)
          AND entry_date < $2
        ",
    )
    .bind(&params.project)
    .bind(before)
    .fetch_all(&state.pool)
    .await?;
    let folded = fold_threads(&rows, QUIET_AFTER_ACTIVE_DAYS);

    if let Some(project) = params.project {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let threads = folded
            .into_iter()
            .next()
            .map(|(_, ts)| ts.into_iter().filter(|t| wanted(t)).take(limit).collect())
            .unwrap_or_default();
        return Ok(Json(OpenQuestionsResponse::Project(ProjectThreads {
            project_path: project,
            from,
            before,
            threads,
        })));
    }

    let mut projects: Vec<ProjectCounts> = folded
        .into_iter()
        .map(|(project_path, ts)| {
            let mut c = ProjectCounts {
                project_path,
                ..ProjectCounts::default()
            };
            for t in ts.iter().filter(|t| wanted(t)) {
                match t.state {
                    ThreadState::Open => c.open += 1,
                    ThreadState::Quiet => c.quiet += 1,
                    ThreadState::Resolved => c.resolved += 1,
                }
            }
            c
        })
        .filter(|c| c.open + c.quiet + c.resolved > 0)
        .collect();
    projects.sort_by(|a, b| {
        (b.open + b.quiet)
            .cmp(&(a.open + a.quiet))
            .then(a.project_path.cmp(&b.project_path))
    });
    Ok(Json(OpenQuestionsResponse::Counts { projects }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn row(date: &str, project: &str, qs: &[(&str, i64)], resolved: &[i64]) -> EntryThreads {
        EntryThreads {
            entry_date: d(date),
            project_path: project.into(),
            open_questions: qs.iter().map(|(q, _)| (*q).to_string()).collect(),
            open_question_threads: qs.iter().map(|(_, id)| *id).collect(),
            resolved_threads: resolved.to_vec(),
        }
    }

    fn thread(folded: &BTreeMap<String, Vec<Thread>>, project: &str, id: i64) -> Thread {
        folded[project]
            .iter()
            .find(|t| t.thread_id == id)
            .unwrap_or_else(|| panic!("thread {id} missing"))
            .clone()
    }

    #[test]
    fn restated_thread_folds_into_one_with_latest_wording() {
        let rows = vec![
            row("2026-10-03", "/p", &[("third wording", 7)], &[]),
            row(
                "2026-10-01",
                "/p",
                &[("first wording", 7), ("other", 8)],
                &[],
            ),
            row("2026-10-02", "/p", &[("second wording", 7)], &[]),
        ];
        let t = thread(&fold_threads(&rows, 3), "/p", 7);
        assert_eq!(t.question, "third wording");
        assert_eq!(t.first_seen, d("2026-10-01"));
        assert_eq!(t.last_seen, d("2026-10-03"));
        assert_eq!(t.days_mentioned, 3);
        assert_eq!(t.state, ThreadState::Open);
    }

    #[test]
    fn resolved_after_last_mention_and_reopened_by_a_later_one() {
        let mut rows = vec![
            row("2026-10-01", "/p", &[("q", 7)], &[]),
            row("2026-10-02", "/p", &[("x", 9)], &[7]),
        ];
        let t = thread(&fold_threads(&rows, 3), "/p", 7);
        assert_eq!(t.state, ThreadState::Resolved);
        assert_eq!(t.resolved_on, Some(d("2026-10-02")));

        rows.push(row("2026-10-03", "/p", &[("q again", 7)], &[]));
        let t = thread(&fold_threads(&rows, 3), "/p", 7);
        assert_eq!(t.state, ThreadState::Open);
        assert_eq!(t.resolved_on, None);
    }

    #[test]
    fn quiet_counts_active_days_not_calendar_days() {
        // Mentioned on the 1st; entries on 3 later active days → quiet.
        let rows = vec![
            row("2026-10-01", "/p", &[("q", 7)], &[]),
            row("2026-10-02", "/p", &[("a", 8)], &[]),
            row("2026-10-03", "/p", &[("b", 9)], &[]),
            row("2026-10-04", "/p", &[("c", 10)], &[]),
        ];
        let f = fold_threads(&rows, 3);
        assert_eq!(thread(&f, "/p", 7).state, ThreadState::Quiet);
        assert_eq!(thread(&f, "/p", 8).state, ThreadState::Open);

        // A two-week gap, then one entry: the thread before the gap is still
        // within the last 3 active days, so not quiet.
        let rows = vec![
            row("2026-09-01", "/p", &[("q", 7)], &[]),
            row("2026-09-02", "/p", &[("a", 8)], &[]),
            row("2026-09-20", "/p", &[("b", 9)], &[]),
        ];
        assert_eq!(
            thread(&fold_threads(&rows, 3), "/p", 7).state,
            ThreadState::Open
        );
    }

    #[test]
    fn projects_stay_apart_even_with_the_same_id() {
        let rows = vec![
            row("2026-10-01", "/a", &[("qa", 7)], &[]),
            row("2026-10-01", "/b", &[("qb", 7)], &[]),
        ];
        let f = fold_threads(&rows, 3);
        assert_eq!(thread(&f, "/a", 7).question, "qa");
        assert_eq!(thread(&f, "/b", 7).question, "qb");
    }

    #[test]
    fn newest_last_seen_first() {
        let rows = vec![
            row("2026-10-01", "/p", &[("old", 1)], &[]),
            row("2026-10-05", "/p", &[("new", 2)], &[]),
        ];
        let ids: Vec<i64> = fold_threads(&rows, 3)["/p"]
            .iter()
            .map(|t| t.thread_id)
            .collect();
        assert_eq!(ids, vec![2, 1]);
    }
}
