//! Port of Java's `/set-tts-voice`.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

use crate::data::SettingsKey;

pub const NAME: &str = "set-tts-voice";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Selects which TTS voice is used to make announcements")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "voice-name",
                "The installed voice to use",
            )
            .required(true),
        )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let voice = command
        .data
        .options()
        .iter()
        .find(|option| option.name == "voice-name")
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) => Some(*value),
            _ => None,
        });
    let Some(voice) = voice else {
        return super::reply_ephemeral(ctx, command, "A `voice-name` is required.").await;
    };
    if !scarlet_core::settings::installed_tts_voices()
        .iter()
        .any(|installed| installed == voice)
    {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("TTS voice `{voice}` is not installed on this system"),
        )
        .await;
    }
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are unavailable.").await;
    };
    if !settings
        .tts_voice_name
        .set(Some(voice.to_string()), "discord")
    {
        return super::reply_ephemeral(
            ctx,
            command,
            &format!("TTS voice `{voice}` could not be selected"),
        )
        .await;
    }
    super::reply_ephemeral(ctx, command, &format!("Setting TTS voice to `{voice}`")).await
}
