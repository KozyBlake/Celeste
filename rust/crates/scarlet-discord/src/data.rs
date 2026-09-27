//! Shared application state reachable from slash-command handlers, via
//! serenity's per-client `TypeMap` (`ctx.data`). Java's
//! `ScarletDiscordCommands` just holds a direct field reference to the
//! enclosing `Scarlet`/`ScarletVRChat`/`ScarletData` instances; serenity's
//! event-handler methods don't carry an equivalent "the app" parameter, so
//! this is the idiomatic serenity substitute — insert once before
//! `Client::start`, read from any command handler via `ctx.data.read()`.

use std::sync::Arc;

use serenity::prelude::TypeMapKey;
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
