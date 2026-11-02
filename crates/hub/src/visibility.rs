//! What a read principal may see (openspec `hub-client-read-scope`, D5, D6).
//!
//! Every read statement that touches sessions, messages, projects, journal
//! entries or stats takes a [`Visibility`] with no default, built only from the
//! caller's [`ReadScope`] and the hub's client-scope state. A lab reader sees a
//! session only when its class is `none` under the generation in force; with
//! no usable policy it sees no session at all.

use crate::state::AppState;

/// A read principal's scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScope {
    /// Machine tokens, `[[readers]]`, the trusted Tailscale identity.
    Lab,
    /// `[[client_readers]]`: Claude-only principals.
    Client,
}

/// What one request may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    /// Every session: client scope is off or observing, or the caller holds client read.
    All,
    /// Lab sessions under this generation only; `None` = no usable policy, nothing.
    Lab(Option<String>),
}

impl Visibility {
    pub fn of(state: &AppState, scope: ReadScope) -> Self {
        let Some(cs) = &state.client_scope else {
            return Visibility::All;
        };
        if scope == ReadScope::Client || !cs.enforcing() {
            return Visibility::All;
        }
        Self::lab(cs)
    }

    /// For the stats-read routes (`/v1/stats/daily`, `/v1/stats/tool-calls`;
    /// openspec stats-read-api D3, ruled (a)): a lab principal sees lab sessions
    /// only in observe as well as under enforce. Their readers are services
    /// that store what they read, so observe's "nothing hidden yet" does not
    /// apply to them.
    pub fn stats_read(state: &AppState, scope: ReadScope) -> Self {
        let Some(cs) = &state.client_scope else {
            return Visibility::All;
        };
        if scope == ReadScope::Client {
            return Visibility::All;
        }
        Self::lab(cs)
    }

    fn lab(cs: &crate::client_state::ClientScope) -> Self {
        Visibility::Lab(
            cs.snapshot()
                .generation()
                .filter(|g| g.len() == 64 && g.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_string),
        )
    }

    pub fn sees_all(&self) -> bool {
        matches!(self, Visibility::All)
    }

    /// A SQL boolean over the `sessions` row aliased `alias`. The generation is
    /// inlined: it is a sha256 hex string the hub computed itself, checked above.
    pub fn session_ok(&self, alias: &str) -> String {
        match self {
            Visibility::All => "true".into(),
            Visibility::Lab(None) => "false".into(),
            // Total (security O1): an unscanned session (NULL class or
            // generation) is NOT visible, under NOT as well as under WHERE.
            Visibility::Lab(Some(g)) => format!(
                "COALESCE({alias}.client_class = 'none' AND {alias}.client_gen = '{g}', false)"
            ),
        }
    }

    /// A SQL boolean: the session with this id expression is visible.
    pub fn session_id_ok(&self, id_expr: &str) -> String {
        match self {
            Visibility::All => "true".into(),
            Visibility::Lab(_) => format!(
                "EXISTS (SELECT 1 FROM sessions vs WHERE vs.id = {id_expr} AND {})",
                self.session_ok("vs")
            ),
        }
    }

    /// A SQL boolean: every session id in this array expression is visible (and
    /// the array is not empty). For journal entries (R2-F).
    pub fn all_sessions_ok(&self, ids_expr: &str) -> String {
        match self {
            Visibility::All => "true".into(),
            Visibility::Lab(_) => format!(
                "COALESCE(cardinality({ids_expr}) > 0 AND NOT EXISTS (
                    SELECT 1 FROM unnest({ids_expr}) AS vid(id)
                    LEFT JOIN sessions vs ON vs.id = vid.id
                    WHERE vs.id IS NULL OR NOT {}), false)",
                self.session_ok("vs")
            ),
        }
    }
}

impl Visibility {
    /// A SQL boolean over a `journal_entries` row (columns prefixed `p`, e.g.
    /// `"j."` or `""`): a lab reader sees an entry only with a current general
    /// route stamp and every session visible (design R2-F). A route-less entry
    /// (every entry before v0.34.0), a stale stamp, a missing session or an
    /// empty session list is client.
    pub fn entry_ok(&self, p: &str) -> String {
        match self {
            Visibility::All => "true".into(),
            Visibility::Lab(None) => "false".into(),
            Visibility::Lab(Some(_)) => format!(
                "COALESCE({p}route IN ('general', 'general-restamped') AND {p}route_at = {p}generated_at AND {}, false)",
                self.all_sessions_ok(&format!("{p}session_ids"))
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicates_by_case() {
        assert_eq!(Visibility::All.session_ok("s"), "true");
        assert_eq!(Visibility::Lab(None).session_ok("s"), "false");
        let g = "a".repeat(64);
        assert_eq!(
            Visibility::Lab(Some(g.clone())).session_ok("s"),
            format!("COALESCE(s.client_class = 'none' AND s.client_gen = '{g}', false)")
        );
    }
}
