//! A real, minimal slash command (`/scarlet-version`) proving the
//! registration -> dispatch -> reply pipeline end to end, standing in for
//! the dozens of commands in `ScarletDiscordCommands.java` (5,765 lines)
//! that are ported incrementally rather than all at once — see PORTING.md.

use serenity::builder::{CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::client::Context;
use serenity::model::application::CommandInteraction;

pub const NAME: &str = "celeste-version";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME).description("Show the running Celeste version")
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let content = format!("{} {}", scarlet_core::identity::APP_NAME, scarlet_core::identity::VERSION);
    let response = CreateInteractionResponseMessage::new().content(content);
    command.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await
}
