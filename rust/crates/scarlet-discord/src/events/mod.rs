//! Port of the gateway-event half of `ScarletDiscordJDA.JDAEvents`.
//!
//! Until now the Rust port handled exactly two gateway events, `ready` and
//! `interaction_create`, so every Discord feature that reacts to something
//! *happening in the server* — rather than to a slash command — could never
//! fire. The Ticket Tool auto-response is the clearest example: its two
//! configuration commands were ported and worked, they wrote the right
//! fields to `discord_bot.json`, and nothing ever read them, because the
//! thing that reads them is `onChannelCreate`.

pub mod audit_emitter;
pub mod audit_poller;
pub mod instance_lifecycle;
pub mod ticket;
