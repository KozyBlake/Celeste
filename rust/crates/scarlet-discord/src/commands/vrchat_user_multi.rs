//! Java's `/vrchat-user-ban-multi` and `/vrchat-user-unban-multi` modal
//! flows. The Java action queue deliberately serializes these operations;
//! Rust performs them sequentially in the modal handler and returns a
//! per-batch result summary, preserving the rate-safe ordering while making
//! failures visible to the invoker.

use serenity::builder::{
    CreateActionRow, CreateCommand, CreateInputText, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateModal,
};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;
use serenity::model::prelude::InputTextStyle;

pub const BAN_NAME: &str = "vrchat-user-ban-multi";
pub const UNBAN_NAME: &str = "vrchat-user-unban-multi";

pub fn register(ban: bool) -> CreateCommand {
    CreateCommand::new(if ban { BAN_NAME } else { UNBAN_NAME }).description(if ban {
        "Ban several VRChat users"
    } else {
        "Unban several VRChat users"
    })
}

pub async fn run(ctx: &Context, command: &CommandInteraction, ban: bool) -> serenity::Result<()> {
    if let Err(message) = super::moderation_context(
        ctx,
        command,
        scarlet_vrchat::client::PERMISSION_GROUP_BANS_MANAGE,
    )
    .await
    {
        return command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content(message)
                        .ephemeral(true),
                ),
            )
            .await;
    }
    let action = if ban { "Ban" } else { "Unban" };
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Modal(
                CreateModal::new(
                    if ban { BAN_NAME } else { UNBAN_NAME },
                    format!("{action} Multiple VRChat Users"),
                )
                .components(vec![CreateActionRow::InputText(
                    CreateInputText::new(
                        InputTextStyle::Paragraph,
                        "Target VRChat User IDs",
                        "target-ids",
                    )
                    .placeholder("User IDs separated by newline, space, or comma")
                    .min_length(10)
                    .max_length(4000)
                    .required(true),
                )]),
            ),
        )
        .await
}
