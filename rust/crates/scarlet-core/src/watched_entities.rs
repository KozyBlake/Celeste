//! Port of `net.sybyline.scarlet.ScarletWatchedEntities<E>`: the same
//! watch-list shape as [`crate::watched_groups::WatchedGroups`] (id ->
//! type/tags/priority/critical/silent/message/notes), generalized over
//! which kind of VRChat entity id is valid instead of Java's type
//! parameter `E` (used there only to pick an embed-building callback,
//! which this port doesn't need -- see the `commands` module docs on
//! dropping Java's paginated embeds).
//!
//! Java constructs two instances directly in `Scarlet`'s field
//! initializers (`watchedUsers`/`watchedAvatars`, `watched_users.json`/
//! `watched_avatars.json`), each with its own id-validity `Pattern`
//! (`VrcIds.id_user`/`id_avatar`). This port reuses one generic
//! [`WatchedEntities`] struct for both, taking the validator as a plain
//! `fn(&str) -> bool` rather than duplicating
//! [`crate::watched_groups::WatchedGroups`]'s hardcoded-prefix version --
//! that struct is left as-is (already shipped with its own `grp_`-specific
//! shape) rather than retrofitted, since the two aren't otherwise coupled.
//!
//! Reuses [`crate::watched_groups::WatchedGroupType`] for `type` (Java has
//! a second, textually-identical `ScarletWatchedEntities.WatchedEntity.Type`
//! enum; duplicating it in Rust would just be two copies of the same six
//! variants).
//!
//! **Not ported in this pass**: `importLegacyCSV`/`importJson`/`exportJson`
//! and `translateAdvisories`/`restoreAdvisories` -- same gaps, same
//! reasoning, as `watched_groups`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::unique_strings::UniqueStrings;
use crate::watched_groups::WatchedGroupType;

/// Port of `VrcIds.id_user`: a legacy 10-character alphanumeric id, or
/// `usr_<uuid>`.
pub fn is_valid_user_id(id: &str) -> bool {
    let uuid = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
    let pattern = format!("^([a-zA-Z0-9]{{10}}|usr_{uuid})$");
    regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(id))
}

/// Port of `VrcIds.id_avatar`: `avtr_<uuid>`.
pub fn is_valid_avatar_id(id: &str) -> bool {
    let uuid = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
    let pattern = format!("^avtr_{uuid}$");
    regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(id))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedEntity {
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
}

impl WatchedEntity {
    pub fn new(id: impl Into<String>) -> Self {
        WatchedEntity {
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
        }
    }
}

pub struct WatchedEntities {
    file: PathBuf,
    validate_id: fn(&str) -> bool,
    entities: Mutex<HashMap<String, WatchedEntity>>,
}

impl WatchedEntities {
    /// Port of the constructor + `load()`: only entries whose `id` passes
    /// `validate_id` are kept, matching Java's `idValid.test(id)` filter.
    pub fn open(dir: &Path, file_name: &str, validate_id: fn(&str) -> bool) -> Self {
        let file = dir.join(file_name);
        let entities: HashMap<String, WatchedEntity> = match std::fs::read_to_string(&file) {
            Ok(contents) => match serde_json::from_str::<Vec<Option<WatchedEntity>>>(&contents) {
                Ok(loaded) => loaded
                    .into_iter()
                    .flatten()
                    .filter(|e| validate_id(&e.id))
                    .map(|e| (e.id.clone(), e))
                    .collect(),
                Err(err) => {
                    tracing::error!("Exception loading watched entities {}: {}", file.display(), err);
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };
        let list = WatchedEntities { file, validate_id, entities: Mutex::new(entities) };
        if !list.file.is_file() {
            list.save();
        }
        list
    }

    fn save(&self) {
        let guard = self.entities.lock().unwrap();
        let entities: Vec<&WatchedEntity> = guard.values().collect();
        match serde_json::to_string_pretty(&entities) {
            Ok(json) => {
                if let Err(err) = crate::file_backups::write(&self.file, &json) {
                    tracing::error!("Exception saving watched entities {}: {}", self.file.display(), err);
                }
            }
            Err(err) => tracing::error!("Exception serializing watched entities {}: {}", self.file.display(), err),
        }
    }

    pub fn is_valid_id(&self, id: &str) -> bool {
        (self.validate_id)(id)
    }

    pub fn get_watched_entity(&self, id: &str) -> Option<WatchedEntity> {
        self.entities.lock().unwrap().get(id).cloned()
    }

    pub fn get_all(&self) -> Vec<WatchedEntity> {
        self.entities.lock().unwrap().values().cloned().collect()
    }

    /// See [`crate::watched_groups::WatchedGroups::add_watched_group`]'s
    /// docs for why this has no `bool` return unlike Java's.
    pub fn add_watched_entity(&self, entity: WatchedEntity) {
        self.entities.lock().unwrap().insert(entity.id.clone(), entity);
        self.save();
    }

    pub fn remove_watched_entity(&self, id: &str) -> bool {
        let removed = self.entities.lock().unwrap().remove(id).is_some();
        if removed {
            self.save();
        }
        removed
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut WatchedEntity)) -> Option<WatchedEntity> {
        let updated = {
            let mut entities = self.entities.lock().unwrap();
            let entity = entities.get_mut(id)?;
            f(entity);
            entity.clone()
        };
        self.save();
        Some(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_id_validator_accepts_legacy_and_usr_prefixed() {
        assert!(is_valid_user_id("abcdefghij"));
        assert!(is_valid_user_id("usr_12345678-1234-1234-1234-123456789012"));
        assert!(!is_valid_user_id("avtr_12345678-1234-1234-1234-123456789012"));
        assert!(!is_valid_user_id("short"));
    }

    #[test]
    fn avatar_id_validator_requires_avtr_prefix() {
        assert!(is_valid_avatar_id("avtr_12345678-1234-1234-1234-123456789012"));
        assert!(!is_valid_avatar_id("usr_12345678-1234-1234-1234-123456789012"));
        assert!(!is_valid_avatar_id("avtr_not-a-uuid"));
    }

    #[test]
    fn add_get_remove_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let entities = WatchedEntities::open(dir.path(), "watched_users.json", is_valid_user_id);
        entities.add_watched_entity(WatchedEntity::new("usr_12345678-1234-1234-1234-123456789012"));
        assert!(entities.get_watched_entity("usr_12345678-1234-1234-1234-123456789012").is_some());
        assert!(entities.remove_watched_entity("usr_12345678-1234-1234-1234-123456789012"));
        assert!(entities.get_watched_entity("usr_12345678-1234-1234-1234-123456789012").is_none());
    }

    #[test]
    fn open_filters_out_invalid_ids() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("watched_avatars.json");
        std::fs::write(&file, r#"[{"id":"avtr_12345678-1234-1234-1234-123456789012"},{"id":"usr_bad"}]"#).unwrap();
        let entities = WatchedEntities::open(dir.path(), "watched_avatars.json", is_valid_avatar_id);
        assert_eq!(entities.get_all().len(), 1);
    }

    #[test]
    fn update_mutates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let entities = WatchedEntities::open(dir.path(), "watched_users.json", is_valid_user_id);
        entities.add_watched_entity(WatchedEntity::new("abcdefghij"));
        let updated = entities.update("abcdefghij", |e| e.critical = true).unwrap();
        assert!(updated.critical);
        assert!(entities.get_watched_entity("abcdefghij").unwrap().critical);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let entities = WatchedEntities::open(dir.path(), "watched_users.json", is_valid_user_id);
            let mut entity = WatchedEntity::new("abcdefghij");
            entity.priority = 3;
            entities.add_watched_entity(entity);
        }
        let reopened = WatchedEntities::open(dir.path(), "watched_users.json", is_valid_user_id);
        assert_eq!(reopened.get_watched_entity("abcdefghij").unwrap().priority, 3);
    }
}
