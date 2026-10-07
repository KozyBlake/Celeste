//! Port of Java's `DPerms` and `discord_perms.json` storage.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serenity::model::guild::Member;
use serenity::model::id::{RoleId, UserId};
use serenity::model::permissions::Permissions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PermissionType {
    SlashCommand,
    MessageCommand,
    UserCommand,
    ButtonPress,
    StringSelect,
    EntitySelect,
    ModalSubmit,
    Other,
}

impl PermissionType {
    pub const ALL: [Self; 8] = [
        Self::SlashCommand,
        Self::MessageCommand,
        Self::UserCommand,
        Self::ButtonPress,
        Self::StringSelect,
        Self::EntitySelect,
        Self::ModalSubmit,
        Self::Other,
    ];

    pub const fn value(self) -> &'static str {
        match self {
            Self::SlashCommand => "slash-command",
            Self::MessageCommand => "message-command",
            Self::UserCommand => "user-command",
            Self::ButtonPress => "button-press",
            Self::StringSelect => "string-select",
            Self::EntitySelect => "entity-select",
            Self::ModalSubmit => "modal-submit",
            Self::Other => "other",
        }
    }

    pub const fn display(self) -> &'static str {
        match self {
            Self::SlashCommand => "Slash Command",
            Self::MessageCommand => "Message Command",
            Self::UserCommand => "User Command",
            Self::ButtonPress => "Button Press",
            Self::StringSelect => "String Select",
            Self::EntitySelect => "Entity Select",
            Self::ModalSubmit => "Modal Submit",
            Self::Other => "Other",
        }
    }

    pub fn of(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.value() == value)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionSet {
    #[serde(default)]
    by_user: HashMap<String, HashMap<String, bool>>,
    #[serde(default)]
    by_role: HashMap<String, HashMap<String, bool>>,
}

#[derive(Debug)]
pub struct DiscordPermissions {
    file: PathBuf,
    sets: Mutex<HashMap<PermissionType, PermissionSet>>,
}

#[derive(Debug, Clone, Copy)]
pub enum PermissionTarget {
    User(UserId),
    Role(RoleId),
}

impl DiscordPermissions {
    pub fn open(data_dir: &Path) -> Self {
        let file = data_dir.join("discord_perms.json");
        let sets = std::fs::read_to_string(&file)
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default();
        Self {
            file,
            sets: Mutex::new(sets),
        }
    }

    pub fn set(
        &self,
        kind: PermissionType,
        target: PermissionTarget,
        permission: &str,
        value: Option<bool>,
    ) -> anyhow::Result<Option<bool>> {
        let previous = {
            let mut sets = self.sets.lock().unwrap_or_else(|e| e.into_inner());
            let set = sets.entry(kind).or_default();
            let (map, id) = match target {
                PermissionTarget::User(id) => (&mut set.by_user, id.to_string()),
                PermissionTarget::Role(id) => (&mut set.by_role, id.to_string()),
            };
            match value {
                Some(value) => map
                    .entry(id)
                    .or_default()
                    .insert(permission.to_string(), value),
                None => {
                    let previous = map
                        .get_mut(&id)
                        .and_then(|permissions| permissions.remove(permission));
                    if map.get(&id).is_some_and(HashMap::is_empty) {
                        map.remove(&id);
                    }
                    previous
                }
            }
        };
        self.save()?;
        Ok(previous)
    }

    pub fn list(
        &self,
        kind: PermissionType,
        target: PermissionTarget,
        member_roles: &[RoleId],
    ) -> BTreeMap<String, bool> {
        let sets = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        let Some(set) = sets.get(&kind) else {
            return BTreeMap::new();
        };
        let mut output = BTreeMap::new();
        match target {
            PermissionTarget::User(id) => {
                if let Some(values) = set.by_user.get(&id.to_string()) {
                    output.extend(values.iter().map(|(key, value)| (key.clone(), *value)));
                }
                for role in member_roles {
                    if let Some(values) = set.by_role.get(&role.to_string()) {
                        for (key, value) in values {
                            output.entry(key.clone()).or_insert(*value);
                        }
                    }
                }
            }
            PermissionTarget::Role(id) => {
                if let Some(values) = set.by_role.get(&id.to_string()) {
                    output.extend(values.iter().map(|(key, value)| (key.clone(), *value)));
                }
            }
        }
        output
    }

    /// The name a permission used to be stored under, for entries written
    /// before the fork dropped the Scarlet name. A grant an admin made
    /// against the old command name must keep working after the rename —
    /// silently losing it would hand someone a denial they have no way to
    /// explain.
    fn legacy_permission_name(permission: &str) -> Option<String> {
        permission
            .strip_prefix("celeste-")
            .map(|rest| format!("scarlet-{rest}"))
    }

    pub fn resolve(&self, kind: PermissionType, member: &Member, permission: &str) -> Option<bool> {
        if member
            .permissions
            .is_some_and(|permissions| permissions.contains(Permissions::ADMINISTRATOR))
        {
            return Some(true);
        }
        let sets = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        let set = sets.get(&kind)?;
        let legacy = Self::legacy_permission_name(permission);
        let lookup = |values: &std::collections::HashMap<String, bool>| {
            values
                .get(permission)
                .or_else(|| legacy.as_deref().and_then(|legacy| values.get(legacy)))
                .or_else(|| values.get("*"))
                .copied()
        };
        let user = set.by_user.get(&member.user.id.to_string());
        if let Some(value) = user.and_then(lookup) {
            return Some(value);
        }
        for role in &member.roles {
            let values = set.by_role.get(&role.to_string());
            if let Some(value) = values.and_then(lookup) {
                return Some(value);
            }
        }
        None
    }

    pub fn check(
        &self,
        kind: PermissionType,
        member: &Member,
        permission: &str,
        fallback: bool,
    ) -> bool {
        self.resolve(kind, member, permission)
            .unwrap_or_else(|| fallback || default_allowed(kind, permission))
    }

    pub fn check_slash(
        &self,
        member: &Member,
        name: &str,
        group: Option<&str>,
        subcommand: Option<&str>,
    ) -> bool {
        if let Some(subcommand) = subcommand {
            if let Some(group) = group {
                if let Some(value) = self.resolve(
                    PermissionType::SlashCommand,
                    member,
                    &format!("{name} {group} {subcommand}"),
                ) {
                    return value;
                }
                if let Some(value) = self.resolve(
                    PermissionType::SlashCommand,
                    member,
                    &format!("{name} {group}"),
                ) {
                    return value;
                }
            } else if let Some(value) = self.resolve(
                PermissionType::SlashCommand,
                member,
                &format!("{name} {subcommand}"),
            ) {
                return value;
            }
        }
        self.check(PermissionType::SlashCommand, member, name, false)
    }

    fn save(&self) -> anyhow::Result<()> {
        let sets = self.sets.lock().unwrap_or_else(|e| e.into_inner());
        let text = serde_json::to_string_pretty(&*sets)?;
        celeste_core::file_backups::write(&self.file, &text)?;
        Ok(())
    }
}

pub fn operation(custom_id: &str) -> &str {
    custom_id
        .split_once(':')
        .map_or(custom_id, |(operation, _)| operation)
}

fn default_allowed(kind: PermissionType, permission: &str) -> bool {
    match kind {
        PermissionType::SlashCommand => matches!(
            permission,
            "link-vrchat-account" | "unlink-vrchat-account" | "tag"
        ),
        PermissionType::ButtonPress => matches!(
            permission,
            "settings"
                | "pagination"
                | "pagination-first"
                | "pagination-last"
                | "pagination-select"
                | "pagination-cancel"
                | "pagination-submit"
                | "edit-tags"
                | "browse-tags"
                | "vrchat-user-edit-manager-notes"
                | "vrchat-user-ban"
                | "timed-ban"
                | "timed-ban-custom"
                | "group-request-accept"
                | "group-request-reject"
                | "group-request-block"
                | "immediate-ban-edit-desc"
                | "immediate-ban-cancel"
                | "immediate-ban-confirm"
                | "vrchat-user-unban"
                | "event-redact"
                | "event-unredact"
                | "new-instance-create"
                | "new-instance-cancel"
                | "new-instance-modal"
                | "edit-desc"
                | "vrchat-report"
                | "view-potential-avatar-matches"
                | "view-snapshot-user"
                | "view-snapshot-user-groups"
                | "view-snapshot-user-represented-group"
                | "submit-evidence"
                | "import-watched-groups"
                | "discord-member-confirm"
                | "discord-member-cancel"
                | "discord-kick-confirm"
                | "discord-kick-cancel"
                | "discord-ban-confirm"
                | "discord-ban-cancel"
                | "linkvrc-verify"
        ),
        PermissionType::StringSelect => matches!(
            permission,
            "settings"
                | "pagination-submission"
                | "select-tags"
                | "select-tags-0"
                | "select-tags-1"
                | "select-tags-2"
                | "select-tags-3"
                | "select-tags-4"
                | "select-tags-search"
                | "watched-remove"
                | "immediate-ban-select-tags"
                | "new-instance-region"
                | "new-instance-access-type"
                | "new-instance-roles"
                | "new-instance-flags"
                | "set-audit-aux-webhooks"
                | "event-roles-1-25"
                | "event-roles-26-50"
                | "event-platforms"
        ),
        PermissionType::ModalSubmit => matches!(
            permission,
            "settings"
                | "pagination-select"
                | "watched-group-set-notes"
                | "watched-entity-set-notes"
                | "vrchat-user-edit-manager-notes"
                | "tag-search"
                | "timed-ban-custom"
                | "immediate-ban-edit-desc"
                | "vrchat-user-ban-multi"
                | "vrchat-user-unban-multi"
                | "new-instance-modal"
                | "edit-desc"
                | "edit-report-template"
                | "schedule-set-roles"
                | "schedule-set-platforms"
        ),
        PermissionType::EntitySelect => false,
        PermissionType::MessageCommand | PermissionType::UserCommand | PermissionType::Other => {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_role_rule_can_delegate_permission_management() {
        let dir = tempfile::tempdir().unwrap();
        let permissions = DiscordPermissions::open(dir.path());
        let role = RoleId::new(2);
        permissions
            .set(
                PermissionType::SlashCommand,
                PermissionTarget::Role(role),
                "celeste-discord-permissions",
                Some(true),
            )
            .unwrap();

        let mut member = Member::default();
        member.roles.push(role);
        member.permissions = Some(Permissions::empty());

        assert!(permissions.check_slash(
            &member,
            "celeste-discord-permissions",
            None,
            Some("set")
        ));
    }

    #[test]
    fn java_file_shape_round_trips_user_and_role_rules() {
        let dir = tempfile::tempdir().unwrap();
        let permissions = DiscordPermissions::open(dir.path());
        permissions
            .set(
                PermissionType::Other,
                PermissionTarget::User(UserId::new(1)),
                "groupex-bans-manage",
                Some(true),
            )
            .unwrap();
        permissions
            .set(
                PermissionType::SlashCommand,
                PermissionTarget::Role(RoleId::new(2)),
                "vrchat-user-ban",
                Some(false),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("discord_perms.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["OTHER"]["byUser"]["1"]["groupex-bans-manage"], true);
        assert_eq!(
            value["SLASH_COMMAND"]["byRole"]["2"]["vrchat-user-ban"],
            false
        );
    }

    #[test]
    fn custom_ids_use_only_the_operation_prefix() {
        assert_eq!(operation("vrchat-user-ban:usr_123"), "vrchat-user-ban");
        assert_eq!(operation("plain"), "plain");
    }

    #[test]
    fn authorized_settings_flows_can_submit_their_editor() {
        assert!(default_allowed(PermissionType::ButtonPress, "settings"));
        assert!(default_allowed(PermissionType::StringSelect, "settings"));
        assert!(default_allowed(PermissionType::ModalSubmit, "settings"));
    }

    #[test]
    fn current_discord_member_confirmation_buttons_are_enabled_by_default() {
        assert!(default_allowed(
            PermissionType::ButtonPress,
            "discord-member-confirm"
        ));
        assert!(default_allowed(
            PermissionType::ButtonPress,
            "discord-member-cancel"
        ));
    }

    #[test]
    fn every_generated_audit_tag_page_is_enabled_by_default() {
        for page in 0..5 {
            assert!(default_allowed(
                PermissionType::StringSelect,
                &format!("select-tags-{page}")
            ));
        }
    }
}
