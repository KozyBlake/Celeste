//! Component/modal interaction routing. The first ported flow is Java's
//! stateful `new-instance-*` picker from `ScarletDiscordUI`.

use serenity::builder::{
    CreateActionRow, CreateInputText, CreateInteractionResponse, CreateInteractionResponseFollowup,
    CreateInteractionResponseMessage, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::ActionRowComponent;
use serenity::model::application::{
    ComponentInteraction, ComponentInteractionDataKind, ModalInteraction,
};
use serenity::model::prelude::InputTextStyle;

use scarlet_vrchat::client::{
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

async fn handle_audit_aux_webhooks(
    ctx: &Context,
    interaction: &ComponentInteraction,
) -> serenity::Result<()> {
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
    let scarlet_data = {
        ctx.data
            .read()
            .await
            .get::<crate::data::ScarletDataKey>()
            .cloned()
    };
    let vrchat_slot = {
        ctx.data
            .read()
            .await
            .get::<crate::data::VrChatClientKey>()
            .cloned()
    };
    let Some(actor_id) = scarlet_data
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
            "Scarlet isn't logged in to VRChat right now.",
        )
        .await;
    };
    let Some(client) = vrchat_slot.read().await.clone() else {
        return ephemeral_component(
            ctx,
            interaction,
            "Scarlet isn't logged in to VRChat right now.",
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
                format!("Could not identify Scarlet's VRChat account: {err}"),
            )
            .await
        }
    };
    for permission in required {
        for (user_id, missing_message) in [
            (&actor_id, "You do not have"),
            (&self_id, "Scarlet's VRChat account does not have"),
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
    if interaction.data.custom_id == crate::commands::ticket_config::MESSAGE_MODAL_ID {
        if let Err(err) = handle_ticket_message_modal(ctx, interaction).await {
            tracing::error!("Error handling Ticket Tool message modal: {err}");
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
    let context = match crate::commands::moderation_context_for_snowflake(
        ctx,
        &interaction.user.id.to_string(),
        scarlet_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
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
        if context.client.get_user(id).await.is_err() {
            failed.push(format!("`{id}` (user does not appear to exist)"));
            continue;
        }
        let is_banned = match context.client.get_group_member(&context.group_id, id).await {
            Ok(Some(member)) => member.banned_at.is_some(),
            Ok(None) => false,
            Err(err) => {
                failed.push(format!("`{id}` (status check failed: {err})"));
                continue;
            }
        };
        if is_banned == ban {
            skipped += 1;
            continue;
        }
        let result = if ban {
            context.client.ban_from_group(&context.group_id, id).await
        } else {
            context.client.unban_from_group(&context.group_id, id).await
        };
        match result {
            Ok(_) => changed += 1,
            Err(err) => failed.push(format!("`{id}` ({err})")),
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
                "\n…and {} more failure(s); see Scarlet's log for details.",
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

async fn handle_watched_notes_modal(
    ctx: &Context,
    interaction: &ModalInteraction,
) -> serenity::Result<()> {
    let parts: Vec<_> = interaction.data.custom_id.split(':').collect();
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
    use super::*;

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
}
