//! Port of `net.sybyline.scarlet.ScarletStaffList` and
//! `CelesteSecretStaffList` -- a small JSON-file-backed set of VRChat user
//! ids. Java implements these as two near-identical hand-copied classes
//! (`staff_list.json` / `secret_staff_list.json`, `getStaffIds`/
//! `getSecretStaffIds`, etc., every method body byte-for-byte the same);
//! since the behavior is genuinely identical, this port gives them one
//! generic type instead and lets callers construct two independent
//! instances (`StaffList::open(dir, "staff_list.json")` /
//! `StaffList::open(dir, "secret_staff_list.json")`) -- a Rust-idiomatic
//! consolidation, not a behavior change.
//!
//! Backed by [`crate::file_backups::write`] for the same crash-safe
//! temp-file-then-rename save Java gets from `FileBackups.writer`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct StaffList {
    file: PathBuf,
    ids: Mutex<HashSet<String>>,
}

impl StaffList {
    /// Port of the constructor + `load()`: reads `dir/file_name` if it
    /// exists (a plain JSON array of strings), or starts empty and writes
    /// an empty file, matching Java's `if (!file.isFile()) { save(); }`.
    pub fn open(dir: &Path, file_name: &str) -> Self {
        let file = dir.join(file_name);
        let ids: HashSet<String> = match std::fs::read_to_string(&file) {
            Ok(contents) => match serde_json::from_str::<Vec<Option<String>>>(&contents) {
                Ok(loaded) => loaded.into_iter().flatten().collect(),
                Err(err) => {
                    tracing::error!("Exception loading staff list {}: {}", file.display(), err);
                    HashSet::new()
                }
            },
            Err(_) => HashSet::new(),
        };
        let list = StaffList { file, ids: Mutex::new(ids) };
        if !list.file.is_file() {
            list.save();
        }
        list
    }

    fn save(&self) {
        let ids = self.get_ids();
        match serde_json::to_string_pretty(&ids) {
            Ok(json) => {
                if let Err(err) = crate::file_backups::write(&self.file, &json) {
                    tracing::error!("Exception saving staff list {}: {}", self.file.display(), err);
                }
            }
            Err(err) => tracing::error!("Exception serializing staff list {}: {}", self.file.display(), err),
        }
    }

    pub fn get_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.ids.lock().unwrap().iter().cloned().collect();
        ids.sort();
        ids
    }

    pub fn is_id(&self, user_id: &str) -> bool {
        self.ids.lock().unwrap().contains(user_id)
    }

    /// Returns `false` (and does not save) if `user_id` was already present,
    /// matching Java's `Set::add`-return-value gate.
    pub fn add_id(&self, user_id: &str) -> bool {
        let added = self.ids.lock().unwrap().insert(user_id.to_string());
        if added {
            self.save();
        }
        added
    }

    /// Returns `false` (and does not save) if `user_id` was not present.
    pub fn remove_id(&self, user_id: &str) -> bool {
        let removed = self.ids.lock().unwrap().remove(user_id);
        if removed {
            self.save();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_empty_and_creates_the_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let list = StaffList::open(dir.path(), "staff_list.json");
        assert!(list.get_ids().is_empty());
        assert!(dir.path().join("staff_list.json").is_file());
    }

    #[test]
    fn add_then_remove_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let list = StaffList::open(dir.path(), "staff_list.json");
        assert!(list.add_id("usr_1"));
        assert!(list.is_id("usr_1"));
        assert_eq!(list.get_ids(), vec!["usr_1".to_string()]);
        assert!(list.remove_id("usr_1"));
        assert!(!list.is_id("usr_1"));
    }

    #[test]
    fn adding_a_duplicate_id_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let list = StaffList::open(dir.path(), "staff_list.json");
        assert!(list.add_id("usr_1"));
        assert!(!list.add_id("usr_1"));
    }

    #[test]
    fn removing_a_missing_id_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let list = StaffList::open(dir.path(), "staff_list.json");
        assert!(!list.remove_id("usr_missing"));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let list = StaffList::open(dir.path(), "staff_list.json");
            list.add_id("usr_1");
            list.add_id("usr_2");
        }
        let reopened = StaffList::open(dir.path(), "staff_list.json");
        assert_eq!(reopened.get_ids(), vec!["usr_1".to_string(), "usr_2".to_string()]);
    }

    #[test]
    fn two_instances_with_different_file_names_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let staff = StaffList::open(dir.path(), "staff_list.json");
        let secret = StaffList::open(dir.path(), "secret_staff_list.json");
        staff.add_id("usr_1");
        assert!(staff.is_id("usr_1"));
        assert!(!secret.is_id("usr_1"));
    }

    #[test]
    fn ignores_null_entries_in_a_loaded_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("staff_list.json");
        std::fs::write(&file, r#"["usr_1", null, "usr_2"]"#).unwrap();
        let list = StaffList::open(dir.path(), "staff_list.json");
        assert_eq!(list.get_ids(), vec!["usr_1".to_string(), "usr_2".to_string()]);
    }
}
