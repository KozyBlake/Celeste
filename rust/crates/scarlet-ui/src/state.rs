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

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::NaiveDateTime;

/// Port of the data half of `ScarletUI.ConnectedPlayer` (the Swing
/// `Action`-backed "View"/"Open" button fields aren't ported — those are
/// widget wiring, not player data).
#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub id: String,
    pub name: String,
    pub joined: NaiveDateTime,
    pub left: Option<NaiveDateTime>,
    pub advisory: Option<String>,
    /// RGB, matching the Java `Color text_color` parameter.
    pub text_color: Option<(u8, u8, u8)>,
    pub priority: i32,
    pub is_rejoin_from_prev: bool,
}

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
}

/// Port of the mutating half of `IScarletUI`'s contract
/// (`playerJoin`/`playerUpdate`/`playerLeave`/`clearInstance`/`fireSort`/
/// `refreshVrchatApiStatus`/`showAnnouncement`/`hasActivePlayers`). Cheap to
/// clone (an `Arc` internally) so both the render loop and background
/// polling tasks can hold a handle.
#[derive(Clone, Default)]
pub struct UiState(std::sync::Arc<Mutex<UiStateInner>>);

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `playerJoin`. `initial_preamble` (true while replaying the
    /// instance's current roster on connect, matching the Java parameter)
    /// is accepted for API parity but doesn't change behavior here — Rust's
    /// state model doesn't need to suppress UI churn during that replay the
    /// way Swing table-update batching might have.
    #[allow(clippy::too_many_arguments)]
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: &str, priority: i32) -> ConnectedPlayer {
        ConnectedPlayer {
            id: id.to_string(),
            name: id.to_string(),
            joined: chrono::Local::now().naive_local(),
            left: None,
            advisory: None,
            text_color: None,
            priority,
            is_rejoin_from_prev: false,
        }
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
