//! Stable machine identity: a UUID persisted in the state directory plus the
//! hostname. The UUID is generated once and reused across restarts so archived
//! records carry consistent machine provenance.

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
        let hostname = gethostname::gethostname().to_string_lossy().into_owned();
        Ok(Identity {
            machine_id,
            hostname,
        })
    }
}
