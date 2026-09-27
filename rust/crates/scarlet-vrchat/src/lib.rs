//! Rust port of Scarlet's VRChat API client (`ScarletVRChat*`).
//!
//! **Status: real, working skeleton covering auth + one endpoint family,
//! not a full port.** `ScarletVRChat.java` is 3,367 lines covering the
//! whole VRChat API surface via the generated `vrchatapi-java` client (see
//! `client`'s module docs for why that isn't replicated here). What's
//! ported: VRChat's non-standard Basic Auth encoding ([`basic_auth`]),
//! login + TOTP two-factor verification, and group audit log lookups
//! ([`client::VrChatClient`]). See PORTING.md for what's still outstanding
//! (persistent encrypted cookie storage, alt-account support, the rest of
//! the API surface).

pub mod basic_auth;
pub mod client;
pub mod models;
