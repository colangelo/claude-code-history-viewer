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
    fn machine_id_persists_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        let second = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(first.machine_id, second.machine_id);
    }
}
