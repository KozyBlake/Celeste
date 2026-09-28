//! Port of `/server-restart restart-now`.

use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

use crate::data::ProcessControllerKey;

pub const NAME: &str = "server-restart";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Restarts the Scarlet application")
        .add_option(CreateCommandOption::new(
            CommandOptionType::SubCommand,
            "restart-now",
            "Restarts the application now",
        ))
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    let restart_now = command.data.options().first().is_some_and(|option| {
        option.name == "restart-now" && matches!(option.value, ResolvedValue::SubCommand(_))
    });
    let feedback = if !restart_now {
        "A valid restart subcommand is required."
    } else {
        let controller = { ctx.data.read().await.get::<ProcessControllerKey>().cloned() };
        match controller {
            Some(controller) if controller.restart() => "Restarting...",
            Some(_) => "Shutdown already queued",
            None => "Application restart control is unavailable",
        }
    };
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content(feedback),
            ),
        )
        .await
}
