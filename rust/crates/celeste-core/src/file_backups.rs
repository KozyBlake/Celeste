//! Port of `net.sybyline.scarlet.util.FileBackups`: crash-safe saves for
//! Celeste's small JSON data files (watched lists, settings, Discord
//! config, ...).
//!
//! [`write`] writes to a temporary sibling file, fsyncs it, and only then:
//! (1) copies the previous version into a `backups/` folder next to the
//! file, stamped with the current date-time and pruned to the newest `keep`
//! copies, then (2) renames the temp file into place (`rename` on the same
//! filesystem is atomic on both POSIX and Windows NTFS, matching Java's
//! `ATOMIC_MOVE` with a plain-rename fallback for the rare filesystem that
//! rejects it). If the write fails partway through, the original file is
//! untouched.
//!
//! Recovery is manual and simple by design: pick a dated file out of
//! `backups/` and copy it back — see [`backups_newest_first`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default number of dated backups retained per file.
pub const DEFAULT_KEEP: usize = 10;

/// Writes `contents` to `target` via a temp-file-then-rename, backing up
/// the previous version first. See the module docs for the crash-safety
/// shape this preserves from the Java version.
pub fn write(target: &Path, contents: &str) -> io::Result<()> {
    write_keep(target, contents, DEFAULT_KEEP)
}

pub fn write_keep(target: &Path, contents: &str, keep: usize) -> io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        "{}.tmp",
        target.file_name().and_then(|n| n.to_str()).unwrap_or("celeste")
    ));

    {
        let file = fs::File::create(&temp)?;
        use std::io::Write;
        let mut file = file;
        file.write_all(contents.as_bytes())?;
        // Force the temp file's bytes to disk before the swap. Without
        // this, a crash can complete the rename while the data blocks are
        // still only in the OS cache, leaving a zero-length target -- the
        // "settings reset themselves after a crash" bug. A failed fsync is
        // not fatal: a non-durable write still beats no write.
        let _ = file.sync_all();
    }

    if target.is_file() {
        backup(target, keep);
    }
    fs::rename(&temp, target)?;
    fsync_dir(parent);
    Ok(())
}

fn fsync_dir(dir: &Path) {
    // Best-effort: fsync the containing directory so the rename itself is
    // durable across a crash. POSIX-only in practice -- opening a directory
    // for read and syncing it fails harmlessly on platforms that don't
    // support it (e.g. Windows), matching the Java version's ignore-and-move-on.
    if let Ok(dir_handle) = fs::File::open(dir) {
        let _ = dir_handle.sync_all();
    }
}

fn backup(target: &Path, keep: usize) {
    if keep == 0 {
        return;
    }
    let parent = match target.parent() {
        Some(p) => p,
        None => return,
    };
    let dir = parent.join("backups");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let name = match target.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_string(),
        None => return,
    };
    let stamped = format!("{name}.{}.bak", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    if let Err(err) = fs::copy(target, dir.join(&stamped)) {
        // A failed backup must never block the save itself.
        tracing::warn!("Could not back up {}: {}", target.display(), err);
        return;
    }
    prune(&dir, &name, keep);
}

fn prune(dir: &Path, base_name: &str, keep: usize) {
    let mut backups = list_backups(dir, base_name);
    if backups.len() <= keep {
        return;
    }
    // newest first (the timestamp stamp sorts lexically)
    backups.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for old in &backups[keep..] {
        if fs::remove_file(old).is_err() {
            tracing::debug!("Could not prune old backup {}", old.display());
        }
    }
}

fn list_backups(dir: &Path, base_name: &str) -> Vec<PathBuf> {
    let prefix = format!("{base_name}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".bak"))
        })
        .collect()
}

/// Dated backups for `target` (from the sibling `backups/` folder), newest
/// first. Empty when none exist.
pub fn backups_newest_first(target: &Path) -> Vec<PathBuf> {
    let Some(parent) = target.parent() else {
        return Vec::new();
    };
    let dir = parent.join("backups");
    let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let mut backups = list_backups(&dir, name);
    backups.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    backups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        write(&target, "{\"a\":1}").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn backs_up_the_previous_version_before_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        write(&target, "v1").unwrap();
        write(&target, "v2").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "v2");
        let backups = backups_newest_first(&target);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), "v1");
    }

    #[test]
    fn prunes_to_the_keep_limit() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        for i in 0..3 {
            write_keep(&target, &format!("v{i}"), 1).unwrap();
            // Ensure distinct timestamps for the backup filename stamp.
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let backups = backups_newest_first(&target);
        assert!(backups.len() <= 1, "expected at most 1 backup, got {}", backups.len());
    }

    #[test]
    fn no_backups_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nonexistent.json");
        assert!(backups_newest_first(&target).is_empty());
    }
}
