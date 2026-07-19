//! Project-identity derivation shared by daemon (producer) and hub (consumer).
//!
//! A project's identity is derived from git facts: the root commit hash and
//! the normalized `origin` URL. Both sides use the SAME functions — the daemon
//! normalizes before sending, the hub re-normalizes defensively before
//! deriving, so a byte-identical `identity_key` is guaranteed regardless of
//! which side computed it.

/// Normalize a git remote URL to `host/path` form.
///
/// Rules (normative in `openspec/specs/project-identity/spec.md`):
/// - credentials/userinfo are stripped (never store tokens),
/// - scp-like syntax (`git@host:path`) becomes `host/path`,
/// - the host is lowercased (path case is preserved),
/// - a trailing `.git` and trailing slashes are stripped.
///
/// Returns `None` for remotes that carry no transferable host identity:
/// local paths, `file://` URLs, and hosts without a dot (a bare local
/// hostname is machine-specific and would collide meaninglessly).
pub fn normalize_remote_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    // Split off an explicit scheme, rejecting non-network ones.
    let (authority_and_path, had_scheme) = match s.find("://") {
        Some(idx) => {
            let scheme = &s[..idx];
            if !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ssh" | "git"
            ) {
                return None; // file://, etc.
            }
            (&s[idx + 3..], true)
        }
        None => (s, false),
    };

    let (host_part, path_part) = if had_scheme {
        // authority/path split at the first '/'.
        match authority_and_path.find('/') {
            Some(idx) => (&authority_and_path[..idx], &authority_and_path[idx + 1..]),
            None => (authority_and_path, ""),
        }
    } else {
        // No scheme. Three shapes carry identity (the middle one makes this
        // function idempotent — the hub re-normalizes the daemon's output):
        //   scp-like    `user@host:path` / `host:path`  (':' before any '/')
        //   normalized  `host/path` or `host:port/path`
        //   local path  → no transferable identity.
        let colon = authority_and_path.find(':');
        let slash = authority_and_path.find('/');
        match (colon, slash) {
            (Some(c), s) if s.map_or(true, |s| c < s) => {
                let before = &authority_and_path[..c];
                let after_colon = &authority_and_path[c + 1..];
                let port = after_colon.split('/').next().unwrap_or("");
                if !before.contains('@')
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit())
                {
                    // `host:port/path` (already-normalized form).
                    // `?` = host:port with no path → no transferable identity.
                    let idx = after_colon.find('/')?;
                    (&authority_and_path[..c + 1 + idx], &after_colon[idx + 1..])
                } else {
                    // scp-like: path starts after the colon.
                    (before, after_colon)
                }
            }
            (_, Some(s)) => (&authority_and_path[..s], &authority_and_path[s + 1..]),
            _ => return None,
        }
    };

    // Strip userinfo (everything up to the last '@' in the authority).
    let host = match host_part.rfind('@') {
        Some(idx) => &host_part[idx + 1..],
        None => host_part,
    };
    let host = host.to_ascii_lowercase();
    // The name (sans port) must be dotted with non-empty labels: rejects local
    // hostnames (`gitea`), relative paths (`..`), and empty authorities.
    let name = host.split(':').next().unwrap_or("");
    if !name.contains('.') || name.split('.').any(str::is_empty) {
        return None;
    }

    // Path: strip leading/trailing slashes, then a trailing `.git`.
    let path = path_part.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if path.is_empty() {
        return None;
    }

    Some(format!("{host}/{path}"))
}

/// Derive the opaque identity key from fingerprint facts.
///
/// Forms: `g:<root>|<remote>` (both), `g:<root>` (root only),
/// `r:<remote>` (remote only — shallow clones), `None` (no fingerprint).
/// The remote is re-normalized here so untrusted input cannot skew the key;
/// the root must be a full 40-hex hash (anything else is discarded).
pub fn derive_identity_key(root_commit: Option<&str>, remote_url: Option<&str>) -> Option<String> {
    let root = root_commit
        .map(str::trim)
        .filter(|r| r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase);
    let remote = remote_url.and_then(normalize_remote_url);
    match (root, remote) {
        (Some(r), Some(m)) => Some(format!("g:{r}|{m}")),
        (Some(r), None) => Some(format!("g:{r}")),
        (None, Some(m)) => Some(format!("r:{m}")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "a3f0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8";

    #[test]
    fn normalizes_https_and_scp_to_same_form() {
        for url in [
            "https://github.com/acme/foo.git",
            "https://github.com/acme/foo",
            "https://github.com/acme/foo/",
            "git@github.com:acme/foo.git",
            "ssh://git@github.com/acme/foo.git",
            "git://github.com/acme/foo.git",
            "https://GitHub.COM/acme/foo.git",
        ] {
            assert_eq!(
                normalize_remote_url(url).as_deref(),
                Some("github.com/acme/foo"),
                "url: {url}"
            );
        }
    }

    #[test]
    fn strips_credentials() {
        assert_eq!(
            normalize_remote_url("https://user:tok3n@github.com/acme/foo.git").as_deref(),
            Some("github.com/acme/foo")
        );
        assert_eq!(
            normalize_remote_url("ssh://git@gitea.cat-bluegill.ts.net:3000/ac/repo.git").as_deref(),
            Some("gitea.cat-bluegill.ts.net:3000/ac/repo")
        );
    }

    #[test]
    fn preserves_path_case_and_port() {
        assert_eq!(
            normalize_remote_url("https://Gitea.Host.TS.net:3000/AC/Repo.git").as_deref(),
            Some("gitea.host.ts.net:3000/AC/Repo")
        );
    }

    #[test]
    fn is_idempotent_over_its_own_output() {
        // The hub re-normalizes what the daemon already normalized.
        for url in [
            "https://github.com/acme/foo.git",
            "ssh://git@gitea.cat-bluegill.ts.net:3000/ac/repo.git",
            "git@github.com:acme/foo.git",
        ] {
            let once = normalize_remote_url(url).unwrap();
            assert_eq!(normalize_remote_url(&once).as_deref(), Some(once.as_str()));
        }
    }

    #[test]
    fn rejects_non_transferable_remotes() {
        for url in [
            "",
            "   ",
            "/srv/git/bare.git",
            "../relative/repo",
            "file:///srv/git/bare.git",
            "git@gitea:ac/repo.git", // dotless host: machine-local name
            "https://github.com",    // no path
            "https://github.com/",
        ] {
            assert_eq!(normalize_remote_url(url), None, "url: {url:?}");
        }
    }

    #[test]
    fn derives_all_key_forms() {
        assert_eq!(
            derive_identity_key(Some(ROOT), Some("git@github.com:acme/foo.git")).as_deref(),
            Some(format!("g:{ROOT}|github.com/acme/foo").as_str())
        );
        assert_eq!(
            derive_identity_key(Some(ROOT), None).as_deref(),
            Some(format!("g:{ROOT}").as_str())
        );
        assert_eq!(
            derive_identity_key(None, Some("https://github.com/acme/foo")).as_deref(),
            Some("r:github.com/acme/foo")
        );
        assert_eq!(derive_identity_key(None, None), None);
    }

    #[test]
    fn key_is_stable_across_remote_syntaxes_and_root_case() {
        let a = derive_identity_key(Some(ROOT), Some("git@github.com:acme/foo.git"));
        let b = derive_identity_key(
            Some(ROOT.to_ascii_uppercase().as_str()),
            Some("https://x:y@GITHUB.com/acme/foo/"),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn fork_same_root_different_remote_gets_distinct_key() {
        let ours = derive_identity_key(Some(ROOT), Some("github.com/colangelo/foo"));
        let theirs = derive_identity_key(Some(ROOT), Some("github.com/upstream/foo"));
        assert_ne!(ours, theirs);
    }

    #[test]
    fn invalid_root_is_discarded_not_propagated() {
        // Too short / non-hex roots degrade to the remote-only form.
        assert_eq!(
            derive_identity_key(Some("abc123"), Some("github.com/acme/foo")).as_deref(),
            Some("r:github.com/acme/foo")
        );
        assert_eq!(derive_identity_key(Some("not-hex-at-all"), None), None);
    }
}
