//! Component/modal interaction routing. The first ported flow is Java's
//! stateful `new-instance-*` picker from `ScarletDiscordUI`.

use serenity::builder::{
    CreateActionRow, CreateEmbed, CreateEmbedAuthor, CreateInputText, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage, CreateModal,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
};
use serenity::client::Context;
use serenity::model::application::ActionRowComponent;
use serenity::model::application::{
    ComponentInteraction, ComponentInteractionDataKind, ModalInteraction,
};
use serenity::model::prelude::InputTextStyle;
use serenity::model::Permissions;

use celeste_vrchat::client::{
    PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE, PERMISSION_GROUP_INSTANCE_OPEN_CREATE,
    PERMISSION_GROUP_INSTANCE_PLUS_CREATE, PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE,
    PERMISSION_GROUP_INSTANCE_RESTRICTED_CREATE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupAccessType {
    Public,
    Plus,
    Members,
}

impl GroupAccessType {
    fn permission(&self) -> &'static str {
        match self {
            Self::Public => PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE,
            Self::Plus => PERMISSION_GROUP_INSTANCE_PLUS_CREATE,
            Self::Members => PERMISSION_GROUP_INSTANCE_OPEN_CREATE,
        }
    }

    fn as_wire_value(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Plus => "plus",
            Self::Members => "members",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstanceCreation {
    pub world_id: String,
    pub group_id: String,
    pub owner_snowflake: String,
    pub region: String,
    pub role_ids: Option<Vec<String>>,
    pub access_type: GroupAccessType,
    pub queue_enabled: bool,
    pub hard_close: bool,
    pub age_gate: bool,
    pub invite_only: Option<bool>,
    pub can_request_invite: Option<bool>,
    pub display_name: Option<String>,
    pub minimum_avatar_performance: Option<String>,
    pub content_drones: bool,
    pub content_emoji: bool,
    pub content_props: bool,
    pub content_pedestals: bool,
    pub content_prints: bool,
    pub content_stickers: bool,
}

impl InstanceCreation {
    pub fn new(world_id: String, group_id: String, owner_snowflake: String) -> Self {
        Self {
            world_id,
            group_id,
            owner_snowflake,
            region: "us".into(),
            role_ids: None,
            access_type: GroupAccessType::Public,
            queue_enabled: true,
            hard_close: false,
            age_gate: false,
            invite_only: None,
            can_request_invite: None,
            display_name: None,
            minimum_avatar_performance: None,
            content_drones: true,
            content_emoji: true,
            content_props: true,
            content_pedestals: true,
            content_prints: true,
            content_stickers: true,
        }
    }

    fn request(&self) -> serde_json::Value {
        let mut request = serde_json::json!({
            "worldId": self.world_id,
            "ownerId": self.group_id,
            "type": "group",
            "region": self.region,
            "groupAccessType": self.access_type.as_wire_value(),
            "queueEnabled": self.queue_enabled,
            "hardClose": self.hard_close,
            "ageGate": self.age_gate,
            "contentSettings": {
                "drones": self.content_drones,
                "emoji": self.content_emoji,
                "props": self.content_props,
                "pedestals": self.content_pedestals,
                "prints": self.content_prints,
                "stickers": self.content_stickers
            }
        });
        let object = request.as_object_mut().expect("json object");
        if self.access_type == GroupAccessType::Members {
            if let Some(role_ids) = &self.role_ids {
                object.insert("roleIds".into(), serde_json::json!(role_ids));
            }
        }
        if let Some(display_name) = self
            .display_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            object.insert("displayName".into(), serde_json::json!(display_name));
        }
        if let Some(invite_only) = self.invite_only {
            object.insert("inviteOnly".into(), serde_json::json!(invite_only));
        }
        if let Some(can_request_invite) = self.can_request_invite {
            object.insert(
                "canRequestInvite".into(),
                serde_json::json!(can_request_invite),
            );
        }
        if let Some(rating) = &self.minimum_avatar_performance {
            object.insert("minimumAvatarPerformance".into(), serde_json::json!(rating));
        }
        request
    }
}

async fn ephemeral_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    message: impl Into<String>,
) -> serenity::Result<()> {
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(message)
                    .ephemeral(true),
            ),
        )
        .await
}

async fn ephemeral_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
    message: impl Into<String>,
) -> serenity::Result<()> {
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(message)
                    .ephemeral(true),
            ),
        )
        .await
}

fn split_custom_id(custom_id: &str) -> Option<(&str, &str)> {
    custom_id
        .split_once(':')
        .filter(|(_, token)| !token.is_empty())
}

pub async fn dispatch_component(ctx: &Context, interaction: &ComponentInteraction) {
    if interaction
        .data
        .custom_id
        .starts_with(crate::commands::settings::MODAL_PREFIX)
    {
        if let Err(err) = crate::commands::settings::component(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error updating setting: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("timed-ban:")
        || interaction.data.custom_id.starts_with("timed-ban-custom:")
    {
        if let Err(err) = handle_timed_ban_button(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling timed ban: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("group-request-") {
        if let Err(err) = handle_group_request(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error responding to group join request: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("import-watched-groups:") {
        if let Err(err) = handle_import_watched_groups(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error importing watched groups from message attachments: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("immediate-ban-") {
        if let Err(err) = handle_immediate_ban_component(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling immediate ban interaction: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("edit-tags:")
        || interaction.data.custom_id.starts_with("browse-tags:")
        || interaction.data.custom_id.starts_with("select-tags")
        || interaction.data.custom_id.starts_with("edit-desc:")
        || interaction.data.custom_id.starts_with("event-redact:")
        || interaction.data.custom_id.starts_with("event-unredact:")
    {
        if let Err(err) = handle_audit_edit_component(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling audit edit interaction: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("view-snapshot-user:")
        || interaction.data.custom_id.starts_with("view-snapshot-user-groups:")
        || interaction.data.custom_id.starts_with("view-snapshot-user-represented-group:")
    {
        if let Err(err) = handle_audit_snapshot(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error showing audit snapshot: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("vrchat-user-edit-manager-notes:") {
        if let Err(err) = handle_manager_notes_button(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error opening VRChat manager notes: {err}");
        }
        return;
    }
    if interaction.data.custom_id == "view-potential-avatar-matches" {
        if let Err(err) = handle_potential_avatar_matches(ctx, interaction).await {
            tracing::error!("Error showing potential avatar matches: {err}");
        }
        return;
    }
    // The Ban/Unban buttons Celeste attaches to its instance-event embeds
    // (port of `ScarletDiscordUI.vrchatUserBan`/`vrchatUserUnban`). Without
    // these the buttons on every emitted embed would do nothing.
    if interaction.data.custom_id.starts_with("vrchat-user-ban:")
        || interaction.data.custom_id.starts_with("vrchat-user-unban:")
    {
        if let Err(err) = handle_embed_ban_button(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling ban/unban button: {err}");
        }
        return;
    }
    if interaction
        .data
        .custom_id
        .starts_with(crate::commands::link_vrchat_account::VERIFY_PREFIX)
    {
        if let Err(err) = crate::commands::link_vrchat_account::verify(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling VRChat link verification: {err}");
        }
        return;
    }
    if interaction
        .data
        .custom_id
        .starts_with("discord-member-confirm:")
        || interaction
            .data
            .custom_id
            .starts_with("discord-member-cancel:")
    {
        if let Err(err) = handle_discord_member_action(ctx, interaction).await {
            tracing::error!("Error handling Discord moderation confirmation: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("discord-ban-confirm:")
        || interaction.data.custom_id.starts_with("discord-kick-confirm:")
        || interaction.data.custom_id == "discord-ban-cancel"
        || interaction.data.custom_id == "discord-kick-cancel"
    {
        if let Err(err) = handle_legacy_discord_member_action(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling the Discord moderation confirmation: {err}");
        }
        return;
    }
    if interaction
        .data
        .custom_id
        .starts_with("set-audit-aux-webhooks:")
    {
        if let Err(err) = handle_audit_aux_webhooks(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling audit auxiliary webhooks: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("watched-remove:") {
        if let Err(err) = handle_watched_remove(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling watched removal: {err}");
        }
        return;
    }
    if !interaction.data.custom_id.starts_with("new-instance-") {
        tracing::warn!(custom_id = %interaction.data.custom_id, "Received unknown component interaction");
        return;
    }
    if let Err(err) = handle_instance_component(ctx, interaction).await {
        tracing::error!(custom_id = %interaction.data.custom_id, "Error handling component interaction: {err}");
    }
}

async fn handle_group_request(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((operation, target_id)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid group join request action.").await;
    };
    let (action, block, past_tense) = match operation {
        "group-request-accept" => ("accept", None, "Accepted"),
        "group-request-reject" => ("reject", Some(false), "Rejected"),
        "group-request-block" => ("reject", Some(true), "Blocked"),
        _ => return ephemeral_component(ctx, interaction, "Unknown group join request action.").await,
    };
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let context = crate::commands::moderation_context_for_snowflake_with_override(
        ctx,
        &interaction.user.id.to_string(),
        interaction.member.as_ref(),
        celeste_vrchat::client::PERMISSION_GROUP_INVITES_MANAGE,
        None,
        None,
    ).await;
    let (message, ephemeral) = match context {
        Ok(context) => {
            // Java (ScarletDiscordUI): `getUser(vrcTargetId, within1day)`.
            let name = context.client.get_user_cached(target_id, celeste_vrchat::entity_cache::within_1_day()).await.ok()
                .and_then(|user| user.display_name)
                .unwrap_or_else(|| target_id.to_string());
            match context.client.respond_group_join_request(
                &context.group_id,
                target_id,
                action,
                block,
            ).await {
                Ok(()) => (format!("{past_tense} {name}'s join request"), false),
                Err(error) => (format!("Failed to respond to {name}'s join request: {error}"), true),
            }
        }
        Err(message) => (message, true),
    };
    let _ = interaction.delete_response(&ctx.http).await;
    interaction.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new().content(message).ephemeral(ephemeral),
    ).await.map(|_| ())
}

async fn handle_import_watched_groups(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let allowed = interaction.member.as_ref().is_some_and(|member| {
        member.permissions.unwrap_or_else(Permissions::empty).intersects(
            Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR,
        )
    });
    if !allowed {
        return ephemeral_component(ctx, interaction, "You do not have permission to import watched groups.").await;
    }
    let Some((_, message_id)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid source message.").await;
    };
    let Ok(message_id) = message_id.parse::<u64>() else {
        return ephemeral_component(ctx, interaction, "Invalid source message.").await;
    };
    let message = match interaction.channel_id.message(&ctx.http, message_id).await {
        Ok(message) => message,
        Err(error) => return ephemeral_component(
            ctx,
            interaction,
            format!("Could not retrieve the source message: {error}"),
        ).await,
    };
    if message.attachments.is_empty() {
        return ephemeral_component(ctx, interaction, "No attachments.").await;
    }
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let watched = { ctx.data.read().await.get::<crate::data::WatchedGroupsKey>().cloned() };
    let Some(watched) = watched else {
        return interaction.create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().content("Watched groups are unavailable.").ephemeral(true),
        ).await.map(|_| ());
    };
    let mut results = Vec::new();
    for attachment in &message.attachments {
        match attachment.download().await {
            Ok(bytes) => results.push(crate::commands::watched_group::import_attachment(
                &watched,
                &attachment.filename,
                &bytes,
            )),
            Err(error) => results.push(format!("Could not download `{}`: {error}", attachment.filename)),
        }
    }
    interaction.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new().content(results.join("\n")).ephemeral(true),
    ).await.map(|_| ())
}

async fn audit_edit_data(
    ctx: &Context,
) -> Option<(
    std::sync::Arc<celeste_core::data::CelesteData>,
    std::sync::Arc<celeste_core::moderation_tags::ModerationTags>,
)> {
    let data = ctx.data.read().await;
    Some((
        data.get::<crate::data::CelesteDataKey>()?.clone(),
        data.get::<crate::data::ModerationTagsKey>()?.clone(),
    ))
}

fn moderation_access(interaction: &ComponentInteraction) -> bool {
    interaction.member.as_ref().is_some_and(|member| {
        member.permissions.unwrap_or_else(Permissions::empty).intersects(
            Permissions::MODERATE_MEMBERS | Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR,
        )
    })
}

async fn handle_audit_edit_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let custom_id = &interaction.data.custom_id;
    let audit_id = custom_id.rsplit_once(':').map(|(_, id)| id).unwrap_or_default();
    if audit_id.is_empty() {
        return ephemeral_component(ctx, interaction, "Invalid audit event id.").await;
    }
    let Some((data, tags)) = audit_edit_data(ctx).await else {
        return ephemeral_component(ctx, interaction, "Moderation data is unavailable.").await;
    };
    if !data.audit_entry_metadata_exists(audit_id) {
        return ephemeral_component(ctx, interaction, "Failed to find the audit event.").await;
    }

    if custom_id.starts_with("event-redact:") || custom_id.starts_with("event-unredact:") {
        if !moderation_access(interaction) {
            return ephemeral_component(ctx, interaction, "You do not have permission to modify audit event moderation state.").await;
        }
        let redact = custom_id.starts_with("event-redact:");
        let mut metadata = data.audit_entry_metadata(audit_id);
        if metadata.entry_redacted == redact {
            return ephemeral_component(
                ctx,
                interaction,
                if redact { "Event is already redacted." } else { "Event isn't yet redacted." },
            ).await;
        }
        metadata.entry_redacted = redact;
        data.set_audit_entry_metadata(audit_id, metadata.clone());
        crate::commands::tag::refresh_aux_message(ctx, &metadata, &tags).await;
        return ephemeral_component(ctx, interaction, if redact { "Event redacted." } else { "Event unredacted." }).await;
    }

    if let Some(denial) = crate::commands::tag::tag_access_denial_for(
        ctx,
        interaction.member.as_ref(),
        &interaction.user.id.to_string(),
        interaction.channel.as_ref().map(|channel| channel.kind),
        &data,
        audit_id,
    )
    .await
    {
        return ephemeral_component(ctx, interaction, denial).await;
    }

    if custom_id.starts_with("edit-desc:") {
        let metadata = data.audit_entry_metadata(audit_id);
        let mut input = CreateInputText::new(
            InputTextStyle::Paragraph,
            "Input description",
            format!("input-desc:{audit_id}"),
        )
        .placeholder("Event description")
        .required(true);
        if let Some(description) = metadata.entry_description {
            input = input.value(description);
        }
        return interaction.create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(format!("edit-desc:{audit_id}"), "Edit description")
                    .components(vec![CreateActionRow::InputText(input)]),
            ),
        ).await;
    }

    if custom_id.starts_with("select-tags") {
        let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind else {
            return ephemeral_component(ctx, interaction, "Expected tag selections.").await;
        };
        let page = custom_id.strip_prefix("select-tags-")
            .and_then(|rest| rest.split_once(':'))
            .and_then(|(page, _)| page.parse::<usize>().ok())
            .unwrap_or(0);
        let all_tags = tags.get_tags();
        let page_values: Vec<String> = all_tags.iter().skip(page * 25).take(25).map(|tag| tag.value.clone()).collect();
        let remove: Vec<String> = page_values.into_iter().filter(|value| !values.contains(value)).collect();
        let metadata = data.audit_entry_metadata_edit_tags(audit_id, &remove, values.clone());
        crate::commands::tag::refresh_aux_message(ctx, &metadata, &tags).await;
        let shown = metadata.entry_tags.iter().map(|value| tags.get_tag_label(value)).collect::<Vec<_>>();
        return ephemeral_component(ctx, interaction, format!("### Setting tags:\n{}", if shown.is_empty() { "*(none)*".to_string() } else { shown.join(", ") })).await;
    }

    let all_tags = tags.get_tags();
    if all_tags.is_empty() {
        return ephemeral_component(ctx, interaction, "No moderation tags!").await;
    }
    let existing = data.audit_entry_metadata(audit_id).entry_tags;
    let rows = all_tags.chunks(25).take(5).enumerate().map(|(page, chunk)| {
        let options = chunk.iter().map(|tag| {
            let label = if tag.label.is_empty() { &tag.value } else { &tag.label };
            let mut option = CreateSelectMenuOption::new(label.chars().take(100).collect::<String>(), tag.value.clone())
                .default_selection(existing.contains(&tag.value));
            if !tag.description.is_empty() {
                option = option.description(tag.description.chars().take(100).collect::<String>());
            }
            option
        }).collect();
        CreateActionRow::SelectMenu(
            CreateSelectMenu::new(format!("select-tags-{page}:{audit_id}"), CreateSelectMenuKind::String { options })
                .placeholder(format!("Select tags ({}-{})", page * 25 + 1, page * 25 + chunk.len()))
                .min_values(0)
                .max_values(chunk.len() as u8),
        )
    }).collect();
    ephemeral_component_with_rows(ctx, interaction, "Select every tag that applies. Use `/tag add` in the thread to search all tags at once.", rows).await
}

async fn handle_audit_snapshot(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((operation, audit_id)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid audit event id.").await;
    };
    let data = { ctx.data.read().await.get::<crate::data::CelesteDataKey>().cloned() };
    let Some(data) = data else {
        return ephemeral_component(ctx, interaction, "Moderation data is unavailable.").await;
    };
    if !data.audit_entry_metadata_exists(audit_id) {
        return ephemeral_component(ctx, interaction, "Failed to find the audit event.").await;
    }
    let metadata = data.audit_entry_metadata(audit_id);
    let (label, snapshot) = match operation {
        "view-snapshot-user" => ("user", metadata.snapshot_target_user),
        "view-snapshot-user-groups" => ("user groups", metadata.snapshot_target_user_groups),
        "view-snapshot-user-represented-group" => ("user represented group", metadata.snapshot_target_user_represented_group),
        _ => return ephemeral_component(ctx, interaction, "Unknown snapshot type.").await,
    };
    let Some(snapshot) = snapshot else {
        return ephemeral_component(ctx, interaction, format!("This event has no {label} snapshot available.")).await;
    };
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let json = serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| snapshot.to_string());
    let pages = crate::commands::paginate_on_lines(&json, 1850);
    for (index, page) in pages.iter().enumerate() {
        interaction.create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(format!("### Snapshot: {label} ({}/{})\n```json\n{page}\n```", index + 1, pages.len()))
                .ephemeral(true),
        ).await?;
    }
    Ok(())
}

async fn handle_manager_notes_button(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((_, target_id)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid VRChat user id.").await;
    };
    let context = match crate::commands::moderation_context_for_snowflake_with_override(
        ctx,
        &interaction.user.id.to_string(),
        interaction.member.as_ref(),
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some("edit manager notes"),
    ).await {
        Ok(context) => context,
        Err(message) => return ephemeral_component(ctx, interaction, message).await,
    };
    let user = match context.client.get_user_cached(target_id, celeste_vrchat::entity_cache::within_1_day()).await {
        Ok(user) => user,
        Err(_) => return ephemeral_component(ctx, interaction, format!("No VRChat user found with id {target_id}")).await,
    };
    let notes = context.client.get_group_member(&context.group_id, target_id).await
        .ok().flatten().and_then(|member| member.manager_notes).unwrap_or_default();
    let display_name = user.display_name.as_deref().unwrap_or(target_id);
    let title = format!("Manager notes for {}", display_name.chars().take(25).collect::<String>());
    let mut input = CreateInputText::new(
        InputTextStyle::Paragraph,
        "Notes",
        format!("manager-notes:{target_id}"),
    ).required(false);
    if !notes.is_empty() {
        input = input.value(notes);
    }
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Modal(
            CreateModal::new(format!("vrchat-user-edit-manager-notes:{target_id}"), title)
                .components(vec![CreateActionRow::InputText(input)]),
        ),
    ).await
}

fn potential_avatar_ids(interaction: &ComponentInteraction) -> Vec<String> {
    extract_potential_avatar_ids(interaction.message.embeds.iter().flat_map(|embed| {
        embed.description.iter().map(String::as_str)
            .chain(embed.fields.iter().map(|field| field.value.as_str()))
    }))
}

fn extract_potential_avatar_ids<'a>(texts: impl Iterator<Item = &'a str>) -> Vec<String> {
    let pattern = regex::Regex::new(r"avtr_[0-9A-Za-z-]+").expect("static avatar id regex");
    let mut ids = Vec::new();
    for id in texts.flat_map(|text| pattern.find_iter(text).map(|found| found.as_str())) {
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
            if ids.len() == 32 { break; }
        }
    }
    ids
}

async fn handle_potential_avatar_matches(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    if !celeste_core::features::avatar_search_enabled() {
        return ephemeral_component(ctx, interaction, "Avatar search is disabled.").await;
    }
    let ids = potential_avatar_ids(interaction);
    if ids.is_empty() {
        return ephemeral_component(ctx, interaction, "No potential avatar IDs were found in this message.").await;
    }
    let shared = { ctx.data.read().await.get::<crate::data::VrChatClientKey>().cloned() };
    let client = match shared {
        Some(shared) => shared.read().await.clone(),
        None => None,
    };
    let Some(client) = client else {
        return ephemeral_component(ctx, interaction, "VRChat is not logged in.").await;
    };
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let mut embeds = Vec::new();
    for id in ids {
        let Ok(avatar) = client.get_avatar(&id).await else { continue };
        let mut embed = CreateEmbed::new()
            .title(avatar.name)
            .url(format!("https://vrchat.com/home/avatar/{}", avatar.id));
        if let Some(description) = avatar.description.filter(|value| !value.is_empty()) {
            embed = embed.description(description);
        }
        if let Some(image) = avatar.image_url.filter(|value| !value.is_empty()) {
            embed = embed.thumbnail(image);
        }
        if let Some(author) = avatar.author_name.as_deref().or(avatar.author_id.as_deref()) {
            let mut builder = CreateEmbedAuthor::new(author);
            if let Some(author_id) = avatar.author_id.as_deref() {
                builder = builder.url(format!("https://vrchat.com/home/user/{author_id}"));
            }
            embed = embed.author(builder);
        }
        embeds.push(embed);
    }
    if embeds.is_empty() {
        return interaction.create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content("VRChat did not return any of the potential avatars.")
                .ephemeral(true),
        ).await.map(|_| ());
    }
    for page in embeds.chunks(4) {
        interaction.create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().embeds(page.to_vec()).ephemeral(true),
        ).await?;
    }
    Ok(())
}

async fn ephemeral_component_with_rows(
    ctx: &Context,
    interaction: &ComponentInteraction,
    message: &str,
    rows: Vec<CreateActionRow>,
) -> serenity::Result<()> {
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content(message).components(rows).ephemeral(true),
        ),
    ).await
}

async fn handle_discord_member_action(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((action, token)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid moderation confirmation.").await;
    };
    let drafts = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordModerationDraftsKey>()
            .cloned()
    };
    let Some(drafts) = drafts else {
        return ephemeral_component(
            ctx,
            interaction,
            "Discord moderation confirmation state is unavailable.",
        )
        .await;
    };
    let draft = { drafts.read().await.get(token).cloned() };
    let Some(draft) = draft else {
        return ephemeral_component(
            ctx,
            interaction,
            "That moderation confirmation has expired.",
        )
        .await;
    };
    if draft.owner_id != interaction.user.id {
        return ephemeral_component(
            ctx,
            interaction,
            "That moderation confirmation belongs to another user.",
        )
        .await;
    }
    drafts.write().await.remove(token);
    if action == "discord-member-cancel" {
        return interaction
            .create_response(
                &ctx.http,
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .content(format!("{} cancelled.", draft.action.title()))
                        .components(Vec::new()),
                ),
            )
            .await;
    }
    if !interaction
        .app_permissions
        .is_some_and(|permissions| permissions.contains(draft.action.permission()))
    {
        return ephemeral_component(
            ctx,
            interaction,
            format!(
                "The bot does not have the **{} Members** permission in this server.",
                draft.action.title()
            ),
        )
        .await;
    }
    let Some(member) = interaction.member.as_ref() else {
        return ephemeral_component(
            ctx,
            interaction,
            "Could not determine your server membership.",
        )
        .await;
    };
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await?;
    let message = match crate::commands::discord_member_action::perform(ctx, draft, member).await {
        Ok(message) | Err(message) => message,
    };
    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(message)
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

async fn handle_legacy_discord_member_action(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    use crate::commands::discord_member_action::DiscordModerationDraft;

    let custom_id = interaction.data.custom_id.as_str();
    if custom_id == "discord-ban-cancel" || custom_id == "discord-kick-cancel" {
        let title = if custom_id.starts_with("discord-ban") { "Ban" } else { "Kick" };
        return interaction.create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(format!("{title} cancelled."))
                    .components(Vec::new()),
            ),
        ).await;
    }

    let Some((action, target_id, reason)) = parse_legacy_discord_member_action(custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid Discord member id.").await;
    };
    if !interaction.app_permissions.is_some_and(|permissions| permissions.contains(action.permission())) {
        return ephemeral_component(
            ctx,
            interaction,
            format!("The bot does not have the **{} Members** permission in this server.", action.title()),
        ).await;
    }
    let Some(member) = interaction.member.as_ref() else {
        return ephemeral_component(ctx, interaction, "Could not determine your server membership.").await;
    };
    let Some(guild_id) = interaction.guild_id else {
        return ephemeral_component(ctx, interaction, "Could not find the configured Discord server.").await;
    };
    let target_id = serenity::model::id::UserId::new(target_id);
    let target_name = guild_id.member(ctx, target_id).await.ok()
        .map(|member| member.display_name().to_string())
        .unwrap_or_else(|| target_id.to_string());
    let draft = DiscordModerationDraft {
        action,
        owner_id: interaction.user.id,
        guild_id,
        target_id,
        target_name,
        reason,
    };
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let message = match crate::commands::discord_member_action::perform(ctx, draft, member).await {
        Ok(message) | Err(message) => message,
    };
    interaction.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new().content(message).ephemeral(true),
    ).await.map(|_| ())
}

fn parse_legacy_discord_member_action(
    custom_id: &str,
) -> Option<(crate::commands::discord_member_action::MemberAction, u64, Option<String>)> {
    use crate::commands::discord_member_action::MemberAction;
    let mut parts = custom_id.splitn(3, ':');
    let action = match parts.next()? {
        "discord-ban-confirm" => MemberAction::Ban,
        "discord-kick-confirm" => MemberAction::Kick,
        _ => return None,
    };
    let target_id = parts.next()?.parse().ok()?;
    let reason = parts.next().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    Some((action, target_id, reason))
}

/// Port of `ScarletDiscordUI.checkConfigAccess`: MANAGE_SERVER or
/// ADMINISTRATOR, else an ephemeral refusal naming the action and a warning
/// in the log. Java applies this to every interaction that *writes*
/// configuration — audit auxiliary webhooks, watched-group notes,
/// watched-entity notes and the report template — and all of those were
/// reachable here by any member, because the generic component gate
/// default-allows their custom ids.
fn config_access_denial(
    member: Option<&serenity::model::guild::Member>,
    action: &str,
) -> Option<String> {
    let Some(member) = member else {
        return Some("Could not determine your guild permissions.".to_string());
    };
    let permissions = member.permissions.unwrap_or_else(Permissions::empty);
    if permissions.intersects(Permissions::MANAGE_GUILD | Permissions::ADMINISTRATOR) {
        return None;
    }
    tracing::warn!(
        "Rejected unauthorized configuration interaction by {} while trying to {action}",
        member.user.id
    );
    Some(format!("You do not have permission to {action}."))
}

async fn handle_audit_aux_webhooks(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    if let Some(denial) =
        config_access_denial(interaction.member.as_ref(), "set audit auxiliary webhooks")
    {
        return ephemeral_component(ctx, interaction, denial).await;
    }
    let Some((_, event_type)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid audit event type.").await;
    };
    if !crate::commands::set_event_channel::valid_audit_type(event_type) {
        return ephemeral_component(
            ctx,
            interaction,
            format!("{event_type} isn't a valid audit log event type"),
        )
        .await;
    }
    let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind else {
        return ephemeral_component(ctx, interaction, "Expected auxiliary webhook selections.")
            .await;
    };
    let config = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordConfigKey>()
            .cloned()
    };
    let Some(config) = config else {
        return ephemeral_component(ctx, interaction, "Discord configuration is unavailable.")
            .await;
    };
    let known = config.string_map("scarletAuxWh2webhookUrl");
    if let Some(unknown) = values.iter().find(|value| !known.contains_key(*value)) {
        return ephemeral_component(
            ctx,
            interaction,
            format!("Unknown auxiliary webhook `{unknown}`."),
        )
        .await;
    }
    if let Err(err) = config.set_map_strings("auditType2scarletAuxWh", event_type, values) {
        return ephemeral_component(
            ctx,
            interaction,
            format!("Could not save auxiliary webhook assignment: {err}"),
        )
        .await;
    }
    let message = if values.is_empty() {
        format!("Removing auxiliary webhooks for {event_type}")
    } else {
        format!(
            "Setting auxiliary webhooks for {event_type}:\n{}",
            values.join(", ")
        )
    };
    ephemeral_component(ctx, interaction, message).await
}

async fn handle_watched_remove(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let parts: Vec<_> = interaction.data.custom_id.split(':').collect();
    let kind = parts.get(1).copied().unwrap_or_default();
    let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind else {
        return ephemeral_component(ctx, interaction, "Expected watched entries to remove.").await;
    };
    let mut removed = 0usize;
    match kind {
        "group" => {
            let watched = {
                ctx.data
                    .read()
                    .await
                    .get::<crate::data::WatchedGroupsKey>()
                    .cloned()
            };
            if let Some(watched) = watched {
                for id in values {
                    removed += usize::from(watched.remove_watched_group(id));
                }
            }
        }
        "user" | "avatar" => {
            let watched = if kind == "user" {
                ctx.data
                    .read()
                    .await
                    .get::<crate::data::WatchedUsersKey>()
                    .cloned()
            } else {
                ctx.data
                    .read()
                    .await
                    .get::<crate::data::WatchedAvatarsKey>()
                    .cloned()
            };
            if let Some(watched) = watched {
                for id in values {
                    removed += usize::from(watched.remove_watched_entity(id));
                }
            }
        }
        _ => return ephemeral_component(ctx, interaction, "Unknown watched-entry type.").await,
    }
    let plural = if kind == "group" {
        "groups"
    } else if kind == "user" {
        "users"
    } else {
        "avatars"
    };
    ephemeral_component(
        ctx,
        interaction,
        format!("Removed {removed} watched {plural} from the list."),
    )
    .await
}

async fn handle_instance_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((action, token)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(
            ctx,
            interaction,
            "This instance creation interaction is invalid.",
        )
        .await;
    };
    let creations = {
        ctx.data
            .read()
            .await
            .get::<crate::data::InstanceCreationsKey>()
            .cloned()
    };
    let Some(creations) = creations else {
        return ephemeral_component(ctx, interaction, "Instance creation state is unavailable.")
            .await;
    };

    let current = { creations.read().await.get(token).cloned() };
    let Some(current) = current else {
        return ephemeral_component(ctx, interaction, "That instance creation flow has expired.")
            .await;
    };
    if current.owner_snowflake != interaction.user.id.to_string() {
        return ephemeral_component(
            ctx,
            interaction,
            "That instance creation flow belongs to a different user or has expired.",
        )
        .await;
    }

    match action {
        "new-instance-cancel" => {
            creations.write().await.remove(token);
            interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::UpdateMessage(
                        CreateInteractionResponseMessage::new()
                            .content("Instance creation cancelled.")
                            .components(Vec::new()),
                    ),
                )
                .await
        }
        "new-instance-modal" => {
            let mut input = CreateInputText::new(
                InputTextStyle::Short,
                "Display name (unknown purpose)",
                format!("display-name:{token}"),
            )
            .required(false);
            if let Some(value) = current.display_name {
                input = input.value(value);
            }
            interaction
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Modal(
                        CreateModal::new(
                            format!("new-instance-modal:{token}"),
                            "Additional options",
                        )
                        .components(vec![CreateActionRow::InputText(input)]),
                    ),
                )
                .await
        }
        "new-instance-create" => {
            create_instance(ctx, interaction, token, current, &creations).await
        }
        "new-instance-region"
        | "new-instance-access-type"
        | "new-instance-roles"
        | "new-instance-flags" => {
            let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind
            else {
                return ephemeral_component(ctx, interaction, "Expected a selection.").await;
            };
            let mut guard = creations.write().await;
            let Some(state) = guard.get_mut(token) else {
                return ephemeral_component(
                    ctx,
                    interaction,
                    "That instance creation flow has expired.",
                )
                .await;
            };
            match action {
                "new-instance-region" => {
                    if let Some(value @ ("us" | "use" | "eu" | "jp")) =
                        values.first().map(String::as_str)
                    {
                        state.region = value.to_string();
                    }
                }
                "new-instance-access-type" => {
                    state.access_type = match values.first().map(String::as_str) {
                        Some("plus") => GroupAccessType::Plus,
                        Some("members") => GroupAccessType::Members,
                        _ => GroupAccessType::Public,
                    };
                }
                "new-instance-roles" => state.role_ids = Some(values.clone()),
                "new-instance-flags" => {
                    let has = |value: &str| values.iter().any(|selected| selected == value);
                    state.queue_enabled = has("queueEnabled");
                    state.hard_close = has("hardClose");
                    state.age_gate = has("ageGate");
                    state.invite_only = Some(has("inviteOnly"));
                    state.can_request_invite = Some(has("canRequestInvite"));
                    state.content_drones = has("contentSettings.drones");
                    state.content_emoji = has("contentSettings.emoji");
                    state.content_props = has("contentSettings.props");
                    state.content_pedestals = has("contentSettings.pedestals");
                    state.content_prints = has("contentSettings.prints");
                    state.content_stickers = has("contentSettings.stickers");
                    state.minimum_avatar_performance = ["Good", "Medium", "Poor"]
                        .into_iter()
                        .find(|rating| has(&format!("minimumAvatarPerformance.{rating}")))
                        .map(str::to_string);
                }
                _ => unreachable!(),
            }
            drop(guard);
            interaction
                .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
                .await
        }
        _ => ephemeral_component(ctx, interaction, "Unknown instance creation action.").await,
    }
}

async fn create_instance(
    ctx: &Context,
    interaction: &ComponentInteraction,
    token: &str,
    creation: InstanceCreation,
    creations: &crate::data::InstanceCreations,
) -> serenity::Result<()> {
    let celeste_data = {
        ctx.data
            .read()
            .await
            .get::<crate::data::CelesteDataKey>()
            .cloned()
    };
    let vrchat_slot = {
        ctx.data
            .read()
            .await
            .get::<crate::data::VrChatClientKey>()
            .cloned()
    };
    let Some(actor_id) = celeste_data
        .and_then(|data| data.global_get_snowflake_id(&interaction.user.id.to_string()))
    else {
        return ephemeral_component(
            ctx,
            interaction,
            "You must link your Discord and VRChat accounts before creating an instance.",
        )
        .await;
    };
    let Some(vrchat_slot) = vrchat_slot else {
        return ephemeral_component(
            ctx,
            interaction,
            "Celeste isn't logged in to VRChat right now.",
        )
        .await;
    };
    let Some(client) = vrchat_slot.read().await.clone() else {
        return ephemeral_component(
            ctx,
            interaction,
            "Celeste isn't logged in to VRChat right now.",
        )
        .await;
    };

    let mut required = vec![creation.access_type.permission()];
    if creation.access_type == GroupAccessType::Members && creation.role_ids.is_some() {
        required.push(PERMISSION_GROUP_INSTANCE_RESTRICTED_CREATE);
    }
    if creation.age_gate {
        required.push(PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE);
    }
    let self_id = match client.get_current_user().await {
        Ok(user) => user.id,
        Err(err) => {
            return ephemeral_component(
                ctx,
                interaction,
                format!("Could not identify Celeste's VRChat account: {err}"),
            )
            .await
        }
    };
    for permission in required {
        for (user_id, missing_message) in [
            (&actor_id, "You do not have"),
            (&self_id, "Celeste's VRChat account does not have"),
        ] {
            match client
                .user_has_group_permission(&creation.group_id, user_id, permission)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    return ephemeral_component(
                        ctx,
                        interaction,
                        format!("{missing_message} the VRChat permission `{permission}`."),
                    )
                    .await
                }
                Err(err) => {
                    return ephemeral_component(
                        ctx,
                        interaction,
                        format!("Could not check VRChat permissions: {err}"),
                    )
                    .await
                }
            }
        }
    }

    creations.write().await.remove(token);
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content("Creating the VRChat instance…")
                    .components(Vec::new()),
            ),
        )
        .await?;

    let result = client.create_instance(&creation.request()).await;
    let message = match result {
        Ok(instance) => {
            let world_id = instance.world_id.as_deref().unwrap_or(&creation.world_id);
            let instance_id = instance
                .instance_id
                .as_deref()
                .or(instance.id.as_deref())
                .unwrap_or("unknown");
            if let Some(target_id) = instance.id.as_deref().or(instance.instance_id.as_deref()) {
                let pending = {
                    ctx.data.read().await
                        .get::<crate::data::PendingModerationKey>()
                        .cloned()
                };
                if let Some(pending) = pending {
                    if let Err(error) = pending.add_pending("group.instance.create", target_id, &actor_id) {
                        tracing::warn!(%error, target_id, "Could not save instance-creation attribution");
                    }
                }
            }
            format!("Created [new VRChat instance](https://vrchat.com/home/launch?worldId={world_id}&instanceId={instance_id}).")
        }
        Err(err) => format!("Failed to create new instance: {err}"),
    };
    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(message)
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

pub async fn dispatch_modal(ctx: &Context, interaction: &ModalInteraction) {
    if interaction.data.custom_id.starts_with("timed-ban-custom:") {
        if let Err(err) = handle_timed_ban_modal(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling custom timed ban: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("immediate-ban-edit-desc:") {
        if let Err(err) = handle_immediate_ban_description(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling immediate ban description: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("edit-desc:") {
        if let Err(err) = handle_audit_description_modal(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling audit description modal: {err}");
        }
        return;
    }
    if interaction.data.custom_id.starts_with("vrchat-user-edit-manager-notes:") {
        if let Err(err) = handle_manager_notes_modal(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error updating VRChat manager notes: {err}");
        }
        return;
    }
    if interaction.data.custom_id == crate::commands::ticket_config::MESSAGE_MODAL_ID {
        if let Err(err) = handle_ticket_message_modal(ctx, interaction).await {
            tracing::error!("Error handling Ticket Tool message modal: {err}");
        }
        return;
    }
    if is_settings_modal(&interaction.data.custom_id) {
        if let Err(err) = handle_setting_modal(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling settings modal: {err}");
        }
        return;
    }
    if matches!(
        interaction.data.custom_id.as_str(),
        "vrchat-user-ban-multi" | "vrchat-user-unban-multi"
    ) {
        if let Err(err) = handle_vrchat_user_multi(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling multi-user moderation modal: {err}");
        }
        return;
    }
    if interaction
        .data
        .custom_id
        .starts_with("watched-group-set-notes:")
        || interaction
            .data
            .custom_id
            .starts_with("watched-entity-set-notes:")
    {
        if let Err(err) = handle_watched_notes_modal(ctx, interaction).await {
            tracing::error!(custom_id = %interaction.data.custom_id, "Error handling watched-notes modal: {err}");
        }
        return;
    }
    if !interaction
        .data
        .custom_id
        .starts_with("new-instance-modal:")
    {
        tracing::warn!(custom_id = %interaction.data.custom_id, "Received unknown modal interaction");
        return;
    }
    if let Err(err) = handle_instance_modal(ctx, interaction).await {
        tracing::error!(custom_id = %interaction.data.custom_id, "Error handling modal interaction: {err}");
    }
}

fn is_settings_modal(custom_id: &str) -> bool {
    // Java intentionally gives the Ticket Tool editor a `settings:` id too.
    // It has its own Discord config persistence path and must not be consumed
    // by the generic typed-settings registry handler.
    custom_id.starts_with(crate::commands::settings::MODAL_PREFIX)
        && custom_id != crate::commands::ticket_config::MESSAGE_MODAL_ID
}

fn parse_duration_millis(input: &str) -> Option<i64> {
    let captures = regex::Regex::new(r"(?i)^(\d+(?:\.\d+)?)\s*([smhdw]?)$").unwrap()
        .captures(input.trim())?;
    let value: f64 = captures.get(1)?.as_str().parse().ok()?;
    let unit = match captures.get(2).map(|m| m.as_str().to_ascii_lowercase()).as_deref() {
        Some("s") => 1_000.0,
        Some("m") => 60_000.0,
        Some("d") => 86_400_000.0,
        Some("w") => 604_800_000.0,
        _ => 3_600_000.0,
    };
    let millis = (value * unit) as i64;
    (millis > 0).then_some(millis)
}

fn format_duration(millis: i64) -> String {
    let total_minutes = millis / 60_000;
    let days = total_minutes / 1440;
    let hours = (total_minutes % 1440) / 60;
    let minutes = total_minutes % 60;
    let mut output = String::new();
    if days > 0 { output.push_str(&format!("{days}d")); }
    if hours > 0 { output.push_str(&format!("{hours}h")); }
    if minutes > 0 && days == 0 { output.push_str(&format!("{minutes}m")); }
    if output.is_empty() { output = format!("{total_minutes}m"); }
    output
}

async fn handle_timed_ban_button(ctx: &Context, interaction: &ComponentInteraction) -> serenity::Result<()> {
    if interaction.data.custom_id.starts_with("timed-ban-custom:") {
        let target_id = interaction.data.custom_id.strip_prefix("timed-ban-custom:").unwrap_or_default();
        return interaction.create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(CreateModal::new(
                format!("timed-ban-custom:{target_id}"),
                "Timed ban",
            ).components(vec![CreateActionRow::InputText(CreateInputText::new(
                InputTextStyle::Short,
                "Duration (30m, 6h, 3d, 2w, or hours)",
                format!("input-duration:{target_id}"),
            ).required(true).placeholder("7d"))])),
        ).await;
    }
    let parts = interaction.data.custom_id.splitn(3, ':').collect::<Vec<_>>();
    let (Ok(hours), Some(target_id)) = (parts.get(1).unwrap_or(&"").parse::<i64>(), parts.get(2)) else {
        return ephemeral_component(ctx, interaction, "Invalid timed-ban duration").await;
    };
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let (message, ephemeral) = perform_timed_ban(
        ctx,
        interaction.user.id.to_string(),
        interaction.member.as_ref(),
        target_id,
        hours * 3_600_000,
    ).await;
    let _ = interaction.delete_response(&ctx.http).await;
    interaction.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new().content(message).ephemeral(ephemeral),
    ).await.map(|_| ())
}

async fn handle_timed_ban_modal(ctx: &Context, interaction: &ModalInteraction) -> serenity::Result<()> {
    let target_id = interaction.data.custom_id.strip_prefix("timed-ban-custom:").unwrap_or_default();
    let Some(duration) = parse_duration_millis(&modal_text(interaction, &format!("input-duration:{target_id}"))) else {
        return ephemeral_modal(ctx, interaction, "Couldn't read that duration. Try e.g. 30m, 6h, 3d, 2w, or a plain number of hours.").await;
    };
    interaction.create_response(&ctx.http, CreateInteractionResponse::Defer(
        CreateInteractionResponseMessage::new().ephemeral(true),
    )).await?;
    let (message, ephemeral) = perform_timed_ban(ctx, interaction.user.id.to_string(), interaction.member.as_ref(), target_id, duration).await;
    let _ = interaction.delete_response(&ctx.http).await;
    interaction.create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(message).ephemeral(ephemeral)).await.map(|_| ())
}

async fn perform_timed_ban(
    ctx: &Context,
    snowflake: String,
    member: Option<&serenity::model::guild::Member>,
    target_id: &str,
    duration: i64,
) -> (String, bool) {
    let settings = { ctx.data.read().await.get::<crate::data::SettingsKey>().cloned() };
    if !settings.is_some_and(|settings| settings.timed_bans_enabled.get()) {
        return ("Timed bans are disabled in Celeste's settings.".into(), true);
    }
    let context = match crate::commands::moderation_context_for_snowflake_with_override(
        ctx, &snowflake, member, celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some("ban users"),
    ).await {
        Ok(context) => context,
        Err(message) => return (message, true),
    };
    let name = match context.client.get_user_cached(target_id, celeste_vrchat::entity_cache::within_1_day()).await {
        Ok(user) => user.display_name.unwrap_or_else(|| target_id.to_string()),
        Err(error) => return (format!("No VRChat user found with id {target_id} ({error})"), true),
    };
    let already_banned = match context.client.get_group_member(&context.group_id, target_id).await {
        Ok(Some(member)) => crate::commands::vrchat_user_ban::is_banned_member(&member),
        Ok(None) => false,
        Err(error) => return (format!("Could not check ban status: {error}"), true),
    };
    let pending = { ctx.data.read().await.get::<crate::data::PendingModerationKey>().cloned() };
    let Some(pending) = pending else { return ("Pending moderation state is unavailable.".into(), true) };
    let expires_at = chrono::Utc::now().timestamp_millis() + duration;
    let actor_id = context.actor_id.clone().unwrap_or_default();
    if !already_banned {
        if let Err(message) = crate::commands::vrchat_user_ban::execute_with_context(ctx, &context, target_id, true).await {
            return (message, true);
        }
    }
    if let Err(error) = pending.set_timed_ban(target_id, crate::pending_moderation::TimedBan { expires_at, actor_user_id: actor_id }) {
        return (format!("The ban succeeded, but the automatic-unban timer could not be saved: {error}"), true);
    }
    let human = format_duration(duration);
    let epoch = expires_at / 1000;
    let message = if already_banned {
        format!("{name} is already banned — scheduled automatic unban in {human} (<t:{epoch}:R>)")
    } else {
        format!("Banned {name} for {human} — automatic unban <t:{epoch}:R>")
    };
    (message, false)
}

async fn pending_ban_store(
    ctx: &Context,
) -> Option<std::sync::Arc<crate::pending_moderation::PendingModeration>> {
    ctx.data.read().await.get::<crate::data::PendingModerationKey>().cloned()
}

async fn handle_immediate_ban_component(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let Some((operation, target_id)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_component(ctx, interaction, "Invalid pending ban.").await;
    };
    let Some(pending) = pending_ban_store(ctx).await else {
        return ephemeral_component(ctx, interaction, "Pending moderation state is unavailable.").await;
    };
    if !pending.is_ban_info_owner(target_id, &interaction.user.id.to_string()) {
        return ephemeral_component(ctx, interaction, "You do not own this pending ban.").await;
    }

    match operation {
        "immediate-ban-select-tags" => {
            let ComponentInteractionDataKind::StringSelect { values } = &interaction.data.kind else {
                return ephemeral_component(ctx, interaction, "Expected tag selections.").await;
            };
            match pending.set_ban_info_tags(target_id, values.clone()) {
                Ok(true) => interaction.create_response(&ctx.http, CreateInteractionResponse::Acknowledge).await,
                Ok(false) => ephemeral_component(ctx, interaction, "No pending ban").await,
                Err(error) => ephemeral_component(ctx, interaction, format!("Could not save tags: {error}")).await,
            }
        }
        "immediate-ban-edit-desc" => interaction.create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(CreateModal::new(
                format!("immediate-ban-edit-desc:{target_id}"),
                "Edit description",
            ).components(vec![CreateActionRow::InputText(CreateInputText::new(
                InputTextStyle::Paragraph,
                "Input description",
                format!("input-desc:{target_id}"),
            ).required(true).placeholder("Event description"))])),
        ).await,
        "immediate-ban-cancel" => {
            let _ = pending.take_ban_info(target_id);
            interaction.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new().content("Pending ban cancelled.").components(Vec::new()),
            )).await
        }
        "immediate-ban-confirm" => {
            let result = crate::commands::vrchat_user_ban::execute(
                ctx,
                &interaction.user.id.to_string(),
                interaction.member.as_ref(),
                target_id,
                true,
            ).await;
            match result {
                Ok(message) => interaction.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new().content(message).components(Vec::new()),
                )).await,
                Err(message) => ephemeral_component(ctx, interaction, message).await,
            }
        }
        _ => ephemeral_component(ctx, interaction, "Unknown pending-ban action.").await,
    }
}

async fn handle_immediate_ban_description(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let target_id = interaction.data.custom_id.strip_prefix("immediate-ban-edit-desc:").unwrap_or_default();
    let Some(pending) = pending_ban_store(ctx).await else {
        return ephemeral_modal(ctx, interaction, "Pending moderation state is unavailable.").await;
    };
    if !pending.is_ban_info_owner(target_id, &interaction.user.id.to_string()) {
        return ephemeral_modal(ctx, interaction, "You do not own this pending ban.").await;
    }
    let description = modal_text(interaction, &format!("input-desc:{target_id}"));
    match pending.set_ban_info_description(target_id, description) {
        Ok(true) => interaction.create_response(&ctx.http, CreateInteractionResponse::Acknowledge).await,
        Ok(false) => ephemeral_modal(ctx, interaction, "No pending ban").await,
        Err(error) => ephemeral_modal(ctx, interaction, format!("Could not save description: {error}")).await,
    }
}

async fn handle_audit_description_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let audit_id = interaction.data.custom_id.strip_prefix("edit-desc:").unwrap_or_default();
    let Some((data, tags)) = audit_edit_data(ctx).await else {
        return ephemeral_modal(ctx, interaction, "Moderation data is unavailable.").await;
    };
    if let Some(denial) = crate::commands::tag::tag_access_denial_for(
        ctx,
        interaction.member.as_ref(),
        &interaction.user.id.to_string(),
        interaction.channel.as_ref().map(|channel| channel.kind),
        &data,
        audit_id,
    )
    .await
    {
        return ephemeral_modal(ctx, interaction, denial).await;
    }
    let description = modal_text(interaction, &format!("input-desc:{audit_id}"));
    let metadata = data.audit_entry_metadata_set_description(audit_id, Some(description.clone()));
    crate::commands::tag::refresh_aux_message(ctx, &metadata, &tags).await;
    ephemeral_modal(ctx, interaction, format!("### Setting description:\n{description}")).await
}

async fn handle_setting_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let setting_id = interaction
        .data
        .custom_id
        .strip_prefix(crate::commands::settings::MODAL_PREFIX)
        .unwrap_or_default();
    let settings = {
        ctx.data
            .read()
            .await
            .get::<crate::data::SettingsKey>()
            .cloned()
    };
    let Some(settings) = settings else {
        return ephemeral_modal(ctx, interaction, "Settings are unavailable.").await;
    };
    let Some(setting) = settings.registry.get(setting_id) else {
        return ephemeral_modal(
            ctx,
            interaction,
            format!("Unknown setting id `{setting_id}`."),
        )
        .await;
    };
    // Java passes the modal's newline-delimited string array directly to
    // `value.split("\\R")`; `FileValued<Vec<String>>` owns that parsing.
    let value = modal_text(interaction, "value");
    if setting.set_from_string(&value, "discord") {
        ephemeral_modal(
            ctx,
            interaction,
            format!("Set {} to {}", setting.name(), value),
        )
        .await
    } else {
        ephemeral_modal(
            ctx,
            interaction,
            format!("Invalid value {} for {}", value, setting.name()),
        )
        .await
    }
}

async fn handle_ticket_message_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let message =
        crate::commands::ticket_config::normalized_message(&modal_text(interaction, "message"));
    let config = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordConfigKey>()
            .cloned()
    };
    let Some(config) = config else {
        return ephemeral_modal(ctx, interaction, "Discord configuration is unavailable.").await;
    };
    if let Err(err) = config.set_string("ticketToolAgeVerificationMessage", Some(&message)) {
        return ephemeral_modal(
            ctx,
            interaction,
            format!("Could not save Ticket Tool message: {err}"),
        )
        .await;
    }
    ephemeral_modal(
        ctx,
        interaction,
        format!("Ticket Tool age-verification auto-response message updated:\n\n{message}"),
    )
    .await
}

fn find_user_ids(input: &str) -> Vec<String> {
    let regex = regex::Regex::new(
        r"(?:[a-zA-Z0-9]{10}|usr_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})",
    )
    .expect("static VRChat user id regex");
    regex
        .find_iter(input)
        .map(|matched| matched.as_str().to_string())
        .collect()
}

async fn handle_vrchat_user_multi(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let ban = interaction.data.custom_id == "vrchat-user-ban-multi";
    let ids = find_user_ids(&modal_text(interaction, "target-ids"));
    if ids.is_empty() {
        return ephemeral_modal(ctx, interaction, "No user ids found!").await;
    }
    let context = match crate::commands::moderation_context_for_snowflake_with_override(
        ctx,
        &interaction.user.id.to_string(),
        interaction.member.as_ref(),
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some(if ban { "ban users" } else { "unban users" }),
    )
    .await
    {
        Ok(context) => context,
        Err(message) => return ephemeral_modal(ctx, interaction, message).await,
    };
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await?;

    let mut changed = 0usize;
    let mut skipped = 0usize;
    let mut failed = Vec::new();
    for id in &ids {
        match crate::commands::vrchat_user_ban::execute_with_context(ctx, &context, id, ban).await {
            Ok(_) => changed += 1,
            Err(message) if message.contains("already banned") || message.contains("not banned") => {
                skipped += 1;
            }
            Err(message) => failed.push(format!("`{id}` ({message})")),
        }
    }
    let verb = if ban { "Banned" } else { "Unbanned" };
    let mut message =
        format!("{verb} {changed} user(s); skipped {skipped} already-correct user(s).");
    if !failed.is_empty() {
        let shown = failed
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        message.push_str(&format!("\nFailed {} user(s):\n{shown}", failed.len()));
        if failed.len() > 10 {
            message.push_str(&format!(
                "\n…and {} more failure(s); see Celeste's log for details.",
                failed.len() - 10
            ));
            for failure in failed.iter().skip(10) {
                tracing::warn!("Multi-user moderation failure: {failure}");
            }
        }
    }
    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(message)
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

fn modal_text(interaction: &ModalInteraction, custom_id: &str) -> String {
    interaction
        .data
        .components
        .iter()
        .flat_map(|row| &row.components)
        .find_map(|component| match component {
            ActionRowComponent::InputText(input) if input.custom_id == custom_id => {
                input.value.clone()
            }
            _ => None,
        })
        .unwrap_or_default()
}

async fn handle_manager_notes_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let target_id = interaction.data.custom_id
        .strip_prefix("vrchat-user-edit-manager-notes:")
        .unwrap_or_default();
    if target_id.is_empty() {
        return ephemeral_modal(ctx, interaction, "Invalid VRChat user id.").await;
    }
    interaction.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;
    let context = crate::commands::moderation_context_for_snowflake_with_override(
        ctx,
        &interaction.user.id.to_string(),
        interaction.member.as_ref(),
        celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
        Some(celeste_core::permission::CelestePermission::GroupexBansManage.id()),
        Some("edit manager notes"),
    ).await;
    let message = match context {
        Ok(context) => {
            match context.client.get_user_cached(target_id, celeste_vrchat::entity_cache::within_1_day()).await {
                Ok(_) => {
                    let notes = modal_text(interaction, &format!("manager-notes:{target_id}"));
                    match context.client.update_group_member_notes(&context.group_id, target_id, &notes).await {
                        Ok(_) => "Updated manager notes.".to_string(),
                        Err(error) => format!("Failed to update manager notes: {error}"),
                    }
                }
                Err(_) => format!("No VRChat user found with id {target_id}"),
            }
        }
        Err(message) => message,
    };
    interaction.create_followup(
        &ctx.http,
        CreateInteractionResponseFollowup::new().content(message).ephemeral(true),
    ).await.map(|_| ())
}

async fn handle_watched_notes_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let parts: Vec<_> = interaction.data.custom_id.split(':').collect();
    // Java's watched-list commands require only USE_APPLICATION_COMMANDS and
    // the server's DPerms rules. The modal is ephemeral to the authorized
    // slash-command caller, so adding a MANAGE_SERVER gate here prevents a
    // legitimately delegated `set-notes` command from ever completing.
    let notes = modal_text(interaction, "notes");
    let updated = match parts.as_slice() {
        ["watched-group-set-notes", group_id] => {
            let watched = {
                ctx.data
                    .read()
                    .await
                    .get::<crate::data::WatchedGroupsKey>()
                    .cloned()
            };
            watched.is_some_and(|watched| {
                watched
                    .update(group_id, |group| group.notes = Some(notes.clone()))
                    .is_some()
            })
        }
        ["watched-entity-set-notes", kind, entity_id] => {
            let watched = match *kind {
                "user" => ctx
                    .data
                    .read()
                    .await
                    .get::<crate::data::WatchedUsersKey>()
                    .cloned(),
                "avatar" => ctx
                    .data
                    .read()
                    .await
                    .get::<crate::data::WatchedAvatarsKey>()
                    .cloned(),
                _ => None,
            };
            watched.is_some_and(|watched| {
                watched
                    .update(entity_id, |entity| entity.notes = Some(notes.clone()))
                    .is_some()
            })
        }
        _ => false,
    };
    let message = if updated {
        "Set notes."
    } else {
        "That watched entry no longer exists."
    };
    ephemeral_modal(ctx, interaction, message).await
}

async fn handle_instance_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let Some((_, token)) = split_custom_id(&interaction.data.custom_id) else {
        return ephemeral_modal(
            ctx,
            interaction,
            "This instance creation interaction is invalid.",
        )
        .await;
    };
    let creations = {
        ctx.data
            .read()
            .await
            .get::<crate::data::InstanceCreationsKey>()
            .cloned()
    };
    let Some(creations) = creations else {
        return ephemeral_modal(ctx, interaction, "Instance creation state is unavailable.").await;
    };
    let mut guard = creations.write().await;
    let Some(state) = guard.get_mut(token) else {
        return ephemeral_modal(ctx, interaction, "That instance creation flow has expired.").await;
    };
    if state.owner_snowflake != interaction.user.id.to_string() {
        return ephemeral_modal(
            ctx,
            interaction,
            "That instance creation flow belongs to a different user or has expired.",
        )
        .await;
    }
    state.display_name = interaction
        .data
        .components
        .iter()
        .flat_map(|row| &row.components)
        .find_map(|component| match component {
            ActionRowComponent::InputText(input)
                if input.custom_id == format!("display-name:{token}") =>
            {
                input.value.clone()
            }
            _ => None,
        })
        .filter(|value| !value.is_empty());
    drop(guard);
    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
}

#[cfg(test)]
mod tests {
    /// Java gates every configuration-writing interaction on MANAGE_SERVER
    /// or ADMINISTRATOR (`ScarletDiscordUI.checkConfigAccess`). These
    /// handlers had no gate at all, and their custom ids are default-allowed
    /// by the generic component check, so any member could rewrite audit
    /// routing.
    #[test]
    fn config_writes_require_manage_server_or_administrator() {
        fn member_with(permissions: Permissions) -> serenity::model::guild::Member {
            let mut member = serenity::model::guild::Member::default();
            member.permissions = Some(permissions);
            member
        }

        assert!(config_access_denial(None, "do the thing").is_some());
        assert!(
            config_access_denial(Some(&member_with(Permissions::empty())), "do the thing")
                .is_some()
        );
        // A plain "can talk here" permission is not enough.
        assert!(
            config_access_denial(Some(&member_with(Permissions::SEND_MESSAGES)), "do the thing")
                .is_some()
        );
        assert!(
            config_access_denial(Some(&member_with(Permissions::MANAGE_GUILD)), "do the thing")
                .is_none()
        );
        assert!(
            config_access_denial(Some(&member_with(Permissions::ADMINISTRATOR)), "do the thing")
                .is_none()
        );
        assert_eq!(
            config_access_denial(Some(&member_with(Permissions::empty())), "set audit auxiliary webhooks"),
            Some("You do not have permission to set audit auxiliary webhooks.".to_string())
        );
    }

    use super::*;

    #[test]
    fn ticket_message_modal_is_not_swallowed_by_generic_settings() {
        let ticket = crate::commands::ticket_config::MESSAGE_MODAL_ID;
        assert!(ticket.starts_with(crate::commands::settings::MODAL_PREFIX));
        assert!(!is_settings_modal(ticket));
        assert!(is_settings_modal("settings:ui_language"));
    }

    #[test]
    fn request_only_includes_roles_for_member_instances() {
        let mut creation = InstanceCreation::new("wrld_test".into(), "grp_test".into(), "1".into());
        creation.role_ids = Some(vec!["grol_test".into()]);
        assert!(creation.request().get("roleIds").is_none());
        creation.access_type = GroupAccessType::Members;
        assert_eq!(
            creation.request()["roleIds"],
            serde_json::json!(["grol_test"])
        );
    }

    #[test]
    fn request_preserves_content_and_avatar_gate_options() {
        let mut creation = InstanceCreation::new("wrld_test".into(), "grp_test".into(), "1".into());
        creation.content_stickers = false;
        creation.minimum_avatar_performance = Some("Good".into());
        let request = creation.request();
        assert_eq!(request["contentSettings"]["stickers"], false);
        assert_eq!(request["minimumAvatarPerformance"], "Good");
    }

    #[test]
    fn extracts_all_legacy_and_uuid_user_ids() {
        assert_eq!(
            find_user_ids("abcdefghij, usr_12345678-1234-1234-1234-123456789abc\ninvalid"),
            ["abcdefghij", "usr_12345678-1234-1234-1234-123456789abc"]
        );
    }

    #[test]
    fn timed_ban_duration_parser_matches_java_units() {
        assert_eq!(parse_duration_millis("30m"), Some(1_800_000));
        assert_eq!(parse_duration_millis("6"), Some(21_600_000));
        assert_eq!(parse_duration_millis("2w"), Some(1_209_600_000));
        assert_eq!(format_duration(3 * 86_400_000 + 2 * 3_600_000), "3d2h");
        assert_eq!(parse_duration_millis("tomorrow"), None);
    }

    #[test]
    fn parses_java_discord_moderation_confirmation_ids() {
        use crate::commands::discord_member_action::MemberAction;
        assert_eq!(
            parse_legacy_discord_member_action("discord-ban-confirm:123:reason: with colon"),
            Some((MemberAction::Ban, 123, Some("reason: with colon".into())))
        );
        assert_eq!(
            parse_legacy_discord_member_action("discord-kick-confirm:456"),
            Some((MemberAction::Kick, 456, None))
        );
        assert_eq!(parse_legacy_discord_member_action("discord-ban-confirm:not-a-user"), None);
    }

    #[test]
    fn extracts_unique_avatar_ids_from_java_descriptions_and_rust_fields() {
        let texts = [
            "Potential ids:\n`avtr_12345678-1234-1234-1234-123456789abc`",
            "`avtr_abcdefab-abcd-abcd-abcd-abcdefabcdef`\navtr_12345678-1234-1234-1234-123456789abc",
        ];
        assert_eq!(extract_potential_avatar_ids(texts.into_iter()), vec![
            "avtr_12345678-1234-1234-1234-123456789abc",
            "avtr_abcdefab-abcd-abcd-abcd-abcdefabcdef",
        ]);
    }
}

/// Port of `ScarletDiscordUI.vrchatUserBan`/`vrchatUserUnban`: the
/// **Ban user** / **Unban user** buttons on an emitted instance-event
/// embed, whose custom id carries the target (`vrchat-user-ban:usr_...`).
///
/// The action itself — linked-account requirement, group ban permission,
/// already-banned/not-banned checks — is the same implementation the
/// `/vrchat-user-ban` command uses, exactly as Java shares
/// `_vrchatUserBan` between its command and its button.
async fn handle_embed_ban_button(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
    let (ban, target_id) = match interaction.data.custom_id.split_once(':') {
        Some(("vrchat-user-ban", target)) => (true, target.to_string()),
        Some(("vrchat-user-unban", target)) => (false, target.to_string()),
        _ => return Ok(()),
    };
    if target_id.trim().is_empty() {
        return Ok(());
    }

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
        )
        .await?;

    let (content, ephemeral) =
        match crate::commands::vrchat_user_ban::execute(ctx, &interaction.user.id.to_string(), interaction.member.as_ref(), &target_id, ban).await {
            // Java announces a successful ban publicly and keeps every
            // refusal ephemeral.
            Ok(message) => (message, false),
            Err(message) => (message, true),
        };

    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().content(content).ephemeral(ephemeral),
        )
        .await
        .map(|_| ())
}
