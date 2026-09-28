//! Port of Java's `/set-voice-channel` configuration command.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::model::channel::{ChannelType, PartialChannel};

use crate::data::DiscordConfigKey;

pub const NAME: &str = "set-voice-channel";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Sets the voice channel in which TTS messages are announced")
        .add_option(CreateCommandOption::new(
            CommandOptionType::Channel,
            "discord-voice-channel",
            "The voice channel to use, or omit to disconnect",
        ))
}

fn selected_channel<'a>(command: &'a CommandInteraction) -> Option<&'a PartialChannel> {
    command
        .data
        .options()
        .into_iter()
        .find(|option| option.name == "discord-voice-channel")
        .and_then(|option| match option.value {
            ResolvedValue::Channel(channel) => Some(channel),
            _ => None,
        })
}

fn is_voice_channel(kind: ChannelType) -> bool {
    kind == ChannelType::Voice
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let selected = selected_channel(command);
    if let Some(channel) = selected {
        if !is_voice_channel(channel.kind) {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!(
                    "The channel {} isn't a voice channel",
                    channel.name.as_deref().unwrap_or("selected")
                ),
            )
            .await;
        }
    }

    let config = { ctx.data.read().await.get::<DiscordConfigKey>().cloned() };
    let Some(config) = config else {
        return super::reply_ephemeral(ctx, command, "Discord configuration is unavailable.").await;
    };
    let channel_id = selected.map(|channel| channel.id.to_string());
    if let Err(error) = config.set_string("audioChannelSf", channel_id.as_deref()) {
        tracing::error!(%error, "Failed to save the Discord voice channel");
        return super::reply_ephemeral(ctx, command, "Failed to save the voice channel.").await;
    }

    let Some(guild_id) = command.guild_id else {
        return super::reply_ephemeral(ctx, command, "This command must be used in a server.").await;
    };
    let Some(manager) = songbird::get(ctx).await else {
        return super::reply_ephemeral(ctx, command, "Discord voice support is unavailable.").await;
    };
    match (selected, channel_id) {
        (Some(channel), Some(channel_id)) => {
            if let Err(error) = manager.join(guild_id, channel.id).await {
                tracing::error!(%error, %channel_id, "Failed to connect to Discord voice");
                return super::reply_ephemeral(
                    ctx,
                    command,
                    &format!("Saved <#{channel_id}>, but could not connect to it: {error}"),
                ).await;
            }
            tracing::info!(%channel_id, "Configured Discord TTS voice channel");
            super::reply_ephemeral(
                ctx,
                command,
                &format!("Enabling TTS announcements in voice channel <#{channel_id}>"),
            )
            .await
        }
        _ => {
            if let Err(error) = manager.remove(guild_id).await {
                tracing::warn!(%error, "Failed to disconnect Discord voice");
            }
            tracing::info!("Disabled Discord TTS voice channel");
            super::reply_ephemeral(ctx, command, "Disabling/disconnecting from voice channel").await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_normal_voice_channels_are_accepted() {
        assert!(is_voice_channel(ChannelType::Voice));
        assert!(!is_voice_channel(ChannelType::Stage));
        assert!(!is_voice_channel(ChannelType::Text));
    }
}
