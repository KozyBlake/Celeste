//! Additive import of durable Java Scarlet data into an existing Celeste
//! installation.
//!
//! This deliberately does not import configuration or credentials. Lists
//! and historical records are merged by their stable id; when both folders
//! contain the same record, Celeste's existing value wins and missing fields
//! from Scarlet are added. The import is staged across a restart so no live
//! in-memory store can overwrite the merged files during shutdown.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REQUEST_FILE: &str = "pending-legacy-import.txt";
const RESULT_FILE: &str = "legacy-import-result.json";

const ARRAY_FILES: [(&str, Option<&str>, Option<usize>); 8] = [
    ("staff_list.json", None, None),
    ("secret_staff_list.json", None, None),
    ("moderation_tags.json", Some("value"), Some(crate::moderation_tags::MAX_TAGS)),
    ("watched_groups.json", Some("id"), None),
    ("watched_users.json", Some("id"), None),
    ("watched_avatars.json", Some("id"), None),
    ("good_pronoun.json", None, None),
    ("bad_pronoun.json", None, None),
];
const OBJECT_FILES: [&str; 2] = ["global.json", "discord_perms.json"];
const RECORD_DIRECTORIES: [&str; 3] = ["usr", "gaud", "ex"];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// Add records Scarlet has that Celeste does not. Existing Celeste
    /// values win conflicts.
    #[default]
    Merge,
    /// Replace each recognized overlapping data file/record with Scarlet's
    /// copy. Configuration and credentials remain excluded in every mode.
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportRequest {
    source: PathBuf,
    #[serde(default)]
    mode: ImportMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportPlan {
    pub overlapping_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReport {
    pub source: PathBuf,
    pub backup: Option<PathBuf>,
    pub files_changed: usize,
    pub records_added: usize,
    pub conflicts_preserved: usize,
    pub files_replaced: usize,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "Scarlet import finished: imported {} record(s) across {} file(s); replaced {} overlapping file(s) and preserved {} existing Celeste conflict(s). Settings and credentials were not imported for security. Double-check the imported data before relying on it. Source left untouched: {}",
            self.records_added,
            self.files_changed,
            self.files_replaced,
            self.conflicts_preserved,
            self.source.display(),
        );
        if let Some(backup) = &self.backup {
            summary.push_str(&format!(" Backup: {}.", backup.display()));
        }
        summary
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredResult {
    message: String,
    success: bool,
}

pub fn queue(data_dir: &Path, source: &Path, mode: ImportMode) -> Result<()> {
    anyhow::ensure!(
        source != data_dir,
        "the Scarlet and Celeste folders are the same"
    );
    anyhow::ensure!(
        crate::paths::settings_file_usable(source),
        "{} is not a readable Scarlet data folder",
        source.display()
    );
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(
        data_dir.join(REQUEST_FILE),
        serde_json::to_string_pretty(&ImportRequest { source: source.to_path_buf(), mode })?,
    )?;
    Ok(())
}

/// Runs a queued import before application stores are opened.
pub fn run_pending(data_dir: &Path) -> Option<Result<ImportReport>> {
    run_pending_with_progress(data_dir, |_, _| {})
}

pub fn run_pending_with_progress(
    data_dir: &Path,
    mut progress: impl FnMut(f32, &str),
) -> Option<Result<ImportReport>> {
    let request = data_dir.join(REQUEST_FILE);
    let text = std::fs::read_to_string(&request).ok()?;
    // Requests made by the first importer build were a plain source path.
    // Retain that format so an update between queue and restart cannot lose
    // the user's requested migration.
    let request_data = serde_json::from_str::<ImportRequest>(&text).unwrap_or_else(|_| ImportRequest {
        source: PathBuf::from(text.trim()),
        mode: ImportMode::Merge,
    });
    let source = request_data.source;
    let result = import_with_progress(&source, data_dir, request_data.mode, &mut progress);
    let stored = match &result {
        Ok(report) => StoredResult { message: report.summary(), success: true },
        Err(error) => StoredResult {
            message: format!(
                "Scarlet import failed: {error}. Celeste settings were not changed and the source folder remains untouched: {}",
                source.display()
            ),
            success: false,
        },
    };
    if let Ok(text) = serde_json::to_string_pretty(&stored) {
        let _ = std::fs::write(data_dir.join(RESULT_FILE), text);
    }
    let _ = std::fs::remove_file(request);
    Some(result)
}

/// Returns the last queued-import result once, for the Settings notice.
pub fn take_result(data_dir: &Path) -> Option<(bool, String)> {
    let path = data_dir.join(RESULT_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let result: StoredResult = serde_json::from_str(&text).ok()?;
    let _ = std::fs::remove_file(path);
    Some((result.success, result.message))
}

pub fn merge(source: &Path, destination: &Path) -> Result<ImportReport> {
    import_with_progress(source, destination, ImportMode::Merge, &mut |_, _| {})
}

pub fn inspect(source: &Path, destination: &Path) -> Result<ImportPlan> {
    anyhow::ensure!(crate::paths::settings_file_usable(source), "{} is not a readable Scarlet data folder", source.display());
    let mut overlapping_files = Vec::new();
    for (file, _, _) in ARRAY_FILES {
        if source.join(file).is_file() && destination.join(file).is_file() {
            overlapping_files.push(file.to_string());
        }
    }
    for file in OBJECT_FILES {
        if source.join(file).is_file() && destination.join(file).is_file() {
            overlapping_files.push(file.to_string());
        }
    }
    for directory in RECORD_DIRECTORIES {
        let source_dir = source.join(directory);
        let Ok(entries) = std::fs::read_dir(&source_dir) else { continue };
        let count = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file() && destination.join(directory).join(entry.file_name()).is_file())
            .count();
        if count > 0 {
            overlapping_files.push(format!("{directory}/ ({count} record{})", if count == 1 { "" } else { "s" }));
        }
    }
    Ok(ImportPlan { overlapping_files })
}

pub fn import_with_progress(
    source: &Path,
    destination: &Path,
    mode: ImportMode,
    progress: &mut impl FnMut(f32, &str),
) -> Result<ImportReport> {
    anyhow::ensure!(
        source != destination,
        "the Scarlet and Celeste folders are the same"
    );
    anyhow::ensure!(
        crate::paths::settings_file_usable(source),
        "{} is not a readable Scarlet data folder",
        source.display()
    );
    std::fs::create_dir_all(destination)?;
    progress(0.02, "Backing up existing Celeste data...");
    let backup = create_backup_if_needed(destination)?;
    let mut report = ImportReport {
        source: source.to_path_buf(),
        backup,
        ..ImportReport::default()
    };

    let total = ARRAY_FILES.len() + OBJECT_FILES.len() + RECORD_DIRECTORIES.len();
    let mut completed = 0usize;
    for (file, key, maximum) in ARRAY_FILES {
        progress(0.05 + 0.9 * completed as f32 / total as f32, &format!("Importing {file}..."));
        match mode {
            ImportMode::Merge => merge_array_file(source, destination, file, key, maximum, &mut report)?,
            ImportMode::Replace => replace_json_file(source, destination, file, true, &mut report)?,
        }
        completed += 1;
    }
    for file in OBJECT_FILES {
        progress(0.05 + 0.9 * completed as f32 / total as f32, &format!("Importing {file}..."));
        match mode {
            ImportMode::Merge => merge_object_file(source, destination, file, &mut report)?,
            ImportMode::Replace => replace_json_file(source, destination, file, false, &mut report)?,
        }
        completed += 1;
    }
    for directory in RECORD_DIRECTORIES {
        progress(0.05 + 0.9 * completed as f32 / total as f32, &format!("Importing {directory}/ records..."));
        merge_record_directory(source, destination, directory, mode, &mut report)?;
        completed += 1;
    }

    progress(1.0, "Scarlet data import complete.");

    Ok(report)
}

fn create_backup_if_needed(destination: &Path) -> Result<Option<PathBuf>> {
    let has_existing_data = std::fs::read_dir(destination)?
        .filter_map(Result::ok)
        .any(|entry| {
            !matches!(
                entry.file_name().to_string_lossy().as_ref(),
                "logs" | "caches" | "backups" | REQUEST_FILE | RESULT_FILE
            )
        });
    if !has_existing_data {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = destination
        .join("backups")
        .join(format!("before-scarlet-import-{stamp}"));
    crate::paths::copy_data_dir(destination, &backup)
        .with_context(|| format!("backing up Celeste data to {}", backup.display()))?;
    Ok(Some(backup))
}

fn read_json(path: &Path) -> Result<Option<Value>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(value))
}

fn write_json(path: &Path, value: &Value, keep_backups: bool) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    if keep_backups {
        crate::file_backups::write(path, &text)?;
    } else {
        crate::file_backups::write_keep(path, &text, 0)?;
    }
    Ok(())
}

fn replace_json_file(
    source: &Path,
    destination: &Path,
    file: &str,
    expect_array: bool,
    report: &mut ImportReport,
) -> Result<()> {
    let Some(source_value) = read_json(&source.join(file))? else {
        return Ok(());
    };
    anyhow::ensure!(
        (expect_array && source_value.is_array()) || (!expect_array && source_value.is_object()),
        "{} has the wrong JSON shape for a recognized Scarlet data file",
        source.join(file).display(),
    );
    let destination_path = destination.join(file);
    if destination_path.is_file() {
        report.files_replaced += 1;
    }
    report.records_added += value_record_count(&source_value);
    write_json(&destination_path, &source_value, true)?;
    report.files_changed += 1;
    Ok(())
}

fn value_record_count(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.len(),
        Value::Object(values) => values.len(),
        Value::Null => 0,
        _ => 1,
    }
}

fn identity(value: &Value, key: Option<&str>) -> Option<String> {
    match key {
        Some(key) => value.get(key)?.as_str().map(str::to_string),
        None => value
            .as_str()
            .map(|value| value.trim().to_ascii_lowercase()),
    }
}

fn merge_array_file(
    source: &Path,
    destination: &Path,
    file: &str,
    key: Option<&str>,
    maximum: Option<usize>,
    report: &mut ImportReport,
) -> Result<()> {
    let Some(Value::Array(source_values)) = read_json(&source.join(file))? else {
        return Ok(());
    };
    let destination_path = destination.join(file);
    let mut destination_values = match read_json(&destination_path)? {
        Some(Value::Array(values)) => values,
        Some(_) => anyhow::bail!("{} is not a JSON array", destination_path.display()),
        None => Vec::new(),
    };
    let mut known: HashSet<String> = destination_values
        .iter()
        .filter_map(|value| identity(value, key))
        .collect();
    let mut added = 0;
    for value in source_values.into_iter().filter(|value| !value.is_null()) {
        let Some(id) = identity(&value, key) else {
            continue;
        };
        if known.contains(&id) {
            report.conflicts_preserved += 1;
            continue;
        }
        if maximum.is_some_and(|maximum| destination_values.len() >= maximum) {
            report.conflicts_preserved += 1;
            continue;
        }
        known.insert(id);
        destination_values.push(value);
        added += 1;
    }
    if added > 0 {
        write_json(&destination_path, &Value::Array(destination_values), true)?;
        report.files_changed += 1;
        report.records_added += added;
    }
    Ok(())
}

fn merge_object_file(
    source: &Path,
    destination: &Path,
    file: &str,
    report: &mut ImportReport,
) -> Result<()> {
    let Some(mut source_value) = read_json(&source.join(file))? else {
        return Ok(());
    };
    let destination_path = destination.join(file);
    let mut destination_value =
        read_json(&destination_path)?.unwrap_or_else(|| Value::Object(Default::default()));
    let before = report.records_added;
    merge_missing(&mut destination_value, &mut source_value, report);
    if report.records_added > before {
        write_json(&destination_path, &destination_value, true)?;
        report.files_changed += 1;
    }
    Ok(())
}

fn merge_missing(destination: &mut Value, source: &mut Value, report: &mut ImportReport) {
    match (destination, source) {
        (Value::Object(destination), Value::Object(source)) => {
            for (key, mut source_value) in std::mem::take(source) {
                if let Some(destination_value) = destination.get_mut(&key) {
                    merge_missing(destination_value, &mut source_value, report);
                } else {
                    destination.insert(key, source_value);
                    report.records_added += 1;
                }
            }
        }
        (Value::Array(destination), Value::Array(source)) => {
            for value in std::mem::take(source) {
                if destination.contains(&value) {
                    report.conflicts_preserved += 1;
                } else {
                    destination.push(value);
                    report.records_added += 1;
                }
            }
        }
        (destination, source) if destination != source => report.conflicts_preserved += 1,
        _ => {}
    }
}

fn merge_record_directory(
    source: &Path,
    destination: &Path,
    directory: &str,
    mode: ImportMode,
    report: &mut ImportReport,
) -> Result<()> {
    let source_dir = source.join(directory);
    let Ok(entries) = std::fs::read_dir(&source_dir) else {
        return Ok(());
    };
    let destination_dir = destination.join(directory);
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination_dir.join(entry.file_name());
        let Some(mut source_value) = read_json(&source_path)? else {
            continue;
        };
        if mode == ImportMode::Replace {
            if destination_path.is_file() {
                report.files_replaced += 1;
            }
            report.records_added += value_record_count(&source_value).max(1);
            write_json(&destination_path, &source_value, false)?;
            report.files_changed += 1;
            continue;
        }
        let mut destination_value =
            read_json(&destination_path)?.unwrap_or_else(|| Value::Object(Default::default()));
        let before = report.records_added;
        merge_missing(&mut destination_value, &mut source_value, report);
        if report.records_added > before {
            write_json(&destination_path, &destination_value, false)?;
            report.files_changed += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_import_preserves_celeste_conflicts_and_skips_settings() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::write(
            destination.path().join("settings.json"),
            r#"{"celeste":true}"#,
        )
        .unwrap();
        std::fs::write(
            source.path().join("staff_list.json"),
            r#"["usr_shared","usr_old"]"#,
        )
        .unwrap();
        std::fs::write(
            destination.path().join("staff_list.json"),
            r#"["usr_shared","usr_new"]"#,
        )
        .unwrap();
        std::fs::write(
            source.path().join("watched_groups.json"),
            r#"[{"id":"grp_shared","notes":"Scarlet"},{"id":"grp_old"}]"#,
        )
        .unwrap();
        std::fs::write(
            destination.path().join("watched_groups.json"),
            r#"[{"id":"grp_shared","notes":"Celeste"},{"id":"grp_new"}]"#,
        )
        .unwrap();

        let report = merge(source.path(), destination.path()).unwrap();
        let settings = std::fs::read_to_string(destination.path().join("settings.json")).unwrap();
        let staff: Vec<String> = serde_json::from_str(
            &std::fs::read_to_string(destination.path().join("staff_list.json")).unwrap(),
        )
        .unwrap();
        let groups: Vec<Value> = serde_json::from_str(
            &std::fs::read_to_string(destination.path().join("watched_groups.json")).unwrap(),
        )
        .unwrap();

        assert_eq!(settings, r#"{"celeste":true}"#);
        assert_eq!(staff, ["usr_shared", "usr_new", "usr_old"]);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0]["notes"], "Celeste");
        assert_eq!(report.records_added, 2);
        assert_eq!(report.conflicts_preserved, 2);
        let backup = report.backup.expect("existing Celeste data is backed up");
        assert_eq!(
            std::fs::read_to_string(backup.join("settings.json")).unwrap(),
            r#"{"celeste":true}"#
        );
    }

    #[test]
    fn per_record_objects_add_missing_fields_without_replacing_existing_values() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::create_dir_all(source.path().join("usr")).unwrap();
        std::fs::create_dir_all(destination.path().join("usr")).unwrap();
        std::fs::write(
            source.path().join("usr/usr_1"),
            r#"{"note":"old","legacy":true,"auditEntryIds":["gaud_old"]}"#,
        )
        .unwrap();
        std::fs::write(
            destination.path().join("usr/usr_1"),
            r#"{"note":"new","auditEntryIds":["gaud_new"]}"#,
        )
        .unwrap();

        merge(source.path(), destination.path()).unwrap();
        let merged: Value = serde_json::from_str(
            &std::fs::read_to_string(destination.path().join("usr/usr_1")).unwrap(),
        )
        .unwrap();
        assert_eq!(merged["note"], "new");
        assert_eq!(merged["legacy"], true);
        assert_eq!(
            merged["auditEntryIds"],
            serde_json::json!(["gaud_new", "gaud_old"])
        );
    }

    #[test]
    fn replacement_mode_is_queued_and_replaces_only_recognized_data() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::write(destination.path().join("settings.json"), r#"{"celeste":true}"#).unwrap();
        std::fs::write(source.path().join("watched_groups.json"), r#"[{"id":"grp_old"}]"#).unwrap();
        std::fs::write(destination.path().join("watched_groups.json"), r#"[{"id":"grp_new"}]"#).unwrap();

        let plan = inspect(source.path(), destination.path()).unwrap();
        assert_eq!(plan.overlapping_files, ["watched_groups.json"]);
        queue(destination.path(), source.path(), ImportMode::Replace).unwrap();
        let mut progress = Vec::new();
        let report = run_pending_with_progress(destination.path(), |fraction, label| {
            progress.push((fraction, label.to_string()));
        })
        .expect("request exists")
        .unwrap();

        assert_eq!(report.files_replaced, 1);
        assert_eq!(
            std::fs::read_to_string(destination.path().join("watched_groups.json")).unwrap(),
            "[\n  {\n    \"id\": \"grp_old\"\n  }\n]"
        );
        assert_eq!(
            std::fs::read_to_string(destination.path().join("settings.json")).unwrap(),
            r#"{"celeste":true}"#
        );
        assert_eq!(progress.first().unwrap().0, 0.02);
        assert_eq!(progress.last().unwrap().0, 1.0);
    }

    #[test]
    fn inspection_summarizes_overlapping_record_directories() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::create_dir_all(source.path().join("usr")).unwrap();
        std::fs::create_dir_all(destination.path().join("usr")).unwrap();
        std::fs::write(source.path().join("usr/usr_same"), "{}").unwrap();
        std::fs::write(destination.path().join("usr/usr_same"), "{}").unwrap();

        assert_eq!(
            inspect(source.path(), destination.path()).unwrap().overlapping_files,
            ["usr/ (1 record)"]
        );
    }
}
