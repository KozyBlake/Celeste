//! Port of `ScarletDiscordJDA.JDAEvents` (the JDA `ListenerAdapter` wired up
//! in the constructor): connection lifecycle, slash-command dispatch, and
//! the gateway events Scarlet reacts to. The per-event bodies live in
//! `crate::events`.

use serenity::async_trait;
use serenity::client::{Context, EventHandler};
use serenity::model::application::Interaction;
use serenity::model::channel::{GuildChannel, Message};
use serenity::model::gateway::Ready;

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
            Interaction::Command(command) => commands::dispatch(&ctx, &command).await,
            Interaction::Autocomplete(command) => {
                commands::dispatch_autocomplete(&ctx, &command).await
            }
            Interaction::Component(component) => {
                crate::interactions::dispatch_component(&ctx, &component).await
            }
            Interaction::Modal(modal) => crate::interactions::dispatch_modal(&ctx, &modal).await,
            _ => {}
        }
    }
}
