//! Shared application state reachable from slash-command handlers, via
//! serenity's per-client `TypeMap` (`ctx.data`). Java's
//! `ScarletDiscordCommands` just holds a direct field reference to the
//! enclosing `Celeste`/`ScarletVRChat`/`ScarletData` instances; serenity's
//! event-handler methods don't carry an equivalent "the app" parameter, so
//! this is the idiomatic serenity substitute — insert once before
//! `Client::start`, read from any command handler via `ctx.data.read()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

    pub fn data_dir(&self) -> &Path {
        self.file.parent().unwrap_or_else(|| Path::new("."))
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
        let root = self.value.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut root = self.value.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut root = self.value.lock().unwrap_or_else(|e| e.into_inner());
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
            let mut root = self.value.lock().unwrap_or_else(|e| e.into_inner());
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
        self.value.lock().unwrap_or_else(|e| e.into_inner()).insert(field.to_string(), value);
        self.save()
    }

    pub fn json(&self, field: &str) -> Option<serde_json::Value> {
        self.value.lock().unwrap_or_else(|e| e.into_inner()).get(field).cloned()
    }

    pub fn update_json(
        &self,
        fields: impl IntoIterator<Item = (&'static str, serde_json::Value)>,
    ) -> anyhow::Result<()> {
        {
            let mut root = self.value.lock().unwrap_or_else(|e| e.into_inner());
            for (field, value) in fields {
                root.insert(field.to_string(), value);
            }
        }
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(&*self.value.lock().unwrap_or_else(|e| e.into_inner()))?;
        Ok(scarlet_core::file_backups::write(&self.file, &json)?)
    }
}

/// Returns the one Discord guild Celeste is configured to serve.
///
/// The Rust setup wizard originally stored this in encrypted preferences as
/// `discord.guild_id`, while the Java-compatible Discord config and several
/// runtime consumers use `discord_bot.json`'s `guildSf`. Keeping this bridge
/// in one place prevents commands, voice reconnect, and audit routing from
/// silently disagreeing about whether a guild is configured.
pub fn configured_guild_id(
    settings: &scarlet_core::settings::Settings,
    config: &DiscordConfigStore,
) -> Option<u64> {
    let registry_value = scarlet_core::settings::registry::registry_string_encrypted(
        settings.encrypted.clone(),
        "discord.guild_id",
    )
    .get_or_null();
    resolve_guild_id(registry_value.as_deref(), config.string("guildSf").as_deref())
}

fn resolve_guild_id(primary: Option<&str>, legacy: Option<&str>) -> Option<u64> {
    primary
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or_else(|| legacy.and_then(|value| value.trim().parse::<u64>().ok()))
}

/// Keeps both the Rust setup key and Java-compatible config field populated.
/// Existing installs can have either one, depending on which build created
/// their data directory.
pub fn synchronize_guild_id(
    settings: &scarlet_core::settings::Settings,
    config: &DiscordConfigStore,
) -> anyhow::Result<Option<u64>> {
    let registry = scarlet_core::settings::registry::registry_string_encrypted(
        settings.encrypted.clone(),
        "discord.guild_id",
    );
    let primary = registry.get_or_null();
    let legacy = config.string("guildSf");
    let resolved = resolve_guild_id(primary.as_deref(), legacy.as_deref());
    let Some(guild_id) = resolved else { return Ok(None) };
    let canonical = guild_id.to_string();
    if primary.as_deref().map(str::trim) != Some(canonical.as_str()) {
        registry.set(Some(canonical.clone()));
    }
    if legacy.as_deref().map(str::trim) != Some(canonical.as_str()) {
        config.set_string("guildSf", Some(&canonical))?;
    }
    Ok(Some(guild_id))
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

pub struct DiscordPermissionsKey;
impl TypeMapKey for DiscordPermissionsKey {
    type Value = Arc<crate::permissions::DiscordPermissions>;
}

pub struct PendingModerationKey;
impl TypeMapKey for PendingModerationKey {
    type Value = Arc<crate::pending_moderation::PendingModeration>;
}

pub struct HttpKey;
impl TypeMapKey for HttpKey {
    type Value = SharedHttp;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRequest {
    Restart,
}

/// Single-shot application lifecycle control used by `/server-restart`.
/// The binary owns the receiver and performs the actual relaunch; keeping
/// that process-level action outside the Discord crate also makes duplicate
/// command invocations deterministic and testable.
pub struct ProcessController {
    queued: AtomicBool,
    sender: tokio::sync::mpsc::UnboundedSender<ProcessRequest>,
}

impl ProcessController {
    pub fn new() -> (
        Arc<Self>,
        tokio::sync::mpsc::UnboundedReceiver<ProcessRequest>,
    ) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(Self {
                queued: AtomicBool::new(false),
                sender,
            }),
            receiver,
        )
    }

    pub fn restart(&self) -> bool {
        if self.queued.swap(true, Ordering::AcqRel) {
            return false;
        }
        if self.sender.send(ProcessRequest::Restart).is_err() {
            self.queued.store(false, Ordering::Release);
            return false;
        }
        true
    }
}

pub struct ProcessControllerKey;
impl TypeMapKey for ProcessControllerKey {
    type Value = Arc<ProcessController>;
}

/// Every piece of shared app state a command handler might need, bundled so
/// `connect_and_serve` (and its callers in `scarlet`/`scarlet-ui`) take one
/// value instead of an ever-growing positional-argument list as more of
/// `Celeste`'s fields get ported. Construct with all fields, then
/// [`SharedState::insert_into`] once before `Client::start`.
pub struct SharedState {
    pub vrchat_client: SharedVrChatClient,
    /// Shared so the `/vrchat-search avatar` command and the player table's
    /// avatar status use one cache and one back-off state, rather than a
    /// client each hitting the same providers twice.
    pub avatar_search: Arc<crate::avatar_search::AvatarSearch>,
    /// Filled in by `connect_and_serve` once the client exists; see
    /// [`SharedHttp`].
    pub http: SharedHttp,
    /// One shared, live view of `discord_bot.json`. Commands mutate this
    /// same instance that background emitters read, so routing changes take
    /// effect immediately instead of only after a restart.
    pub discord_config: Arc<DiscordConfigStore>,
    pub voice_manager: Arc<songbird::Songbird>,
    pub process_controller: Arc<ProcessController>,
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
    /// Populate a newly-built serenity client. Taking `&self` is deliberate:
    /// the Discord supervisor may need to build another client after a
    /// gateway failure, while every reconnect must keep pointing at the same
    /// process-wide stores and VRChat session.
    pub fn insert_into(&self, data: &mut TypeMap) {
        match synchronize_guild_id(&self.settings, &self.discord_config) {
            Ok(Some(guild_id)) => tracing::debug!(guild_id, "Discord guild configuration synchronized"),
            Ok(None) => tracing::warn!("No Discord guild is configured"),
            Err(error) => tracing::error!(%error, "Failed to synchronize Discord guild configuration"),
        }
        let discord_permissions = Arc::new(crate::permissions::DiscordPermissions::open(
            self.discord_config.data_dir(),
        ));
        let pending_moderation = Arc::new(crate::pending_moderation::PendingModeration::open(
            self.discord_config.data_dir(),
        ));
        data.insert::<VrChatClientKey>(self.vrchat_client.clone());
        data.insert::<HttpKey>(self.http.clone());
        data.insert::<ScarletDataKey>(self.scarlet_data.clone());
        data.insert::<SettingsKey>(self.settings.clone());
        data.insert::<AvatarSearchKey>(self.avatar_search.clone());
        data.insert::<DiscordConfigKey>(self.discord_config.clone());
        data.insert::<DiscordPermissionsKey>(discord_permissions);
        data.insert::<PendingModerationKey>(pending_moderation);
        data.insert::<ProcessControllerKey>(self.process_controller.clone());
        data.insert::<StaffListKey>(self.staff_list.clone());
        data.insert::<SecretStaffListKey>(self.secret_staff_list.clone());
        data.insert::<ModerationTagsKey>(self.moderation_tags.clone());
        data.insert::<WatchedGroupsKey>(self.watched_groups.clone());
        data.insert::<WatchedUsersKey>(self.watched_users.clone());
        data.insert::<WatchedAvatarsKey>(self.watched_avatars.clone());
        data.insert::<InstanceCreationsKey>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<DiscordModerationDraftsKey>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<PendingVrChatLinksKey>(Arc::new(RwLock::new(HashMap::new())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn process_controller_queues_only_one_restart() {
        let (controller, mut requests) = ProcessController::new();
        assert!(controller.restart());
        assert!(!controller.restart());
        assert_eq!(requests.recv().await, Some(ProcessRequest::Restart));
    }

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

    #[test]
    fn rust_setup_guild_takes_precedence_with_java_fallback() {
        assert_eq!(resolve_guild_id(Some(" 123 "), Some("456")), Some(123));
        assert_eq!(resolve_guild_id(None, Some("456")), Some(456));
        assert_eq!(resolve_guild_id(Some("invalid"), Some("456")), Some(456));
        assert_eq!(resolve_guild_id(None, None), None);
    }

    #[test]
    fn setup_guild_is_copied_to_java_config_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let settings = scarlet_core::settings::Settings::open(dir.path());
        scarlet_core::settings::registry::registry_string_encrypted(
            settings.encrypted.clone(),
            "discord.guild_id",
        )
        .set(Some("123".to_string()));
        let config = DiscordConfigStore::open(dir.path());

        assert_eq!(synchronize_guild_id(&settings, &config).unwrap(), Some(123));
        assert_eq!(config.string("guildSf").as_deref(), Some("123"));
        assert_eq!(
            DiscordConfigStore::open(dir.path()).string("guildSf").as_deref(),
            Some("123")
        );
    }
}
