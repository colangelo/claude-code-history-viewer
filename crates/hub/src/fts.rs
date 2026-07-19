//! Full-text-search query helpers (issue #19).
//!
//! Both search surfaces parse `q` with `websearch_to_tsquery('simple', …)`,
//! which matches whole lexemes only — `distill` can never match `distiller`
//! (the `simple` config does no stemming and websearch syntax has no prefix
//! operator). For PLAIN queries we therefore build an additional
//! prefix-matching tsquery (`'distill':*`) that the SQL ORs in; queries using
//! websearch syntax (phrases, OR, negation) keep their exact semantics and
//! get no prefix variant — prefixing a negated or quoted term would change
//! meaning, not broaden it.

/// Build the prefix tsquery string for `to_tsquery('simple', …)`, or `None`
/// when the query uses websearch syntax (or yields no tokens).
pub fn prefix_tsquery(q: &str) -> Option<String> {
    let raw = q.trim();
    if raw.is_empty()
        || raw.contains('"')
        || raw
            .split_whitespace()
            .any(|w| w == "OR" || w.starts_with('-'))
    {
        return None;
    }
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        // `simple` lowercases lexemes; quote each token so hyphen-split parts
        // and digits parse as single lexemes, and escape embedded quotes.
        .map(|t| format!("'{}':*", t.to_lowercase().replace('\'', "''")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" & "))
    }
}

#[cfg(test)]
mod tests {
    use super::prefix_tsquery;

    #[test]
    fn plain_single_term_gets_prefix() {
        assert_eq!(prefix_tsquery("distill"), Some("'distill':*".into()));
    }

    #[test]
    fn multiple_terms_are_anded() {
        assert_eq!(
            prefix_tsquery("token cache"),
            Some("'token':* & 'cache':*".into())
        );
    }

    #[test]
    fn punctuation_splits_like_the_simple_config() {
        assert_eq!(
            prefix_tsquery("cchv-distill"),
            Some("'cchv':* & 'distill':*".into())
        );
    }

    #[test]
    fn uppercase_is_folded() {
        assert_eq!(prefix_tsquery("Hermes"), Some("'hermes':*".into()));
    }

    #[test]
    fn websearch_syntax_disables_prefixing() {
        assert_eq!(prefix_tsquery("\"exact phrase\""), None);
        assert_eq!(prefix_tsquery("foo OR bar"), None);
        assert_eq!(prefix_tsquery("foo -bar"), None);
    }

    #[test]
    fn empty_and_symbol_only_yield_none() {
        assert_eq!(prefix_tsquery("   "), None);
        assert_eq!(prefix_tsquery("!!!"), None);
    }
}
