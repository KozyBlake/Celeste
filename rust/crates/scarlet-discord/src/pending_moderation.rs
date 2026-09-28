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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedBan {
    pub expires_at: i64,
    pub actor_user_id: String,
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

    pub fn add_pending(&self, event_type: &str, target: &str, actor: &str) -> anyhow::Result<bool> {
        let key = format!("{event_type}:{target}");
        self.edit_pending(|pending| {
            if pending.contains_key(&key) {
                return false;
            }
            pending.insert(key, actor.to_string());
            true
        })
    }

    pub fn take_pending(&self, event_type: &str, target: &str) -> anyhow::Result<Option<String>> {
        let key = format!("{event_type}:{target}");
        self.edit_pending(|pending| pending.remove(&key))
    }

    pub fn take_pending_from_dir(
        data_dir: &Path,
        event_type: &str,
        target: &str,
    ) -> anyhow::Result<Option<String>> {
        Self::open(data_dir).take_pending(event_type, target)
    }

    pub fn set_timed_ban(&self, target: &str, timed_ban: TimedBan) -> anyhow::Result<()> {
        self.edit_timed_bans(|timed| { timed.insert(target.to_string(), timed_ban); })
    }

    pub fn remove_timed_ban(&self, target: &str) -> anyhow::Result<bool> {
        self.edit_timed_bans(|timed| timed.remove(target).is_some())
    }

    pub fn timed_bans(&self) -> HashMap<String, TimedBan> {
        let _file_guard = FILE_LOCK.lock().unwrap();
        let mut value = self.value.lock().unwrap();
        if let Some(fresh) = self.read_disk() { *value = fresh; }
        value.get("timedBans").cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
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

    fn edit_pending<T>(&self, edit: impl FnOnce(&mut HashMap<String, String>) -> T) -> anyhow::Result<T> {
        let _file_guard = FILE_LOCK.lock().unwrap();
        let mut value = self.value.lock().unwrap();
        if let Some(fresh) = self.read_disk() {
            *value = fresh;
        }
        let mut pending: HashMap<String, String> = value.get("pendingModActions").cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let output = edit(&mut pending);
        value.as_object_mut().expect("object established by open").insert(
            "pendingModActions".to_string(), serde_json::to_value(pending)?);
        let text = serde_json::to_string_pretty(&*value)?;
        scarlet_core::file_backups::write(&self.file, &text)?;
        Ok(output)
    }

    fn edit_timed_bans<T>(&self, edit: impl FnOnce(&mut HashMap<String, TimedBan>) -> T) -> anyhow::Result<T> {
        let _file_guard = FILE_LOCK.lock().unwrap();
        let mut value = self.value.lock().unwrap();
        if let Some(fresh) = self.read_disk() { *value = fresh; }
        let mut timed: HashMap<String, TimedBan> = value.get("timedBans").cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let output = edit(&mut timed);
        value.as_object_mut().expect("object established by open").insert(
            "timedBans".to_string(), serde_json::to_value(timed)?);
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

    #[test]
    fn pending_actions_use_java_keys_and_deduplicate() {
        let dir = tempfile::tempdir().unwrap();
        let store = PendingModeration::open(dir.path());
        assert!(store.add_pending("group.user.ban", "usr_target", "usr_actor").unwrap());
        assert!(!store.add_pending("group.user.ban", "usr_target", "usr_other").unwrap());
        assert_eq!(
            store.take_pending("group.user.ban", "usr_target").unwrap().as_deref(),
            Some("usr_actor")
        );
        assert!(store.take_pending("group.user.ban", "usr_target").unwrap().is_none());

        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("pending_moderation_actions.json")).unwrap()
        ).unwrap();
        assert!(saved["pendingModActions"].as_object().unwrap().is_empty());
    }

    #[test]
    fn timed_bans_round_trip_in_java_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = PendingModeration::open(dir.path());
        store.set_timed_ban("usr_target", TimedBan {
            expires_at: 1234,
            actor_user_id: "usr_actor".into(),
        }).unwrap();
        let timed = store.timed_bans();
        assert_eq!(timed["usr_target"].expires_at, 1234);
        assert_eq!(timed["usr_target"].actor_user_id, "usr_actor");
        assert!(store.remove_timed_ban("usr_target").unwrap());
    }
}
