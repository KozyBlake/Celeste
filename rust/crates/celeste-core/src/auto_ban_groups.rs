//! Persistent group affiliations that trigger an automatic group ban.
//!
//! This is intentionally separate from `watched_groups.json`: watching a
//! group is advisory, while adding one here authorizes an external
//! moderation action when a matching player joins the configured group's
//! live instance.  Keeping the files separate makes that boundary visible
//! and prevents an existing watch list from suddenly becoming destructive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoBanGroup {
    pub id: String,
    /// Learned from the same membership response that triggered a match.
    #[serde(
        rename = "lastSeenName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_seen_name: Option<String>,
}

impl AutoBanGroup {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            last_seen_name: None,
        }
    }
}

pub struct AutoBanGroups {
    file: PathBuf,
    groups: Mutex<HashMap<String, AutoBanGroup>>,
}

impl AutoBanGroups {
    pub fn open(dir: &Path) -> Self {
        let file = dir.join("auto_ban_groups.json");
        let groups = std::fs::read_to_string(&file)
            .ok()
            .and_then(|contents| {
                serde_json::from_str::<Vec<Option<AutoBanGroup>>>(&contents)
                    .map_err(|error| {
                        tracing::error!(
                            path = %file.display(),
                            %error,
                            "Could not load automatic-ban groups"
                        );
                    })
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .filter(|group| valid_group_id(&group.id))
            .map(|group| (group.id.clone(), group))
            .collect();
        let store = Self {
            file,
            groups: Mutex::new(groups),
        };
        if !store.file.is_file() {
            store.save();
        }
        store
    }

    pub fn contains(&self, group_id: &str) -> bool {
        self.groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(group_id)
    }

    pub fn get(&self, group_id: &str) -> Option<AutoBanGroup> {
        self.groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(group_id)
            .cloned()
    }

    pub fn get_all(&self) -> Vec<AutoBanGroup> {
        let mut groups: Vec<_> = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        groups.sort_by(|left, right| {
            left.last_seen_name
                .as_deref()
                .unwrap_or(&left.id)
                .to_lowercase()
                .cmp(
                    &right
                        .last_seen_name
                        .as_deref()
                        .unwrap_or(&right.id)
                        .to_lowercase(),
                )
        });
        groups
    }

    pub fn add(&self, group_id: &str) -> Result<bool, &'static str> {
        let group_id = group_id.trim();
        if !valid_group_id(group_id) {
            return Err("Enter a VRChat group ID beginning with grp_.");
        }
        let inserted = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(group_id.to_string(), AutoBanGroup::new(group_id))
            .is_none();
        if inserted {
            self.save();
        }
        Ok(inserted)
    }

    pub fn remove(&self, group_id: &str) -> bool {
        let removed = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(group_id)
            .is_some();
        if removed {
            self.save();
        }
        removed
    }

    /// Saves a newly observed display name without rewriting the file when
    /// nothing changed.
    pub fn record_seen_name(&self, group_id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let changed = {
            let mut groups = self
                .groups
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(group) = groups.get_mut(group_id) else {
                return;
            };
            if group.last_seen_name.as_deref() == Some(name) {
                false
            } else {
                group.last_seen_name = Some(name.to_string());
                true
            }
        };
        if changed {
            self.save();
        }
    }

    fn save(&self) {
        let mut groups: Vec<_> = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        groups.sort_by(|left, right| left.id.cmp(&right.id));
        match serde_json::to_string_pretty(&groups) {
            Ok(json) => {
                if let Err(error) = crate::file_backups::write(&self.file, &json) {
                    tracing::error!(
                        path = %self.file.display(),
                        %error,
                        "Could not save automatic-ban groups"
                    );
                }
            }
            Err(error) => tracing::error!(%error, "Could not serialize automatic-ban groups"),
        }
    }
}

fn valid_group_id(value: &str) -> bool {
    value.starts_with("grp_") && value.len() > 4 && !value.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_separate_empty_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let groups = AutoBanGroups::open(dir.path());
        assert!(groups.get_all().is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("auto_ban_groups.json")).unwrap(),
            "[]"
        );
    }

    #[test]
    fn add_remove_and_name_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let groups = AutoBanGroups::open(dir.path());
        assert!(groups.add("grp_one").unwrap());
        assert!(!groups.add("grp_one").unwrap(), "duplicates are harmless");
        groups.record_seen_name("grp_one", "Example Group");

        let reopened = AutoBanGroups::open(dir.path());
        assert!(reopened.contains("grp_one"));
        assert_eq!(
            reopened.get_all()[0].last_seen_name.as_deref(),
            Some("Example Group")
        );
        assert!(reopened.remove("grp_one"));
        assert!(!reopened.remove("grp_one"));
    }

    #[test]
    fn malformed_ids_never_enter_the_action_list() {
        let dir = tempfile::tempdir().unwrap();
        let groups = AutoBanGroups::open(dir.path());
        assert!(groups.add("usr_not_a_group").is_err());
        assert!(groups.add("grp_bad id").is_err());
        assert!(groups.get_all().is_empty());
    }
}
