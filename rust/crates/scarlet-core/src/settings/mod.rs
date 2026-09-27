//! Port of `net.sybyline.scarlet.ScarletSettings`.
//!
//! Split across submodules along the same lines the Java class mixes
//! together in one file:
//! - [`store`] — the JSON-file-backed `getJson`/`getString`/`getObject`/...
//!   half (`SettingsStore`).
//! - [`registry`] — the `Preferences`-backed `Registry*` values, replaced
//!   with a flat file store plus [`crate::secure_store::EncryptedPrefs`]
//!   for the encrypted ones (see that module's docs for why).
//! - [`file_valued`] — the `FileValued<T>` typed-settings registry.
//!
//! ## Not ported here: interactive dialogs and the backup-warning popup
//!
//! `requireInput`/`requireConfirmYesNo`/`requireSelect` (and their
//! `*Async` variants) show a Swing dialog when a GUI is available, falling
//! back to console I/O when headless. Since there's no Swing in Rust, this
//! module defines [`UserPrompt`] as the seam: [`ConsolePrompt`] ports the
//! headless fallback path faithfully (it's plain stdin/stdout, no
//! Java-specific dependency), and a GUI-backed implementation belongs in
//! `scarlet-ui` once that crate's toolkit is chosen (see PORTING.md).
//! Likewise, `warnBackupBeforeSecureStoreChanges`'s message content and
//! ack-file gating are ported (any UI can reuse
//! [`backup_warning::is_acknowledged`]/[`backup_warning::acknowledge`]),
//! but showing it is left to whichever UI (console or graphical) is
//! actually running.

pub mod backup_warning;
pub mod file_valued;
pub mod prompt;
pub mod registry;
pub mod store;

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::secure_store::EncryptedPrefs;
use file_valued::{FileValued, SettingsRegistry};
use registry::{RegistryStore, RegistryStringValued};
use store::SettingsStore;

/// Resolves the password protecting the local encrypted secret store.
/// Port of `Scarlet`'s `globalPW`/`fallbackGlobalPw`: `SCARLET_GLOBAL_PW`
/// env var first (Java also checked the `scarlet.global.pw` system
/// property, which Rust has no equivalent of -- env var covers the same
/// "override from the launch environment" use case), then a persisted
/// random 256-bit key at `<data_dir>/global-prefs.key`, generated once and
/// reused thereafter.
pub fn resolve_global_password(data_dir: &Path) -> String {
    if let Ok(pw) = std::env::var("SCARLET_GLOBAL_PW") {
        return pw;
    }
    let key_path = data_dir.join("global-prefs.key");
    if let Ok(existing) = std::fs::read_to_string(&key_path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    use base64::Engine;
    use rand::RngCore;
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
    let _ = std::fs::create_dir_all(data_dir);
    if std::fs::write(&key_path, &encoded).is_err() {
        tracing::error!("Failed to persist local fallback encryption password at {}", key_path.display());
    }
    encoded
}

pub struct Settings {
    pub store: Arc<SettingsStore>,
    pub registry: SettingsRegistry,
    pub registry_store: Arc<RegistryStore>,
    pub encrypted: Arc<EncryptedPrefs>,

    pub last_run_version: RegistryStringValued<String>,
    pub last_announcement_id: RegistryStringValued<String>,
    pub last_run_time: RegistryStringValued<DateTime<Utc>>,
    pub last_audit_query: RegistryStringValued<DateTime<Utc>>,
    pub last_instances_check: RegistryStringValued<DateTime<Utc>>,
    pub last_auth_refresh: RegistryStringValued<DateTime<Utc>>,
    pub last_update_check: RegistryStringValued<DateTime<Utc>>,
    pub last_announcement_check: RegistryStringValued<DateTime<Utc>>,
    pub next_poll_action: RegistryStringValued<DateTime<Utc>>,
    pub next_mod_summary: RegistryStringValued<DateTime<Utc>>,
    pub next_outstanding_mod: RegistryStringValued<DateTime<Utc>>,
    pub last_cache_cleanup: RegistryStringValued<DateTime<Utc>>,
    pub last_instance_joined: RegistryStringValued<chrono::NaiveDateTime>,
    pub ui_bounds: RegistryStringValued<registry::Rectangle>,

    pub heuristic_kick_count: Arc<FileValued<i32>>,
    pub heuristic_period_days: Arc<FileValued<i32>>,
    pub outstanding_period_days: Arc<FileValued<i32>>,
    pub cache_cleanup_enabled: Arc<FileValued<bool>>,
    pub cache_cleanup_days: Arc<FileValued<i32>>,

    settings_file: std::path::PathBuf,
    settings_file_last_modified: Option<std::time::SystemTime>,
    has_version_changed_since_last_run: std::sync::Mutex<Option<bool>>,
}

impl Settings {
    pub fn open(data_dir: &Path) -> Self {
        let settings_path = data_dir.join("settings.json");
        let settings_file_last_modified = std::fs::metadata(&settings_path).and_then(|m| m.modified()).ok();
        let store = Arc::new(SettingsStore::new(settings_path.clone()));
        let registry_store = Arc::new(RegistryStore::open(data_dir));
        let global_password = resolve_global_password(data_dir);
        let encrypted = Arc::new(EncryptedPrefs::open(data_dir, &global_password));
        let registry = SettingsRegistry::new(store.clone());

        Settings {
            last_run_version: registry::registry_string(registry_store.clone(), "lastRunVersion"),
            last_announcement_id: registry::registry_string(registry_store.clone(), "lastAnnouncementId"),
            last_run_time: registry::registry_offset_date_time(registry_store.clone(), "lastRunTime"),
            last_audit_query: registry::registry_offset_date_time(registry_store.clone(), "lastAuditQuery"),
            last_instances_check: registry::registry_offset_date_time(registry_store.clone(), "lastInstancesCheck"),
            last_auth_refresh: registry::registry_offset_date_time(registry_store.clone(), "lastAuthRefresh"),
            last_update_check: registry::registry_offset_date_time(registry_store.clone(), "lastUpdateCheck"),
            last_announcement_check: registry::registry_offset_date_time(registry_store.clone(), "lastAnnouncementCheck"),
            next_poll_action: registry::registry_offset_date_time(registry_store.clone(), "nextPollAction"),
            next_mod_summary: registry::registry_offset_date_time(registry_store.clone(), "nextModSummary"),
            next_outstanding_mod: registry::registry_offset_date_time(registry_store.clone(), "nextOutstandingMod"),
            last_cache_cleanup: registry::registry_offset_date_time(registry_store.clone(), "lastCacheCleanup"),
            last_instance_joined: registry::registry_local_date_time(registry_store.clone(), "lastInstanceJoined"),
            ui_bounds: registry::registry_rectangle(registry_store.clone(), "uiBounds"),

            heuristic_kick_count: registry.int_range("heuristicKickCount", "Heuristic Kick Count", 3, 1, 10),
            heuristic_period_days: registry.int_range("heuristicPeriodDays", "Heuristic Period (days)", 3, 1, 30),
            outstanding_period_days: registry.int_range("outstandingPeriodDays", "Outstanding Period (days)", 3, 1, 30),
            cache_cleanup_enabled: registry.boolean("cache_cleanup_enabled", "Auto-delete old cached files", true),
            cache_cleanup_days: registry.int_range("cache_cleanup_days", "Delete files older than (days)", 7, 1, 30),

            registry,
            store,
            registry_store,
            encrypted,
            settings_file: settings_path,
            settings_file_last_modified,
            has_version_changed_since_last_run: std::sync::Mutex::new(None),
        }
    }

    /// Port of `checkHasVersionChangedSinceLastRun`.
    pub fn check_has_version_changed_since_last_run(&self, current_version: &str) -> bool {
        if let Some(cached) = *self.has_version_changed_since_last_run.lock().unwrap() {
            return cached;
        }
        let changed = match self.last_run_time.get_or_null() {
            None => true,
            Some(last_run_time) => {
                let settings_modified: DateTime<Utc> =
                    self.settings_file_last_modified.map(DateTime::<Utc>::from).unwrap_or_else(Utc::now);
                if settings_modified > last_run_time + chrono::Duration::milliseconds(1_000) {
                    true
                } else {
                    self.last_run_version.get_or_null().as_deref() != Some(current_version)
                }
            }
        };
        *self.has_version_changed_since_last_run.lock().unwrap() = Some(changed);
        changed
    }

    /// Port of `updateRunVersionAndTime`.
    pub fn update_run_version_and_time(&self, current_version: &str) {
        self.last_run_version.set(Some(current_version.to_string()));
        self.last_run_time.set(Some(Utc::now()));
    }

    pub fn settings_file(&self) -> &Path {
        &self.settings_file
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_has_changed_version() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        assert!(settings.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn matching_version_and_recent_run_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        settings.update_run_version_and_time("0.4.20");
        // Re-check must not be cached from the earlier fresh-install call
        // in this same instance, so open a second handle onto the same dir.
        let settings2 = Settings::open(dir.path());
        assert!(!settings2.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn different_version_is_changed() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        settings.update_run_version_and_time("0.4.19");
        let settings2 = Settings::open(dir.path());
        assert!(settings2.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn global_password_persists_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let a = resolve_global_password(dir.path());
        let b = resolve_global_password(dir.path());
        assert_eq!(a, b);
    }

    #[test]
    fn env_var_overrides_persisted_key() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SCARLET_GLOBAL_PW", "explicit-password");
        assert_eq!(resolve_global_password(dir.path()), "explicit-password");
        std::env::remove_var("SCARLET_GLOBAL_PW");
    }

    #[test]
    fn heuristic_settings_have_javas_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        assert_eq!(settings.heuristic_kick_count.get(), 3);
        assert_eq!(settings.heuristic_period_days.get(), 3);
        assert_eq!(settings.outstanding_period_days.get(), 3);
        assert!(settings.cache_cleanup_enabled.get());
        assert_eq!(settings.cache_cleanup_days.get(), 7);
    }
}
