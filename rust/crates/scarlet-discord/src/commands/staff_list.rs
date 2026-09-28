//! Port of `ScarletDiscordCommands.StaffList` (`@SlashCmd("staff-list")`,
//! backed by `Scarlet.staffList`/`staff_list.json`). This is also where
//! Discord<->VRChat account linking happens (`_add`'s required
//! `discord-user` option calls `ScarletData.linkIdToSnowflake`) — the same
//! flow every other moderation command's "you must have linked ids" error
//! message points users at (see `super::linkedIdsReply`/
//! `super::moderation_context`'s docs).
//!
//! The shared `list`/`add`/`delete` logic lives in this module as
//! [`run_generic`] (and [`register_generic`] for command registration) and
//! is reused by [`super::secret_staff_list`] (`CelesteSecretStaffList`/
//! `secret_staff_list.json`), which Java implements as a second,
//! byte-for-byte-identical `_StaffList` subclass — one shared
//! implementation parameterized by which [`scarlet_core::staff_list::StaffList`]
//! and display name to use, rather than duplicating the command body.
//!
//! `vrchat-user`/`vrchat-role` use Java-compatible live autocomplete for
//! `io.github.vrchatapi.model.User`/`GroupRole` (see the `commands` module
//! docs). `list` renders each entry as one text line via
//! [`super::paginate_on_lines`] instead of Java's paginated per-user embeds
//! (thumbnail/status description included). Like `add-role`/`remove-role`
//! and Java's own `StaffList`/`SecretStaffList` (`@DefaultPerms(Permission.USE_APPLICATION_COMMANDS)`,
//! no in-code `checkMemberHasVRChatPermission` call), there's no permission
//! gate here beyond whatever Discord-side command permissions a server
//! configures — [`super::vrchat_context`] is used only to resolve the
//! client/group id needed to look up display names and (optionally)
//! assign a role, not to authorize the action.

use std::sync::Arc;

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue};
use serenity::model::mention::Mentionable;

use scarlet_core::staff_list::StaffList;

pub const NAME: &str = "staff-list";

pub fn register() -> CreateCommand {
    register_generic(NAME, "Configures the staff list")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(list) = ({ ctx.data.read().await.get::<crate::data::StaffListKey>().cloned() }) else {
        super::defer(ctx, command).await?;
        return super::reply_ephemeral(ctx, command, "The staff list is not available.").await;
    };
    run_generic(ctx, command, list, "staff").await
}

pub(crate) fn register_generic(name: &str, description: &str) -> CreateCommand {
    CreateCommand::new(name)
        .description(description)
        .add_option(CreateCommandOption::new(CommandOptionType::SubCommand, "list", "Lists all users on this list")
            .add_sub_option(super::pagination_option()))
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "add", "Adds a user to this list")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "vrchat-user", "The VRChat user id (usr_...)")
                        .required(true)
                        .set_autocomplete(true),
                )
                // Java registers `_discordUser` as `SlashOption.ofMember("discord-user", ..., true)`
                // and `StaffList.add`/`SecretStaffList.add` do not carry `@Required(false)`,
                // so the option is required on this subcommand (unlike, say, `vrchat-role`,
                // which `_vrchatRoleOpt` registers with `required = false`).
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::User, "discord-user", "The Discord user")
                        .required(true),
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "vrchat-role",
                    "Optional VRChat group role id (grol_...) to also grant",
                ).set_autocomplete(true)),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "delete", "Removes a user from this list")
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::String, "vrchat-user", "The VRChat user id (usr_...)")
                        .required(true)
                        .set_autocomplete(true),
                )
                .add_sub_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "vrchat-role",
                    "Optional VRChat group role id (grol_...) to also revoke",
                ).set_autocomplete(true)),
        )
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(*s),
        _ => None,
    })
}

pub(crate) async fn run_generic(
    ctx: &Context,
    command: &CommandInteraction,
    list: Arc<StaffList>,
    info_name: &str,
) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    match subcommand.name {
        "list" => run_list(ctx, command, sub_options, &list, info_name).await,
        "add" => run_add(ctx, command, sub_options, &list, info_name).await,
        "delete" => run_delete(ctx, command, sub_options, &list, info_name).await,
        other => super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await,
    }
}

fn staff_entry_line(
    user_id: &str,
    display_name: Option<&str>,
    member: Option<&scarlet_vrchat::models::GroupMember>,
    roles_by_id: &std::collections::HashMap<String, String>,
    linked_snowflake: Option<&str>,
) -> String {
    let name = display_name.unwrap_or(user_id);
    let mut line = format!("[{name}](<https://vrchat.com/home/user/{user_id}>) `{user_id}`");
    if let Some(member) = member {
        if let Some(joined) = &member.joined_at {
            line.push_str(&format!(" — joined {joined}"));
        }
        if !member.role_ids.is_empty() {
            let names: Vec<&str> =
                member.role_ids.iter().map(|id| roles_by_id.get(id).map(String::as_str).unwrap_or(id.as_str())).collect();
            line.push_str(&format!(" — roles: {}", names.join(", ")));
        }
    }
    if let Some(snowflake) = linked_snowflake {
        line.push_str(&format!(" — linked Discord: <@{snowflake}>"));
    }
    line
}

async fn run_list(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>], list: &StaffList, info_name: &str) -> serenity::Result<()> {
    let ids = list.get_ids();
    if ids.is_empty() {
        return super::reply_ephemeral(ctx, command, &format!("No users are on the {info_name} list.")).await;
    }

    let scarlet_data = { ctx.data.read().await.get::<crate::data::ScarletDataKey>().cloned() };
    let context = super::vrchat_context(ctx).await.ok();
    let roles_by_id = match &context {
        Some(context) => super::vrchat_group::role_names(&context.client, &context.group_id).await,
        None => std::collections::HashMap::new(),
    };

    let mut lines = Vec::new();
    for id in &ids {
        let (display_name, member) = match &context {
            Some(context) => {
                let user = context.client.get_user(id).await.ok();
                let member = context.client.get_group_member(&context.group_id, id).await.ok().flatten();
                (user.and_then(|u| u.display_name), member)
            }
            None => (None, None),
        };
        let linked_snowflake = scarlet_data.as_ref().and_then(|d| d.user_metadata_get_snowflake(id));
        lines.push(staff_entry_line(id, display_name.as_deref(), member.as_ref(), &roles_by_id, linked_snowflake.as_deref()));
    }

    let heading = format!("### {info_name} list", info_name = capitalize(info_name));
    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n", Some(&heading)),
    ).await
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

async fn run_add(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    list: &StaffList,
    info_name: &str,
) -> serenity::Result<()> {
    let Some(user_id) = find_string(sub_options, "vrchat-user") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user` is required.").await;
    };
    let discord_user = sub_options.iter().find(|o| o.name == "discord-user").and_then(|o| match &o.value {
        ResolvedValue::User(user, _) => Some(*user),
        _ => None,
    });
    let role_id = find_string(sub_options, "vrchat-role");

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {user_id} ({err})")).await;
        }
    };

    let mut lines = Vec::new();

    if let Some(discord_user) = discord_user {
        if let Some(scarlet_data) = { ctx.data.read().await.get::<crate::data::ScarletDataKey>().cloned() } {
            scarlet_data.link_id_to_snowflake(user_id, &discord_user.id.to_string());
            lines.push(format!("Associating {} with VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>)", discord_user.mention()));
        }
    }

    if !list.add_id(user_id) {
        lines.push(format!("VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) is already on the {info_name} list"));
    } else {
        lines.push(format!("Adding VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) to the {info_name} list"));
    }

    if let Some(role_id) = role_id {
        let member = context.client.get_group_member(&context.group_id, user_id).await.ok().flatten();
        let already_has_role = member_already_has_role(member.as_ref().map(|m| m.role_ids.as_slice()), role_id);
        if already_has_role {
            lines.push(format!("VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) already has the role `{role_id}`"));
        } else {
            match context.client.add_group_member_role(&context.group_id, user_id, role_id).await {
                Ok(_) => lines.push(format!("Adding the role `{role_id}` to [{display_name}](<https://vrchat.com/home/user/{user_id}>)")),
                Err(err) => lines.push(format!("Failed to add the role `{role_id}`: {err}")),
            }
        }
    }

    command
        .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(lines.join("\n")).ephemeral(true))
        .await
        .map(|_| ())
}

/// Java (`ScarletDiscordCommands._StaffList._add`) only reports "already has the role" when
/// the membership lookup returned a member carrying it; a failed/absent lookup falls through
/// to `addGroupRole`.
fn member_already_has_role(member_role_ids: Option<&[String]>, role_id: &str) -> bool {
    member_role_ids.map(|ids| ids.iter().any(|id| id == role_id)).unwrap_or(false)
}

/// Java (`ScarletDiscordCommands._StaffList._remove`) mirrors the above: only a member that
/// was actually found and lacks the role short-circuits; a null lookup still calls
/// `removeGroupRole`.
fn member_already_lacks_role(member_role_ids: Option<&[String]>, role_id: &str) -> bool {
    member_role_ids.map(|ids| !ids.iter().any(|id| id == role_id)).unwrap_or(false)
}

async fn run_delete(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    list: &StaffList,
    info_name: &str,
) -> serenity::Result<()> {
    let Some(user_id) = find_string(sub_options, "vrchat-user") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user` is required.").await;
    };
    let role_id = find_string(sub_options, "vrchat-role");

    let context = super::vrchat_context(ctx).await.ok();

    let mut lines = Vec::new();
    let display_name = match &context {
        Some(context) => match context.client.get_user(user_id).await {
            Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
            Err(_) => {
                lines.push(format!("No VRChat user found with id {user_id}"));
                user_id.to_string()
            }
        },
        None => user_id.to_string(),
    };

    if !list.remove_id(user_id) {
        lines.push(format!("VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) is not on the {info_name} list"));
    } else {
        lines.push(format!("Removing VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) from the {info_name} list"));
    }

    if let Some(role_id) = role_id {
        let Some(context) = &context else {
            lines.push(format!("Cannot remove the role `{role_id}`: no VRChat session is available."));
            return command
                .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(lines.join("\n")).ephemeral(true))
                .await
                .map(|_| ());
        };
        // Java (ScarletDiscordCommands._StaffList._remove) only short-circuits when the
        // membership lookup actually returns a member that lacks the role; a null lookup
        // still falls through to removeGroupRole.
        let member = context.client.get_group_member(&context.group_id, user_id).await.ok().flatten();
        let already_lacks_role = member_already_lacks_role(member.as_ref().map(|m| m.role_ids.as_slice()), role_id);
        if already_lacks_role {
            lines.push(format!("VRChat user [{display_name}](<https://vrchat.com/home/user/{user_id}>) already lacks the role `{role_id}`"));
        } else {
            match context.client.remove_group_member_role(&context.group_id, user_id, role_id).await {
                Ok(_) => lines.push(format!("Removing the role `{role_id}` from [{display_name}](<https://vrchat.com/home/user/{user_id}>)")),
                Err(err) => lines.push(format!("Failed to remove the role `{role_id}`: {err}")),
            }
        }
    }

    command
        .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(lines.join("\n")).ephemeral(true))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_membership_lookup_still_attempts_the_role_change() {
        // Java falls through to add/removeGroupRole when getGroupMembership returns null.
        assert!(!member_already_has_role(None, "grol_1"));
        assert!(!member_already_lacks_role(None, "grol_1"));
    }

    #[test]
    fn present_membership_short_circuits_like_java() {
        let with = ["grol_1".to_string()];
        let without: [String; 0] = [];
        assert!(member_already_has_role(Some(&with), "grol_1"));
        assert!(!member_already_has_role(Some(&without), "grol_1"));
        assert!(member_already_lacks_role(Some(&without), "grol_1"));
        assert!(!member_already_lacks_role(Some(&with), "grol_1"));
    }

    #[test]
    fn capitalize_uppercases_first_char_only() {
        assert_eq!(capitalize("staff"), "Staff");
        assert_eq!(capitalize("secret staff"), "Secret staff");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn staff_entry_line_includes_roles_and_linked_discord() {
        let member = scarlet_vrchat::models::GroupMember {
            role_ids: vec!["grol_1".to_string()],
            joined_at: Some("2024-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let mut roles = std::collections::HashMap::new();
        roles.insert("grol_1".to_string(), "Moderator".to_string());
        let line = staff_entry_line("usr_1", Some("Alice"), Some(&member), &roles, Some("12345"));
        assert!(line.contains("Alice"));
        assert!(line.contains("Moderator"));
        assert!(line.contains("joined 2024-01-01T00:00:00Z"));
        assert!(line.contains("<@12345>"));
    }

    #[test]
    fn staff_entry_line_falls_back_to_id_with_no_extra_data() {
        let line = staff_entry_line("usr_1", None, None, &std::collections::HashMap::new(), None);
        assert_eq!(line, "[usr_1](<https://vrchat.com/home/user/usr_1>) `usr_1`");
    }
}
