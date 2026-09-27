//! Port of (part of) `ScarletDiscordCommands.VrchatGroups`
//! (`@SlashCmd("vrchat-group")` — a subcommand-group command). Java's
//! `VrchatGroups` has ~15 subcommands; ported so far: `add-role`,
//! `remove-role`, `search-members`, `list-bans`, `list-invites`,
//! `list-join-requests`, `view-announcement`, `create-announcement`,
//! `delete-announcement`, `list-posts`, `create-post`, `delete-post`,
//! `transfer-check`, `transfer-start`, `transfer-cancel`, `close-instance`.
//! **Not ported**: `open-instance` -- Java's version builds an interactive
//! flow (select menus for access type/roles/region/content flags, then
//! Create/Cancel/"Additional options..." buttons, then a followup modal)
//! backed by `ScarletDiscordJDA.instanceCreation` state and component/modal
//! interaction handlers. This crate has no component- or modal-interaction
//! plumbing at all yet (only slash-command interactions are dispatched) --
//! that's a separate, larger subsystem to build first, not something to
//! fake as a single-shot command without losing the real picker UX. Tracked
//! in PORTING.md rather than silently skipped.
//!
//! **Simplified**: `vrchat-user-id`/`vrchat-role-id`/`post-role-id` are
//! plain string options rather than Java's autocomplete-resolved
//! `io.github.vrchatapi.model.User`/`GroupRole` (see the `commands` module
//! docs). `search-members`/`list-bans`/`list-invites`/`list-join-requests`/
//! `list-posts` render each item as plain text instead of Java's paginated
//! per-item embeds (`discord.interactions.Pagination` with
//! `MessageEmbed[]`) — still one entry per item with the same core fields,
//! just plain text split via [`super::paginate_on_lines`].
//! `create-announcement`/`create-post` drop the optional `image-file-id`
//! option (VRChat file uploads aren't ported — a Discord attachment would
//! need to be re-uploaded to VRChat's file API first, a separate
//! not-yet-ported endpoint). `delete-announcement`/`delete-post` keep
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

use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue};

use scarlet_vrchat::client::{
    PERMISSION_ALL, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE, PERMISSION_GROUP_BANS_MANAGE, PERMISSION_GROUP_INSTANCE_MANAGE,
    PERMISSION_GROUP_INVITES_MANAGE, PERMISSION_GROUP_MEMBERS_VIEWALL,
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

    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "list-invites", "Lists pending invites sent by this group")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Integer, "result-limit", "Maximum results to fetch (1-100)")
                    .min_int_value(1)
                    .max_int_value(100),
            ),
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

    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "list-posts", "Lists recent group posts")
            .add_sub_option(CreateCommandOption::new(CommandOptionType::Boolean, "public-only", "Only show public posts"))
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Integer, "result-limit", "Maximum results to fetch (1-100)")
                    .min_int_value(1)
                    .max_int_value(100),
            ),
    );
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "create-post", "Creates a group post")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "post-title", "Post title").required(true).max_length(100),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "post-text", "Post body").required(true).max_length(2000),
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "post-visibility", "Who can see the post")
                    .required(true)
                    .add_string_choice("Group Members", "group")
                    .add_string_choice("Public", "public"),
            )
            .add_sub_option(CreateCommandOption::new(CommandOptionType::Boolean, "send-notification", "Notify group members"))
            .add_sub_option(CreateCommandOption::new(
                CommandOptionType::String,
                "post-role-id",
                "Optional VRChat group role id (grol_...) that can see a group-only post",
            )),
    );
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "delete-post", "Deletes a group post")
            .add_sub_option(CreateCommandOption::new(CommandOptionType::String, "post-id", "The group post id").required(true))
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "confirm-delete", "Type DELETE exactly to confirm")
                    .required(true),
            ),
    );

    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "transfer-check", "Checks whether a user can receive group ownership")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)").required(true),
            ),
    );
    command = command.add_option(
        CreateCommandOption::new(CommandOptionType::SubCommand, "transfer-start", "Starts or accepts group ownership transfer")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-user-id", "The VRChat user id (usr_...)").required(true),
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
        CreateCommandOption::new(CommandOptionType::SubCommand, "transfer-cancel", "Cancels an active group ownership transfer")
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
        CreateCommandOption::new(CommandOptionType::SubCommand, "close-instance", "Closes an instance")
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::String, "vrchat-location", "The instance to close (worldId:instanceId)")
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
        "close-instance" => close_instance(ctx, command, sub_options).await,
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

async fn list_invites(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let limit = find_int(sub_options, "result-limit").unwrap_or(25).clamp(1, 100) as u32;

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_INVITES_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let invites = match context.client.get_group_invites(&context.group_id, limit, 0).await {
        Ok(invites) => invites,
        Err(err) => return super::reply_ephemeral(ctx, command, &format!("Failed to fetch group invites: {err}")).await,
    };
    if invites.is_empty() {
        return super::reply_ephemeral(ctx, command, "No pending group invites were returned.").await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let mut lines = vec!["### Group Invite list".to_string()];
    lines.extend(invites.iter().map(|m| member_line(m, &roles_by_id)));

    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n"), 2000)).await
}

async fn list_join_requests(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let blocked = find_bool(sub_options, "blocked-requests").unwrap_or(false);
    let limit = find_int(sub_options, "result-limit").unwrap_or(25).clamp(1, 100) as u32;

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_INVITES_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let requests = match context.client.get_group_join_requests(&context.group_id, blocked, limit, 0).await {
        Ok(requests) => requests,
        Err(err) => return super::reply_ephemeral(ctx, command, &format!("Failed to fetch join requests: {err}")).await,
    };
    if requests.is_empty() {
        let message = if blocked { "No blocked join requests were returned." } else { "No pending join requests were returned." };
        return super::reply_ephemeral(ctx, command, message).await;
    }

    let roles_by_id = role_names(&context.client, &context.group_id).await;
    let heading = if blocked { "### Blocked Join Request list" } else { "### Join Request list" };
    let mut lines = vec![heading.to_string()];
    lines.extend(requests.iter().map(|m| member_line(m, &roles_by_id)));

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

fn post_line(post: &scarlet_vrchat::models::GroupPost) -> String {
    let id = post.id.as_deref().unwrap_or("unknown");
    let title = post.title.as_deref().filter(|t| !t.is_empty()).unwrap_or(id);
    let visibility = post.visibility.as_deref().unwrap_or("unknown");
    format!("### {title}\n`{id}` — visibility: {visibility}\n{}", post.text.as_deref().unwrap_or(""))
}

async fn list_posts(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let public_only = find_bool(sub_options, "public-only").unwrap_or(false);
    let limit = find_int(sub_options, "result-limit").unwrap_or(25).clamp(1, 100) as u32;

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let posts = match context.client.get_group_posts(&context.group_id, public_only, limit, 0).await {
        Ok(posts) => posts,
        Err(err) => return super::reply_ephemeral(ctx, command, &format!("Failed to fetch group posts: {err}")).await,
    };
    if posts.is_empty() {
        return super::reply_ephemeral(ctx, command, "No group posts were returned.").await;
    }

    let lines: Vec<String> = posts.iter().map(post_line).collect();
    super::send_pages(ctx, command, super::paginate_on_lines(&lines.join("\n\n"), 2000)).await
}

async fn create_post(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let (Some(title), Some(text), Some(visibility)) = (
        find_string(sub_options, "post-title"),
        find_string(sub_options, "post-text"),
        find_string(sub_options, "post-visibility"),
    ) else {
        return super::reply_ephemeral(ctx, command, "`post-title`, `post-text`, and `post-visibility` are all required.").await;
    };
    let send_notification = find_bool(sub_options, "send-notification").unwrap_or(false);
    let role_id = find_string(sub_options, "post-role-id");

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.create_group_post(&context.group_id, title, text, visibility, send_notification, role_id).await {
        Ok(post) => {
            command
                .create_followup(
                    &ctx.http,
                    CreateInteractionResponseFollowup::new()
                        .content(format!("Created group post.\n{}", post_line(&post)))
                        .ephemeral(true),
                )
                .await
                .map(|_| ())
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to create the group post: {err}")).await,
    }
}

async fn delete_post(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
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

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_ANNOUNCEMENT_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.delete_group_post(&context.group_id, post_id).await {
        Ok(()) => super::reply_ephemeral(ctx, command, &format!("Deleted group post `{post_id}`.")).await,
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to delete group post `{post_id}`: {err}")).await,
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

async fn transfer_check(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let Some(user_id) = find_string(sub_options, "vrchat-user-id") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user-id` is required.").await;
    };

    let context = match super::moderation_context(ctx, command, PERMISSION_ALL).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {user_id} ({err})")).await;
        }
    };

    let transferable = match context.client.get_group_transferability(&context.group_id, user_id).await {
        Ok(transferable) => transferable,
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("Failed to check group transfer eligibility: {err}")).await;
        }
    };
    let Some(req) = transferable.requirements else {
        return super::reply_ephemeral(ctx, command, "VRChat did not return transfer eligibility details.").await;
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

async fn transfer_start(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let (Some(user_id), Some(confirm_group_id)) =
        (find_string(sub_options, "vrchat-user-id"), find_string(sub_options, "confirm-group-id"))
    else {
        return super::reply_ephemeral(ctx, command, "`vrchat-user-id` and `confirm-group-id` are both required.").await;
    };

    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    if confirm_group_id != context.group_id {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("Transfer cancelled. The confirmation must exactly match this group id: `{}`", context.group_id),
        )
        .await;
    }

    let context = match super::moderation_context(ctx, command, PERMISSION_ALL).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    let display_name = match context.client.get_user(user_id).await {
        Ok(user) => user.display_name.unwrap_or_else(|| user_id.to_string()),
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("No VRChat user found with id {user_id} ({err})")).await;
        }
    };

    match context.client.initiate_or_accept_group_transfer(&context.group_id, user_id).await {
        Ok(()) => super::reply_ephemeral(ctx, command, &format!("Submitted the group transfer request for {display_name}.")).await,
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to submit the group transfer request: {err}")).await,
    }
}

async fn transfer_cancel(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
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

    let context = match super::moderation_context(ctx, command, PERMISSION_ALL).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.cancel_group_transfer(&context.group_id).await {
        Ok(()) => super::reply_ephemeral(ctx, command, "Cancelled the active group transfer.").await,
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to cancel the group transfer: {err}")).await,
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

async fn close_instance(ctx: &Context, command: &CommandInteraction, sub_options: &[ResolvedOption<'_>]) -> serenity::Result<()> {
    let Some(location) = find_string(sub_options, "vrchat-location") else {
        return super::reply_ephemeral(ctx, command, "`vrchat-location` is required.").await;
    };
    let Some((world_id, instance_id)) = split_location(location) else {
        return super::reply_ephemeral(ctx, command, "Invalid instance location. Expected format: `worldId:instanceId`.").await;
    };
    let hard_close = find_bool(sub_options, "close-hard");
    let close_in_minutes = find_int(sub_options, "close-in-minutes");
    let closed_at = close_in_minutes.map(|minutes| chrono::Utc::now() + chrono::Duration::minutes(minutes));

    let context = match super::moderation_context(ctx, command, PERMISSION_GROUP_INSTANCE_MANAGE).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };

    match context.client.close_instance(world_id, instance_id, hard_close, closed_at).await {
        Ok(instance) => {
            let id = instance.id.as_deref().unwrap_or(instance_id);
            super::reply_ephemeral(ctx, command, &format!("Closed instance `{id}`")).await
        }
        Err(err) => super::reply_ephemeral(ctx, command, &format!("Failed to close instance `{location}`: {err}")).await,
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
        assert_eq!(split_location("wrld_1:12345~region(us)"), Some(("wrld_1", "12345~region(us)")));
    }

    #[test]
    fn split_location_rejects_missing_colon_or_empty_halves() {
        assert_eq!(split_location("wrld_1"), None);
        assert_eq!(split_location(":12345"), None);
        assert_eq!(split_location("wrld_1:"), None);
    }

    #[test]
    fn yes_no_formats_booleans() {
        assert_eq!(yes_no(true), "Yes");
        assert_eq!(yes_no(false), "No");
    }
}
