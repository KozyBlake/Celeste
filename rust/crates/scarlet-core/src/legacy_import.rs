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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReport {
    pub source: PathBuf,
    pub backup: Option<PathBuf>,
    pub files_changed: usize,
    pub records_added: usize,
    pub conflicts_preserved: usize,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "Scarlet import finished: added {} record(s) across {} file(s); preserved {} existing Celeste conflict(s). Settings and credentials were not imported. Source left untouched: {}",
            self.records_added,
            self.files_changed,
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

pub fn queue(data_dir: &Path, source: &Path) -> Result<()> {
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
        source.to_string_lossy().as_bytes(),
    )?;
    Ok(())
}

/// Runs a queued import before application stores are opened.
pub fn run_pending(data_dir: &Path) -> Option<Result<ImportReport>> {
    let request = data_dir.join(REQUEST_FILE);
    let source = std::fs::read_to_string(&request).ok()?;
    let source = PathBuf::from(source.trim());
    let result = merge(&source, data_dir);
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
    let backup = create_backup_if_needed(destination)?;
    let mut report = ImportReport {
        source: source.to_path_buf(),
        backup,
        ..ImportReport::default()
    };

    for file in ["staff_list.json", "secret_staff_list.json"] {
        merge_array_file(source, destination, file, None, None, &mut report)?;
    }
    merge_array_file(
        source,
        destination,
        "moderation_tags.json",
        Some("value"),
        Some(crate::moderation_tags::MAX_TAGS),
        &mut report,
    )?;
    for file in [
        "watched_groups.json",
        "watched_users.json",
        "watched_avatars.json",
    ] {
        merge_array_file(source, destination, file, Some("id"), None, &mut report)?;
    }
    for file in ["good_pronoun.json", "bad_pronoun.json"] {
        merge_array_file(source, destination, file, None, None, &mut report)?;
    }

    for file in ["global.json", "discord_perms.json"] {
        merge_object_file(source, destination, file, &mut report)?;
    }
    for directory in ["usr", "gaud", "ex"] {
        merge_record_directory(source, destination, directory, &mut report)?;
    }

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
}
