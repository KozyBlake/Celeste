//! Shared application state reachable from slash-command handlers, via
//! serenity's per-client `TypeMap` (`ctx.data`). Java's
//! `ScarletDiscordCommands` just holds a direct field reference to the
//! enclosing `Scarlet`/`ScarletVRChat`/`ScarletData` instances; serenity's
//! event-handler methods don't carry an equivalent "the app" parameter, so
//! this is the idiomatic serenity substitute — insert once before
//! `Client::start`, read from any command handler via `ctx.data.read()`.

use std::sync::Arc;

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

/// Every piece of shared app state a command handler might need, bundled so
/// `connect_and_serve` (and its callers in `scarlet`/`scarlet-ui`) take one
/// value instead of an ever-growing positional-argument list as more of
/// `Scarlet`'s fields get ported. Construct with all fields, then
/// [`SharedState::insert_into`] once before `Client::start`.
pub struct SharedState {
    pub vrchat_client: SharedVrChatClient,
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
        data.insert::<ScarletDataKey>(self.scarlet_data);
        data.insert::<SettingsKey>(self.settings);
        data.insert::<StaffListKey>(self.staff_list);
        data.insert::<SecretStaffListKey>(self.secret_staff_list);
        data.insert::<ModerationTagsKey>(self.moderation_tags);
        data.insert::<WatchedGroupsKey>(self.watched_groups);
        data.insert::<WatchedUsersKey>(self.watched_users);
        data.insert::<WatchedAvatarsKey>(self.watched_avatars);
    }
}
