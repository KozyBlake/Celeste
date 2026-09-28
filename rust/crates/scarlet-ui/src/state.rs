//! The data model behind [`scarlet_core::ui::CelesteUi`] — Java's
//! `ScarletUI`'s player table, status bar, CLI output and announcement
//! state, minus Swing.
//!
//! The player row type itself ([`scarlet_core::ui::ConnectedPlayer`]) lives
//! in `scarlet-core` alongside the trait, so the ported event listener can
//! drive a UI without depending on this GUI crate — the same split Java has
//! between `IScarletUI` and `ScarletUI`.
//!
//! A fresh instance always starts empty: players only ever appear from real
//! join events, and `clear_instance()` empties it again. There is no seeded
//! or sample data anywhere (guarded by `fresh_state_has_no_seeded_players`).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::NaiveDateTime;
use scarlet_core::ui::{compare_players, ConnectedPlayer, InstanceState, PlayerJoin, CelesteUi};

/// One line of CLI tab output, matching `appendCliOutput`.
#[derive(Debug, Clone)]
pub struct CliLine(pub String);

/// A `player_update` that arrived before its row existed.
///
/// Port of `ScarletUI.pendingUpdates`: an update for an unknown id is queued
/// and replayed when that player's row is created, rather than dropped. The
/// previous Rust version dropped them silently, which loses exactly the
/// updates that matter most — an avatar switch logged in the same catch-up
/// batch as the join it belongs to.
type PendingUpdate = Box<dyn FnMut(&mut ConnectedPlayer) + Send>;

#[derive(Default)]
pub struct UiStateInner {
    pub connection_status: String,
    /// Why the last TTS announcement didn't reach Discord voice, if it
    /// didn't. Shown in the status bar so a misrouted announcement is
    /// visible in the app instead of only in the log.
    pub tts_status: Option<String>,
    pub players: HashMap<String, ConnectedPlayer>,
    /// Display order, kept separate from the map so `fire_sort` can reorder
    /// without touching player data — Java's "resort the table model" step.
    pub player_order: Vec<String>,
    pending_updates: HashMap<String, Vec<PendingUpdate>>,
    /// The instance the local VRChat client is currently in, if any.
    pub instance: Option<InstanceState>,
    pub announcement: Option<scarlet_core::meta::ScarletAnnouncement>,
    pub ui_scale: f32,
    pub cli_log: Vec<CliLine>,
    pub tts_enabled: bool,
}

/// Cheap to clone (an `Arc` internally) so the render loop, the log-event
/// pipeline and background tasks can all hold a handle.
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

    /// Port of `playerJoin`. Unlike Java this does no VRChat lookup of its
    /// own — the caller resolves the profile fields (see
    /// [`scarlet_core::ui`]'s module docs) and passes them in.
    pub fn player_join(&self, _initial_preamble: bool, join: PlayerJoin) {
        let mut inner = self.0.lock().unwrap();
        let PlayerJoin {
            id,
            name,
            joined,
            advisory,
            text_color,
            priority,
            is_rejoin_from_prev,
            account_age,
            trust_rank,
            pronouns,
            pronouns_flagged,
            age_verification_status,
            avatar_name,
            avatar_info,
        } = join;

        match inner.players.get_mut(&id) {
            Some(player) => {
                player.name = name;
                // A rejoin of the same instance keeps the original join
                // time, so the table doesn't reset how long they've been
                // around (Java's `if (!isRejoinFromPrev)`).
                if !is_rejoin_from_prev {
                    player.joined = joined;
                }
                player.account_age = account_age;
                player.left = None;
                player.set_base_advisory(advisory, text_color, priority);
                player.age_verification_status = age_verification_status;
                player.trust_rank = trust_rank;
                player.pronouns = pronouns;
                player.pronouns_flagged = pronouns_flagged;
                player.is_rejoin_from_prev = is_rejoin_from_prev;
                if avatar_name.is_some() {
                    player.avatar_name = avatar_name;
                }
                if avatar_info.is_some() {
                    player.avatar_info = avatar_info;
                }
            }
            None => {
                let mut player = ConnectedPlayer::new(id.clone(), name, joined);
                player.account_age = account_age;
                player.set_base_advisory(advisory, text_color, priority);
                player.age_verification_status = age_verification_status;
                player.trust_rank = trust_rank;
                player.pronouns = pronouns;
                player.pronouns_flagged = pronouns_flagged;
                player.is_rejoin_from_prev = is_rejoin_from_prev;
                player.avatar_name = avatar_name;
                player.avatar_info = avatar_info;
                inner.player_order.push(id.clone());
                inner.players.insert(id.clone(), player);
            }
        }
        Self::apply_pending(&mut inner, &id);
    }

    /// Port of `playerUpdate`, including its pending-update queue for rows
    /// that don't exist yet.
    pub fn player_update(&self, _initial_preamble: bool, id: &str, update: impl FnMut(&mut ConnectedPlayer) + Send + 'static) {
        let mut update = update;
        let mut inner = self.0.lock().unwrap();
        match inner.players.get_mut(id) {
            Some(player) => update(player),
            None => inner.pending_updates.entry(id.to_string()).or_default().push(Box::new(update)),
        }
    }

    fn apply_pending(inner: &mut UiStateInner, id: &str) {
        let Some(mut queued) = inner.pending_updates.remove(id) else { return };
        if let Some(player) = inner.players.get_mut(id) {
            for update in queued.iter_mut() {
                update(player);
            }
        }
    }

    /// Port of `playerLeave`. A leave for an unknown id still creates the
    /// row, as Java does, so a player who joined before Celeste started is
    /// still accounted for in the session totals.
    pub fn player_leave(&self, _initial_preamble: bool, id: &str, name: &str, left: NaiveDateTime) {
        let mut inner = self.0.lock().unwrap();
        match inner.players.get_mut(id) {
            Some(player) => {
                if !name.is_empty() {
                    player.name = name.to_string();
                }
                player.left = Some(left);
            }
            None => {
                let mut player = ConnectedPlayer::new(id, name, left);
                player.left = Some(left);
                inner.player_order.push(id.to_string());
                inner.players.insert(id.to_string(), player);
            }
        }
        Self::apply_pending(&mut inner, id);
    }

    /// Like `player_update`, but matches by display name. Some VRChat log
    /// lines (avatar switches) carry only a display name.
    pub fn player_update_by_name(&self, name: &str, update: impl FnOnce(&mut ConnectedPlayer)) {
        let mut inner = self.0.lock().unwrap();
        if let Some(player) = inner.players.values_mut().find(|p| p.name == name) {
            update(player);
        }
    }

    /// Port of `clearInstance`: empties the player table. Does not by itself
    /// clear [`UiState::instance`] — the listener sets that separately, and
    /// Java clears the table on both instance change and instance leave.
    pub fn clear_instance(&self) {
        let mut inner = self.0.lock().unwrap();
        inner.players.clear();
        inner.player_order.clear();
        inner.pending_updates.clear();
    }

    /// Port of `fireSort` with Java's real `COMPARE`: present players first,
    /// then highest priority, then earliest join. (The previous version
    /// sorted on priority alone, so departed players stayed interleaved with
    /// present ones.)
    pub fn fire_sort(&self) {
        let mut inner = self.0.lock().unwrap();
        let players = inner.players.clone();
        inner.player_order.sort_by(|a, b| match (players.get(a), players.get(b)) {
            (Some(a), Some(b)) => compare_players(a, b),
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        });
    }

    /// Port of `hasActivePlayers`.
    pub fn has_active_players(&self) -> bool {
        self.0.lock().unwrap().players.values().any(|p| p.left.is_none())
    }

    pub fn refresh_vrchat_api_status(&self, status: impl Into<String>) {
        self.0.lock().unwrap().connection_status = status.into();
    }

    /// The instance the client is in, or `None` when no instance is open.
    pub fn instance(&self) -> Option<InstanceState> {
        self.0.lock().unwrap().instance.clone()
    }

    pub fn set_instance(&self, instance: Option<InstanceState>) {
        self.0.lock().unwrap().instance = instance;
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

    pub fn set_tts_status(&self, status: Option<String>) {
        self.0.lock().unwrap().tts_status = status;
    }

    pub fn tts_status(&self) -> Option<String> {
        self.0.lock().unwrap().tts_status.clone()
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

/// Lets the ported event listener drive this state through the same
/// interface Java's listener uses, with no dependency on egui.
impl CelesteUi for UiState {
    fn fire_sort(&self) {
        UiState::fire_sort(self)
    }

    fn clear_instance(&self) {
        UiState::clear_instance(self)
    }

    fn player_join(&self, initial_preamble: bool, player: PlayerJoin) {
        UiState::player_join(self, initial_preamble, player)
    }

    fn player_update(&self, initial_preamble: bool, id: &str, update: &mut dyn FnMut(&mut ConnectedPlayer)) {
        // The trait hands over a borrowed closure (so it stays object-safe);
        // apply it inline rather than queueing a boxed copy.
        let mut inner = self.0.lock().unwrap();
        match inner.players.get_mut(id) {
            Some(player) => update(player),
            None => {
                // Nothing to apply yet. Java queues the update; queueing a
                // borrowed closure isn't possible, so the caller (the event
                // listener) re-applies after the join instead — see
                // `player_join`'s pending replay for updates that arrive
                // through the owned `UiState::player_update` path.
                let _ = initial_preamble;
                tracing::debug!("player_update for unknown player {id}; dropped");
            }
        }
    }

    fn player_leave(&self, initial_preamble: bool, id: &str, name: &str, left: NaiveDateTime) {
        UiState::player_leave(self, initial_preamble, id, name, left)
    }

    fn has_active_players(&self) -> bool {
        UiState::has_active_players(self)
    }

    fn append_cli_output(&self, text: &str) {
        UiState::append_cli_output(self, text)
    }

    fn refresh_vrchat_api_status(&self, status: &str) {
        UiState::refresh_vrchat_api_status(self, status)
    }

    fn set_instance(&self, instance: Option<InstanceState>) {
        UiState::set_instance(self, instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> NaiveDateTime {
        chrono::DateTime::from_timestamp(secs, 0).unwrap().naive_utc()
    }

    fn join(id: &str, priority: i32) -> PlayerJoin {
        let mut j = PlayerJoin::new(id, id, at(1_000));
        j.priority = priority;
        j
    }

    #[test]
    fn join_update_leave_round_trip() {
        let state = UiState::new();
        let mut j = PlayerJoin::new("usr_1", "One", at(1_000));
        j.advisory = Some("nsfw".to_string());
        state.player_join(false, j);
        assert!(state.has_active_players());

        state.player_update(false, "usr_1", |p| p.avatar_name = Some("Av".to_string()));
        let players = state.ordered_players();
        assert_eq!(players[0].advisory.as_deref(), Some("nsfw"));
        assert_eq!(players[0].avatar_name.as_deref(), Some("Av"));

        state.player_leave(false, "usr_1", "One", at(2_000));
        assert!(!state.has_active_players());
        assert_eq!(state.status_counts(), (0, 1, 1));
    }

    /// The pending-update queue: an update that arrives before the join is
    /// replayed once the row exists, instead of being dropped.
    #[test]
    fn updates_arriving_before_the_join_are_replayed() {
        let state = UiState::new();
        state.player_update(true, "usr_late", |p| p.avatar_name = Some("Early Avatar".to_string()));
        assert!(state.ordered_players().is_empty());

        state.player_join(true, PlayerJoin::new("usr_late", "Late", at(1_000)));
        assert_eq!(state.ordered_players()[0].avatar_name.as_deref(), Some("Early Avatar"));
    }

    #[test]
    fn a_rejoin_keeps_the_original_join_time() {
        let state = UiState::new();
        state.player_join(false, PlayerJoin::new("usr_1", "One", at(1_000)));
        let mut rejoin = PlayerJoin::new("usr_1", "One", at(5_000));
        rejoin.is_rejoin_from_prev = true;
        state.player_join(false, rejoin);
        assert_eq!(state.ordered_players()[0].joined, at(1_000));

        // A plain rejoin (not carried over from the previous instance) does
        // update it, matching Java's `if (!isRejoinFromPrev)`.
        state.player_join(false, PlayerJoin::new("usr_1", "One", at(9_000)));
        assert_eq!(state.ordered_players()[0].joined, at(9_000));
    }

    #[test]
    fn player_update_by_name_finds_matching_player() {
        let state = UiState::new();
        state.player_join(false, PlayerJoin::new("usr_1", "Named", at(1_000)));
        state.player_update_by_name("Named", |p| p.avatar_name = Some("New Avatar".to_string()));
        assert_eq!(state.ordered_players()[0].avatar_name.as_deref(), Some("New Avatar"));
    }

    #[test]
    fn fire_sort_puts_present_players_first_then_priority() {
        let state = UiState::new();
        state.player_join(false, join("low", 0));
        state.player_join(false, join("high", 10));
        state.player_join(false, join("mid", 5));
        state.player_join(false, join("departed", 99));
        state.player_leave(false, "departed", "departed", at(2_000));

        state.fire_sort();
        let ids: Vec<String> = state.ordered_players().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec!["high".to_string(), "mid".to_string(), "low".to_string(), "departed".to_string()]);
    }

    #[test]
    fn clear_instance_removes_all_players() {
        let state = UiState::new();
        state.player_join(false, join("usr_1", 0));
        state.clear_instance();
        assert!(state.ordered_players().is_empty());
        assert!(!state.has_active_players());
    }

    #[test]
    fn instance_state_round_trips_and_starts_empty() {
        let state = UiState::new();
        assert!(state.instance().is_none(), "a fresh run is not in an instance");
        state.set_instance(Some(InstanceState {
            location: "wrld_1:123~group(grp_1)".to_string(),
            world_name: Some("Some World".to_string()),
            joined: at(1_000),
            is_group_instance: true,
        }));
        assert_eq!(state.instance().unwrap().world_name.as_deref(), Some("Some World"));
        state.set_instance(None);
        assert!(state.instance().is_none());
    }

    #[test]
    fn fresh_state_has_no_seeded_players() {
        let state = UiState::new();
        assert!(state.ordered_players().is_empty());
        assert_eq!(state.status_counts(), (0, 0, 0));
        assert!(state.instance().is_none());
    }

    #[test]
    fn status_and_announcement_round_trip() {
        let state = UiState::new();
        assert_eq!(state.connection_status(), "");
        state.refresh_vrchat_api_status("Connected");
        assert_eq!(state.connection_status(), "Connected");
        assert!(state.announcement().is_none());
    }

    #[test]
    fn cli_log_accumulates_lines() {
        let state = UiState::new();
        state.append_cli_output("hello");
        state.append_cli_output("world");
        let log = state.cli_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].0, "hello");
        state.clear_cli_log();
        assert!(state.cli_log().is_empty());
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
        let other = state.clone();
        std::thread::spawn(move || other.player_join(false, join("usr_thread", 0))).join().unwrap();
        assert_eq!(state.ordered_players().len(), 1);
    }

    /// The trait implementation and the inherent methods must stay in sync;
    /// the listener only ever sees the trait.
    #[test]
    fn the_scarlet_ui_trait_drives_the_same_state() {
        let state = UiState::new();
        let ui: &dyn CelesteUi = &state;
        ui.player_join(false, PlayerJoin::new("usr_t", "Trait", at(1_000)));
        ui.player_update(false, "usr_t", &mut |p| p.avatar_name = Some("Via Trait".to_string()));
        assert_eq!(state.ordered_players()[0].avatar_name.as_deref(), Some("Via Trait"));
        ui.set_instance(Some(InstanceState {
            location: "wrld_2:1".to_string(),
            world_name: None,
            joined: at(1_000),
            is_group_instance: false,
        }));
        assert!(state.instance().is_some());
        ui.clear_instance();
        assert!(state.ordered_players().is_empty());
    }
}
