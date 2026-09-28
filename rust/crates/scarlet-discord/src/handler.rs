//! Port of `ScarletDiscordJDA.JDAEvents` (the JDA `ListenerAdapter` wired up
//! in the constructor): connection lifecycle, slash-command dispatch, and
//! the gateway events Celeste reacts to. The per-event bodies live in
//! `crate::events`.

use serenity::async_trait;
use serenity::builder::{
    CreateAutocompleteResponse, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use serenity::client::{Context, EventHandler};
use serenity::model::application::{CommandInteraction, Interaction, ResolvedValue};
use serenity::model::channel::{GuildChannel, Message};
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, GuildId};

use crate::commands;

pub struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!(
            "Connected to Discord as {} ({})",
            ready.user.name,
            ready.user.id
        );
        commands::register_global_commands(&ctx).await;
        commands::clear_stale_guild_commands(&ctx).await;
        reconnect_voice_channel(&ctx).await;
    }

    /// Port of `JDAEvents.onChannelCreate`. A new Ticket Tool channel is
    /// what triggers the age-verification auto-response.
    async fn channel_create(&self, ctx: Context, channel: GuildChannel) {
        crate::events::ticket::on_channel_create(&ctx, &channel).await;
    }

    /// Port of `JDAEvents.onMessageReceived`.
    async fn message(&self, ctx: Context, message: Message) {
        crate::events::ticket::on_message(&ctx, &message).await;
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => {
                if slash_permitted(&ctx, &command).await {
                    commands::dispatch(&ctx, &command).await
                } else {
                    deny_command(&ctx, &command).await;
                }
            }
            Interaction::Autocomplete(command) => {
                if slash_permitted(&ctx, &command).await {
                    commands::dispatch_autocomplete(&ctx, &command).await
                } else if let Err(error) = command
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Autocomplete(CreateAutocompleteResponse::new()),
                    )
                    .await
                {
                    tracing::debug!(%error, "Could not reject unauthorized autocomplete");
                }
            }
            Interaction::Component(component) => {
                let permission_type = match &component.data.kind {
                    serenity::model::application::ComponentInteractionDataKind::Button => {
                        crate::permissions::PermissionType::ButtonPress
                    }
                    serenity::model::application::ComponentInteractionDataKind::StringSelect {
                        ..
                    } => crate::permissions::PermissionType::StringSelect,
                    _ => crate::permissions::PermissionType::EntitySelect,
                };
                if interaction_permitted(
                    &ctx,
                    component.guild_id,
                    component.member.as_ref(),
                    permission_type,
                    crate::permissions::operation(&component.data.custom_id),
                )
                .await
                {
                    crate::interactions::dispatch_component(&ctx, &component).await
                } else if let Err(error) = component
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(permission_denied())
                                .ephemeral(true),
                        ),
                    )
                    .await
                {
                    tracing::debug!(%error, "Could not reject unauthorized component interaction");
                }
            }
            Interaction::Modal(modal) => {
                if interaction_permitted(
                    &ctx,
                    modal.guild_id,
                    modal.member.as_ref(),
                    crate::permissions::PermissionType::ModalSubmit,
                    crate::permissions::operation(&modal.data.custom_id),
                )
                .await
                {
                    crate::interactions::dispatch_modal(&ctx, &modal).await
                } else if let Err(error) = modal
                    .create_response(
                        &ctx.http,
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(permission_denied())
                                .ephemeral(true),
                        ),
                    )
                    .await
                {
                    tracing::debug!(%error, "Could not reject unauthorized modal interaction");
                }
            }
            _ => {}
        }
    }
}

fn permission_denied() -> &'static str {
    "You do not have permission to perform this action.\n(Have your admin use `/scarlet-discord-permissions` to assign permissions)"
}

async fn configured_guild(ctx: &Context, guild_id: Option<GuildId>) -> bool {
    let configured = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordConfigKey>()
            .and_then(|config| config.string("guildSf"))
    };
    match (configured.and_then(|id| id.parse::<u64>().ok()), guild_id) {
        (Some(configured), Some(actual)) => configured == actual.get(),
        _ => false,
    }
}

async fn interaction_permitted(
    ctx: &Context,
    guild_id: Option<GuildId>,
    member: Option<&serenity::model::guild::Member>,
    kind: crate::permissions::PermissionType,
    operation: &str,
) -> bool {
    if !configured_guild(ctx, guild_id).await {
        return false;
    }
    let Some(member) = member else { return false };
    if member_is_owner_or_admin(ctx, guild_id, member) {
        return true;
    }
    let permissions = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordPermissionsKey>()
            .cloned()
    };
    permissions.is_some_and(|permissions| permissions.check(kind, member, operation, false))
}

async fn slash_permitted(ctx: &Context, command: &CommandInteraction) -> bool {
    if !configured_guild(ctx, command.guild_id).await {
        return false;
    }
    let Some(member) = command.member.as_ref() else {
        return false;
    };
    if member_is_owner_or_admin(ctx, command.guild_id, member) {
        return true;
    }
    let options = command.data.options();
    let (group, subcommand) = match options.first() {
        Some(option) => match &option.value {
            ResolvedValue::SubCommand(_) => (None, Some(option.name)),
            ResolvedValue::SubCommandGroup(children) => {
                (Some(option.name), children.first().map(|child| child.name))
            }
            _ => (None, None),
        },
        None => (None, None),
    };
    let permissions = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordPermissionsKey>()
            .cloned()
    };
    permissions.is_some_and(|permissions| {
        permissions.check_slash(member, &command.data.name, group, subcommand)
    })
}

pub(crate) fn member_is_owner_or_admin(
    ctx: &Context,
    guild_id: Option<GuildId>,
    member: &serenity::model::guild::Member,
) -> bool {
    if member.permissions.is_some_and(|permissions| {
        permissions.contains(serenity::model::permissions::Permissions::ADMINISTRATOR)
    }) {
        return true;
    }
    guild_id
        .and_then(|guild_id| ctx.cache.guild(guild_id).map(|guild| guild.owner_id))
        .is_some_and(|owner_id| owner_id == member.user.id)
}

async fn deny_command(ctx: &Context, command: &CommandInteraction) {
    if let Err(error) = command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(permission_denied())
                    .ephemeral(true),
            ),
        )
        .await
    {
        tracing::debug!(%error, "Could not reject unauthorized command");
    }
}

async fn reconnect_voice_channel(ctx: &Context) {
    let config = {
        ctx.data
            .read()
            .await
            .get::<crate::data::DiscordConfigKey>()
            .cloned()
    };
    let Some(config) = config else { return };
    let Some(guild_id) = config
        .string("guildSf")
        .and_then(|id| id.parse::<u64>().ok())
    else {
        return;
    };
    let Some(channel_id) = config
        .string("audioChannelSf")
        .and_then(|id| id.parse::<u64>().ok())
    else {
        return;
    };
    let Some(manager) = songbird::get(ctx).await else {
        return;
    };
    match manager
        .join(GuildId::new(guild_id), ChannelId::new(channel_id))
        .await
    {
        Ok(_) => tracing::info!(
            guild_id,
            channel_id,
            "Reconnected Discord TTS voice channel"
        ),
        Err(error) => {
            tracing::error!(%error, guild_id, channel_id, "Failed to reconnect Discord TTS voice channel")
        }
    }
}
