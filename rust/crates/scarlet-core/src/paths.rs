//! Port of `Scarlet.java`'s data-folder resolution (its static initialiser,
//! `defaultDataDir`, `settingsFileUsable`, `findAllUsableDataDirs`), plus
//! the remembered-choice step this port adds.
//!
//! The Rust port previously fell back to the process's working directory,
//! so running the binary from a checkout scattered `settings.json`,
//! `secure-prefs.json`, `logs/` and `backups/` into whatever folder you
//! happened to be in — and a second run from elsewhere silently started
//! from a blank config. Java never does that: it resolves a per-user data
//! directory.
//!
//! Resolution order:
//!
//! 1. `CELESTE_HOME` (or `SCARLET_HOME`, still honoured), if set — an
//!    explicit choice that is never overridden. A value of `";"` means
//!    "next to the executable", as in Java (where it means the jar's
//!    directory).
//! 2. A directory remembered from a previous run's choice (see
//!    [`remembered_data_dir`]); this is the part Java has no equivalent for,
//!    because Java never asks.
//! 3. The platform default, if it already holds a usable `settings.json`.
//! 4. Auto-discovery: any other known Celeste data location holding a
//!    usable `settings.json` — an older default, an old Windows-style path
//!    used on Linux by earlier builds, a bare `~/Scarlet`, or the
//!    executable's own directory. This is what makes an install that has
//!    drifted between versions keep working instead of booting blank.
//! 5. Nothing found: the platform default, which the caller may offer to
//!    change before anything is written (a first run).

use std::path::{Path, PathBuf};

use crate::identity::{GROUP, LEGACY_NAME, NAME, UPSTREAM_GROUP};

/// Where the remembered choice is stored, relative to the platform default
/// directory. A pointer file rather than a setting, because it has to be
/// readable *before* any settings file has been located.
const POINTER_FILE: &str = "data-dir.txt";

/// How the data directory was decided, so the caller can tell a first run
/// from a resumed one without re-deriving any of this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataDirSource {
    /// `SCARLET_HOME` was set.
    Explicit,
    /// A previous run's remembered choice.
    Remembered,
    /// The platform default, which already holds a config.
    PlatformDefault,
    /// Another known location that holds a config.
    Discovered,
    /// Nothing exists yet: this is a first run, and the caller may ask the
    /// user where the data should live.
    FirstRun,
}

#[derive(Debug, Clone)]
pub struct ResolvedDataDir {
    pub path: PathBuf,
    pub source: DataDirSource,
    /// Other locations that also hold a usable config. Java prompts when
    /// there is more than one rather than silently picking; the caller can
    /// surface these the same way.
    pub other_candidates: Vec<PathBuf>,
}

/// Port of `defaultDataDir`.
pub fn default_data_dir() -> PathBuf {
    default_data_dir_for(GROUP)
}

fn default_data_dir_for(group: &str) -> PathBuf {
    let home = home_dir();
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        if let Some(xdg) = non_empty_env("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join(group).join(NAME);
        }
        return home.join(".local/share").join(group).join(NAME);
    }
    if let Some(local_app_data) = non_empty_env("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join(group).join(NAME);
    }
    if cfg!(windows) {
        return home.join("AppData/Local").join(group).join(NAME);
    }
    home.join(format!(".{group}")).join(NAME)
}

/// Port of `settingsFileUsable`: `settings.json` exists and parses to a
/// non-empty JSON object — a real, loadable config, not missing, empty or
/// corrupt.
pub fn settings_file_usable(dir: &Path) -> bool {
    let file = dir.join("settings.json");
    let Ok(metadata) = std::fs::metadata(&file) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return false;
    }
    match std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    {
        Some(serde_json::Value::Object(map)) => !map.is_empty(),
        _ => false,
    }
}

/// Port of `findAllUsableDataDirs`: every known data location, in Java's
/// order, that holds a usable config and isn't `primary`.
pub fn find_all_usable_data_dirs(primary: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Current Celeste first, then both known Java Scarlet publishers. The
    // upstream build used SybylineNetwork/Scarlet; the maintained fork used
    // KozyBlake/Scarlet before being renamed to Celeste.
    for (group, name) in [
        (GROUP, NAME),
        (GROUP, LEGACY_NAME),
        (UPSTREAM_GROUP, LEGACY_NAME),
    ] {
        candidates.extend(known_data_dir_candidates(group, name));
    }
    if let Some(exe_dir) = executable_dir() {
        candidates.push(exe_dir);
    }

    let primary_key = canonical_key(primary);
    let mut seen: Vec<String> = vec![primary_key];
    let mut found = Vec::new();
    for candidate in candidates {
        if !settings_file_usable(&candidate) {
            continue;
        }
        let key = canonical_key(&candidate);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        found.push(candidate);
    }
    found
}

/// Finds only Java-era Scarlet installations. This powers the Settings
/// importer without offering another Celeste directory as a legacy source.
pub fn find_legacy_data_dirs(primary: &Path) -> Vec<PathBuf> {
    let candidates = [(GROUP, LEGACY_NAME), (UPSTREAM_GROUP, LEGACY_NAME)]
        .into_iter()
        .flat_map(|(group, name)| known_data_dir_candidates(group, name));
    let primary_key = canonical_key(primary);
    let mut seen = vec![primary_key];
    let mut found = Vec::new();
    for candidate in candidates {
        if !settings_file_usable(&candidate) {
            continue;
        }
        let key = canonical_key(&candidate);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        found.push(candidate);
    }
    found
}

fn known_data_dir_candidates(group: &str, name: &str) -> Vec<PathBuf> {
    let home = home_dir();
    let mut candidates = Vec::new();
    if let Some(xdg) = non_empty_env("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(xdg).join(group).join(name));
    }
    candidates.push(home.join(".local/share").join(group).join(name));
    if let Some(local_app_data) = non_empty_env("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join(group).join(name));
    }
    candidates.push(home.join("AppData/Local").join(group).join(name));
    candidates.push(home.join(format!(".{group}")).join(name));
    candidates.push(home.join(name));
    candidates
}

/// The directory a previous run was told to use, if any.
pub fn remembered_data_dir() -> Option<PathBuf> {
    let pointer = default_data_dir().join(POINTER_FILE);
    let text = std::fs::read_to_string(pointer).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Remembers `dir` as the data directory for future runs. Writing the
/// pointer into the platform default location (not into `dir` itself) is
/// what lets the next run find a directory it has no other way to guess.
pub fn remember_data_dir(dir: &Path) -> std::io::Result<()> {
    let default = default_data_dir();
    std::fs::create_dir_all(&default)?;
    std::fs::write(default.join(POINTER_FILE), dir.to_string_lossy().as_bytes())
}

/// Forgets a remembered choice, so resolution falls back to the platform
/// default and discovery.
pub fn forget_data_dir() -> std::io::Result<()> {
    let pointer = default_data_dir().join(POINTER_FILE);
    match std::fs::remove_file(pointer) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Copies durable application data into a new directory without modifying
/// the source. Logs, caches and backups are intentionally skipped: they are
/// regenerable bulk and can be very large on long-running installations.
///
/// This is shared by first-run Scarlet migration and Settings' **Move data
/// folder** action so both paths preserve exactly the same files.
pub fn copy_data_dir(from: &Path, to: &Path) -> std::io::Result<usize> {
    const SKIP: [&str; 3] = ["logs", "caches", "backups"];
    std::fs::create_dir_all(to)?;
    let mut copied = 0usize;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIP.iter().any(|skip| name == std::ffi::OsStr::new(skip)) {
            continue;
        }
        let source = entry.path();
        let destination = to.join(&name);
        if entry.file_type()?.is_dir() {
            copied += copy_data_dir(&source, &destination)?;
        } else {
            std::fs::copy(&source, &destination)?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Resolves the data directory. Creates nothing — the caller decides
/// whether to create it or to ask first (see [`DataDirSource::FirstRun`]).
pub fn resolve_data_dir() -> ResolvedDataDir {
    // `CELESTE_HOME` is the current name; `SCARLET_HOME` keeps working, so
    // existing launch scripts and shortcuts don't break on the rename.
    if let Some(home) = non_empty_env("CELESTE_HOME").or_else(|| non_empty_env("SCARLET_HOME")) {
        let trimmed = home.trim().to_string();
        if trimmed == ";" {
            if let Some(exe_dir) = executable_dir() {
                return ResolvedDataDir {
                    path: exe_dir,
                    source: DataDirSource::Explicit,
                    other_candidates: Vec::new(),
                };
            }
        } else {
            let path = PathBuf::from(&trimmed);
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            return ResolvedDataDir {
                path,
                source: DataDirSource::Explicit,
                other_candidates: Vec::new(),
            };
        }
    }

    if let Some(remembered) = remembered_data_dir() {
        // A moved/deleted drive must not strand Celeste in a new blank
        // folder forever. Ignore stale pointers and continue through normal
        // legacy discovery so the operator can choose again.
        if settings_file_usable(&remembered) {
            let others = find_all_usable_data_dirs(&remembered);
            return ResolvedDataDir {
                path: remembered,
                source: DataDirSource::Remembered,
                other_candidates: others,
            };
        }
    }

    let default = default_data_dir();
    if settings_file_usable(&default) {
        let others = find_all_usable_data_dirs(&default);
        return ResolvedDataDir {
            path: default,
            source: DataDirSource::PlatformDefault,
            other_candidates: others,
        };
    }

    let mut discovered = find_all_usable_data_dirs(&default);
    if !discovered.is_empty() {
        let chosen = discovered.remove(0);
        return ResolvedDataDir {
            path: chosen,
            source: DataDirSource::Discovered,
            other_candidates: discovered,
        };
    }

    ResolvedDataDir {
        path: default,
        source: DataDirSource::FirstRun,
        other_candidates: Vec::new(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    #[cfg(not(windows))]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile);
    }
    PathBuf::from(".")
}

/// Java uses the jar's directory; the equivalent here is the directory the
/// running binary sits in.
pub fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn canonical_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests set process-wide environment variables, which cargo runs
    /// in parallel threads by default — without a lock they would see each
    /// other's `SCARLET_HOME`/`HOME` and fail intermittently.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TestEnvironment {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl TestEnvironment {
        fn isolated(home: &Path) -> Self {
            let keys = [
                "CELESTE_HOME",
                "SCARLET_HOME",
                "XDG_DATA_HOME",
                "LOCALAPPDATA",
                "HOME",
                "USERPROFILE",
            ];
            let saved = keys
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect();
            std::env::remove_var("CELESTE_HOME");
            std::env::remove_var("SCARLET_HOME");
            for key in ["XDG_DATA_HOME", "LOCALAPPDATA", "HOME", "USERPROFILE"] {
                std::env::set_var(key, home);
            }
            Self { saved }
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn a_config_is_only_usable_when_it_is_a_non_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!settings_file_usable(dir.path()), "missing file");

        std::fs::write(dir.path().join("settings.json"), "").unwrap();
        assert!(!settings_file_usable(dir.path()), "empty file");

        std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
        assert!(!settings_file_usable(dir.path()), "empty object");

        std::fs::write(dir.path().join("settings.json"), "{ not json").unwrap();
        assert!(!settings_file_usable(dir.path()), "corrupt file");

        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"vrchat_group_id":"grp_x"}"#,
        )
        .unwrap();
        assert!(settings_file_usable(dir.path()));
    }

    #[test]
    fn an_explicit_home_wins_under_either_env_var() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();

        std::env::set_var("CELESTE_HOME", dir.path());
        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Explicit);
        assert_eq!(canonical_key(&resolved.path), canonical_key(dir.path()));
        std::env::remove_var("CELESTE_HOME");

        // The pre-rename variable still works, so existing launch scripts
        // don't break.
        std::env::set_var("SCARLET_HOME", dir.path());
        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Explicit);
        assert_eq!(canonical_key(&resolved.path), canonical_key(dir.path()));
        std::env::remove_var("SCARLET_HOME");
    }

    /// The rename must not strand an existing install: a config sitting in
    /// the old KozyBlake/Scarlet folder is found and adopted.
    #[test]
    fn a_pre_rename_data_folder_is_discovered() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fake_home = tempfile::tempdir().unwrap();
        let _environment = TestEnvironment::isolated(fake_home.path());

        let legacy = fake_home.path().join(GROUP).join(LEGACY_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("settings.json"),
            r#"{"vrchat_group_id":"grp_x"}"#,
        )
        .unwrap();

        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Discovered);
        assert_eq!(resolved.path, legacy);

    }

    #[test]
    fn an_upstream_sybyline_scarlet_folder_is_discovered() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fake_home = tempfile::tempdir().unwrap();
        let _environment = TestEnvironment::isolated(fake_home.path());

        let legacy = fake_home.path().join(UPSTREAM_GROUP).join(LEGACY_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("settings.json"),
            r#"{"vrchat_group_id":"grp_upstream"}"#,
        )
        .unwrap();

        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Discovered);
        assert_eq!(resolved.path, legacy);

    }

    #[test]
    fn a_stale_remembered_folder_does_not_hide_a_legacy_install() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fake_home = tempfile::tempdir().unwrap();
        let _environment = TestEnvironment::isolated(fake_home.path());

        let default = default_data_dir();
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(
            default.join(POINTER_FILE),
            fake_home
                .path()
                .join("missing")
                .to_string_lossy()
                .as_bytes(),
        )
        .unwrap();
        let legacy = fake_home.path().join(UPSTREAM_GROUP).join(LEGACY_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("settings.json"), r#"{"legacy":true}"#).unwrap();

        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Discovered);
        assert_eq!(resolved.path, legacy);

    }

    #[test]
    fn migration_copy_keeps_durable_data_and_leaves_the_source_untouched() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(source.path().join("gaud")).unwrap();
        std::fs::create_dir_all(source.path().join("logs")).unwrap();
        std::fs::write(source.path().join("settings.json"), "{\"legacy\":true}").unwrap();
        std::fs::write(source.path().join("moderation_tags.json"), "[]").unwrap();
        std::fs::write(source.path().join("gaud").join("entry.json"), "{}").unwrap();
        std::fs::write(source.path().join("logs").join("scarlet_log.txt"), "bulk").unwrap();

        let copied = copy_data_dir(source.path(), destination.path()).unwrap();

        assert_eq!(copied, 3);
        assert!(destination.path().join("settings.json").is_file());
        assert!(destination.path().join("moderation_tags.json").is_file());
        assert!(destination.path().join("gaud").join("entry.json").is_file());
        assert!(!destination.path().join("logs").exists());
        assert!(source.path().join("settings.json").is_file());
    }

    /// The behaviour that made this necessary: with no config anywhere, the
    /// answer is the platform data directory and a first-run marker — never
    /// the process's working directory.
    #[test]
    fn a_first_run_resolves_to_the_platform_directory_not_the_working_directory() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fake_home = tempfile::tempdir().unwrap();
        let _environment = TestEnvironment::isolated(fake_home.path());

        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::FirstRun);
        assert_eq!(resolved.path, fake_home.path().join(GROUP).join(NAME));
        assert_ne!(resolved.path, std::env::current_dir().unwrap());

    }

    #[test]
    fn a_remembered_choice_is_used_on_the_next_run() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fake_home = tempfile::tempdir().unwrap();
        let chosen = tempfile::tempdir().unwrap();
        let _environment = TestEnvironment::isolated(fake_home.path());

        std::fs::write(
            chosen.path().join("settings.json"),
            r#"{"vrchat_group_id":"grp_remembered"}"#,
        )
        .unwrap();
        remember_data_dir(chosen.path()).unwrap();
        let resolved = resolve_data_dir();
        assert_eq!(resolved.source, DataDirSource::Remembered);
        assert_eq!(resolved.path, chosen.path());

        forget_data_dir().unwrap();
        assert_eq!(resolve_data_dir().source, DataDirSource::FirstRun);
    }
}
