//! Stable machine identity: a UUID persisted in the state directory plus the
//! hostname. The UUID is generated once and reused across restarts so archived
//! records carry consistent machine provenance.
//!
//! `CCHV_HOSTNAME` overrides the reported hostname — history restored from
//! another machine's backups (paired with a state dir holding that machine's
//! id) must be attributed to the source machine, not the restore host.

use std::path::Path;
use uuid::Uuid;

use crate::fs_atomic::write_atomic;

#[derive(Debug, Clone)]
pub struct Identity {
    pub machine_id: Uuid,
    pub hostname: String,
}

impl Identity {
    /// Load the machine id from `<state_dir>/machine_id`, creating it on first run.
    pub fn load_or_create(state_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let path = state_dir.join("machine_id");
        let machine_id = if path.exists() {
            let text = std::fs::read_to_string(&path)?;
            Uuid::parse_str(text.trim())
                .map_err(|e| anyhow::anyhow!("corrupt machine_id at {}: {e}", path.display()))?
        } else {
            let id = Uuid::new_v4();
            write_atomic(&path, id.to_string().as_bytes())?;
            id
        };
        let hostname = resolve_hostname();
        if std::env::var("CCHV_HOSTNAME").map_or(true, |v| v.trim().is_empty())
            && looks_bonjour_collided(&hostname)
        {
            tracing::warn!(
                hostname = %hostname,
                "hostname looks like a Bonjour collision rename (<name>-<N>.local). The hub \
                 matches healthz/ingest ?exclude= and the deploy gates on the exact name, so \
                 this machine is now reported under a different one. Pin the real name with \
                 CCHV_HOSTNAME in the daemon's launchd plist (#43)."
            );
        }
        Ok(Identity {
            machine_id,
            hostname,
        })
    }
}

/// The hostname to attribute ingests to: `CCHV_HOSTNAME` when set and
/// non-empty, else the system hostname.
fn resolve_hostname() -> String {
    match std::env::var("CCHV_HOSTNAME") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => gethostname::gethostname().to_string_lossy().into_owned(),
    }
}

/// True when `host` has the shape macOS Bonjour gives a `LocalHostName` it had
/// to rename on a clash: `<name>-<digits>`, optionally followed by `.local`
/// (`m4m-2.local`). A heuristic for a warning only: a host genuinely named
/// `build-2` matches too, which is why nothing acts on it automatically.
fn looks_bonjour_collided(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    let base = lower.strip_suffix(".local").unwrap_or(&lower);
    match base.rsplit_once('-') {
        Some((name, n)) => {
            !name.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var tests share process state — keep them in one test (the suite
    // already runs with --test-threads=1 for the same reason elsewhere).
    #[test]
    fn hostname_override_applies_only_when_set_and_non_empty() {
        std::env::remove_var("CCHV_HOSTNAME");
        let system = gethostname::gethostname().to_string_lossy().into_owned();
        assert_eq!(resolve_hostname(), system);

        std::env::set_var("CCHV_HOSTNAME", "");
        assert_eq!(resolve_hostname(), system);

        std::env::set_var("CCHV_HOSTNAME", "  ");
        assert_eq!(resolve_hostname(), system);

        std::env::set_var("CCHV_HOSTNAME", "ac-mbp");
        assert_eq!(resolve_hostname(), "ac-mbp");

        std::env::set_var("CCHV_HOSTNAME", " ac-mbp ");
        assert_eq!(resolve_hostname(), "ac-mbp");

        std::env::remove_var("CCHV_HOSTNAME");
    }

    #[test]
    fn bonjour_collision_shape_is_recognised() {
        // The renames Bonjour applies on a LocalHostName clash (#43), with
        // and without the `.local` suffix gethostname() usually carries.
        for collided in [
            "m4m-2.local",
            "ac-mbm5-2.local",
            "ac-mbm5-13",
            "M4M-2.LOCAL",
        ] {
            assert!(looks_bonjour_collided(collided), "{collided}");
        }
        for clean in [
            "m4m.local",
            "ac-mbm5.local",
            "ac-mbm5",
            "m4m",
            "",
            "-2.local",
            "host-.local",
            "build-2x.local",
        ] {
            assert!(!looks_bonjour_collided(clean), "{clean}");
        }
    }

    #[test]
    fn machine_id_persists_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        let second = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(first.machine_id, second.machine_id);
    }
}
