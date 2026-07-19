//! Identity-scoped project filtering (`project=identity:<key>`).
//!
//! The `project` query param keeps its byte-exact plain semantics; the
//! reserved `identity:` prefix switches to server-side expansion into the
//! identity's member paths — fingerprinted project rows sharing the key PLUS
//! manually aliased (typically moved-away) paths. Expansion is server-side
//! because only the hub holds the alias table, and path-based (rather than
//! `identity_key`-based) matching so un-fingerprinted rows of the same path
//! on other machines fold in too.
//!
//! Runtime `sqlx::query*` on purpose: the CI gate builds with `SQLX_OFFLINE`
//! (see the note at the top of `journal.rs`).

use sqlx::PgPool;

/// Reserved prefix on the `project` filter param. Real project paths are
/// absolute filesystem paths and can never collide with it.
pub const IDENTITY_PREFIX: &str = "identity:";

/// A resolved `project` filter: exactly one of the two forms is active.
/// `paths: Some(vec![])` (unknown identity) correctly matches nothing.
#[derive(Debug, Default)]
pub struct ProjectScope {
    /// Plain filter value — existing `name = $ OR project_path = $` semantics.
    pub plain: Option<String>,
    /// Identity expansion — `project_path = ANY($)` semantics.
    pub paths: Option<Vec<String>>,
}

/// Resolve the raw `project` param into a [`ProjectScope`].
///
/// `include_worktrees=false` drops a member path only when EVERY project row
/// binding that path to the identity is a linked worktree — a path that is a
/// main checkout anywhere stays included (inclusion is the safe default).
/// Aliased paths are dead paths, never worktrees, and are always included.
pub async fn resolve_project_scope(
    pool: &PgPool,
    project: Option<&str>,
    include_worktrees: bool,
) -> Result<ProjectScope, sqlx::Error> {
    let Some(project) = project else {
        return Ok(ProjectScope::default());
    };
    let Some(key) = project.strip_prefix(IDENTITY_PREFIX) else {
        return Ok(ProjectScope {
            plain: Some(project.to_string()),
            paths: None,
        });
    };

    let paths = sqlx::query_scalar::<_, String>(
        r"
        SELECT project_path FROM projects
        WHERE identity_key = $1
        GROUP BY project_path
        HAVING $2 OR NOT bool_and(git_worktree)
        UNION
        SELECT project_path FROM project_identity_aliases
        WHERE identity_key = $1
        ",
    )
    .bind(key)
    .bind(include_worktrees)
    .fetch_all(pool)
    .await?;

    Ok(ProjectScope {
        plain: None,
        paths: Some(paths),
    })
}
