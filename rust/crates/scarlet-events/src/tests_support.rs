//! Test doubles for the listener: a UI that records what it was told, and
//! an emitter that records what was emitted. Both are the real traits, so
//! these tests exercise the same call paths the app does.

use std::sync::{Arc, Mutex};

use chrono::NaiveDateTime;
use scarlet_core::ui::{ConnectedPlayer, InstanceState, PlayerJoin, CelesteUi};

#[derive(Debug, Default)]
pub struct UiCalls {
    pub joins: Vec<PlayerJoin>,
    pub leaves: Vec<(String, NaiveDateTime)>,
    pub clears: usize,
    pub instance: Option<InstanceState>,
    pub cli: Vec<String>,
    /// Players as the UI would hold them, so updates can be asserted.
    pub players: Vec<ConnectedPlayer>,
}

#[derive(Debug, Default, Clone)]
pub struct RecordingUi(pub Arc<Mutex<UiCalls>>);

impl RecordingUi {
    pub fn calls(&self) -> std::sync::MutexGuard<'_, UiCalls> {
        self.0.lock().unwrap()
    }

    pub fn player(&self, id: &str) -> Option<ConnectedPlayer> {
        self.calls().players.iter().find(|p| p.id == id).cloned()
    }
}

impl CelesteUi for RecordingUi {
    fn fire_sort(&self) {}

    fn clear_instance(&self) {
        let mut calls = self.0.lock().unwrap();
        calls.clears += 1;
        calls.players.clear();
    }

    fn player_join(&self, _initial_preamble: bool, player: PlayerJoin) {
        let mut calls = self.0.lock().unwrap();
        let mut row = ConnectedPlayer::new(player.id.clone(), player.name.clone(), player.joined);
        row.set_base_advisory(player.advisory.clone(), player.text_color, player.priority);
        row.avatar_name = player.avatar_name.clone();
        row.trust_rank = player.trust_rank;
        row.pronouns = player.pronouns.clone();
        row.pronouns_flagged = player.pronouns_flagged;
        row.is_rejoin_from_prev = player.is_rejoin_from_prev;
        calls.players.retain(|p| p.id != player.id);
        calls.players.push(row);
        calls.joins.push(player);
    }

    fn player_update(&self, _initial_preamble: bool, id: &str, update: &mut dyn FnMut(&mut ConnectedPlayer)) {
        let mut calls = self.0.lock().unwrap();
        if let Some(player) = calls.players.iter_mut().find(|p| p.id == id) {
            update(player);
        }
    }

    fn player_leave(&self, _initial_preamble: bool, id: &str, _name: &str, left: NaiveDateTime) {
        let mut calls = self.0.lock().unwrap();
        if let Some(player) = calls.players.iter_mut().find(|p| p.id == id) {
            player.left = Some(left);
        }
        calls.leaves.push((id.to_string(), left));
    }

    fn has_active_players(&self) -> bool {
        self.0.lock().unwrap().players.iter().any(|p| p.left.is_none())
    }

    fn append_cli_output(&self, text: &str) {
        self.0.lock().unwrap().cli.push(text.to_string());
    }

    fn refresh_vrchat_api_status(&self, _status: &str) {}

    fn set_instance(&self, instance: Option<InstanceState>) {
        self.0.lock().unwrap().instance = instance;
    }
}

/// Records every emission, and reports itself as emitting for every type so
/// the gating under test is the listener's own group-instance check rather
/// than "nothing is configured".
#[derive(Debug, Default, Clone)]
pub struct RecordingEmitter(pub Arc<Mutex<Vec<String>>>);

impl RecordingEmitter {
    pub fn events(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    fn record(&self, what: String) {
        self.0.lock().unwrap().push(what);
    }
}

#[async_trait::async_trait]
impl crate::AuditEmitter for RecordingEmitter {
    async fn is_emitting(&self, _event_type: &str) -> bool {
        true
    }
    async fn emit_user_join(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str) {
        self.record(format!("user_join:{u}"));
    }
    async fn emit_user_leave(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str) {
        self.record(format!("user_leave:{u}"));
    }
    async fn emit_staff_join(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str) {
        self.record(format!("staff_join:{u}"));
    }
    async fn emit_staff_leave(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str) {
        self.record(format!("staff_leave:{u}"));
    }
    async fn emit_user_avatar(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str, a: &str, _p: &[String]) {
        self.record(format!("user_avatar:{u}:{a}"));
    }
    async fn emit_vtk_initiated(
        &self,
        _t: NaiveDateTime,
        _l: &str,
        _ti: Option<&str>,
        td: &str,
        _ai: Option<&str>,
        _ad: Option<&str>,
    ) {
        self.record(format!("vtk:{td}"));
    }
    async fn emit_user_spawn_pedestal(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str, _c: &str, i: &str) {
        self.record(format!("pedestal:{u}:{i}"));
    }
    async fn emit_user_spawn_sticker(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str, s: &str) {
        self.record(format!("sticker:{u}:{s}"));
    }
    async fn emit_user_spawn_prop(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str, p: &str) {
        self.record(format!("prop:{u}:{p}"));
    }
    async fn emit_user_video(&self, _t: NaiveDateTime, _l: &str, u: &str, _d: &str, url: &str, _title: &str) {
        self.record(format!("video:{u}:{url}"));
    }
}

/// Records what would have been spoken.
#[derive(Debug, Default, Clone)]
pub struct RecordingTts(pub Arc<Mutex<Vec<String>>>);

impl RecordingTts {
    pub fn spoken(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl crate::TtsSink for RecordingTts {
    fn speak(&self, _marker: &str, text: &str) {
        self.0.lock().unwrap().push(text.to_string());
    }
    fn speak_after_nuisance_alert(&self, _marker: &str, text: &str) {
        self.0.lock().unwrap().push(format!("[nuisance chime] {text}"));
    }
    fn speak_mixed_character_name_alert(&self, _marker: &str) {
        self.0.lock().unwrap().push("[mixed-name alert]".to_string());
    }
}
