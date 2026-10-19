//! Port of `net.sybyline.scarlet.ScarletData`: the JSON-file data store for
//! Discord<->VRChat account links, per-user metadata, audit-log entry
//! metadata, live-instance tracking, and custom (synthetic) audit events.
//!
//! ## Scope decision: no type-erased `Datum<?>`/`Data<?>` registry
//!
//! Java's `getDatum(name, type, createDefault)`/`getData(kind, type)` are a
//! generic, type-erased registry (`Map<String, Datum<?>>` /
//! `Map<String, Data<?>>`, with a runtime `Class` check on re-lookup). But
//! grepping the whole codebase shows they're **only ever called from inside
//! `CelesteData` itself**, for exactly five fixed (name-or-kind, type)
//! pairs: `global.json`/`GlobalMetadata`, `live.json`/`LiveInstancesMetadata`,
//! and the `usr`/`gaud`/`ex` kinds. Nothing external reaches into the
//! generic registry. So this port skips the type erasure entirely and gives
//! `CelesteData` five concrete, statically-typed fields instead — the
//! *shape* of each abstraction ([`ObjDatum<T>`]'s dirty-tracked singleton
//! file, [`SubData<T>`]'s LRU-cached per-id files with save-on-evict) is
//! still ported faithfully, just not hidden behind `dyn Any`-style
//! indirection nobody needed.
//!
//! ## Placeholder: `AuditEntryMetadata.entry`
//!
//! Java's field is `io.github.vrchatapi.model.GroupAuditLogEntry`, a
//! generated VRChat API model type. That hasn't been ported yet (it belongs
//! with the rest of the VRChat API client — see PORTING.md task 6), so this
//! is `serde_json::Value` for now: JSON round-trips correctly either way
//! (Gson serializes the Java model as a plain JSON object too), and every
//! accessor here that reads `entry`'s "data" sub-object works the same
//! against a `Value`. Swap this for the real ported type once it exists,
//! rather than inventing a partial model now.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::unique_strings::UniqueStrings;

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path, or_new: bool) -> Option<T> {
    if !path.is_file() {
        tracing::warn!("Reading data: no such file: {}", path.display());
    } else {
        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(value) => return Some(value),
                Err(err) => tracing::error!("Exception reading data {}: {}", path.display(), err),
            },
            Err(err) => tracing::error!("Exception reading data {}: {}", path.display(), err),
        }
    }
    if or_new {
        Some(T::default())
    } else {
        None
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T, keep: usize) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(value) {
        Ok(json) => {
            if let Err(err) = crate::file_backups::write_keep(path, &json, keep) {
                tracing::error!("Exception writing data {}: {}", path.display(), err);
            }
        }
        Err(err) => tracing::error!("Exception writing data {}: {}", path.display(), err),
    }
}

/// Port of `ScarletData.ObjDatum<T>`: a single dirty-tracked JSON file
/// (`global.json`, `live.json`).
pub struct ObjDatum<T> {
    path: PathBuf,
    value: Mutex<Option<T>>,
    dirty: Mutex<bool>,
}

impl<T: Clone + Default + Serialize + serde::de::DeserializeOwned> ObjDatum<T> {
    fn new(dir: &Path, name: &str) -> Self {
        ObjDatum { path: dir.join(name), value: Mutex::new(None), dirty: Mutex::new(false) }
    }

    pub fn exists(&self) -> bool {
        self.value.lock().unwrap_or_else(|e| e.into_inner()).is_some() || self.path.is_file()
    }

    pub fn mark_dirty(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }

    /// Test-and-clear, matching `pollDirty`.
    pub fn poll_dirty(&self) -> bool {
        std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn save(&self) {
        *self.dirty.lock().unwrap_or_else(|e| e.into_inner()) = false;
        if let Some(value) = self.value.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            write_json(&self.path, value, crate::file_backups::DEFAULT_KEEP);
        }
    }

    pub fn save_if_dirty(&self) {
        if self.poll_dirty() {
            self.save();
        }
    }

    pub fn get(&self) -> T {
        let mut guard = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = read_json(&self.path, true);
        }
        guard.clone().unwrap()
    }

    /// Reads, applies `edit`, marks dirty, and returns the updated value.
    pub fn update(&self, edit: impl FnOnce(&mut T)) -> T {
        let mut guard = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = read_json(&self.path, true);
        }
        let value = guard.as_mut().unwrap();
        edit(value);
        let result = value.clone();
        drop(guard);
        self.mark_dirty();
        result
    }
}

/// Port of `ScarletData.SubData<T>`: an LRU-cached collection of per-id JSON
/// files under `<dir>/<kind>/<id>`, saving evicted entries automatically
/// (matching Java's `LRUMap.ofSynchronized(this::save)`).
pub struct SubData<T> {
    subdir: PathBuf,
    values: Mutex<lru::LruCache<String, T>>,
    dirties: Mutex<HashSet<String>>,
}

const SUB_DATA_CACHE_SIZE: usize = 1024;

impl<T: Clone + Default + Serialize + serde::de::DeserializeOwned> SubData<T> {
    fn new(dir: &Path, kind: &str) -> Self {
        SubData {
            subdir: dir.join(kind),
            values: Mutex::new(lru::LruCache::new(std::num::NonZeroUsize::new(SUB_DATA_CACHE_SIZE).unwrap())),
            dirties: Mutex::new(HashSet::new()),
        }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.subdir.join(id)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.values.lock().unwrap_or_else(|e| e.into_inner()).contains(id) || self.path(id).is_file()
    }

    fn save_one(&self, id: &str, value: &T) {
        // High-cardinality per-entity files: keep=0, no dated backups per
        // user/audit-entry/event -- only the crash-safe atomic write.
        write_json(&self.path(id), value, 0);
    }

    /// Loads (from cache or disk, defaulting if absent) without marking
    /// dirty.
    pub fn get(&self, id: &str) -> T {
        let mut cache = self.values.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = cache.get(id) {
            return v.clone();
        }
        let value: T = read_json(&self.path(id), true).unwrap();
        cache.put(id.to_string(), value.clone());
        value
    }

    pub fn mark_dirty(&self, id: &str) {
        self.dirties.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
    }

    pub fn poll_dirty(&self, id: &str) -> bool {
        self.dirties.lock().unwrap_or_else(|e| e.into_inner()).remove(id)
    }

    /// Reads, applies `edit`, persists immediately, and returns the updated
    /// value (matches the common `xMetadata(id, edit)` call shape used
    /// throughout Celeste).
    pub fn update(&self, id: &str, edit: impl FnOnce(&mut T)) -> T {
        let mut value = self.get(id);
        edit(&mut value);
        self.put(id, value.clone());
        value
    }

    pub fn put(&self, id: &str, value: T) {
        self.save_one(id, &value);
        self.values.lock().unwrap_or_else(|e| e.into_inner()).put(id.to_string(), value);
        self.dirties.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    pub fn save(&self) {
        let cache = self.values.lock().unwrap_or_else(|e| e.into_inner());
        for (id, value) in cache.iter() {
            self.save_one(id, value);
        }
    }

    pub fn save_if_dirty(&self) {
        let dirty_ids: Vec<String> = self.dirties.lock().unwrap_or_else(|e| e.into_inner()).drain().collect();
        let cache = self.values.lock().unwrap_or_else(|e| e.into_inner());
        for id in dirty_ids {
            if let Some(value) = cache.peek(&id) {
                self.save_one(&id, value);
            }
        }
    }
}

pub struct CelesteData {
    dir: PathBuf,
    pub live_instances: ObjDatum<LiveInstancesMetadata>,
    pub global: ObjDatum<GlobalMetadata>,
    pub users: SubData<UserMetadata>,
    pub audit_entries: SubData<AuditEntryMetadata>,
    pub custom_events: SubData<CustomEvent>,
}

impl CelesteData {
    pub fn open(dir: &Path) -> Self {
        CelesteData {
            dir: dir.to_path_buf(),
            live_instances: ObjDatum::new(dir, "live.json"),
            global: ObjDatum::new(dir, "global.json"),
            users: SubData::new(dir, "usr"),
            audit_entries: SubData::new(dir, "gaud"),
            custom_events: SubData::new(dir, "ex"),
        }
    }

    pub fn save_dirty(&self) {
        self.live_instances.save_if_dirty();
        self.global.save_if_dirty();
        self.users.save_if_dirty();
        self.audit_entries.save_if_dirty();
        self.custom_events.save_if_dirty();
    }

    pub fn save_all(&self) {
        self.live_instances.save();
        self.global.save();
        self.users.save();
        self.audit_entries.save();
        self.custom_events.save();
    }

    // ---- global metadata (Discord<->VRChat snowflake links) ----

    pub fn global_get_snowflake_id(&self, user_snowflake: &str) -> Option<String> {
        self.global.get().user_snowflake_to_user_id.get(user_snowflake).cloned()
    }
    pub fn global_set_snowflake_id(&self, user_snowflake: &str, user_id: &str) {
        self.global.update(|g| {
            g.user_snowflake_to_user_id.insert(user_snowflake.to_string(), user_id.to_string());
        });
    }
    pub fn global_remove_snowflake_id(&self, user_snowflake: &str) {
        self.global.update(|g| {
            g.user_snowflake_to_user_id.remove(user_snowflake);
        });
    }

    // ---- live instances ----

    pub fn live_instances_get_location_audit(&self, location: &str, remove: bool) -> Option<String> {
        if remove {
            let mut result = None;
            self.live_instances.update(|li| {
                result = li.location_to_audit_entry_id.remove(location);
            });
            result
        } else {
            self.live_instances.get().location_to_audit_entry_id.get(location).cloned()
        }
    }
    pub fn live_instances_set_location_audit(&self, location: &str, audit_entry_id: &str) {
        self.live_instances.update(|li| {
            li.location_to_audit_entry_id.insert(location.to_string(), audit_entry_id.to_string());
        });
    }
    pub fn live_instances_get_locations(&self) -> Vec<String> {
        self.live_instances.get().location_to_audit_entry_id.into_keys().collect()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn live_instances_set_location_instance_embed_message(
        &self,
        location: &str,
        guild_snowflake: Option<String>,
        channel_snowflake: Option<String>,
        message_snowflake: Option<String>,
        thread_snowflake: Option<String>,
        aux_message_snowflake: Option<String>,
        opened_at: DateTime<Utc>,
    ) {
        self.live_instances.update(|li| {
            li.location_to_instance_embed_message.insert(
                location.to_string(),
                InstanceEmbedMessage {
                    guild_snowflake,
                    channel_snowflake,
                    message_snowflake,
                    thread_snowflake,
                    aux_message_snowflake,
                    opened_at: Some(opened_at),
                    closed_at: None,
                    playerlist_message_snowflake: None,
                },
            );
        });
    }
    pub fn live_instances_get_location_instance_embed_message(
        &self,
        location: &str,
        remove: bool,
    ) -> Option<InstanceEmbedMessage> {
        if remove {
            let mut result = None;
            self.live_instances.update(|li| {
                result = li.location_to_instance_embed_message.remove(location);
            });
            result
        } else {
            self.live_instances.get().location_to_instance_embed_message.get(location).cloned()
        }
    }
    pub fn live_instances_set_location_closed_at(&self, location: &str, closed_at: DateTime<Utc>) {
        self.live_instances.update(|li| {
            if let Some(message) = li.location_to_instance_embed_message.get_mut(location) {
                message.closed_at = Some(closed_at);
            }
        });
    }
    pub fn live_instances_set_playerlist_message(&self, location: &str, message_snowflake: String) {
        self.live_instances.update(|li| {
            if let Some(message) = li.location_to_instance_embed_message.get_mut(location) {
                message.playerlist_message_snowflake = Some(message_snowflake);
            }
        });
    }

    // ---- user metadata ----

    pub fn user_metadata(&self, user_id: &str) -> UserMetadata {
        self.users.get(user_id)
    }
    pub fn user_metadata_exists(&self, user_id: &str) -> bool {
        self.users.exists(user_id)
    }
    pub fn set_user_metadata(&self, user_id: &str, value: UserMetadata) {
        self.users.put(user_id, value);
    }
    pub fn user_metadata_set_snowflake(&self, user_id: &str, user_snowflake: Option<String>) -> UserMetadata {
        self.users.update(user_id, |m| m.user_snowflake = user_snowflake)
    }
    pub fn user_metadata_get_snowflake(&self, user_id: &str) -> Option<String> {
        self.users.get(user_id).user_snowflake
    }

    // ---- audit entry metadata ----

    pub fn audit_entry_metadata(&self, audit_entry_id: &str) -> AuditEntryMetadata {
        self.audit_entries.get(audit_entry_id)
    }
    pub fn audit_entry_metadata_exists(&self, audit_entry_id: &str) -> bool {
        self.audit_entries.exists(audit_entry_id)
    }
    pub fn set_audit_entry_metadata(&self, audit_entry_id: &str, value: AuditEntryMetadata) {
        self.audit_entries.put(audit_entry_id, value);
    }
    pub fn audit_entry_metadata_set_tags(&self, audit_entry_id: &str, tags: Vec<String>) -> AuditEntryMetadata {
        self.audit_entries.update(audit_entry_id, |m| {
            m.entry_tags.clear();
            m.entry_tags.add_all(tags);
        })
    }
    pub fn audit_entry_metadata_edit_tags(
        &self,
        audit_entry_id: &str,
        remove: &[String],
        add: Vec<String>,
    ) -> AuditEntryMetadata {
        self.audit_entries.update(audit_entry_id, |m| {
            m.entry_tags.remove_all(remove.iter().map(String::as_str));
            m.entry_tags.add_all(add);
        })
    }
    pub fn audit_entry_metadata_set_description(&self, audit_entry_id: &str, description: Option<String>) -> AuditEntryMetadata {
        let description = description.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
        self.audit_entries.update(audit_entry_id, |m| m.entry_description = description)
    }

    // ---- custom events ----

    pub fn custom_event(&self, custom_event_id: &str) -> CustomEvent {
        self.custom_events.get(custom_event_id)
    }
    pub fn set_custom_event(&self, custom_event_id: &str, value: CustomEvent) {
        self.custom_events.put(custom_event_id, value);
    }

    /// Port of `customEvent_new`. `type_ex_id` is the `GroupAuditTypeEx`'s
    /// string id (see the module docs on why `CelesteData` takes the id
    /// rather than that enum type directly).
    #[allow(clippy::too_many_arguments)]
    pub fn custom_event_new(
        &self,
        type_ex_id: &str,
        timestamp: DateTime<Utc>,
        actor_id: Option<String>,
        actor_display_name: Option<String>,
        target_id: Option<String>,
        target_display_name: Option<String>,
        data: Option<Value>,
    ) -> String {
        let type_hash = crate::java_hash::java_string_hash_code(type_ex_id);
        let id = loop {
            let random: i32 = rand::random();
            let msb = timestamp.timestamp_millis() as u64;
            let lsb = ((random as u32 as u64) << 32) | (type_hash as u32 as u64);
            let uuid = uuid::Uuid::from_u64_pair(msb, lsb);
            let candidate = uuid.to_string();
            if !self.dir.join("ex").join(&candidate).is_file() {
                break candidate;
            }
        };
        let event = CustomEvent {
            id: id.clone(),
            type_ex: type_ex_id.to_string(),
            actor_id,
            actor_display_name,
            target_id,
            target_display_name,
            timestamp: Some(timestamp),
            data,
        };
        self.set_custom_event(&id, event);
        id
    }

    /// Port of `customEvent_filter`.
    pub fn custom_event_filter(&self, type_ex_id: Option<&str>, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> Vec<String> {
        let beginning = from.map(|d| d.timestamp_millis()).unwrap_or(i64::MIN);
        let end = to.map(|d| d.timestamp_millis()).unwrap_or(i64::MAX);
        let type_hash = type_ex_id.map(crate::java_hash::java_string_hash_code).unwrap_or(0);
        let ex_dir = self.dir.join("ex");
        let Ok(entries) = std::fs::read_dir(&ex_dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                let uuid = uuid::Uuid::parse_str(&name).ok()?;
                let (msb, lsb) = uuid.as_u64_pair();
                let epoch_millis = msb as i64;
                let hc = (lsb & 0xFFFF_FFFF) as u32 as i32;
                let matches = (type_hash == 0 || type_hash == hc) && beginning <= epoch_millis && epoch_millis <= end;
                matches.then_some(name)
            })
            .collect()
    }

    pub fn link_id_to_snowflake(&self, user_id: &str, user_snowflake: &str) {
        self.user_metadata_set_snowflake(user_id, Some(user_snowflake.to_string()));
        self.global_set_snowflake_id(user_snowflake, user_id);
    }

    /// Removes the Discord<->VRChat link for the given Discord snowflake,
    /// clearing both sides. Returns the VRChat user id that was linked, if
    /// any.
    pub fn unlink_by_snowflake(&self, user_snowflake: &str) -> Option<String> {
        let user_id = self.global_get_snowflake_id(user_snowflake);
        self.global_remove_snowflake_id(user_snowflake);
        if let Some(user_id) = &user_id {
            self.user_metadata_set_snowflake(user_id, None);
        }
        user_id
    }

    /// Removes the Discord<->VRChat link for the given VRChat user id,
    /// clearing both sides. Returns the Discord snowflake that was linked,
    /// if any.
    pub fn unlink_by_id(&self, user_id: &str) -> Option<String> {
        let user_snowflake = self.user_metadata_get_snowflake(user_id);
        self.user_metadata_set_snowflake(user_id, None);
        if let Some(user_snowflake) = &user_snowflake {
            self.global_remove_snowflake_id(user_snowflake);
        }
        user_snowflake
    }
}

/// Java's Gson is configured without a `FieldNamingPolicy`
/// (`util/JsonAdapters.java`), so it writes each field under its Java name —
/// which is camelCase. These structs were serialising Rust's snake_case
/// field names instead, which meant a Java install's `global.json`,
/// `live.json`, `usr/<id>` and `gaud/<id>` were found, deserialised as
/// **all-default**, and then re-saved empty: every account link, tag,
/// description, redaction flag and evidence record silently lost on first
/// read. `rename_all = "camelCase"` fixes the wire format; the `alias`
/// attributes keep reading the snake_case files written by Rust builds
/// between the port starting and this fix, so nothing written during
/// testing is stranded either. The pinning test at the bottom of this
/// module is what stops this regressing again.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalMetadata {
    /// Java: `public Map<String, String> userSnowflake2userId` — the
    /// Discord↔VRChat account link table every "linked ids" check reads.
    #[serde(rename = "userSnowflake2userId", alias = "user_snowflake_to_user_id", default)]
    pub user_snowflake_to_user_id: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveInstancesMetadata {
    /// Java: `location2AuditEntryId`.
    #[serde(rename = "location2AuditEntryId", alias = "location_to_audit_entry_id", default)]
    pub location_to_audit_entry_id: std::collections::HashMap<String, String>,
    /// Java: `location2instanceEmbedMessage` — note the lower-case `i`,
    /// which is why this can't be derived from the Rust name.
    #[serde(
        rename = "location2instanceEmbedMessage",
        alias = "location_to_instance_embed_message",
        default
    )]
    pub location_to_instance_embed_message: std::collections::HashMap<String, InstanceEmbedMessage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceEmbedMessage {
    #[serde(alias = "guild_snowflake")]
    pub guild_snowflake: Option<String>,
    #[serde(alias = "channel_snowflake")]
    pub channel_snowflake: Option<String>,
    #[serde(alias = "message_snowflake")]
    pub message_snowflake: Option<String>,
    #[serde(alias = "thread_snowflake")]
    pub thread_snowflake: Option<String>,
    #[serde(alias = "aux_message_snowflake")]
    pub aux_message_snowflake: Option<String>,
    #[serde(alias = "opened_at")]
    pub opened_at: Option<DateTime<Utc>>,
    #[serde(alias = "closed_at")]
    pub closed_at: Option<DateTime<Utc>>,
    /// Snowflake of the live player list message in the instance thread.
    #[serde(alias = "playerlist_message_snowflake")]
    pub playerlist_message_snowflake: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMetadata {
    #[serde(alias = "user_snowflake")]
    pub user_snowflake: Option<String>,
    #[serde(default, alias = "audit_entry_ids")]
    pub audit_entry_ids: Vec<String>,
    #[serde(default, alias = "evidence_submissions")]
    pub evidence_submissions: Vec<EvidenceSubmission>,
}

impl UserMetadata {
    pub fn add_audit_entry_id(&mut self, audit_entry_id: String) {
        self.audit_entry_ids.push(audit_entry_id);
    }
    pub fn user_case_evidence_count(&self) -> usize {
        self.evidence_submissions.len()
    }
    pub fn add_user_case_evidence(&mut self, submission: EvidenceSubmission) {
        self.evidence_submissions.push(submission);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSubmission {
    #[serde(alias = "audit_entry_id")]
    pub audit_entry_id: Option<String>,
    #[serde(alias = "submitter_snowflake")]
    pub submitter_snowflake: Option<String>,
    #[serde(alias = "submitter_display_name")]
    pub submitter_display_name: Option<String>,
    #[serde(alias = "submission_time")]
    pub submission_time: Option<DateTime<Utc>>,
    #[serde(alias = "file_name")]
    pub file_name: Option<String>,
    #[serde(alias = "file_path")]
    pub file_path: Option<String>,
    pub url: Option<String>,
    #[serde(alias = "proxy_url")]
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntryMetadata {
    #[serde(alias = "guild_snowflake")]
    pub guild_snowflake: Option<String>,
    #[serde(alias = "channel_snowflake")]
    pub channel_snowflake: Option<String>,
    #[serde(alias = "message_snowflake")]
    pub message_snowflake: Option<String>,
    #[serde(alias = "thread_snowflake")]
    pub thread_snowflake: Option<String>,
    #[serde(alias = "aux_message_snowflake")]
    pub aux_message_snowflake: Option<String>,

    /// Placeholder for `io.github.vrchatapi.model.GroupAuditLogEntry` --
    /// see the module docs.
    pub entry: Option<Value>,

    #[serde(default, alias = "entry_tags")]
    pub entry_tags: UniqueStrings,
    #[serde(alias = "entry_description")]
    pub entry_description: Option<String>,
    #[serde(default, alias = "entry_redacted")]
    pub entry_redacted: bool,

    #[serde(alias = "aux_actor_id")]
    pub aux_actor_id: Option<String>,
    #[serde(alias = "aux_actor_display_name")]
    pub aux_actor_display_name: Option<String>,

    #[serde(alias = "parent_event_id")]
    pub parent_event_id: Option<String>,

    #[serde(alias = "aux_data")]
    pub aux_data: Option<Map<String, Value>>,

    #[serde(alias = "snapshot_target_user")]
    pub snapshot_target_user: Option<Value>,
    #[serde(alias = "snapshot_target_user_groups")]
    pub snapshot_target_user_groups: Option<Value>,
    #[serde(alias = "snapshot_target_user_represented_group")]
    pub snapshot_target_user_represented_group: Option<Value>,
}

impl AuditEntryMetadata {
    pub fn has_tags(&self) -> bool {
        !self.entry_tags.is_empty()
    }
    pub fn has_description(&self) -> bool {
        self.entry_description.as_ref().is_some_and(|d| !d.trim().is_empty())
    }

    pub fn has_message(&self) -> bool {
        self.guild_snowflake.is_some() && self.channel_snowflake.is_some() && self.message_snowflake.is_some()
    }
    pub fn message_delineated(&self) -> Option<String> {
        self.has_message().then(|| {
            format!(
                "{}/{}/{}",
                self.guild_snowflake.as_ref().unwrap(),
                self.channel_snowflake.as_ref().unwrap(),
                self.message_snowflake.as_ref().unwrap()
            )
        })
    }
    pub fn message_url(&self) -> Option<String> {
        self.message_delineated().map(|d| format!("https://discord.com/channels/{d}"))
    }

    pub fn has_channel(&self) -> bool {
        self.guild_snowflake.is_some() && self.channel_snowflake.is_some()
    }
    pub fn channel_delineated(&self) -> Option<String> {
        self.has_channel()
            .then(|| format!("{}/{}", self.guild_snowflake.as_ref().unwrap(), self.channel_snowflake.as_ref().unwrap()))
    }
    pub fn channel_url(&self) -> Option<String> {
        self.channel_delineated().map(|d| format!("https://discord.com/channels/{d}"))
    }

    pub fn has_aux_message(&self) -> bool {
        self.guild_snowflake.is_some() && self.channel_snowflake.is_some() && self.aux_message_snowflake.is_some()
    }
    pub fn aux_message_delineated(&self) -> Option<String> {
        self.has_aux_message().then(|| {
            format!(
                "{}/{}/{}",
                self.guild_snowflake.as_ref().unwrap(),
                self.channel_snowflake.as_ref().unwrap(),
                self.aux_message_snowflake.as_ref().unwrap()
            )
        })
    }
    pub fn aux_message_url(&self) -> Option<String> {
        self.aux_message_delineated().map(|d| format!("https://discord.com/channels/{d}"))
    }

    pub fn has_thread(&self) -> bool {
        self.guild_snowflake.is_some() && self.thread_snowflake.is_some()
    }
    pub fn thread_delineated(&self) -> Option<String> {
        self.has_thread()
            .then(|| format!("{}/{}", self.guild_snowflake.as_ref().unwrap(), self.thread_snowflake.as_ref().unwrap()))
    }
    pub fn thread_url(&self) -> Option<String> {
        self.thread_delineated().map(|d| format!("https://discord.com/channels/{d}"))
    }

    pub fn has_some_url(&self) -> bool {
        self.has_thread() || self.has_message() || self.has_aux_message()
    }
    pub fn some_url(&self) -> Option<String> {
        if self.has_thread() {
            self.thread_url()
        } else if self.has_message() {
            self.message_url()
        } else if self.has_aux_message() {
            self.aux_message_url()
        } else {
            None
        }
    }

    pub fn has_aux_actor(&self) -> bool {
        self.aux_actor_id.is_some()
    }
    pub fn has_parent_event(&self) -> bool {
        self.parent_event_id.is_some()
    }

    /// Port of `hasNonEmptyData`/`getData()`: reads `entry.data` as a JSON
    /// object (the placeholder `entry` Value's "data" field, standing in
    /// for `GroupAuditLogEntry.getData()`).
    pub fn has_non_empty_data(&self) -> bool {
        self.get_data().is_some_and(|d| !d.is_empty())
    }
    pub fn get_data(&self) -> Option<&Map<String, Value>> {
        self.entry.as_ref()?.get("data")?.as_object()
    }
    pub fn get_data_field(&self, name: &str) -> Option<&Value> {
        self.get_data()?.get(name)
    }

    pub fn get_aux_data(&self, name: &str) -> Option<&Value> {
        self.aux_data.as_ref()?.get(name)
    }
    pub fn set_aux_data(&mut self, name: &str, value: Value) {
        self.aux_data.get_or_insert_with(Map::new).insert(name.to_string(), value);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEvent {
    pub id: String,
    #[serde(alias = "type_ex")]
    pub type_ex: String,
    #[serde(alias = "actor_id")]
    pub actor_id: Option<String>,
    #[serde(alias = "actor_display_name")]
    pub actor_display_name: Option<String>,
    #[serde(alias = "target_id")]
    pub target_id: Option<String>,
    #[serde(alias = "target_display_name")]
    pub target_display_name: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub data: Option<Value>,
}

#[cfg(test)]
mod tests {
    /// Pins the on-disk key names to Java's. Java's Gson has no
    /// `FieldNamingPolicy`, so each key is the Java field name verbatim;
    /// these structs briefly serialised Rust's snake_case names instead,
    /// which made a Java data directory load as all-defaults and then get
    /// re-saved empty. Any rename here is a silent data loss, so it has to
    /// break a test.
    #[test]
    fn metadata_json_keys_match_javas_field_names() {
        let meta = AuditEntryMetadata {
            guild_snowflake: Some("1".into()),
            channel_snowflake: Some("2".into()),
            message_snowflake: Some("3".into()),
            thread_snowflake: Some("4".into()),
            aux_message_snowflake: Some("5".into()),
            entry_description: Some("d".into()),
            entry_redacted: true,
            aux_actor_id: Some("usr_1".into()),
            aux_actor_display_name: Some("Someone".into()),
            parent_event_id: Some("p".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&meta).unwrap();
        for key in [
            "guildSnowflake",
            "channelSnowflake",
            "messageSnowflake",
            "threadSnowflake",
            "auxMessageSnowflake",
            "entryTags",
            "entryDescription",
            "entryRedacted",
            "auxActorId",
            "auxActorDisplayName",
            "parentEventId",
            "snapshotTargetUser",
            "snapshotTargetUserGroups",
            "snapshotTargetUserRepresentedGroup",
        ] {
            assert!(json.get(key).is_some(), "AuditEntryMetadata is missing Java's `{key}`");
        }

        let user = UserMetadata {
            user_snowflake: Some("12345".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&user).unwrap();
        for key in ["userSnowflake", "auditEntryIds", "evidenceSubmissions"] {
            assert!(json.get(key).is_some(), "UserMetadata is missing Java's `{key}`");
        }

        let evidence = EvidenceSubmission::default();
        let json = serde_json::to_value(&evidence).unwrap();
        for key in [
            "auditEntryId",
            "submitterSnowflake",
            "submitterDisplayName",
            "submissionTime",
            "fileName",
            "filePath",
            "proxyUrl",
        ] {
            assert!(json.get(key).is_some(), "EvidenceSubmission is missing Java's `{key}`");
        }

        // Java names these three maps idiosyncratically; camelCasing the
        // Rust field names would produce the wrong key for all of them.
        let json = serde_json::to_value(GlobalMetadata::default()).unwrap();
        assert!(json.get("userSnowflake2userId").is_some());
        let json = serde_json::to_value(LiveInstancesMetadata::default()).unwrap();
        assert!(json.get("location2AuditEntryId").is_some());
        assert!(json.get("location2instanceEmbedMessage").is_some());

        let json = serde_json::to_value(InstanceEmbedMessage::default()).unwrap();
        for key in ["guildSnowflake", "openedAt", "closedAt", "playerlistMessageSnowflake"] {
            assert!(json.get(key).is_some(), "InstanceEmbedMessage is missing Java's `{key}`");
        }

        let json = serde_json::to_value(CustomEvent::default()).unwrap();
        for key in ["typeEx", "actorId", "actorDisplayName", "targetId", "targetDisplayName"] {
            assert!(json.get(key).is_some(), "CustomEvent is missing Java's `{key}`");
        }
    }

    /// A Java-written record loads with its values intact — the actual
    /// failure mode, rather than just the key spelling.
    #[test]
    fn a_java_written_record_round_trips_with_its_values() {
        let java = serde_json::json!({
            "guildSnowflake": "111",
            "entryTags": ["tag.a", "tag.b"],
            "entryDescription": "why they were banned",
            "entryRedacted": true,
            "auxActorId": "usr_actor"
        });
        let meta: AuditEntryMetadata = serde_json::from_value(java).unwrap();
        assert_eq!(meta.guild_snowflake.as_deref(), Some("111"));
        assert_eq!(meta.entry_description.as_deref(), Some("why they were banned"));
        assert!(meta.entry_redacted);
        assert_eq!(meta.aux_actor_id.as_deref(), Some("usr_actor"));
        assert_eq!(meta.entry_tags.len(), 2);

        let links = serde_json::json!({ "userSnowflake2userId": { "12345": "usr_1" } });
        let global: GlobalMetadata = serde_json::from_value(links).unwrap();
        assert_eq!(global.user_snowflake_to_user_id.get("12345").map(String::as_str), Some("usr_1"));
    }

    /// The snake_case files Rust builds wrote before this fix still load,
    /// so testing data isn't stranded by the correction.
    #[test]
    fn a_snake_case_record_from_an_earlier_rust_build_still_loads() {
        let old = serde_json::json!({
            "guild_snowflake": "111",
            "entry_description": "from an earlier build",
            "entry_redacted": true
        });
        let meta: AuditEntryMetadata = serde_json::from_value(old).unwrap();
        assert_eq!(meta.guild_snowflake.as_deref(), Some("111"));
        assert_eq!(meta.entry_description.as_deref(), Some("from an earlier build"));
        assert!(meta.entry_redacted);

        let old = serde_json::json!({ "user_snowflake_to_user_id": { "12345": "usr_1" } });
        let global: GlobalMetadata = serde_json::from_value(old).unwrap();
        assert_eq!(global.user_snowflake_to_user_id.get("12345").map(String::as_str), Some("usr_1"));
    }

    use super::*;

    #[test]
    fn global_snowflake_link_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        assert_eq!(data.global_get_snowflake_id("123"), None);
        data.global_set_snowflake_id("123", "usr_abc");
        assert_eq!(data.global_get_snowflake_id("123"), Some("usr_abc".to_string()));
    }

    #[test]
    fn live_instance_playerlist_and_close_state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        let opened = Utc::now();
        data.live_instances_set_location_instance_embed_message(
            "wrld_1:instance",
            Some("1".into()),
            Some("2".into()),
            Some("3".into()),
            Some("4".into()),
            Some("5".into()),
            opened,
        );
        data.live_instances_set_playerlist_message("wrld_1:instance", "6".into());
        let closed = opened + chrono::Duration::minutes(10);
        data.live_instances_set_location_closed_at("wrld_1:instance", closed);
        let stored = data
            .live_instances_get_location_instance_embed_message("wrld_1:instance", false)
            .unwrap();
        assert_eq!(stored.playerlist_message_snowflake.as_deref(), Some("6"));
        assert_eq!(stored.opened_at, Some(opened));
        assert_eq!(stored.closed_at, Some(closed));
    }

    #[test]
    fn user_metadata_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let data = CelesteData::open(dir.path());
            data.user_metadata_set_snowflake("usr_abc", Some("123".to_string()));
        }
        let data2 = CelesteData::open(dir.path());
        assert_eq!(data2.user_metadata_get_snowflake("usr_abc"), Some("123".to_string()));
    }

    #[test]
    fn link_and_unlink_by_snowflake() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        data.link_id_to_snowflake("usr_abc", "123");
        assert_eq!(data.global_get_snowflake_id("123"), Some("usr_abc".to_string()));
        assert_eq!(data.user_metadata_get_snowflake("usr_abc"), Some("123".to_string()));

        let unlinked = data.unlink_by_snowflake("123");
        assert_eq!(unlinked, Some("usr_abc".to_string()));
        assert_eq!(data.global_get_snowflake_id("123"), None);
        assert_eq!(data.user_metadata_get_snowflake("usr_abc"), None);
    }

    #[test]
    fn link_and_unlink_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        data.link_id_to_snowflake("usr_abc", "123");
        let unlinked = data.unlink_by_id("usr_abc");
        assert_eq!(unlinked, Some("123".to_string()));
        assert_eq!(data.global_get_snowflake_id("123"), None);
    }

    #[test]
    fn audit_entry_tags_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        data.audit_entry_metadata_set_tags("gaud_1", vec!["nsfw".to_string(), "griefing".to_string()]);
        let meta = data.audit_entry_metadata("gaud_1");
        assert!(meta.has_tags());
        assert!(meta.entry_tags.contains("nsfw"));
        assert!(meta.entry_tags.contains("griefing"));

        data.audit_entry_metadata_edit_tags("gaud_1", &["nsfw".to_string()], vec!["resolved".to_string()]);
        let meta2 = data.audit_entry_metadata("gaud_1");
        assert!(!meta2.entry_tags.contains("nsfw"));
        assert!(meta2.entry_tags.contains("griefing"));
        assert!(meta2.entry_tags.contains("resolved"));
    }

    #[test]
    fn audit_entry_metadata_url_helpers() {
        let mut meta = AuditEntryMetadata::default();
        assert!(!meta.has_message());
        meta.guild_snowflake = Some("1".to_string());
        meta.channel_snowflake = Some("2".to_string());
        meta.message_snowflake = Some("3".to_string());
        assert!(meta.has_message());
        assert_eq!(meta.message_url().as_deref(), Some("https://discord.com/channels/1/2/3"));
    }

    #[test]
    fn custom_event_new_and_filter_by_type_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        let now = Utc::now();
        let id_a = data.custom_event_new("groupex.instance.staff.join", now, None, None, None, None, None);
        let id_b = data.custom_event_new("groupex.instance.user.join", now, None, None, None, None, None);

        let staff_events = data.custom_event_filter(Some("groupex.instance.staff.join"), None, None);
        assert_eq!(staff_events, vec![id_a.clone()]);

        let all_events = data.custom_event_filter(None, None, None);
        assert_eq!(all_events.len(), 2);
        assert!(all_events.contains(&id_a));
        assert!(all_events.contains(&id_b));

        let none_before = data.custom_event_filter(None, None, Some(now - chrono::Duration::seconds(10)));
        assert!(none_before.is_empty());
    }

    #[test]
    fn exists_reports_true_after_write() {
        let dir = tempfile::tempdir().unwrap();
        let data = CelesteData::open(dir.path());
        assert!(!data.audit_entry_metadata_exists("gaud_1"));
        data.set_audit_entry_metadata("gaud_1", AuditEntryMetadata::default());
        assert!(data.audit_entry_metadata_exists("gaud_1"));
    }
}
