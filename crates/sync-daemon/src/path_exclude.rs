//! Project paths this machine never uploads (`project_path_exclude`).
//!
//! Matched against a project's decoded path (the directory the agent ran in)
//! before any session of it is parsed, so an excluded project's transcripts
//! never leave the machine. Patterns are shell globs: `*` also crosses `/`,
//! so `/home/agent/client*` covers every directory below such a path too.

use glob::Pattern;

#[derive(Debug, Default, Clone)]
pub struct PathExclude {
    patterns: Vec<Pattern>,
}

impl PathExclude {
    /// Every pattern must parse: a typo must stop the daemon, not upload
    /// what it was meant to keep back.
    pub fn parse(globs: &[String]) -> anyhow::Result<Self> {
        let patterns = globs
            .iter()
            .map(|g| {
                Pattern::new(g)
                    .map_err(|e| anyhow::anyhow!("project_path_exclude: bad glob {g:?}: {e}"))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(PathExclude { patterns })
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn excludes(&self, project_path: &str) -> bool {
        self.patterns.iter().any(|p| p.matches(project_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(globs: &[&str]) -> PathExclude {
        PathExclude::parse(&globs.iter().map(|g| (*g).to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn a_prefix_glob_covers_the_tree_below_it() {
        let r = rules(&["/home/agent/client*"]);
        assert!(r.excludes("/home/agent/client"));
        assert!(r.excludes("/home/agent/client-canary"));
        assert!(r.excludes("/home/agent/client-canary/sub/dir"));
        assert!(!r.excludes("/home/agent/infra"));
        assert!(!r.excludes("/home/agent/src/client-notes"));
    }

    #[test]
    fn no_patterns_excludes_nothing() {
        assert!(!rules(&[]).excludes("/home/agent/client"));
    }

    #[test]
    fn a_bad_glob_is_an_error_not_a_silent_pass() {
        let err = PathExclude::parse(&["/home/agent/[client".to_string()]).unwrap_err();
        assert!(err.to_string().contains("project_path_exclude"));
    }
}
