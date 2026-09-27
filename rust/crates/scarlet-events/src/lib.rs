//! Rust port of `net.sybyline.scarlet.ScarletEventListener` — the piece that
//! turns parsed VRChat log events into instance state, player rows,
//! advisories, spoken announcements and audit emissions.
//!
//! This crate exists because that class is the *hub* of Scarlet: in Java it
//! reaches into the UI, the VRChat client, the Discord client, the data
//! store, the watched lists and TTS through the `Scarlet` god-object. Giving
//! it a crate of its own, talking to the UI through
//! [`scarlet_core::ui::ScarletUi`] and to Discord through [`AuditEmitter`],
//! keeps that hub from forcing every subsystem into one crate.
//!
//! Before this existed the Rust port wired the log tailer *straight into the
//! UI state*: joins and leaves appeared as rows, and nothing else in Java's
//! pipeline happened at all — no `clientLocation`, so nothing knew an
//! instance was open; no watched-list checks; no group-instance gate; no
//! Discord emission; no TTS.

pub mod emitter;
#[cfg(test)]
mod tests_support;
pub mod listener;

pub use emitter::{AuditEmitter, NoopAuditEmitter, NoopTts, TtsSink};
pub use listener::ScarletEventListener;
