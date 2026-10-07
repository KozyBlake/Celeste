//! Celeste's `/celeste-version` slash command, corresponding to the project
//! identity functionality in `ScarletDiscordCommands.java`.

use serenity::builder::{CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

pub const NAME: &str = "celeste-version";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Show the running Celeste version")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let content = format!("{} {}", celeste_core::identity::APP_NAME, celeste_core::identity::VERSION);
    let response = CreateInteractionResponseMessage::new().content(content);
    command.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await
}
