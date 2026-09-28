//! Java's message-context attachment chooser, with the in-scope watched-group import action.

use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandType};

pub const NAME: &str = "submit-attachments";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).kind(CommandType::Message)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let Some(message) = command.data.resolved.messages.values().next() else {
        return command.create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Could not resolve the selected message.")
                    .ephemeral(true),
            ),
        ).await;
    };
    command.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("Select submission type")
                .components(vec![CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("import-watched-groups:{}", message.id.get()))
                        .label("Import watched groups"),
                ])])
                .ephemeral(true),
        ),
    ).await
}
