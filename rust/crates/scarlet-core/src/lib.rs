//! Rust port of the `net.sybyline.scarlet` core package: configuration,
//! settings, persisted data and shared primitives used by every other
//! subsystem crate.

pub mod change_listener;
pub mod features;
pub mod file_backups;
pub mod identity;
pub mod json_cache;
pub mod log;
pub mod maths;
pub mod meta;
pub mod permission;
pub mod secure_store;
