//! Atomic file write: write to a temp file in the same directory, then rename
//! over the target. Guards the machine-id and checkpoint files against torn
//! writes on crash.

use std::io::Write;
use std::path::Path;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!(
        "tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("f")
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // Windows rename fails if the destination exists; remove first there.
    #[cfg(target_os = "windows")]
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path)
}
