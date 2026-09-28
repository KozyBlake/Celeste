//! Port of (part of) `ScarletDiscordCommands.VrchatGroups`
//! (`@SlashCmd("vrchat-group")` — a subcommand-group command). Java's
//! `VrchatGroups` has ~15 subcommands; ported so far: `add-role`,
//! `remove-role`, `audit-types`, `search-members`, `list-bans`, `list-invites`,
//! `list-join-requests`, `view-announcement`, `create-announcement`,
//! `delete-announcement`, `list-posts`, `create-post`, `delete-post`,
//! `transfer-check`, `transfer-start`, `transfer-cancel`, `open-instance`,
//! `close-instance`. `open-instance` includes Java's full stateful picker:
//! access, roles, region, content flags, avatar gate, create/cancel buttons,
//! and the additional-options modal.
//!
//! `vrchat-user`/`vrchat-role` use Java-compatible autocomplete;
//! `post-role` uses the configured group's roles, matching Java's autocomplete-resolved
//! `io.github.vrchatapi.model.User`/`GroupRole` (see the `commands` module
//! docs). `search-members`/`list-bans`/`list-invites`/`list-join-requests`/
//! `list-posts` render each item as plain text instead of Java's paginated
//! per-item embeds (`discord.interactions.Pagination` with
//! `MessageEmbed[]`) — still one entry per item with the same core fields,
//! just plain text split via [`super::paginate_on_lines`].
//! `create-announcement`/`create-post` pass Java's optional existing
//! `image-file-id` through to VRChat. `delete-announcement`/`delete-post` keep
//! Java's exact safety check: `confirm-delete` must be the literal string
//! `DELETE`; `transfer-start`/`transfer-cancel` likewise require
//! `confirm-group-id` to exactly match the configured group id.
//! `close-instance` splits `vrchat-location` on the first `:` the same way
//! Java's `Location.of` does, without porting the rest of `Location`'s
//! tilde-delimited field parsing (owner/type/region/nonce) since
//! `closeInstance` itself never uses those fields either. Permission
//! checking uses [`super::moderation_context`] for every subcommand here
//! except `add-role`/`remove-role`, which intentionally have no permission
//! gate, matching Java's own ungated originals; `transfer-start`/
//! `transfer-cancel` check the group-id confirmation with the ungated
//! [`super::vrchat_context`] first (matching Java, which checks the
//! confirmation before the permission gate) and only then run the real
//! `group_all`-permission-gated transfer.

use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption,
    CreateInteractionResponseFollowup, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::model::prelude::ButtonStyle;

use scarlet_vrchat::client::{
    PERMISSION_ALL, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, PERMISSION_GROUP_AUDIT_VIEW,
    PERMISSION_GROUP_BANS_MANAGE, PERMISSION_GROUP_INSTANCE_MANAGE,
    PERMISSION_GROUP_INVITES_MANAGE, PERMISSION_GROUP_MEMBERS_VIEWALL,
};

pub const NAME: &str = "vrchat-group";

pub fn register() -> CreateCommand {
    let user_and_role_options = [
        CreateCommandOption::new(
            CommandOptionType::String,
            "vrchat-user",
            "The VRChat user id (usr_...)",
        )
        .required(true)
        .set_autocomplete(true),
        CreateCommandOption::new(
            CommandOptionType::String,
            "vrchat-role",
            "The VRChat group role id (grol_...)",
        )
        .required(true)
        .set_autocomplete(true),
    ];

    let mut command = CreateCommand::new(NAME).description("Manages the configured VRChat group");
    command = command.add_option(CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "audit-types",
        "Lists audit event types currently present in this group",
    ));
    for (name, desc) in [
        ("add-role", "Adds a VRChat Group Role"),
        ("remove-role", "Removes a VRChat Group Role"),
    ] {
        let mut option = CreateCommandOption::new(CommandOptionType::SubCommand, name, desc);
        for sub_option in user_and_role_options.clone() {
            option = option.add_sub_option(sub_option);
        }
        command = command.add_option(option);
    }

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "search-members",
            "Searches this group's members",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "member-search",
                "Name or user id to search within this group",
            )
            .required(true)
            .min_length(1)
            .max_length(100),
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "result-limit",
                "Maximum results to fetch (1-100)",
            )
            .min_int_value(1)
            .max_int_value(100),
        )
        .add_sub_option(super::pagination_option()),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list-bans",
            "Lists banned users in this group",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "result-limit",
                "Maximum results to fetch (1-100)",
            )
            .min_int_value(1)
            .max_int_value(100),
        )
        .add_sub_option(super::pagination_option()),
    );

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list-invites",
            "Lists pending invites sent by this group",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "result-limit",
                "Maximum results to fetch (1-100)",
            )
            .min_int_value(1)
            .max_int_value(100),
        )
        .add_sub_option(super::pagination_option()),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list-join-requests",
            "Lists pending or blocked join requests for this group",
        )
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "blocked-requests",
            "Show blocked requests instead of pending requests",
        ))
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "result-limit",
                "Maximum results to fetch (1-100)",
            )
            .min_int_value(1)
            .max_int_value(100),
        )
        .add_sub_option(super::pagination_option()),
    );

    command = command.add_option(CreateCommandOption::new(
        CommandOptionType::SubCommand,
        "view-announcement",
        "Shows the current group announcement",
    ));
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "create-announcement",
            "Creates or replaces the group announcement",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "announcement-title",
                "Announcement title",
            )
            .required(true)
            .min_length(1)
            .max_length(100),
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "announcement-text",
                "Announcement text",
            )
            .required(true)
            .min_length(1)
            .max_length(1000),
        )
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "send-notification",
            "Send a notification to group members",
        ))
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::String,
            "image-file-id",
            "Optional VRChat file id for the announcement image",
        )),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "delete-announcement",
            "Deletes the current group announcement",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "confirm-delete",
                "Type DELETE exactly to confirm",
            )
            .required(true),
        ),
    );

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "list-posts",
            "Lists recent group posts",
        )
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "public-only",
            "Only show public posts",
        ))
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "result-limit",
                "Maximum results to fetch (1-100)",
            )
            .min_int_value(1)
            .max_int_value(100),
        )
        .add_sub_option(super::pagination_option()),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "create-post",
            "Creates a group post",
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "post-title", "Post title")
                .required(true)
                .max_length(100),
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "post-text", "Post body")
                .required(true)
                .max_length(2000),
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "post-visibility",
                "Who can see the post",
            )
            .required(true)
            .add_string_choice("Group Members", "group")
            .add_string_choice("Public", "public"),
        )
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "send-notification",
            "Notify group members",
        ))
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::String,
            "image-file-id",
            "Optional VRChat file id for the post image",
        ))
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::String,
            "post-role",
            "Optional VRChat group role id (grol_...) that can see a group-only post",
        ).set_autocomplete(true)),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "delete-post",
            "Deletes a group post",
        )
        .add_sub_option(
            CreateCommandOption::new(CommandOptionType::String, "post-id", "The group post id")
                .required(true),
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "confirm-delete",
                "Type DELETE exactly to confirm",
            )
            .required(true),
        ),
    );

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "transfer-check",
            "Checks whether a user can receive group ownership",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "vrchat-user",
                "The VRChat user id (usr_...)",
            )
            .required(true)
            .set_autocomplete(true),
        ),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "transfer-start",
            "Starts or accepts group ownership transfer",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "vrchat-user",
                "The VRChat user id (usr_...)",
            )
            .required(true)
            .set_autocomplete(true),
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "confirm-group-id",
                "Type this group's id exactly to confirm",
            )
            .required(true),
        ),
    );
    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "transfer-cancel",
            "Cancels an active group ownership transfer",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "confirm-group-id",
                "Type this group's id exactly to confirm",
            )
            .required(true),
        ),
    );

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "open-instance",
            "Opens a group instance",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "vrchat-world",
                "The VRChat world id (wrld_...)",
            )
            .required(true)
            .set_autocomplete(true),
        ),
    );

    command = command.add_option(
        CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "close-instance",
            "Closes an instance",
        )
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "vrchat-location",
                "The instance to close (worldId:instanceId)",
            )
            .required(true),
        )
        .add_sub_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "close-hard",
            "Whether to kick users currently in the instance",
        ))
        .add_sub_option(
            CreateCommandOption::new(
                CommandOptionType::Integer,
                "close-in-minutes",
                "The number of minutes to wait before closing the instance (1-600)",
            )
            .min_int_value(1)
            .max_int_value(600),
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
        "audit-types" => audit_types(ctx, command).await,
        "add-role" => role_change(ctx, command, sub_options, true).await,
        "remove-role" => role_change(ctx, command, sub_options, false).await,
        "search-members" => search_members(ctx, command, sub_options).await,
        "list-bans" => list_bans(ctx, command, sub_options).await,
        "list-invites" => list_invites(ctx, command, sub_options).await,
        "list-join-requests" => list_join_requests(ctx, command, sub_options).await,
        "view-announcement" => view_announcement(ctx, command).await,
        "create-announcement" => create_announcement(ctx, command, sub_options).await,
        "delete-announcement" => delete_announcement(ctx, command, sub_options).await,
        "list-posts" => list_posts(ctx, command, sub_options).await,
        "create-post" => create_post(ctx, command, sub_options).await,
        "delete-post" => delete_post(ctx, command, sub_options).await,
        "transfer-check" => transfer_check(ctx, command, sub_options).await,
        "transfer-start" => transfer_start(ctx, command, sub_options).await,
        "transfer-cancel" => transfer_cancel(ctx, command, sub_options).await,
        "open-instance" => open_instance(ctx, command, sub_options).await,
        "close-instance" => close_instance(ctx, command, sub_options).await,
        other => {
            super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await
        }
    }
}

async fn audit_types(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let context = match super::group_moderation_context(ctx, command, PERMISSION_GROUP_AUDIT_VIEW, "view audit event types").await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let mut types = match context
        .client
        .get_group_audit_log_entry_types(&context.group_id)
        .await
    {
        Ok(types) => types,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch audit event types from VRChat: {err}"),
            )
            .await
        }
    };
    if types.is_empty() {
        return super::reply_ephemeral(
            ctx,
            command,
            "No audit event types were returned for this group.",
        )
        .await;
    }
    types.sort();
    let mut output = vec!["### Group audit event types".to_string()];
    output.extend(
        types
            .iter()
            .map(|event_type| format!("- `{event_type}` - {}", audit_type_title(event_type))),
    );
    super::send_pages(
        ctx,
        command,
        super::paginate_on_lines(&output.join("\n"), 2000),
    )
    .await
}

fn audit_type_title(event_type: &str) -> String {
    match event_type {
        "group.instance.close" => "Instance Close",
        "group.instance.create" => "Instance Create",
        "group.instance.kick" => "Instance Kick",
        "group.instance.warn" => "Instance Warn",
        "group.invite.create" => "Invite Create",
        "group.invite.cancel" => "Invite Cancel",
        "group.member.join" => "Member Join",
        "group.member.leave" => "Member Leave",
        "group.member.remove" => "Member Remove",
        "group.member.role.assign" => "Member Role Assign",
        "group.member.role.unassign" => "Member Role Unassign",
        "group.member.user.update" => "Member User Update",
        "group.instance.announcement" => "Announcement",
        "group.post.create" => "Post Create",
        "group.post.delete" => "Post Delete",
        "group.post.update" => "Post Update",
        "group.calendarEvent.create" => "Event Create",
        "group.calendarEvent.delete" => "Event Delete",
        "group.request.block" => "Request Block",
        "group.request.create" => "Request Create",
        "group.request.reject" => "Request Reject",
        "group.role.create" => "Role Create",
        "group.role.delete" => "Role Delete",
        "group.role.update" => "Role Update",
        "group.gallery.create" => "Gallery Create",
        "group.gallery.delete" => "Gallery Delete",
        "group.gallery.update" => "Gallery Update",
        "group.transfer.accept" => "Transfer Accept",
        "group.transfer.cancel" => "Transfer Cancel",
        "group.transfer.start" => "Transfer Start",
        "group.create" => "Create",
        "group.update" => "Update",
        "group.user.ban" => "User Ban",
        "group.user.unban" => "User Unban",
        other => {
            return other
                .strip_prefix("group.")
                .unwrap_or(other)
                .split('.')
                .map(title_word)
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
    .to_string()
}

fn title_word(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn find_string<'a>(options: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::String(s) => Some(*s),
            _ => None,
        })
}

fn find_int(options: &[ResolvedOption<'_>], name: &str) -> Option<i64> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
            ResolvedValue::Integer(n) => Some(*n),
            _ => None,
        })
}

fn find_bool(options: &[ResolvedOption<'_>], name: &str) -> Option<bool> {
    options
        .iter()
        .find(|o| o.name == name)
        .and_then(|o| match &o.value {
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
    let (Some(user_id), Some(role_id)) = (
        find_string(sub_options, "vrchat-user"),
        find_string(sub_options, "vrchat-role"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-user` and `vrchat-role` are both required.",
        )
        .await;
    };

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {user_id} ({err})"),
            )
            .await;
        }
    };
    let role_name = role_names(&context.client, &context.group_id)
        .await
        .remove(role_id)
        .unwrap_or_else(|| role_id.to_string());
    let user_link = format!("https://vrchat.com/home/user/{user_id}");
    let role_link = format!(
        "https://vrchat.com/home/group/{}/settings/roles/{role_id}",
        context.group_id
    );

    if let Ok(Some(member)) = context
        .client
        .get_group_member(&context.group_id, user_id)
        .await
    {
        let has_role = member.role_ids.contains(&role_id.to_string());
        if add && has_role {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "VRChat user [{display_name}]({user_link}) already has the role [{role_name}]({role_link})"
                ),
            )
            .await;
        }
        if !add && !has_role {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "VRChat user [{display_name}]({user_link}) already lacks the role [{role_name}]({role_link})"
                ),
            )
            .await;
        }
    }

    let result = if add {
        context
            .client
            .add_group_member_role(&context.group_id, user_id, role_id)
            .await
    } else {
        context
            .client
            .remove_group_member_role(&context.group_id, user_id, role_id)
            .await
    };

    match result {
        Ok(_) => {
            let verb = if add { "Adding" } else { "Removing" };
            let prep = if add { "to" } else { "from" };
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!(
                            "{verb} the role [{role_name}]({role_link}) {prep} [{display_name}]({user_link})"
                        ))
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
/// `pub(crate)` so `staff_list`/`secret_staff_list` can reuse it rather than
/// duplicating the same fresh-`get_group_roles`-lookup logic.
pub(crate) async fn role_names(
    client: &scarlet_vrchat::client::VrChatClient,
    group_id: &str,
) -> std::collections::HashMap<String, String> {
    match client.get_group_roles(group_id).await {
        Ok(roles) => roles.into_iter().map(|r| (r.id, r.name)).collect(),
        Err(_) => std::collections::HashMap::new(),
    }
}

fn member_line(
    member: &scarlet_vrchat::models::GroupMember,
    roles_by_id: &std::collections::HashMap<String, String>,
) -> String {
    let user_id = member.user_id.as_deref().unwrap_or("unknown");
    let role_names: Vec<&str> = member
        .role_ids
        .iter()
        .map(|id| {
            roles_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(id.as_str())
        })
        .collect();
    let roles = if role_names.is_empty() {
        "(none)".to_string()
    } else {
        role_names.join(", ")
    };
    let status = member.membership_status.as_deref().unwrap_or("unknown");
    format!("[`{user_id}`](<https://vrchat.com/home/user/{user_id}>) — status: {status}, roles: {roles}")
}

async fn search_members(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(query) = find_string(sub_options, "member-search") else {
        return super::reply_ephemeral(ctx, command, "`member-search` is required.").await;
    };
    let limit = find_int(sub_options, "result-limit")
        .unwrap_or(25)
        .clamp(1, 100) as u32;

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_MEMBERS_VIEWALL, "search group members").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    let result = match context
        .client
        .search_group_members(&context.group_id, query, limit, 0)
        .await
    {
        Ok(result) => result,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to search group members: {err}"),
            )
            .await
        }
    };
    if result.results.is_empty() {
        return super::reply_ephemeral(ctx, command, "No group members matched that search.").await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = Vec::new();
    lines.extend(result.results.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(
        ctx,
        command,
        super::paginate_entries(
            &lines,
            super::entries_per_page(sub_options),
            "\n",
            Some("### Group Member search results"),
        ),
    )
    .await
}

async fn list_bans(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let limit = find_int(sub_options, "result-limit")
        .unwrap_or(25)
        .clamp(1, 100) as u32;

    let context = match super::group_moderation_context(ctx, command, PERMISSION_GROUP_BANS_MANAGE, "list group bans").await
    {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let bans = match context
        .client
        .get_group_bans(&context.group_id, limit, 0)
        .await
    {
        Ok(bans) => bans,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch group bans: {err}"),
            )
            .await
        }
    };
    if bans.is_empty() {
        return super::reply_ephemeral(ctx, command, "No banned users were returned.").await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = Vec::new();
    lines.extend(bans.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n", Some("### Group Ban list")),
    )
    .await
}

async fn list_invites(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let limit = find_int(sub_options, "result-limit")
        .unwrap_or(25)
        .clamp(1, 100) as u32;

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_INVITES_MANAGE, "list group invites").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    let invites = match context
        .client
        .get_group_invites(&context.group_id, limit, 0)
        .await
    {
        Ok(invites) => invites,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch group invites: {err}"),
            )
            .await
        }
    };
    if invites.is_empty() {
        return super::reply_ephemeral(ctx, command, "No pending group invites were returned.")
            .await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = Vec::new();
    lines.extend(invites.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n", Some("### Group Invite list")),
    )
    .await
}

async fn list_join_requests(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let blocked = find_bool(sub_options, "blocked-requests").unwrap_or(false);
    let limit = find_int(sub_options, "result-limit")
        .unwrap_or(25)
        .clamp(1, 100) as u32;

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_INVITES_MANAGE, "list group join requests").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    let requests = match context
        .client
        .get_group_join_requests(&context.group_id, blocked, limit, 0)
        .await
    {
        Ok(requests) => requests,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch join requests: {err}"),
            )
            .await
        }
    };
    if requests.is_empty() {
        let message = if blocked {
            "No blocked join requests were returned."
        } else {
            "No pending join requests were returned."
        };
        return super::reply_ephemeral(ctx, command, message).await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let heading = if blocked {
        "### Blocked Join Request list"
    } else {
        "### Join Request list"
    };
    let mut lines = Vec::new();
    lines.extend(requests.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n", Some(heading)),
    )
    .await
}

async fn view_announcement(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "view group announcements").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    match context
        .client
        .get_group_announcement(&context.group_id)
        .await
    {
        Ok(announcement) => {
            super::reply_ephemeral(ctx, command, &announcement_text(&announcement)).await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch the group announcement: {err}"),
            )
            .await
        }
    }
}

async fn create_announcement(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let (Some(title), Some(text)) = (
        find_string(sub_options, "announcement-title"),
        find_string(sub_options, "announcement-text"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`announcement-title` and `announcement-text` are both required.",
        )
        .await;
    };
    let send_notification = find_bool(sub_options, "send-notification").unwrap_or(false);
    let image_id = find_string(sub_options, "image-file-id");

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "create group announcements").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    match context
        .client
        .create_group_announcement(&context.group_id, title, text, send_notification, image_id)
        .await
    {
        Ok(announcement) => command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new()
                    .content(format!(
                        "Created group announcement.\n{}",
                        announcement_text(&announcement)
                    ))
                    .ephemeral(true),
            )
            .await
            .map(|_| ()),
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to create the group announcement: {err}"),
            )
            .await
        }
    }
}

async fn delete_announcement(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
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

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "delete group announcements").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    match context
        .client
        .delete_group_announcement(&context.group_id)
        .await
    {
        Ok(()) => super::reply_ephemeral(ctx, command, "Deleted the group announcement.").await,
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to delete the group announcement: {err}"),
            )
            .await
        }
    }
}

fn post_line(post: &scarlet_vrchat::models::GroupPost) -> String {
    let id = post.id.as_deref().unwrap_or("unknown");
    let title = post
        .title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(id);
    let visibility = post.visibility.as_deref().unwrap_or("unknown");
    format!(
        "### {title}\n`{id}` — visibility: {visibility}\n{}",
        post.text.as_deref().unwrap_or("")
    )
}

async fn list_posts(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let public_only = find_bool(sub_options, "public-only").unwrap_or(false);
    let limit = find_int(sub_options, "result-limit")
        .unwrap_or(25)
        .clamp(1, 100) as u32;

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "list group posts").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    let posts = match context
        .client
        .get_group_posts(&context.group_id, public_only, limit, 0)
        .await
    {
        Ok(posts) => posts,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to fetch group posts: {err}"),
            )
            .await
        }
    };
    if posts.is_empty() {
        return super::reply_ephemeral(ctx, command, "No group posts were returned.").await;
    }

    let lines: Vec<String> = posts.iter().map(post_line).collect();
    super::send_pages(
        ctx,
        command,
        super::paginate_entries(&lines, super::entries_per_page(sub_options), "\n\n", None),
    )
    .await
}

async fn create_post(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let (Some(title), Some(text), Some(visibility)) = (
        find_string(sub_options, "post-title"),
        find_string(sub_options, "post-text"),
        find_string(sub_options, "post-visibility"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`post-title`, `post-text`, and `post-visibility` are all required.",
        )
        .await;
    };
    let send_notification = find_bool(sub_options, "send-notification").unwrap_or(false);
    let image_id = find_string(sub_options, "image-file-id");
    let role_id = find_string(sub_options, "post-role");

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "create group posts").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    match context
        .client
        .create_group_post(
            &context.group_id,
            title,
            text,
            visibility,
            send_notification,
            image_id,
            role_id,
        )
        .await
    {
        Ok(post) => command
            .create_followup(
                &ctx.http,
                CreateInteractionResponseFollowup::new()
                    .content(format!("Created group post.\n{}", post_line(&post)))
                    .ephemeral(true),
            )
            .await
            .map(|_| ()),
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to create the group post: {err}"),
            )
            .await
        }
    }
}

async fn delete_post(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(post_id) = find_string(sub_options, "post-id") else {
        return super::reply_ephemeral(ctx, command, "`post-id` is required.").await;
    };
    let Some(confirm) = find_string(sub_options, "confirm-delete") else {
        return super::reply_ephemeral(ctx, command, "`confirm-delete` is required.").await;
    };
    if confirm != "DELETE" {
        return super::reply_ephemeral(
            ctx,
            command,
            "Deletion cancelled. Type `DELETE` exactly in the confirmation option to delete the post.",
        )
        .await;
    }

    let context =
        match super::group_moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, "delete group posts").await {
            Ok(context) => context,
            Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
        };

    match context
        .client
        .delete_group_post(&context.group_id, post_id)
        .await
    {
        Ok(()) => {
            super::reply_ephemeral(ctx, command, &format!("Deleted group post `{post_id}`.")).await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to delete group post `{post_id}`: {err}"),
            )
            .await
        }
    }
}

fn yes_no(value: Option<bool>) -> &'static str {
    // Java: nullable Boolean -> Unknown / Yes / No.
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

async fn transfer_check(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(user_id) = find_string(sub_options, "vrchat-user") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user` is required.").await;
    };

    let context = match super::group_moderation_context(ctx, command, PERMISSION_ALL, "check group transfer eligibility").await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {user_id} ({err})"),
            )
            .await;
        }
    };

    let transferable = match context
        .client
        .get_group_transferability(&context.group_id, user_id)
        .await
    {
        Ok(transferable) => transferable,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to check group transfer eligibility: {err}"),
            )
            .await;
        }
    };
    let Some(req) = transferable.requirements else {
        return super::reply_ephemeral(
            ctx,
            command,
            "VRChat did not return transfer eligibility details.",
        )
        .await;
    };

    let message = format!(
        "### Group Transfer Check: {display_name}\n\
        Group not monetized: {}\n\
        Target has VRC+: {}\n\
        Target has verified email: {}\n\
        Target can own more groups: {}\n\
        Target is group member: {}",
        yes_no(req.group_not_monetized),
        yes_no(req.has_vrc_plus),
        yes_no(req.has_verified_email),
        yes_no(req.target_can_own_more_groups),
        yes_no(req.target_is_group_member),
    );
    super::reply_ephemeral(ctx, command, &message).await
}

async fn transfer_start(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let (Some(user_id), Some(confirm_group_id)) = (
        find_string(sub_options, "vrchat-user"),
        find_string(sub_options, "confirm-group-id"),
    ) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "`vrchat-user` and `confirm-group-id` are both required.",
        )
        .await;
    };

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if confirm_group_id != context.group_id {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!(
                "Transfer cancelled. The confirmation must exactly match this group id: `{}`",
                context.group_id
            ),
        )
        .await;
    }

    let context = match super::group_moderation_context(ctx, command, PERMISSION_ALL, "transfer group ownership").await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {user_id} ({err})"),
            )
            .await;
        }
    };

    match context
        .client
        .initiate_or_accept_group_transfer(&context.group_id, user_id)
        .await
    {
        Ok(()) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Submitted the group transfer request for {display_name}."),
            )
            .await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to submit the group transfer request: {err}"),
            )
            .await
        }
    }
}

async fn transfer_cancel(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(confirm_group_id) = find_string(sub_options, "confirm-group-id") else {
        return super::reply_ephemeral(ctx, command, "`confirm-group-id` is required.").await;
    };

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if confirm_group_id != context.group_id {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Transfer cancellation skipped. The confirmation must exactly match this group id: `{}`", context.group_id),
        )
        .await;
    }

    let context = match super::group_moderation_context(ctx, command, PERMISSION_ALL, "cancel group ownership transfer").await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context
        .client
        .cancel_group_transfer(&context.group_id)
        .await
    {
        Ok(()) => {
            super::reply_ephemeral(ctx, command, "Cancelled the active group transfer.").await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to cancel the group transfer: {err}"),
            )
            .await
        }
    }
}

/// Splits a VRChat location string into `(worldId, instanceId)` at the
/// first `:`, matching `Location.of(String)`'s own split. Java's `Location`
/// goes on to parse the instance half into structured fields (owner, type,
/// region, nonce...) for other display purposes, but `closeInstance` itself
/// only ever needs the two raw halves, so that structured parsing isn't
/// ported here -- not a gap for this subcommand, since Java doesn't use it
/// for this call either.
fn split_location(location: &str) -> Option<(&str, &str)> {
    let (world, instance) = location.split_once(':')?;
    if world.is_empty() || instance.is_empty() {
        return None;
    }
    Some((world, instance))
}

async fn open_instance(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(world_id) = find_string(sub_options, "vrchat-world") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-world` is required.").await;
    };
    let scarlet_data = {
        ctx.data
            .read()
            .await
            .get::<crate::data::ScarletDataKey>()
            .cloned()
    };
    let Some(actor_id) =
        scarlet_data.and_then(|data| data.global_get_snowflake_id(&command.user.id.to_string()))
    else {
        return super::reply_ephemeral(
            ctx,
            command,
            "You must link your Discord and VRChat accounts before creating an instance.",
        )
        .await;
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if let Err(err) = context.client.get_user(&actor_id).await {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("No VRChat user found with id {actor_id}: {err}"),
        )
        .await;
    }
    let (world, roles) = tokio::join!(
        context.client.get_world(world_id),
        context.client.get_group_roles(&context.group_id)
    );
    let world = match world {
        Ok(world) => world,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat world found with id {world_id}: {err}"),
            )
            .await
        }
    };
    let roles = match roles {
        Ok(roles) => roles,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not load the group's roles: {err}"),
            )
            .await
        }
    };

    let token = uuid::Uuid::new_v4().to_string();
    let creations = {
        ctx.data
            .read()
            .await
            .get::<crate::data::InstanceCreationsKey>()
            .cloned()
    };
    let Some(creations) = creations else {
        return super::reply_ephemeral(ctx, command, "Instance creation state is unavailable.")
            .await;
    };
    creations.write().await.insert(
        token.clone(),
        crate::interactions::InstanceCreation::new(
            world.id.clone(),
            context.group_id,
            command.user.id.to_string(),
        ),
    );

    let access = CreateSelectMenu::new(
        format!("new-instance-access-type:{token}"),
        CreateSelectMenuKind::String {
            options: vec![
                CreateSelectMenuOption::new("Group Public", "public").default_selection(true),
                CreateSelectMenuOption::new("Group Plus", "plus"),
                CreateSelectMenuOption::new("Group Members", "members"),
            ],
        },
    );
    let role_options: Vec<_> = roles
        .into_iter()
        .take(25)
        .map(|role| CreateSelectMenuOption::new(role.name, role.id))
        .collect();
    let role_count = role_options.len() as u8;
    let roles = (!role_options.is_empty()).then(|| {
        CreateSelectMenu::new(
            format!("new-instance-roles:{token}"),
            CreateSelectMenuKind::String {
                options: role_options,
            },
        )
        .placeholder("Select roles (Group Members)")
        .min_values(1)
        .max_values(role_count)
    });
    let region = CreateSelectMenu::new(
        format!("new-instance-region:{token}"),
        CreateSelectMenuKind::String {
            options: vec![
                CreateSelectMenuOption::new("Region: US", "us").default_selection(true),
                CreateSelectMenuOption::new("Region: US East", "use"),
                CreateSelectMenuOption::new("Region: Europe", "eu"),
                CreateSelectMenuOption::new("Region: Japan", "jp"),
            ],
        },
    );
    let default_flag = |label: &str, value: &str| {
        CreateSelectMenuOption::new(label, value).default_selection(true)
    };
    let flags = CreateSelectMenu::new(
        format!("new-instance-flags:{token}"),
        CreateSelectMenuKind::String {
            options: vec![
                default_flag(
                    "Join queue (users wait to connect when full)",
                    "queueEnabled",
                ),
                CreateSelectMenuOption::new(
                    "Hard close (kick connected users on close)",
                    "hardClose",
                ),
                CreateSelectMenuOption::new("Age gate (Age Verified 18+ users only)", "ageGate"),
                CreateSelectMenuOption::new("Invite Only", "inviteOnly"),
                CreateSelectMenuOption::new("Invite Requests", "canRequestInvite"),
                default_flag("Content: drones", "contentSettings.drones"),
                default_flag("Content: emoji", "contentSettings.emoji"),
                default_flag("Content: items", "contentSettings.props"),
                default_flag("Content: pedestals", "contentSettings.pedestals"),
                default_flag("Content: prints", "contentSettings.prints"),
                default_flag("Content: stickers", "contentSettings.stickers"),
                CreateSelectMenuOption::new(
                    "Avatar gate: Poor or better",
                    "minimumAvatarPerformance.Poor",
                ),
                CreateSelectMenuOption::new(
                    "Avatar gate: Medium or better",
                    "minimumAvatarPerformance.Medium",
                ),
                CreateSelectMenuOption::new(
                    "Avatar gate: Good or better",
                    "minimumAvatarPerformance.Good",
                ),
            ],
        },
    )
    .placeholder("Options & Content Settings")
    .min_values(0)
    .max_values(14);
    let buttons = vec![
        CreateButton::new(format!("new-instance-create:{token}"))
            .label("Create")
            .style(ButtonStyle::Success),
        CreateButton::new(format!("new-instance-cancel:{token}"))
            .label("Cancel")
            .style(ButtonStyle::Danger),
        CreateButton::new(format!("new-instance-modal:{token}"))
            .label("Additional options…")
            .style(ButtonStyle::Secondary),
    ];
    let mut components = vec![CreateActionRow::SelectMenu(access)];
    if let Some(roles) = roles {
        components.push(CreateActionRow::SelectMenu(roles));
    }
    components.extend([
        CreateActionRow::SelectMenu(region),
        CreateActionRow::SelectMenu(flags),
        CreateActionRow::Buttons(buttons),
    ]);
    command.create_followup(&ctx.http, CreateInteractionResponseFollowup::new()
        .content(format!("Create a new [{}](https://vrchat.com/home/world/{}) instance:\nLeave avatar gate unchecked for no limit; select one Poor, Medium, or Good preset to set the minimum avatar performance.", world.name, world.id))
        .components(components).ephemeral(true)).await.map(|_| ())
}

async fn close_instance(
    ctx: &Context,
    command: &CommandInteraction,
    sub_options: &[ResolvedOption<'_>],
) -> serenity::Result<()> {
    let Some(location) = find_string(sub_options, "vrchat-location") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-location` is required.").await;
    };
    let Some((world_id, instance_id)) = split_location(location) else {
        return super::reply_ephemeral(
            ctx,
            command,
            "Invalid instance location. Expected format: `worldId:instanceId`.",
        )
        .await;
    };
    let hard_close = find_bool(sub_options, "close-hard");
    let close_in_minutes = find_int(sub_options, "close-in-minutes");
    let closed_at =
        close_in_minutes.map(|minutes| chrono::Utc::now() + chrono::Duration::minutes(minutes));

    let context = match super::member_only_moderation_context(
        ctx,
        command,
        PERMISSION_GROUP_INSTANCE_MANAGE,
        "You do not have permission to close instances",
    )
    .await
    {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context
        .client
        .close_instance(world_id, instance_id, hard_close, closed_at)
        .await
    {
        Ok(instance) => {
            let id = instance.id.as_deref().unwrap_or(instance_id);
            super::reply_ephemeral(ctx, command, &format!("Closed instance `{id}`")).await
        }
        Err(err) => {
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to close instance `{location}`: {err}"),
            )
            .await
        }
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
        assert!(
            line.contains("Moderator"),
            "expected resolved role name in: {line}"
        );
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

    #[test]
    fn post_line_falls_back_to_id_when_title_is_blank() {
        let post = scarlet_vrchat::models::GroupPost {
            id: Some("not_1".to_string()),
            title: Some("".to_string()),
            text: Some("body".to_string()),
            visibility: Some("group".to_string()),
            ..Default::default()
        };
        let line = post_line(&post);
        assert!(line.contains("not_1"));
        assert!(line.contains("visibility: group"));
        assert!(line.contains("body"));
    }

    #[test]
    fn post_line_renders_title_when_present() {
        let post = scarlet_vrchat::models::GroupPost {
            id: Some("not_1".to_string()),
            title: Some("Big Event".to_string()),
            ..Default::default()
        };
        assert!(post_line(&post).contains("Big Event"));
    }

    #[test]
    fn split_location_splits_on_first_colon() {
        assert_eq!(split_location("wrld_1:12345"), Some(("wrld_1", "12345")));
        assert_eq!(
            split_location("wrld_1:12345~region(us)"),
            Some(("wrld_1", "12345~region(us)"))
        );
    }

    #[test]
    fn split_location_rejects_missing_colon_or_empty_halves() {
        assert_eq!(split_location("wrld_1"), None);
        assert_eq!(split_location(":12345"), None);
        assert_eq!(split_location("wrld_1:"), None);
    }

    #[test]
    fn yes_no_formats_booleans() {
        assert_eq!(yes_no(Some(true)), "Yes");
        assert_eq!(yes_no(Some(false)), "No");
        assert_eq!(yes_no(None), "Unknown");
    }

    #[test]
    fn audit_titles_match_java_and_fallback_cleanly() {
        assert_eq!(
            audit_type_title("group.calendarEvent.create"),
            "Event Create"
        );
        assert_eq!(audit_type_title("group.user.ban"), "User Ban");
        assert_eq!(audit_type_title("group.future.action"), "Future Action");
    }

    #[test]
    fn java_string_length_limits_are_registered() {
        let value = serde_json::to_value(register()).unwrap();
        let options = value["options"].as_array().unwrap();
        let option = |subcommand: &str, name: &str| {
            options
                .iter()
                .find(|option| option["name"] == subcommand)
                .and_then(|subcommand| subcommand["options"].as_array())
                .and_then(|options| options.iter().find(|option| option["name"] == name))
                .cloned()
                .unwrap()
        };
        let search = option("search-members", "member-search");
        assert_eq!(search["min_length"], 1);
        assert_eq!(search["max_length"], 100);
        let title = option("create-announcement", "announcement-title");
        assert_eq!(title["min_length"], 1);
        assert_eq!(title["max_length"], 100);
        let text = option("create-announcement", "announcement-text");
        assert_eq!(text["min_length"], 1);
        assert_eq!(text["max_length"], 1000);
    }
}
