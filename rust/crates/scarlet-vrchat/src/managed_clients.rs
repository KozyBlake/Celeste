//! Persistent desired-state configuration for long-running VRChat clients.
//!
//! A slot is identified by VRChat's numeric local `--profile`, not by a PID
//! or window title. PIDs change after a crash; the profile and exact desired
//! instance survive it. The supervisor in `scarlet` reconciles these records
//! against the group-instance API and the local process table.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "managed_vrchat_clients.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstanceTemplate {
    pub world_id: String,
    #[serde(default = "default_access")]
    pub group_access_type: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub age_gate: bool,
    #[serde(default = "default_true")]
    pub queue_enabled: bool,
    #[serde(default)]
    pub minimum_avatar_performance: String,
}

fn default_access() -> String { "public".to_string() }
fn default_region() -> String { "us".to_string() }
fn default_true() -> bool { true }

impl Default for InstanceTemplate {
    fn default() -> Self {
        Self {
            world_id: String::new(),
            group_access_type: default_access(),
            region: default_region(),
            display_name: String::new(),
            age_gate: false,
            queue_enabled: true,
            minimum_avatar_performance: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedClientSlot {
    pub id: String,
    pub label: String,
    /// VRChat's numeric local profile. This is deliberately not a `usr_` id.
    pub profile: u32,
    #[serde(default)]
    pub expected_user_id: String,
    #[serde(default)]
    pub expected_display_name: String,
    #[serde(default)]
    pub desired_location: String,
    pub template: InstanceTemplate,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_restart: bool,
    #[serde(default)]
    pub auto_recreate_instance: bool,
    #[serde(default)]
    pub last_status: String,
    #[serde(default)]
    pub last_error: String,
}

impl Default for ManagedClientSlot {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            label: "VRChat monitor".to_string(),
            profile: 1,
            expected_user_id: String::new(),
            expected_display_name: String::new(),
            desired_location: String::new(),
            template: InstanceTemplate::default(),
            enabled: true,
            auto_restart: true,
            auto_recreate_instance: false,
            last_status: "Not started".to_string(),
            last_error: String::new(),
        }
    }
}

impl ManagedClientSlot {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.label.trim().is_empty(), "enter a slot name");
        // Profile zero is not a free choice. Two separate things already
        // claim it: the legacy single-client instance follow relaunches
        // profile zero by hand (`instance_lifecycle.rs`), and
        // `launch::profile_pids` deliberately treats a lone VRChat started
        // with no `--profile` argument *as* profile zero. A managed slot on
        // zero would therefore fight the follower for the same client, and
        // could stop a client the operator launched themselves.
        anyhow::ensure!(
            self.profile != 0,
            "profile 0 is reserved for a manually launched client and the single-client instance follow; use 1 or higher"
        );
        anyhow::ensure!(
            self.expected_user_id.is_empty() || self.expected_user_id.starts_with("usr_"),
            "the expected account must be blank or a usr_ ID"
        );
        anyhow::ensure!(
            self.desired_location.is_empty() || split_location(&self.desired_location).is_some(),
            "the assigned instance must contain a world and instance ID"
        );
        anyhow::ensure!(
            self.template.world_id.is_empty() || self.template.world_id.starts_with("wrld_"),
            "the replacement world must be blank or a wrld_ ID"
        );
        anyhow::ensure!(
            matches!(self.template.group_access_type.as_str(), "public" | "plus" | "members"),
            "group access must be public, plus, or members"
        );
        anyhow::ensure!(
            matches!(self.template.region.as_str(), "us" | "use" | "eu" | "jp"),
            "region must be us, use, eu, or jp"
        );
        anyhow::ensure!(self.template.display_name.chars().count() <= 32, "instance names may contain at most 32 characters");
        if self.auto_recreate_instance {
            anyhow::ensure!(!self.template.world_id.is_empty(), "automatic replacement needs a world ID");
        }
        Ok(())
    }

    pub fn creation_request(&self, group_id: &str) -> serde_json::Value {
        let mut request = serde_json::json!({
            "worldId": self.template.world_id,
            "ownerId": group_id,
            "type": "group",
            "groupAccessType": self.template.group_access_type,
            "region": self.template.region,
            "queueEnabled": self.template.queue_enabled,
            "hardClose": false,
            "ageGate": self.template.age_gate,
        });
        if !self.template.display_name.trim().is_empty() {
            request["displayName"] = serde_json::json!(self.template.display_name.trim());
        }
        if !self.template.minimum_avatar_performance.is_empty() {
            request["minimumAvatarPerformance"] =
                serde_json::json!(self.template.minimum_avatar_performance);
        }
        request
    }
}

pub fn split_location(location: &str) -> Option<(&str, &str)> {
    let (world, instance) = location.trim().split_once(':')?;
    (world.starts_with("wrld_") && !instance.is_empty()).then_some((world, instance))
}

pub fn location_from_instance(
    instance: &crate::models::Instance,
    fallback_world: &str,
) -> Option<String> {
    let world = instance.world_id.as_deref().unwrap_or(fallback_world);
    let id = instance.instance_id.as_deref().or(instance.id.as_deref())?;
    if id.contains(':') {
        Some(id.to_string())
    } else {
        Some(format!("{world}:{id}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientLogSession {
    pub file: PathBuf,
    pub user_id: String,
    pub display_name: String,
    pub location: Option<String>,
    pub recently_active: bool,
    pub clean_quit: bool,
}

/// Inventory recent VRChat output logs independently. The original Celeste
/// tailer follows only the newest file; managed clients need to identify all
/// simultaneous sessions by the `User Authenticated` record in each file.
pub fn discover_log_sessions(directory: &Path) -> Vec<ClientLogSession> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("output_log_") && name.ends_with(".txt"))
        })
        .collect::<Vec<_>>();
    files.sort();
    files.reverse();
    files.truncate(64);
    files.into_iter().filter_map(|file| inspect_log(&file)).collect()
}

fn inspect_log(path: &Path) -> Option<ClientLogSession> {
    const HEAD_BYTES: u64 = 4 * 1024 * 1024;
    const TAIL_BYTES: u64 = 8 * 1024 * 1024;
    let metadata = std::fs::metadata(path).ok()?;
    let recently_active = metadata.modified().ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age <= std::time::Duration::from_secs(120));
    let mut user = None;
    let mut location = None;
    let mut clean_quit = false;

    let file = std::fs::File::open(path).ok()?;
    for line in BufReader::new(file.take(HEAD_BYTES)).lines().map_while(Result::ok) {
        inspect_line(&line, &mut user, &mut location, &mut clean_quit);
        if user.is_some() && location.is_some() {
            break;
        }
    }
    let mut file = std::fs::File::open(path).ok()?;
    let start = metadata.len().saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut lines = BufReader::new(file).lines();
    if start > 0 {
        let _ = lines.next(); // discard the partial first line
    }
    for line in lines.map_while(Result::ok) {
        inspect_line(&line, &mut user, &mut location, &mut clean_quit);
    }
    let (display_name, user_id) = user?;
    Some(ClientLogSession {
        file: path.to_path_buf(),
        user_id,
        display_name,
        location,
        recently_active,
        clean_quit,
    })
}

fn inspect_line(
    line: &str,
    user: &mut Option<(String, String)>,
    location: &mut Option<String>,
    clean_quit: &mut bool,
) {
    let Some((_timestamp, _level, text)) = crate::log_tail::parse_log_line(line) else { return };
    match crate::log_tail::parse_entry_text(text) {
        Some(crate::log_tail::LogEventKind::UserAuthenticated { display_name, user_id }) => {
            *user = Some((display_name, user_id));
        }
        Some(crate::log_tail::LogEventKind::UserJoined { location: joined }) => {
            *location = Some(joined);
        }
        Some(crate::log_tail::LogEventKind::UserLeft) => *location = None,
        Some(crate::log_tail::LogEventKind::UserQuit { .. }) => *clean_quit = true,
        _ => {}
    }
}

pub struct ManagedClientStore {
    path: PathBuf,
    slots: Mutex<Vec<ManagedClientSlot>>,
}

impl ManagedClientStore {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);
        let slots = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, slots: Mutex::new(slots) }
    }

    pub fn list(&self) -> Vec<ManagedClientSlot> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn upsert(&self, slot: ManagedClientSlot) -> Result<()> {
        slot.validate()?;
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::ensure!(
            !slots.iter().any(|existing| existing.id != slot.id && existing.profile == slot.profile),
            "profile {} is already assigned to another managed client",
            slot.profile
        );
        if let Some(existing) = slots.iter_mut().find(|existing| existing.id == slot.id) {
            *existing = slot;
        } else {
            slots.push(slot);
        }
        save(&self.path, &slots)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.retain(|slot| slot.id != id);
        save(&self.path, &slots)
    }

    pub fn update_runtime(
        &self,
        id: &str,
        desired_location: Option<String>,
        status: impl Into<String>,
        error: impl Into<String>,
    ) -> Result<()> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let slot = slots.iter_mut().find(|slot| slot.id == id).context("managed client no longer exists")?;
        let mut changed = false;
        if let Some(location) = desired_location {
            if slot.desired_location != location {
                slot.desired_location = location;
                changed = true;
            }
        }
        let status = status.into();
        let error = error.into();
        if slot.last_status != status {
            slot.last_status = status;
            changed = true;
        }
        if slot.last_error != error {
            slot.last_error = error;
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        save(&self.path, &slots)
    }
}

fn save(path: &Path, slots: &[ManagedClientSlot]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(slots)?)?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("saving managed clients to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_unique_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = ManagedClientStore::open(dir.path());
        let mut first = ManagedClientSlot::default();
        first.profile = 2;
        store.upsert(first.clone()).unwrap();
        let mut duplicate = ManagedClientSlot::default();
        duplicate.profile = 2;
        assert!(store.upsert(duplicate).is_err());
        assert_eq!(ManagedClientStore::open(dir.path()).list(), [first]);
    }

    #[test]
    fn replacement_request_is_group_owned() {
        let mut slot = ManagedClientSlot::default();
        slot.template.world_id = "wrld_1".into();
        slot.template.group_access_type = "plus".into();
        slot.template.age_gate = true;
        let request = slot.creation_request("grp_1");
        assert_eq!(request["ownerId"], "grp_1");
        assert_eq!(request["groupAccessType"], "plus");
        assert_eq!(request["ageGate"], true);
    }

    #[test]
    fn instance_response_becomes_an_exact_location() {
        let instance = crate::models::Instance {
            instance_id: Some("123~group(grp_1)~region(us)".into()),
            ..Default::default()
        };
        assert_eq!(
            location_from_instance(&instance, "wrld_1").as_deref(),
            Some("wrld_1:123~group(grp_1)~region(us)")
        );
    }

    #[test]
    fn discovers_each_authenticated_live_log_independently() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("output_log_2026-10-06_12-00-00.txt");
        let second = dir.path().join("output_log_2026-10-06_12-00-01.txt");
        std::fs::write(
            &first,
            "2026.10.06 12:00:00 Debug      -  User Authenticated: First (usr_first)\n\
             2026.10.06 12:00:01 Debug      -  [Behaviour] Joining wrld_one:1~group(grp_1)\n",
        )
        .unwrap();
        std::fs::write(
            &second,
            "2026.10.06 12:00:00 Debug      -  User Authenticated: Second (usr_second)\n\
             2026.10.06 12:00:01 Debug      -  [Behaviour] Joining wrld_two:2~group(grp_1)\n",
        )
        .unwrap();
        let sessions = discover_log_sessions(dir.path());
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|session| {
            session.user_id == "usr_first"
                && session.location.as_deref() == Some("wrld_one:1~group(grp_1)")
        }));
        assert!(sessions.iter().any(|session| {
            session.user_id == "usr_second"
                && session.location.as_deref() == Some("wrld_two:2~group(grp_1)")
        }));
    }

    /// Profile zero is contested ground: the legacy instance follow
    /// relaunches it directly, and `launch::profile_pids` maps a VRChat
    /// started with no `--profile` argument onto it. A managed slot there
    /// would fight the follower, or stop a client the operator started by
    /// hand. The UI picks from 1 upward for the same reason, but the
    /// validator is what actually guarantees it.
    #[test]
    fn a_slot_cannot_claim_the_reserved_profile() {
        let mut slot = ManagedClientSlot { profile: 0, ..Default::default() };
        let error = slot.validate().expect_err("profile 0 must be rejected");
        assert!(error.to_string().contains("reserved"), "the reason is stated: {error}");

        slot.profile = 1;
        slot.validate().expect("profile 1 is allowed");
    }

    /// The default must itself be valid, or the UI's "add client" button
    /// would hand the operator a slot that cannot be saved.
    #[test]
    fn the_default_slot_is_valid() {
        ManagedClientSlot::default().validate().expect("default slot validates");
    }
}
