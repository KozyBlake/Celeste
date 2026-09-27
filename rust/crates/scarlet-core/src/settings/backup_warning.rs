//! Port of the ack-file half of `ScarletSettings.warnBackupBeforeSecureStoreChanges`:
//! a one-time "back up your data before we touch the secure store" notice,
//! gated so it only shows once per data directory.
//!
//! Showing the notice is a UI concern (a dialog box, a console banner, ...)
//! left to whichever UI is running; this module only owns the pure parts:
//! whether it's already been acknowledged, recording that it has, and the
//! message text so every UI shows the same wording.

use std::path::{Path, PathBuf};

const ACK_FILENAME: &str = "backup-warning-ack-v1.flag";

fn ack_file(data_dir: &Path) -> PathBuf {
    data_dir.join(ACK_FILENAME)
}

pub fn is_acknowledged(data_dir: &Path) -> bool {
    ack_file(data_dir).is_file()
}

pub fn acknowledge(data_dir: &Path) {
    let path = ack_file(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = chrono::Utc::now().timestamp_millis();
    if let Err(err) = std::fs::write(&path, format!("acknowledged={now}")) {
        tracing::warn!("Failed to persist backup warning acknowledgement at {}: {}", path.display(), err);
    }
}

/// The notice text shown to the user (matches the Java version's wording,
/// updated only where it named Swing-specific mechanics).
pub fn message(data_dir: &Path, settings_file: &Path) -> String {
    format!(
        "Back up your KozyBlake/Scarlet data before startup continues.\n\n\
         Scarlet may reinitialize or lose access to previously stored secure settings and credentials during initialization.\n\
         If something goes wrong, a backup is the safest way to recover.\n\n\
         Data that could be reset, reverted, or need recovery includes:\n\
         - Discord bot token\n\
         - VRChat usernames, passwords, TOTP secrets, and cookie/session data\n\
         - Other secure values stored in the encrypted credential store\n\n\
         Recommended current files/folders to copy now:\n\
         - {}\n\
         - {}\n\n\
         Startup will continue after this reminder is acknowledged.",
        data_dir.display(),
        settings_file.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unacknowledged_by_default_then_acknowledges() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_acknowledged(dir.path()));
        acknowledge(dir.path());
        assert!(is_acknowledged(dir.path()));
    }
}
