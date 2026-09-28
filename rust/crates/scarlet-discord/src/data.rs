//! Shared application state reachable from slash-command handlers, via
//! serenity's per-client `TypeMap` (`ctx.data`). Java's
//! `ScarletDiscordCommands` just holds a direct field reference to the
//! enclosing `Scarlet`/`ScarletVRChat`/`ScarletData` instances; serenity's
//! event-handler methods don't carry an equivalent "the app" parameter, so
//! this is the idiomatic serenity substitute — insert once before
//! `Client::start`, read from any command handler via `ctx.data.read()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use serenity::prelude::{TypeMap, TypeMapKey};
use tokio::sync::RwLock;

/// The primary VRChat account's client, once (if) logged in. A `RwLock`
/// around an `Option` rather than a plain `Arc<VrChatClient>` because the
/// Discord gateway connection and the VRChat login don't necessarily
/// resolve in either order or at all: the bot can come up before login
/// finishes, before it's even attempted (no stored credentials, 2FA
/// pending in the setup wizard), or after a login attempt fails outright.
/// Every command that needs VRChat must already handle "not logged in"
/// as a real, expected state (Java's `ScarletVRChat` has the same
/// several-auth-states reality) rather than assuming a client exists.
pub type SharedVrChatClient = Arc<RwLock<Option<Arc<scarlet_vrchat::client::VrChatClient>>>>;

pub struct VrChatClientKey;
impl TypeMapKey for VrChatClientKey {
    type Value = SharedVrChatClient;
}

pub struct ScarletDataKey;
impl TypeMapKey for ScarletDataKey {
    type Value = Arc<scarlet_core::data::ScarletData>;
}

pub struct SettingsKey;
impl TypeMapKey for SettingsKey {
    type Value = Arc<scarlet_core::settings::Settings>;
}

pub struct AvatarSearchKey;
impl TypeMapKey for AvatarSearchKey {
    type Value = Arc<crate::avatar_search::AvatarSearch>;
}

/// Port of `Scarlet.staffList` (`staff_list.json`).
pub struct StaffListKey;
impl TypeMapKey for StaffListKey {
    type Value = Arc<scarlet_core::staff_list::StaffList>;
}

/// Port of `Scarlet.secretStaffList` (`secret_staff_list.json`) -- the same
/// shape as [`StaffListKey`], a separate independent list.
pub struct SecretStaffListKey;
impl TypeMapKey for SecretStaffListKey {
    type Value = Arc<scarlet_core::staff_list::StaffList>;
}

/// Port of `Scarlet.moderationTags` (`moderation_tags.json`).
pub struct ModerationTagsKey;
impl TypeMapKey for ModerationTagsKey {
    type Value = Arc<scarlet_core::moderation_tags::ModerationTags>;
}

/// Port of `Scarlet.watchedGroups` (`watched_groups.json`).
pub struct WatchedGroupsKey;
impl TypeMapKey for WatchedGroupsKey {
    type Value = Arc<scarlet_core::watched_groups::WatchedGroups>;
}

/// Port of `Scarlet.watchedUsers` (`watched_users.json`).
pub struct WatchedUsersKey;
impl TypeMapKey for WatchedUsersKey {
    type Value = Arc<scarlet_core::watched_entities::WatchedEntities>;
}

/// Port of `Scarlet.watchedAvatars` (`watched_avatars.json`).
pub struct WatchedAvatarsKey;
impl TypeMapKey for WatchedAvatarsKey {
    type Value = Arc<scarlet_core::watched_entities::WatchedEntities>;
}

/// In-progress Discord instance pickers, keyed by the opaque token embedded
/// in component custom ids. Equivalent to `ScarletDiscordJDA.instanceCreation`.
pub type InstanceCreations = Arc<RwLock<HashMap<String, crate::interactions::InstanceCreation>>>;

pub struct InstanceCreationsKey;
impl TypeMapKey for InstanceCreationsKey {
    type Value = InstanceCreations;
}

pub type DiscordModerationDrafts =
    Arc<RwLock<HashMap<String, crate::commands::discord_member_action::DiscordModerationDraft>>>;
pub struct DiscordModerationDraftsKey;
impl TypeMapKey for DiscordModerationDraftsKey {
    type Value = DiscordModerationDrafts;
}

pub type PendingVrChatLinks =
    Arc<RwLock<HashMap<String, crate::commands::link_vrchat_account::PendingVrChatLink>>>;
pub struct PendingVrChatLinksKey;
impl TypeMapKey for PendingVrChatLinksKey {
    type Value = PendingVrChatLinks;
}

/// Java-compatible, unknown-field-preserving `discord_bot.json` storage.
/// Keeping the raw object is important while the port is incremental: saving
/// one Rust-supported field must not erase settings still owned by Java.
pub struct DiscordConfigStore {
    file: PathBuf,
    value: Mutex<serde_json::Map<String, serde_json::Value>>,
}

impl DiscordConfigStore {
    pub fn open(data_dir: &Path) -> Self {
        let file = data_dir.join("discord_bot.json");
        let value = std::fs::read_to_string(&file)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Self {
            file,
            value: Mutex::new(value),
        }
    }

    pub fn map_value(&self, map_name: &str, key: &str) -> Option<String> {
        self.value
            .lock()
            .unwrap()
            .get(map_name)?
            .as_object()?
            .get(key)?
            .as_str()
            .map(str::to_string)
    }

    pub fn string(&self, field: &str) -> Option<String> {
        self.value
            .lock()
            .unwrap()
            .get(field)?
            .as_str()
            .map(str::to_string)
    }

    pub fn string_map(&self, map_name: &str) -> HashMap<String, String> {
        self.value
            .lock()
            .unwrap()
            .get(map_name)
            .and_then(|value| value.as_object())
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn map_strings(&self, map_name: &str, key: &str) -> Vec<String> {
        let root = self.value.lock().unwrap();
        let Some(value) = root
            .get(map_name)
            .and_then(|value| value.as_object())
            .and_then(|map| map.get(key))
        else {
            return Vec::new();
        };
        match value {
            serde_json::Value::String(value) => vec![value.clone()],
            serde_json::Value::Array(values) => values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn set_map_strings(
        &self,
        map_name: &str,
        key: &str,
        values: &[String],
    ) -> anyhow::Result<()> {
        {
            let mut root = self.value.lock().unwrap();
            let map = root
                .entry(map_name.to_string())
                .or_insert_with(|| serde_json::json!({}));
            if !map.is_object() {
                *map = serde_json::json!({});
            }
            let map = map.as_object_mut().expect("object assigned above");
            match values {
                [] => {
                    map.remove(key);
                }
                [value] => {
                    map.insert(key.to_string(), serde_json::Value::String(value.clone()));
                }
                values => {
                    map.insert(
                        key.to_string(),
                        serde_json::Value::Array(
                            values
                                .iter()
                                .cloned()
                                .map(serde_json::Value::String)
                                .collect(),
                        ),
                    );
                }
            }
        }
        self.save()
    }

    pub fn set_map_value(
        &self,
        map_name: &str,
        key: &str,
        value: Option<&str>,
    ) -> anyhow::Result<()> {
        {
            let mut root = self.value.lock().unwrap();
            let map = root
                .entry(map_name.to_string())
                .or_insert_with(|| serde_json::json!({}));
            if !map.is_object() {
                *map = serde_json::json!({});
            }
            let map = map.as_object_mut().expect("object assigned above");
            match value {
                Some(value) => {
                    map.insert(
                        key.to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                }
                None => {
                    map.remove(key);
                }
            }
        }
        self.save()
    }

    pub fn set_string(&self, field: &str, value: Option<&str>) -> anyhow::Result<()> {
        {
            let mut root = self.value.lock().unwrap();
            root.insert(
                field.to_string(),
                value.map_or(serde_json::Value::Null, |value| {
                    serde_json::Value::String(value.to_string())
                }),
            );
        }
        self.save()
    }

    pub fn set_json(&self, field: &str, value: serde_json::Value) -> anyhow::Result<()> {
        self.value.lock().unwrap().insert(field.to_string(), value);
        self.save()
    }

    pub fn json(&self, field: &str) -> Option<serde_json::Value> {
        self.value.lock().unwrap().get(field).cloned()
    }

    pub fn update_json(
        &self,
        fields: impl IntoIterator<Item = (&'static str, serde_json::Value)>,
    ) -> anyhow::Result<()> {
        {
            let mut root = self.value.lock().unwrap();
            for (field, value) in fields {
                root.insert(field.to_string(), value);
            }
        }
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(&*self.value.lock().unwrap())?;
        Ok(scarlet_core::file_backups::write(&self.file, &json)?)
    }
}

/// The connected client's HTTP handle, published once the client is built
/// so code outside the gateway (the ported event listener's audit emitter)
/// can post to Discord. `None` until then, and `None` again for a run with
/// no Discord token at all.
pub type SharedHttp = Arc<RwLock<Option<Arc<serenity::http::Http>>>>;

pub struct DiscordConfigKey;
impl TypeMapKey for DiscordConfigKey {
    type Value = Arc<DiscordConfigStore>;
}

pub struct HttpKey;
impl TypeMapKey for HttpKey {
    type Value = SharedHttp;
}

/// Every piece of shared app state a command handler might need, bundled so
/// `connect_and_serve` (and its callers in `scarlet`/`scarlet-ui`) take one
/// value instead of an ever-growing positional-argument list as more of
/// `Scarlet`'s fields get ported. Construct with all fields, then
/// [`SharedState::insert_into`] once before `Client::start`.
pub struct SharedState {
    pub vrchat_client: SharedVrChatClient,
    /// Filled in by `connect_and_serve` once the client exists; see
    /// [`SharedHttp`].
    pub http: SharedHttp,
    /// One shared, live view of `discord_bot.json`. Commands mutate this
    /// same instance that background emitters read, so routing changes take
    /// effect immediately instead of only after a restart.
    pub discord_config: Arc<DiscordConfigStore>,
    pub scarlet_data: Arc<scarlet_core::data::ScarletData>,
    pub settings: Arc<scarlet_core::settings::Settings>,
    pub staff_list: Arc<scarlet_core::staff_list::StaffList>,
    pub secret_staff_list: Arc<scarlet_core::staff_list::StaffList>,
    pub moderation_tags: Arc<scarlet_core::moderation_tags::ModerationTags>,
    pub watched_groups: Arc<scarlet_core::watched_groups::WatchedGroups>,
    pub watched_users: Arc<scarlet_core::watched_entities::WatchedEntities>,
    pub watched_avatars: Arc<scarlet_core::watched_entities::WatchedEntities>,
}

impl SharedState {
    pub fn insert_into(self, data: &mut TypeMap) {
        data.insert::<VrChatClientKey>(self.vrchat_client);
        data.insert::<HttpKey>(self.http);
        data.insert::<ScarletDataKey>(self.scarlet_data);
        data.insert::<SettingsKey>(self.settings);
        data.insert::<AvatarSearchKey>(Arc::new(crate::avatar_search::AvatarSearch::default()));
        data.insert::<DiscordConfigKey>(self.discord_config);
        data.insert::<StaffListKey>(self.staff_list);
        data.insert::<SecretStaffListKey>(self.secret_staff_list);
        data.insert::<ModerationTagsKey>(self.moderation_tags);
        data.insert::<WatchedGroupsKey>(self.watched_groups);
        data.insert::<WatchedUsersKey>(self.watched_users);
        data.insert::<WatchedAvatarsKey>(self.watched_avatars);
        data.insert::<InstanceCreationsKey>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<DiscordModerationDraftsKey>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<PendingVrChatLinksKey>(Arc::new(RwLock::new(HashMap::new())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discord_config_preserves_unknown_java_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("discord_bot.json"),
            r#"{"guildSf":"123","futureJavaField":{"x":1}}"#,
        )
        .unwrap();
        let config = DiscordConfigStore::open(dir.path());
        config
            .set_map_value("auditType2channelSf", "group.user.ban", Some("456"))
            .unwrap();
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("discord_bot.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["guildSf"], "123");
        assert_eq!(saved["futureJavaField"]["x"], 1);
        assert_eq!(saved["auditType2channelSf"]["group.user.ban"], "456");
        config
            .set_map_strings(
                "auditType2scarletAuxWh",
                "group.user.ban",
                &["one".into(), "two".into()],
            )
            .unwrap();
        assert_eq!(
            config.map_strings("auditType2scarletAuxWh", "group.user.ban"),
            ["one", "two"]
        );
    }
}
