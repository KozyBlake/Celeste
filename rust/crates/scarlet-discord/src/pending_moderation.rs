//! Java-compatible immediate-ban metadata from `pending_moderation_actions.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

static FILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingBanInfo {
    pub owner_snowflake: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct PendingModeration {
    file: PathBuf,
    value: Mutex<serde_json::Value>,
}

impl PendingModeration {
    pub fn open(data_dir: &Path) -> Self {
        let file = data_dir.join("pending_moderation_actions.json");
        let value = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        Self { file, value: Mutex::new(value) }
    }

    pub fn add_ban_info(&self, target: &str, owner: &str) -> anyhow::Result<bool> {
        let added = self.edit_bans(|bans| {
            if bans.contains_key(target) { return false; }
            bans.insert(target.to_string(), PendingBanInfo {
                owner_snowflake: Some(owner.to_string()),
                ..PendingBanInfo::default()
            });
            true
        })?;
        Ok(added)
    }

    pub fn is_ban_info_owner(&self, target: &str, owner: &str) -> bool {
        self.bans().get(target).is_some_and(|info| {
            info.owner_snowflake.as_deref().is_none_or(|stored| stored == owner)
        })
    }

    pub fn set_ban_info_tags(&self, target: &str, tags: Vec<String>) -> anyhow::Result<bool> {
        self.edit_bans(|bans| match bans.get_mut(target) {
            Some(info) => { info.tags = Some(tags); true }
            None => false,
        })
    }

    pub fn set_ban_info_description(&self, target: &str, description: String) -> anyhow::Result<bool> {
        self.edit_bans(|bans| match bans.get_mut(target) {
            Some(info) => { info.description = Some(description); true }
            None => false,
        })
    }

    pub fn take_ban_info(&self, target: &str) -> anyhow::Result<Option<PendingBanInfo>> {
        self.edit_bans(|bans| bans.remove(target))
    }

    /// The audit poller does not run inside Serenity's type map, so it opens
    /// the same Java-compatible file at the moment a ban audit arrives.
    pub fn take_from_dir(data_dir: &Path, target: &str) -> anyhow::Result<Option<PendingBanInfo>> {
        Self::open(data_dir).take_ban_info(target)
    }

    fn bans(&self) -> HashMap<String, PendingBanInfo> {
        let _file_guard = FILE_LOCK.lock().unwrap();
        let mut value = self.value.lock().unwrap();
        if let Some(fresh) = self.read_disk() {
            *value = fresh;
        }
        value.get("pendingBanInfo").cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    fn edit_bans<T>(&self, edit: impl FnOnce(&mut HashMap<String, PendingBanInfo>) -> T) -> anyhow::Result<T> {
        let _file_guard = FILE_LOCK.lock().unwrap();
        let mut value = self.value.lock().unwrap();
        if let Some(fresh) = self.read_disk() {
            *value = fresh;
        }
        let mut bans: HashMap<String, PendingBanInfo> = value.get("pendingBanInfo").cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let output = edit(&mut bans);
        value.as_object_mut().expect("object established by open").insert(
            "pendingBanInfo".to_string(), serde_json::to_value(bans)?);
        let text = serde_json::to_string_pretty(&*value)?;
        scarlet_core::file_backups::write(&self.file, &text)?;
        Ok(output)
    }

    fn read_disk(&self) -> Option<serde_json::Value> {
        std::fs::read_to_string(&self.file).ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .filter(serde_json::Value::is_object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_java_sibling_fields_and_round_trips_ban_info() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pending_moderation_actions.json"), r#"{"timedBans":{"usr_x":{"expiresAt":1}}}"#).unwrap();
        let store = PendingModeration::open(dir.path());
        assert!(store.add_ban_info("usr_target", "123").unwrap());
        assert!(!store.add_ban_info("usr_target", "456").unwrap());
        assert!(store.set_ban_info_tags("usr_target", vec!["spam".into()]).unwrap());
        assert!(store.set_ban_info_description("usr_target", "reason".into()).unwrap());
        let info = store.take_ban_info("usr_target").unwrap().unwrap();
        assert_eq!(info.owner_snowflake.as_deref(), Some("123"));
        assert_eq!(info.tags.unwrap(), vec!["spam"]);
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("pending_moderation_actions.json")).unwrap()).unwrap();
        assert!(saved["timedBans"]["usr_x"].is_object());
    }
}
