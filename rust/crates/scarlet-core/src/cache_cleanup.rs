//! Safe stale-file cleanup, ported from `ScarletCacheCleanup` and extended
//! to cover VRChat's own `output_log_*.txt` files at the owner's request.
//!
//! Only known disposable filename families are considered. The newest
//! Celeste/Scarlet log and newest VRChat output log are always protected,
//! even when their modification time is unexpectedly old, because those
//! are the files the running processes can still be writing or tailing.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub eligible: usize,
    pub deleted: usize,
    pub failed: usize,
    pub bytes_freed: u64,
}

fn modified_before(path: &Path, cutoff: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified < cutoff)
}

fn known_celeste_log(name: &str) -> bool {
    ["celeste_log_", "scarlet_log_"]
        .iter()
        .any(|prefix| name.starts_with(prefix) && name.ends_with(".txt"))
}

fn known_vrchat_log(name: &str) -> bool {
    name.starts_with("output_log_") && name.ends_with(".txt")
}

fn regular_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect()
}

fn newest_matching(dir: &Path, predicate: fn(&str) -> bool) -> Option<PathBuf> {
    regular_files(dir)
        .into_iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(predicate))
        // These log families contain sortable timestamps in their names.
        .max_by(|left, right| left.file_name().cmp(&right.file_name()))
}

/// Builds a deletion plan without touching anything. This is shared by the
/// automatic scheduler and the Settings confirmation UI.
pub fn collect_eligible(data_dir: &Path, vrchat_log_dir: &Path, cutoff: SystemTime) -> Vec<PathBuf> {
    let mut result = Vec::new();

    // Java's generated/cached TTS files.
    result.extend(regular_files(&data_dir.join("tts")).into_iter().filter(|path| modified_before(path, cutoff)));

    // Java only scans the immediate entity-kind directories under caches/;
    // preserve that boundary so cleanup can never wander into arbitrary
    // nested user content.
    if let Ok(kinds) = std::fs::read_dir(data_dir.join("caches")) {
        for kind in kinds.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_dir()) {
            result.extend(regular_files(&kind).into_iter().filter(|path| modified_before(path, cutoff)));
        }
    }

    let celeste_logs = data_dir.join("logs");
    let protected_celeste = newest_matching(&celeste_logs, known_celeste_log);
    result.extend(regular_files(&celeste_logs).into_iter().filter(|path| {
        path.file_name().and_then(|name| name.to_str()).is_some_and(known_celeste_log)
            && protected_celeste.as_ref() != Some(path)
            && modified_before(path, cutoff)
    }));

    // This is an intentional extension beyond ScarletCacheCleanup: the
    // owner specifically wants old VRChat client logs included. Protecting
    // the newest timestamped log makes this safe while VRChat is running.
    let protected_vrchat = newest_matching(vrchat_log_dir, known_vrchat_log);
    result.extend(regular_files(vrchat_log_dir).into_iter().filter(|path| {
        path.file_name().and_then(|name| name.to_str()).is_some_and(known_vrchat_log)
            && protected_vrchat.as_ref() != Some(path)
            && modified_before(path, cutoff)
    }));

    result.sort();
    result.dedup();
    result
}

pub fn delete_files(files: &[PathBuf]) -> CleanupReport {
    let mut report = CleanupReport { eligible: files.len(), ..CleanupReport::default() };
    for path in files {
        let size = std::fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0);
        match std::fs::remove_file(path) {
            Ok(()) => {
                report.deleted += 1;
                report.bytes_freed += size;
                tracing::debug!(path = %path.display(), "Deleted stale cached file");
            }
            Err(error) => {
                report.failed += 1;
                tracing::warn!(path = %path.display(), %error, "Failed to delete stale cached file");
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn protects_live_logs_and_ignores_unknown_files() {
        let data = tempfile::tempdir().unwrap();
        let vrc = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("logs")).unwrap();
        std::fs::create_dir_all(data.path().join("caches/usr")).unwrap();

        let old_celeste = data.path().join("logs/celeste_log_2026-01-01_00-00-00.txt");
        let live_celeste = data.path().join("logs/celeste_log_2026-01-02_00-00-00.txt");
        let unrelated = data.path().join("logs/notes.txt");
        let cache = data.path().join("caches/usr/usr_1");
        let old_vrc = vrc.path().join("output_log_2026-01-01_00-00-00.txt");
        let live_vrc = vrc.path().join("output_log_2026-01-02_00-00-00.txt");
        for path in [&old_celeste, &live_celeste, &unrelated, &cache, &old_vrc, &live_vrc] {
            std::fs::write(path, b"old").unwrap();
        }

        let eligible = collect_eligible(
            data.path(),
            vrc.path(),
            SystemTime::now() + Duration::from_secs(1),
        );
        assert!(eligible.contains(&old_celeste));
        assert!(eligible.contains(&old_vrc));
        assert!(eligible.contains(&cache));
        assert!(!eligible.contains(&live_celeste));
        assert!(!eligible.contains(&live_vrc));
        assert!(!eligible.contains(&unrelated));
    }

    #[test]
    fn deletion_report_counts_bytes_and_failures() {
        let dir = tempfile::tempdir().unwrap();
        let exists = dir.path().join("exists");
        let missing = dir.path().join("missing");
        std::fs::write(&exists, b"1234").unwrap();
        let report = delete_files(&[exists.clone(), missing]);
        assert_eq!(report.eligible, 2);
        assert_eq!(report.deleted, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.bytes_freed, 4);
        assert!(!exists.exists());
    }
}
