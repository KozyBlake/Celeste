//! Port of (part of) `ScarletDiscordCommands.VrchatGroups`
//! (`@SlashCmd("vrchat-group")` — a subcommand-group command). Java's
//! `VrchatGroups` has ~15 subcommands; ported so far: `add-role`,
//! `remove-role`, `search-members`, `list-bans`, `view-announcement`,
//! `create-announcement`, `delete-announcement`. The rest (list-invites,
//! list-join-requests, posts, transfers, open/close-instance) are a tracked
//! gap (see PORTING.md) — adding more subcommands under this same parent
//! command is how they'll land, matching Java's actual
//! `/vrchat-group <subcommand>` shape rather than inventing separate
//! top-level commands.
//!
//! **Simplified**: `vrchat-user-id`/`vrchat-role-id` are plain string
//! options rather than Java's autocomplete-resolved
//! `io.github.vrchatapi.model.User`/`GroupRole` (see the `commands` module
//! docs). `search-members`/`list-bans` render each member as one text line
//! instead of Java's paginated per-member embeds
//! (`discord.interactions.Pagination` with `MessageEmbed[]`) — still one
//! line per member with the same core fields (id, display name, roles,
//! status), just plain text split via [`super::paginate_on_lines`].
//! `create-announcement` drops the optional `image-file-id` option (VRChat
//! file uploads aren't ported — a Discord attachment would need to be
//! re-uploaded to VRChat's file API first, a separate not-yet-ported
//! endpoint). `delete-announcement` keeps Java's exact safety check: the
//! `confirm-delete` option must be the literal string `DELETE`. Permission
//! checking uses [`super::moderation_context`] for every subcommand here
//! except `add-role`/`remove-role`, which intentionally have no permission
//! gate, matching Java's own ungated originals.

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue};

use scarlet_vrchat::client::{
    PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, PERMISSION_GROUP_BANS_MANAGE, PERMISSION_GROUP_MEMBERS_VIEWALL,
};

pub const NAME: &str = "vrchat-group";

pub fn register() -> CreateCommand {
    let user_and_role_options = [
        CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)").required(true),
        CreateCommandOption::new(CommandOptionType::String, "vrchat-role-id", "The VRChat group role id (grol_...)")
            .required(true),
    ];

    let mut command = CreateCommand::new(NAME).description("Manages the configured VRChat group");
    for (name, desc) in [("add-role", "Adds a VRChat Group Role"), ("remove-role", "Removes a VRChat Group Role")] {
        let mut option = CreateCommandOption::new(CommandOptionType::SubCommand, name, desc);
        for sub_option in user_and_role_options.clone() {
            option = option.add_sub_option(sub_option);
        }
        command = command.add_option(option);
    }

    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "search-members", "Searches this group's members")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "member-search", "Name or user id to search within this group")
                    .required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Integer, "result-limit", "Maximum results to fetch (1-100)")
                    .min_int_value(1)
                    .max_int_value(100),
            ),
    );
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "list-bans", "Lists banned users in this group")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Integer, "result-limit", "Maximum results to fetch (1-100)")
                    .min_int_value(1)
                    .max_int_value(100),
            ),
    );

    command = command.add_option(CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "view-announcement",
        "Shows the current group announcement",
    ));
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "create-announcement", "Creates or replaces the group announcement")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "announcement-title", "Announcement title").required(true),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "announcement-text", "Announcement text").required(true),
            )
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "send-notification",
                "Send a notification to group members",
            )),
    );
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "delete-announcement", "Deletes the current group announcement")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "confirm-delete", "Type DELETE exactly to confirm")
                    .required(true),
            ),
    );

    command
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };

    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };

    match subcommand.name {
        "add-role" => role_change(ctx, command, sub_options, true).await,
        "remove-role" => role_change(ctx, command, sub_options, false).await,
        "search-members" => search_members(ctx, command, sub_options).await,
        "list-bans" => list_bans(ctx, command, sub_options).await,
        "view-announcement" => view_announcement(ctx, command).await,
        "create-announcement" => create_announcement(ctx, command, sub_options).await,
        "delete-announcement" => delete_announcement(ctx, command, sub_options).await,
        other => super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await,
    }
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::String(s) => Some(*s),
        _ => None,
    })
}

fn find_int(options: &[ResolvedOption<'_>], name: &str) -> Option<i64> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::Integer(n) => Some(*n),
        _ => None,
    })
}

fn find_bool(options: &[ResolvedOption<'_>], name: &str) -> Option<bool> {
    options.iter().find(|o| o.name == name).and_then(|o| match &o.value {
        ResolvedValue::Boolean(b) => Some(*b),
        _ => None,
    })
}

fn announcement_text(announcement: &scarlet_vrchat::models::GroupAnnouncement) -> String {
    if announcement.is_empty() {
        return "*(No announcement is currently set.)*".to_string();
    }
    let title = announcement.title.as_deref().unwrap_or("(untitled)");
    let text = announcement.text.as_deref().unwrap_or("");
    format!("### {title}\n{text}")
}

async fn role_change(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
    add: bool,
) -> serenity::Result<()> {
    let (Some(user_id), Some(role_id)) = (find_string(sub_options, "vrchat-user-id"), find_string(sub_options, "vrchat-role-id"))
    else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user-id` and `vrchat-role-id` are both required.").await;
    };

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

    if let Ok(Some(member)) = context.client.get_group_member(&context.group_id, user_id).await {
        let has_role = member.role_ids.contains(&role_id.to_string());
        if add && has_role {
            return super::reply_ephemeral(ctx, command, &format!("{display_name} already has that role.")).await;
        }
        if !add && !has_role {
            return super::reply_ephemeral(ctx, command, &format!("{display_name} already lacks that role.")).await;
        }
    }

    let result = if add {
        context.client.add_group_member_role(&context.group_id, user_id, role_id).await
    } else {
        context.client.remove_group_member_role(&context.group_id, user_id, role_id).await
    };

    match result {
        Ok(_) => {
            let verb = if add { "Added" } else { "Removed" };
            let prep = if add { "to" } else { "from" };
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("{verb} the role `{role_id}` {prep} {display_name}"))
                        .ephemeral(true),
                )
                .await
                .map(|_| ())
        }
        Err(err) => {
            let verb = if add { "add" } else { "remove" };
            super::reply_ephemeral(ctx, command, &format!("Failed to {verb} role: {err}")).await
        }
    }
}

/// Builds a `role_id -> role_name` lookup for rendering members with role
/// names instead of bare ids, matching Java's `roleNames`. Best-effort: on
/// failure, returns an empty map and callers fall back to showing raw ids.
async fn role_names(client: &scarlet_vrchat::client::VrChatClient, group_id: &str) -> std::collections::HashMap<String, String> {
    match client.get_group_roles(group_id).await {
        Ok(roles) => roles.into_iter().map(|r| (r.id, r.name)).collect(),
        Err(_) => std::collections::HashMap::new(),
    }
}

fn member_line(member: &scarlet_vrchat::models::GroupMember, roles_by_id: &std::collections::HashMap<String, String>) -> String {
    let user_id = member.user_id.as_deref().unwrap_or("unknown");
    let role_names: Vec<&str> =
        member.role_ids.iter().map(|id| roles_by_id.get(id).map(String::as_str).unwrap_or(id.as_str())).collect();
    let roles = if role_names.is_empty() { "(none)".to_string() } else { role_names.join(", ") };
    let status = member.membership_status.as_deref().unwrap_or("unknown");
    format!("[`{user_id}`](<https://vrchat.com/home/user/{user_id}>) — status: {status}, roles: {roles}")
}

async fn search_members(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let Some(query) = find_string(sub_options, "member-search") else {
        return super::reply_ephemeral(ctx, command, "`member-search` is required.").await;
    };
    let limit = find_int(sub_options, "result-limit").unwrap_or(25).clamp(1, 100) as u32;

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_MEMBERS_VIEWALL).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let result = match context.client.search_group_members(&context.group_id, query, limit, 0).await {
        Ok(result) => result,
        Err(err) => return super::reply_ephemeral(ctx, command, &format!("Failed to search group members: {err}")).await,
    };
    if result.results.is_empty() {
        return super::reply_ephemeral(ctx, command, "No group members matched that search.").await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = vec!["### Group Member search results".to_string()];
    lines.extend(result.results.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n"), 2000)).await
}

async fn list_bans(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let limit = find_int(sub_options, "result-limit").unwrap_or(25).clamp(1, 100) as u32;

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_BANS_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let bans = match context.client.get_group_bans(&context.group_id, limit, 0).await {
        Ok(bans) => bans,
        Err(err) => return super::reply_ephemeral(ctx, command, &format!("Failed to fetch group bans: {err}")).await,
    };
    if bans.is_empty() {
        return super::reply_ephemeral(ctx, command, "No banned users were returned.").await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = vec!["### Group Ban list".to_string()];
    lines.extend(bans.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n"), 2000)).await
}

async fn view_announcement(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.get_group_announcement(&context.group_id).await {
        Ok(announcement) => super::reply_ephemeral(ctx, command, &announcement_text(&announcement)).await,
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to fetch the group announcement: {err}")).await,
    }
}

async fn create_announcement(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let (Some(title), Some(text)) =
        (find_string(sub_options, "announcement-title"), find_string(sub_options, "announcement-text"))
    else {
        return super::reply_ephemeral(ctx, command, "`announcement-title` and `announcement-text` are both required.").await;
    };
    let send_notification = find_bool(sub_options, "send-notification").unwrap_or(false);

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.create_group_announcement(&context.group_id, title, text, send_notification).await {
        Ok(announcement) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("Created group announcement.\n{}", announcement_text(&announcement)))
                        .ephemeral(true),
                )
                .await
                .map(|_| ())
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to create the group announcement: {err}")).await,
    }
}

async fn delete_announcement(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let Some(confirm) = find_string(sub_options, "confirm-delete") else {
        return super::reply_ephemeral(ctx, command, "`confirm-delete` is required.").await;
    };
    if confirm != "DELETE" {
        return super::reply_ephemeral(
            ctx,
            command,
            "Deletion cancelled. Type `DELETE` exactly in the confirmation option to delete the announcement.",
        )
        .await;
    }

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.delete_group_announcement(&context.group_id).await {
        Ok(()) => super::reply_ephemeral(ctx, command, "Deleted the group announcement.").await,
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to delete the group announcement: {err}")).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_vrchat::models::GroupMember;
    use std::collections::HashMap;

    fn member(user_id: &str, role_ids: Vec<&str>, status: Option<&str>) -> GroupMember {
        GroupMember {
            user_id: Some(user_id.to_string()),
            role_ids: role_ids.into_iter().map(String::from).collect(),
            membership_status: status.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn member_line_resolves_role_names_when_known() {
        let mut roles = HashMap::new();
        roles.insert("grol_1".to_string(), "Moderator".to_string());
        let line = member_line(&member("usr_1", vec!["grol_1"], Some("member")), &roles);
        assert!(line.contains("Moderator"), "expected resolved role name in: {line}");
        assert!(line.contains("usr_1"));
        assert!(line.contains("status: member"));
    }

    #[test]
    fn member_line_falls_back_to_raw_role_id_when_unknown() {
        let roles = HashMap::new();
        let line = member_line(&member("usr_1", vec!["grol_unknown"], None), &roles);
        assert!(line.contains("grol_unknown"));
        assert!(line.contains("status: unknown"));
    }

    #[test]
    fn member_line_shows_none_for_no_roles() {
        let roles = HashMap::new();
        let line = member_line(&member("usr_1", vec![], Some("member")), &roles);
        assert!(line.contains("roles: (none)"));
    }

    #[test]
    fn announcement_text_reports_no_announcement_for_an_empty_object() {
        let announcement = scarlet_vrchat::models::GroupAnnouncement::default();
        assert!(announcement.is_empty());
        assert!(announcement_text(&announcement).contains("No announcement is currently set"));
    }

    #[test]
    fn announcement_text_renders_title_and_body() {
        let announcement = scarlet_vrchat::models::GroupAnnouncement {
            id: Some("gann_1".to_string()),
            title: Some("Event tonight".to_string()),
            text: Some("Come join us!".to_string()),
            ..Default::default()
        };
        let rendered = announcement_text(&announcement);
        assert!(rendered.contains("Event tonight"));
        assert!(rendered.contains("Come join us!"));
    }
}
