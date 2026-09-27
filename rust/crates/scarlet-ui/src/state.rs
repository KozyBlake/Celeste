//! Shared UI state: the data half of `ScarletUI`'s live player list and
//! status display, kept separate from rendering so it can be mutated from
//! any thread (Discord/VRChat polling tasks) and read by the render loop
//! without either side needing to know about the other.
//!
//! Java's Swing UI has an implicit single-threaded rule (only the AWT Event
//! Dispatch Thread may touch Swing components; `IScarletUI`'s methods
//! internally marshal onto it). egui's immediate-mode render loop has no
//! such requirement — any thread can mutate state behind a `Mutex`, and the
//! next frame just renders whatever is there. That's a real architectural
//! difference from the Java version, not an oversight: there's no EDT to
//! port.
//!
//! Every field here starts empty and is only ever populated by real
//! join/update/leave calls for the instance currently being watched — there
//! is no seeded/sample data. A fresh run (or a `clear_instance()`) means an
//! empty table until VRChat polling (not wired up yet — see PORTING.md)
//! actually reports players.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::NaiveDateTime;
use scarlet_core::trust_rank::TrustRank;

/// Port of the data half of `ScarletUI.ConnectedPlayer` and its
/// `propstable` column list (the Swing `Action`-backed "View"/"Open"/"Copy"/
/// "Ban" button *behavior* isn't ported — those are actions the caller
/// wires up, not player data; the table just needs to know a row exists to
/// render a button for one).
#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub id: String,
    pub name: String,
    pub pronouns: Option<String>,
    pub avatar_name: Option<String>,
    /// e.g. "Good"/"Medium"/"Poor"/"VeryPoor", from VRChat's avatar
    /// performance analysis. `None` until avatar info is looked up.
    pub performance_rating: Option<String>,
    /// Account age at time of join, as (years, months, days) — matching the
    /// Java `Period` field's display shape ("8y 8m 8d" in the table).
    pub account_age: Option<(i32, i32, i32)>,
    pub joined: NaiveDateTime,
    pub left: Option<NaiveDateTime>,
    pub advisory: Option<String>,
    /// Raw VRChat `ageVerificationStatus` value (e.g. "18+", "hidden") —
    /// not yet a typed enum, see module docs on `AgeVerificationStatus` in
    /// PORTING.md.
    pub age_verification_status: Option<String>,
    pub trust_rank: TrustRank,
    /// RGB, matching the Java `Color text_color` parameter.
    pub text_color: Option<(u8, u8, u8)>,
    pub priority: i32,
    pub is_rejoin_from_prev: bool,
}

impl ConnectedPlayer {
    /// Minimal constructor for the fields `IScarletUI::playerJoin` actually
    /// receives; the richer fields (pronouns, avatar, performance, account
    /// age, trust rank) are filled in later via `player_update` once a
    /// VRChat user lookup resolves, matching the Java version's own
    /// two-phase fill (join fires immediately from the audit/join event;
    /// the user profile lookup that supplies the rest is async there too).
    pub fn new(id: impl Into<String>, name: impl Into<String>, joined: NaiveDateTime) -> Self {
        ConnectedPlayer {
            id: id.into(),
            name: name.into(),
            pronouns: None,
            avatar_name: None,
            performance_rating: None,
            account_age: None,
            joined,
            left: None,
            advisory: None,
            age_verification_status: None,
            trust_rank: TrustRank::Unknown,
            text_color: None,
            priority: 0,
            is_rejoin_from_prev: false,
        }
    }
}

/// One line of CLI tab output, matching `appendCliOutput`.
#[derive(Debug, Clone)]
pub struct CliLine(pub String);

#[derive(Debug, Default)]
pub struct UiStateInner {
    pub connection_status: String,
    pub players: HashMap<String, ConnectedPlayer>,
    /// Insertion/sort order for display; kept separate from the map so
    /// `fire_sort` can reorder without touching player data, matching the
    /// Java version's separate "resort the table model" step.
    pub player_order: Vec<String>,
    pub announcement: Option<scarlet_core::meta::ScarletAnnouncement>,
    pub ui_scale: f32,
    pub cli_log: Vec<CliLine>,
    pub tts_enabled: bool,
}

/// Port of the mutating half of `IScarletUI`'s contract
/// (`playerJoin`/`playerUpdate`/`playerLeave`/`clearInstance`/`fireSort`/
/// `refreshVrchatApiStatus`/`showAnnouncement`/`hasActivePlayers`). Cheap to
/// clone (an `Arc` internally) so both the render loop and background
/// polling tasks can hold a handle.
#[derive(Clone)]
pub struct UiState(std::sync::Arc<Mutex<UiStateInner>>);

impl Default for UiState {
    fn default() -> Self {
        UiState(std::sync::Arc::new(Mutex::new(UiStateInner { tts_enabled: true, ..Default::default() })))
    }
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `playerJoin`. `initial_preamble` (true while replaying the
    /// instance's current roster on connect, matching the Java parameter)
    /// is accepted for API parity but doesn't change behavior here — Rust's
    /// state model doesn't need to suppress UI churn during that replay the
    /// way Swing table-update batching might have.
    pub fn player_join(&self, _initial_preamble: bool, player: ConnectedPlayer) {
        let mut inner = self.0.lock().unwrap();
        if !inner.player_order.contains(&player.id) {
            inner.player_order.push(player.id.clone());
        }
        inner.players.insert(player.id.clone(), player);
    }

    /// Port of `playerUpdate`.
    pub fn player_update(&self, _initial_preamble: bool, id: &str, update: impl FnOnce(&mut ConnectedPlayer)) {
        let mut inner = self.0.lock().unwrap();
        if let Some(player) = inner.players.get_mut(id) {
            update(player);
        }
    }

    /// Port of `playerLeave`.
    pub fn player_leave(&self, _initial_preamble: bool, id: &str, left: NaiveDateTime) {
        let mut inner = self.0.lock().unwrap();
        if let Some(player) = inner.players.get_mut(id) {
            player.left = Some(left);
        }
    }

    /// Like `player_update`, but matches by display name instead of id.
    /// Some VRChat log lines (e.g. avatar switches) only give a display
    /// name, not a user id — matching the Java version, which resolves
    /// those against the currently-known roster the same way.
    pub fn player_update_by_name(&self, name: &str, update: impl FnOnce(&mut ConnectedPlayer)) {
        let mut inner = self.0.lock().unwrap();
        if let Some(player) = inner.players.values_mut().find(|p| p.name == name) {
            update(player);
        }
    }

    /// Port of `clearInstance`.
    pub fn clear_instance(&self) {
        let mut inner = self.0.lock().unwrap();
        inner.players.clear();
        inner.player_order.clear();
    }

    /// Port of `fireSort`: re-orders the display list by priority
    /// (matching the Java table's sort column), highest first.
    pub fn fire_sort(&self) {
        let mut inner = self.0.lock().unwrap();
        let players = inner.players.clone();
        inner.player_order.sort_by(|a, b| {
            let pa = players.get(a).map(|p| p.priority).unwrap_or(0);
            let pb = players.get(b).map(|p| p.priority).unwrap_or(0);
            pb.cmp(&pa)
        });
    }

    /// Port of `hasActivePlayers`.
    pub fn has_active_players(&self) -> bool {
        let inner = self.0.lock().unwrap();
        inner.players.values().any(|p| p.left.is_none())
    }

    pub fn refresh_vrchat_api_status(&self, status: impl Into<String>) {
        self.0.lock().unwrap().connection_status = status.into();
    }

    pub fn show_announcement(&self, announcement: scarlet_core::meta::ScarletAnnouncement) {
        self.0.lock().unwrap().announcement = Some(announcement);
    }

    pub fn set_ui_scale(&self, scale: f32) {
        self.0.lock().unwrap().ui_scale = scale;
    }

    /// Snapshot of players in current display order, for rendering.
    pub fn ordered_players(&self) -> Vec<ConnectedPlayer> {
        let inner = self.0.lock().unwrap();
        inner.player_order.iter().filter_map(|id| inner.players.get(id).cloned()).collect()
    }

    pub fn connection_status(&self) -> String {
        self.0.lock().unwrap().connection_status.clone()
    }

    pub fn announcement(&self) -> Option<scarlet_core::meta::ScarletAnnouncement> {
        self.0.lock().unwrap().announcement.clone()
    }

    /// Port of `updateStatusBar`'s counts: (present, left, total).
    pub fn status_counts(&self) -> (usize, usize, usize) {
        let inner = self.0.lock().unwrap();
        let total = inner.players.len();
        let present = inner.players.values().filter(|p| p.left.is_none()).count();
        (present, total - present, total)
    }

    /// Port of `appendCliOutput`.
    pub fn append_cli_output(&self, text: impl Into<String>) {
        self.0.lock().unwrap().cli_log.push(CliLine(text.into()));
    }

    pub fn cli_log(&self) -> Vec<CliLine> {
        self.0.lock().unwrap().cli_log.clone()
    }

    pub fn clear_cli_log(&self) {
        self.0.lock().unwrap().cli_log.clear();
    }

    pub fn tts_enabled(&self) -> bool {
        self.0.lock().unwrap().tts_enabled
    }

    pub fn set_tts_enabled(&self, enabled: bool) {
        self.0.lock().unwrap().tts_enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: &str, priority: i32) -> ConnectedPlayer {
        let mut p = ConnectedPlayer::new(id, id, chrono::Local::now().naive_local());
        p.priority = priority;
        p
    }

    #[test]
    fn join_update_leave_round_trip() {
        let state = UiState::new();
        state.player_join(false, player("usr_1", 0));
        assert!(state.has_active_players());

        state.player_update(false, "usr_1", |p| p.advisory = Some("nsfw".to_string()));
        let players = state.ordered_players();
        assert_eq!(players[0].advisory.as_deref(), Some("nsfw"));

        state.player_leave(false, "usr_1", chrono::Local::now().naive_local());
        assert!(!state.has_active_players());
    }

    #[test]
    fn player_update_by_name_finds_matching_player() {
        let state = UiState::new();
        state.player_join(false, player("usr_1", 0));
        state.player_update_by_name("usr_1", |p| p.avatar_name = Some("New Avatar".to_string()));
        let players = state.ordered_players();
        assert_eq!(players[0].avatar_name.as_deref(), Some("New Avatar"));

        // No matching name: silently does nothing (mirrors the Java
        // version's behavior of ignoring updates for unknown players).
        state.player_update_by_name("nonexistent", |p| p.avatar_name = Some("Should Not Apply".to_string()));
        assert_eq!(state.ordered_players()[0].avatar_name.as_deref(), Some("New Avatar"));
    }

    #[test]
    fn fire_sort_orders_by_priority_descending() {
        let state = UiState::new();
        state.player_join(false, player("low", 1));
        state.player_join(false, player("high", 10));
        state.player_join(false, player("mid", 5));
        state.fire_sort();
        let ids: Vec<String> = state.ordered_players().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["high".to_string(), "mid".to_string(), "low".to_string()]);
    }

    #[test]
    fn clear_instance_removes_all_players() {
        let state = UiState::new();
        state.player_join(false, player("usr_1", 0));
        state.clear_instance();
        assert!(state.ordered_players().is_empty());
        assert!(!state.has_active_players());
    }

    #[test]
    fn fresh_state_has_no_seeded_players() {
        // Guards against ever "helpfully" seeding sample rows again: a new
        // instance must start empty and only reflect real join events.
        let state = UiState::new();
        assert!(state.ordered_players().is_empty());
        assert_eq!(state.status_counts(), (0, 0, 0));
    }

    #[test]
    fn status_and_announcement_round_trip() {
        let state = UiState::new();
        assert_eq!(state.connection_status(), "");
        state.refresh_vrchat_api_status("Connected");
        assert_eq!(state.connection_status(), "Connected");

        assert!(state.announcement().is_none());
        state.show_announcement(scarlet_core::meta::ScarletAnnouncement {
            id: "a1".to_string(),
            severity: Some("warning".to_string()),
            title: Some("Heads up".to_string()),
            message: Some("details".to_string()),
            url: None,
            expires: None,
        });
        assert_eq!(state.announcement().unwrap().id, "a1");
    }

    #[test]
    fn status_counts_match_present_left_total() {
        let state = UiState::new();
        state.player_join(false, player("a", 0));
        state.player_join(false, player("b", 0));
        state.player_leave(false, "a", chrono::Local::now().naive_local());
        assert_eq!(state.status_counts(), (1, 1, 2));
    }

    #[test]
    fn cli_log_accumulates_lines() {
        let state = UiState::new();
        state.append_cli_output("hello");
        state.append_cli_output("world");
        let log = state.cli_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].0, "hello");
    }

    #[test]
    fn tts_defaults_on_and_toggles() {
        let state = UiState::new();
        assert!(state.tts_enabled());
        state.set_tts_enabled(false);
        assert!(!state.tts_enabled());
    }

    #[test]
    fn state_handle_is_shareable_across_threads() {
        let state = UiState::new();
        let state2 = state.clone();
        let handle = std::thread::spawn(move || {
            state2.player_join(false, player("usr_1", 0));
        });
        handle.join().unwrap();
        assert!(state.has_active_players());
    }
}
