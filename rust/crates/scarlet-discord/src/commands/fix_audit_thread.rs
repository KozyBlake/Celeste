//! Message-context repair command for moderation audit threads.

use std::sync::LazyLock;

use regex::Regex;
use serenity::builder::{
    CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
    CreateThread,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandType};
use serenity::model::channel::AutoArchiveDuration;
use serenity::model::id::ChannelId;

pub const NAME: &str = "fix-audit-thread";

static AUDIT_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"gaud_[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .expect("static audit id regex")
});

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).kind(CommandType::Message)
}

fn audit_id_from_message(message: &serenity::model::channel::Message) -> Option<&str> {
    message.embeds.iter()
        .filter_map(|embed| embed.footer.as_ref())
        .find_map(|footer| AUDIT_ID.find(&footer.text).map(|matched| matched.as_str()))
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    command.create_response(
        &ctx.http,
        CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true)),
    ).await?;

    let Some(message) = command.data.resolved.messages.values().next() else {
        return super::reply_ephemeral(ctx, command, "Could not resolve the selected message.").await;
    };
    let Some(audit_id) = audit_id_from_message(message) else {
        return super::reply_ephemeral(ctx, command, "Message does not appear to be an audit entry").await;
    };
    let (data, tags) = {
        let shared = ctx.data.read().await;
        (
            shared.get::<crate::data::ScarletDataKey>().cloned(),
            shared.get::<crate::data::ModerationTagsKey>().cloned(),
        )
    };
    let Some(data) = data else {
        return super::reply_ephemeral(ctx, command, "Celeste data is unavailable.").await;
    };
    let Some(tags) = tags else {
        return super::reply_ephemeral(ctx, command, "Moderation tags are unavailable.").await;
    };
    if !data.audit_entry_metadata_exists(audit_id) {
        return super::reply_ephemeral(ctx, command, &format!("Failed to find data for entry `{audit_id}`")).await;
    }
    let mut metadata = data.audit_entry_metadata(audit_id);
    let Some(entry_value) = metadata.entry.as_ref() else {
        return super::reply_ephemeral(ctx, command, &format!("Entry `{audit_id}` has no stored audit data")).await;
    };
    let entry: scarlet_vrchat::models::GroupAuditLogEntry = match serde_json::from_value(entry_value.clone()) {
        Ok(entry) => entry,
        Err(error) => return super::reply_ephemeral(
            ctx,
            command,
            &format!("Stored data for entry `{audit_id}` is invalid: {error}"),
        ).await,
    };

    let thread_id = if let Some(thread) = message.thread.as_ref() {
        thread.id
    } else if let Some(id) = metadata.thread_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()) {
        ChannelId::new(id)
    } else {
        match message.channel_id.create_thread_from_message(
            &ctx.http,
            message.id,
            CreateThread::new(crate::events::audit_poller::thread_name(&entry))
                .auto_archive_duration(AutoArchiveDuration::OneHour),
        ).await {
            Ok(thread) => thread.id,
            Err(error) => return super::reply_ephemeral(
                ctx,
                command,
                &format!("Failed to create the audit thread for `{audit_id}`: {error}"),
            ).await,
        }
    };

    metadata.guild_snowflake = command.guild_id.map(|id| id.get().to_string())
        .or(metadata.guild_snowflake);
    metadata.channel_snowflake = Some(message.channel_id.get().to_string());
    metadata.message_snowflake = Some(message.id.get().to_string());
    metadata.thread_snowflake = Some(thread_id.get().to_string());

    let existing_aux = match metadata.aux_message_snowflake.as_deref().and_then(|id| id.parse::<u64>().ok()) {
        Some(id) => thread_id.message(&ctx.http, id).await.ok(),
        None => None,
    };
    if existing_aux.is_none() {
        let aux = thread_id.send_message(
            &ctx.http,
            CreateMessage::new()
                .content(crate::events::audit_poller::moderation_aux_content(&metadata, &tags))
                .components(crate::events::audit_poller::moderation_controls(&entry)),
        ).await?;
        metadata.aux_message_snowflake = Some(aux.id.get().to_string());
    }
    data.set_audit_entry_metadata(audit_id, metadata);

    super::reply_ephemeral(
        ctx,
        command,
        &format!("Fixed audit thread for entry `{audit_id}`: <#{}>", thread_id.get()),
    ).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_id_pattern_only_accepts_group_audit_ids() {
        let text = "Celeste 0.4.20 ✧ gaud_12345678-1234-abcd-9876-1234567890ab";
        assert_eq!(AUDIT_ID.find(text).unwrap().as_str(), "gaud_12345678-1234-abcd-9876-1234567890ab");
        assert!(AUDIT_ID.find("usr_12345678-1234-abcd-9876-1234567890ab").is_none());
    }
}
