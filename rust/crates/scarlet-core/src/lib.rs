//! Rust port of the `net.sybyline.scarlet` core package: configuration,
//! settings, persisted data and shared primitives used by every other
//! subsystem crate.

pub mod change_listener;
pub mod cache_cleanup;
pub mod data;
pub mod features;
pub mod file_backups;
pub mod i18n;
pub mod identity;
pub mod instance_roster;
pub mod java_hash;
pub mod json_cache;
pub mod log;
pub mod maths;
pub mod meta;
pub mod misc_utils;
pub mod moderation_tags;
pub mod names;
pub mod pacer;
pub mod paths;
pub mod permission;
pub mod pronoun_lists;
pub mod pronoun_validator;
pub mod secure_store;
pub mod settings;
pub mod staff_list;
pub mod trust_rank;
pub mod tts_voices;
pub mod ui;
pub mod unique_strings;
pub mod versioned_file;
pub mod watched_entities;
pub mod watched_groups;
