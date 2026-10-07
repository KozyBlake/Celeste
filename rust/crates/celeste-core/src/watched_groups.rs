//! Port of `net.sybyline.scarlet.ScarletWatchedGroups`: a JSON-file-backed
//! map of VRChat group id -> watch metadata (`watched_groups.json`), used to
//! flag groups as malicious/nuisance/community/affiliated with a priority,
//! critical/silent flags, a TTS announcement message, and free-form notes
//! and tags.
//!
//! **Not ported in this pass**: `translateAdvisories`/`restoreAdvisories` (the same not-ported
//! LibreTranslate integration as `celeste_core::moderation_tags`).
//! `message_original`/`notes_original`/`translated_lang` are kept on
//! [`WatchedGroup`] purely so a shared data directory round-trips them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::unique_strings::UniqueStrings;

/// Port of `ScarletWatchedGroups.WatchedGroup.Type`. String values use the
/// exact Java `Enum.name()` spelling via explicit `serde(rename)`, matching
/// the convention in `settings::enums`, so a `watched_groups.json` shared
/// with a Java install round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WatchedGroupType {
    #[default]
    #[serde(rename = "UNKNOWN")]
    Unknown,
    #[serde(rename = "MALICIOUS")]
    Malicious,
    #[serde(rename = "NUISANCE")]
    Nuisance,
    #[serde(rename = "COMMUNITY")]
    Community,
    #[serde(rename = "AFFILIATED")]
    Affiliated,
    #[serde(rename = "OTHER")]
    Other,
}

impl WatchedGroupType {
    pub const ALL: [WatchedGroupType; 6] = [
        WatchedGroupType::Unknown,
        WatchedGroupType::Malicious,
        WatchedGroupType::Nuisance,
        WatchedGroupType::Community,
        WatchedGroupType::Affiliated,
        WatchedGroupType::Other,
    ];

    /// The exact Java `Enum.name()` string (`DEnumString::value()`/`display()`
    /// both just return `this.name()`).
    pub fn as_str(&self) -> &'static str {
        match self {
            WatchedGroupType::Unknown => "UNKNOWN",
            WatchedGroupType::Malicious => "MALICIOUS",
            WatchedGroupType::Nuisance => "NUISANCE",
            WatchedGroupType::Community => "COMMUNITY",
            WatchedGroupType::Affiliated => "AFFILIATED",
            WatchedGroupType::Other => "OTHER",
        }
    }

    /// Port of the enum's `text_color` field: the row colour a player gets
    /// in the instance table when they're in a group of this type.
    /// `UNKNOWN` and `OTHER` carry no colour in Java either.
    pub fn text_color(&self) -> Option<(u8, u8, u8)> {
        match self {
            WatchedGroupType::Malicious => Some((235, 95, 95)),
            WatchedGroupType::Nuisance => Some((235, 165, 65)),
            WatchedGroupType::Community => Some((95, 200, 120)),
            WatchedGroupType::Affiliated => Some((95, 165, 235)),
            WatchedGroupType::Unknown | WatchedGroupType::Other => None,
        }
    }

    pub fn parse(value: &str) -> Option<WatchedGroupType> {
        WatchedGroupType::ALL
            .into_iter()
            .find(|t| t.as_str() == value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedGroup {
    pub id: String,
    #[serde(default)]
    pub r#type: WatchedGroupType,
    #[serde(default)]
    pub tags: UniqueStrings,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub critical: bool,
    #[serde(default)]
    pub silent: bool,
    pub message: Option<String>,
    pub notes: Option<String>,
    #[serde(rename = "messageOriginal", default)]
    pub message_original: Option<String>,
    #[serde(rename = "notesOriginal", default)]
    pub notes_original: Option<String>,
    #[serde(rename = "translatedLang", default)]
    pub translated_lang: Option<String>,
    /// The group's name the last time VRChat reported it, learned from the
    /// membership lookups that already happen when someone is flagged.
    ///
    /// A watched entry is keyed by `grp_` id and carries no name of its
    /// own, so a group that renames keeps being detected while every human
    /// reference to it silently goes stale — a moderator checks the profile
    /// for the name in their own `message`, sees a different group, and
    /// concludes the flag was wrong. Recording what VRChat currently calls
    /// it is what makes a rename visible.
    ///
    /// Additive and camelCase, so a `watched_groups.json` shared with the
    /// Java build round-trips: Gson ignores keys it does not know.
    #[serde(rename = "lastSeenName", default)]
    pub last_seen_name: Option<String>,
}

impl WatchedGroup {
    pub fn new(id: impl Into<String>) -> Self {
        WatchedGroup {
            id: id.into(),
            r#type: WatchedGroupType::Unknown,
            tags: UniqueStrings::new(),
            priority: 0,
            critical: false,
            silent: false,
            message: None,
            notes: None,
            message_original: None,
            notes_original: None,
            translated_lang: None,
            last_seen_name: None,
        }
    }
}

pub struct WatchedGroups {
    file: PathBuf,
    groups: Mutex<HashMap<String, WatchedGroup>>,
}

impl WatchedGroups {
    /// Port of the constructor + `load()`: only entries whose `id` starts
    /// with `grp_` are kept, matching Java's filter (defends against a
    /// hand-edited or legacy-imported file carrying a stray/malformed id).
    pub fn open(dir: &Path, file_name: &str) -> Self {
        let file = dir.join(file_name);
        let groups: HashMap<String, WatchedGroup> = match std::fs::read_to_string(&file) {
            Ok(contents) => match serde_json::from_str::<Vec<Option<WatchedGroup>>>(&contents) {
                Ok(loaded) => loaded
                    .into_iter()
                    .flatten()
                    .filter(|g| g.id.starts_with("grp_"))
                    .map(|g| (g.id.clone(), g))
                    .collect(),
                Err(err) => {
                    tracing::error!(
                        "Exception loading watched groups {}: {}",
                        file.display(),
                        err
                    );
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };
        let list = WatchedGroups {
            file,
            groups: Mutex::new(groups),
        };
        if !list.file.is_file() {
            list.save();
        }
        list
    }

    fn save(&self) {
        let guard = self.groups.lock().unwrap_or_else(|e| e.into_inner());
        let groups: Vec<&WatchedGroup> = guard.values().collect();
        match serde_json::to_string_pretty(&groups) {
            Ok(json) => {
                if let Err(err) = crate::file_backups::write(&self.file, &json) {
                    tracing::error!(
                        "Exception saving watched groups {}: {}",
                        self.file.display(),
                        err
                    );
                }
            }
            Err(err) => tracing::error!(
                "Exception serializing watched groups {}: {}",
                self.file.display(),
                err
            ),
        }
    }

    pub fn get_watched_group(&self, group_id: &str) -> Option<WatchedGroup> {
        self.groups.lock().unwrap_or_else(|e| e.into_inner()).get(group_id).cloned()
    }

    pub fn get_all(&self) -> Vec<WatchedGroup> {
        self.groups.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    /// Serializes the current list in the same pretty-printed JSON array
    /// format as the Java `exportJson` implementation.
    pub fn export_json(&self) -> Result<Vec<u8>> {
        let groups: Vec<WatchedGroup> = self.groups.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        serde_json::to_vec_pretty(&groups).context("serializing watched groups")
    }

    /// Imports Java-compatible watched-group JSON. Invalid ids and `null`
    /// array entries are ignored, just as they are when loading the file.
    pub fn import_json(&self, input: &[u8], overwrite: bool) -> Result<usize> {
        let groups: Vec<Option<WatchedGroup>> =
            serde_json::from_slice(input).context("parsing watched groups JSON")?;
        Ok(self.import_groups(groups.into_iter().flatten(), overwrite))
    }

    /// Imports the six-column legacy Excel CSV used by Celeste Java:
    /// id, legacy type, unused, critical, message, tags.
    pub fn import_legacy_csv(&self, input: &[u8], overwrite: bool) -> Result<usize> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(input);
        let mut groups = Vec::new();
        for (index, record) in reader.records().enumerate() {
            let record = record.with_context(|| format!("parsing CSV row {}", index + 1))?;
            if record.len() < 6 {
                anyhow::bail!(
                    "CSV row {} has {} columns; expected at least 6",
                    index + 1,
                    record.len()
                );
            }
            let mut group = WatchedGroup::new(&record[0]);
            group.r#type = legacy_type(&record[1]);
            group.critical = record[3].eq_ignore_ascii_case("true");
            group.message = Some(record[4].to_string());
            for tag in record[5]
                .split([',', ';', '/', '|'])
                .filter(|tag| !tag.is_empty())
            {
                group.tags.add(tag.to_lowercase());
            }
            groups.push(group);
        }
        Ok(self.import_groups(groups, overwrite))
    }

    fn import_groups(
        &self,
        groups: impl IntoIterator<Item = WatchedGroup>,
        overwrite: bool,
    ) -> usize {
        let mut imported = 0;
        {
            let mut current = self.groups.lock().unwrap_or_else(|e| e.into_inner());
            for group in groups
                .into_iter()
                .filter(|group| group.id.starts_with("grp_"))
            {
                if overwrite || !current.contains_key(&group.id) {
                    current.insert(group.id.clone(), group);
                    imported += 1;
                }
            }
        }
        self.save();
        imported
    }

    /// Port of `addWatchedGroup`. Java's version returns `boolean` for a
    /// `null` `watchedGroup` guard that Rust's type system already makes
    /// impossible (a caller can't pass a "null" `WatchedGroup`), so this
    /// just adds it -- always what Java's `true` branch does.
    pub fn add_watched_group(&self, group: WatchedGroup) {
        self.groups.lock().unwrap_or_else(|e| e.into_inner()).insert(group.id.clone(), group);
        self.save();
    }

    pub fn remove_watched_group(&self, group_id: &str) -> bool {
        let removed = self.groups.lock().unwrap_or_else(|e| e.into_inner()).remove(group_id).is_some();
        if removed {
            self.save();
        }
        removed
    }

    /// Records what VRChat currently calls this group, and reports a
    /// rename.
    ///
    /// Returns `Some(previous_name)` only when a *different* name was
    /// already stored — that is a rename worth telling someone about.
    /// First sighting returns `None`: there is nothing to compare against
    /// yet, and reporting every entry as "renamed" the first time Celeste
    /// sees it would be noise that trains people to ignore the signal.
    ///
    /// Saves only when something changed, because this runs on every flag.
    pub fn record_seen_name(&self, group_id: &str, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let previous = {
            let mut groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
            let Some(group) = groups.get_mut(group_id) else { return None };
            match group.last_seen_name.as_deref() {
                Some(stored) if stored == name => return None, // unchanged; no write
                Some(stored) => {
                    let stored = stored.to_string();
                    group.last_seen_name = Some(name.to_string());
                    Some(stored)
                }
                None => {
                    group.last_seen_name = Some(name.to_string());
                    None
                }
            }
        };
        self.save();
        previous
    }

    /// Mutates the watched group in place via `f`, then saves -- the shape
    /// every `set-*`/`add-tag`/`remove-tag` subcommand needs (look up,
    /// check/compare, mutate, save), matching Java's "get the reference,
    /// mutate its fields, call `.save()`" pattern without needing a lock
    /// held across the whole call.
    pub fn update(
        &self,
        group_id: &str,
        f: impl FnOnce(&mut WatchedGroup),
    ) -> Option<WatchedGroup> {
        let updated = {
            let mut groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
            let group = groups.get_mut(group_id)?;
            f(group);
            group.clone()
        };
        self.save();
        Some(updated)
    }
}

fn legacy_type(value: &str) -> WatchedGroupType {
    match value {
        "TOXIC" => WatchedGroupType::Malicious,
        "WATCH" => WatchedGroupType::Nuisance,
        "SPECIAL" => WatchedGroupType::Community,
        "PARTNER" => WatchedGroupType::Affiliated,
        _ => WatchedGroupType::Other,
    }
}

#[cfg(test)]
mod tests {
    /// The confusion this guards: "Black Squad" renamed itself, the watched
    /// entry stores only a grp_ id, and a moderator checking the profile
    /// saw a different group name and concluded the flag was wrong. The
    /// first sighting is not a rename; the second, different one is.
    #[test]
    fn records_the_current_name_and_reports_only_a_real_rename() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        groups.add_watched_group(WatchedGroup::new("grp_1"));

        assert_eq!(groups.record_seen_name("grp_1", "Black Squad"), None, "first sighting is not a rename");
        assert_eq!(groups.get_watched_group("grp_1").unwrap().last_seen_name.as_deref(), Some("Black Squad"));

        assert_eq!(groups.record_seen_name("grp_1", "Black Squad"), None, "same name twice is not a rename");

        assert_eq!(
            groups.record_seen_name("grp_1", "masmavia").as_deref(),
            Some("Black Squad"),
            "a different name is a rename, and reports what it was called before"
        );
        assert_eq!(groups.get_watched_group("grp_1").unwrap().last_seen_name.as_deref(), Some("masmavia"));
    }

    #[test]
    fn ignores_blank_names_and_unknown_groups() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        groups.add_watched_group(WatchedGroup::new("grp_1"));
        assert_eq!(groups.record_seen_name("grp_1", "   "), None);
        assert!(groups.get_watched_group("grp_1").unwrap().last_seen_name.is_none());
        assert_eq!(groups.record_seen_name("grp_missing", "Whatever"), None);
    }

    /// A watched_groups.json shared with the Java build must round-trip:
    /// the new key is additive, and an entry written without it loads.
    #[test]
    fn last_seen_name_round_trips_and_is_optional() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("watched_groups.json"),
            br#"[{"id":"grp_1","type":"MALICIOUS","message":"Kick On Site"}]"#,
        )
        .unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        assert!(groups.get_watched_group("grp_1").unwrap().last_seen_name.is_none());

        groups.record_seen_name("grp_1", "masmavia");
        let reopened = WatchedGroups::open(dir.path(), "watched_groups.json");
        assert_eq!(reopened.get_watched_group("grp_1").unwrap().last_seen_name.as_deref(), Some("masmavia"));
        let raw = std::fs::read_to_string(dir.path().join("watched_groups.json")).unwrap();
        assert!(raw.contains("lastSeenName"), "camelCase key, for Java compatibility");
    }

    use super::*;

    #[test]
    fn watched_group_type_round_trips_java_names() {
        for t in WatchedGroupType::ALL {
            assert_eq!(WatchedGroupType::parse(t.as_str()), Some(t));
        }
        assert_eq!(WatchedGroupType::Malicious.as_str(), "MALICIOUS");
    }

    #[test]
    fn add_get_remove_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        assert!(groups.get_watched_group("grp_1").is_none());
        groups.add_watched_group(WatchedGroup::new("grp_1"));
        assert!(groups.get_watched_group("grp_1").is_some());
        assert!(groups.remove_watched_group("grp_1"));
        assert!(groups.get_watched_group("grp_1").is_none());
        assert!(!groups.remove_watched_group("grp_1"));
    }

    #[test]
    fn open_filters_out_non_group_ids() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("watched_groups.json");
        std::fs::write(&file, r#"[{"id":"grp_1"},{"id":"usr_bad"}]"#).unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        assert_eq!(groups.get_all().len(), 1);
        assert!(groups.get_watched_group("grp_1").is_some());
    }

    #[test]
    fn update_mutates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        groups.add_watched_group(WatchedGroup::new("grp_1"));
        let updated = groups.update("grp_1", |g| g.critical = true).unwrap();
        assert!(updated.critical);
        assert!(groups.get_watched_group("grp_1").unwrap().critical);
    }

    #[test]
    fn update_on_missing_group_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        assert!(groups
            .update("grp_missing", |g| g.critical = true)
            .is_none());
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
            let mut group = WatchedGroup::new("grp_1");
            group.priority = 5;
            group.tags.add("toxic");
            groups.add_watched_group(group);
        }
        let reopened = WatchedGroups::open(dir.path(), "watched_groups.json");
        let group = reopened.get_watched_group("grp_1").unwrap();
        assert_eq!(group.priority, 5);
        assert!(group.tags.contains("toxic"));
    }

    #[test]
    fn imports_legacy_csv_and_exports_json() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        let csv =
            b"grp_one,TOXIC,unused,true,Alert,TagA;TAGB\ninvalid,WATCH,unused,false,nope,tag\n";
        assert_eq!(groups.import_legacy_csv(csv, true).unwrap(), 1);
        let group = groups.get_watched_group("grp_one").unwrap();
        assert_eq!(group.r#type, WatchedGroupType::Malicious);
        assert!(group.critical);
        assert!(group.tags.contains("taga"));
        let exported: Vec<WatchedGroup> =
            serde_json::from_slice(&groups.export_json().unwrap()).unwrap();
        assert_eq!(exported.len(), 1);
    }

    #[test]
    fn json_import_honors_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let groups = WatchedGroups::open(dir.path(), "watched_groups.json");
        let mut original = WatchedGroup::new("grp_one");
        original.priority = 1;
        groups.add_watched_group(original);
        assert_eq!(
            groups
                .import_json(
                    br#"[{"id":"grp_one","priority":9},{"id":"grp_two"}]"#,
                    false
                )
                .unwrap(),
            1
        );
        assert_eq!(groups.get_watched_group("grp_one").unwrap().priority, 1);
        assert_eq!(
            groups
                .import_json(br#"[{"id":"grp_one","priority":9}]"#, true)
                .unwrap(),
            1
        );
        assert_eq!(groups.get_watched_group("grp_one").unwrap().priority, 9);
    }
}
