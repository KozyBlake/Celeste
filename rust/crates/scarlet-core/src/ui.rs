//! Port of `net.sybyline.scarlet.IScarletUI` and the data half of
//! `ScarletUI.ConnectedPlayer`.
//!
//! Java's event listener talks to the UI through this interface, with a
//! no-op `ScarletUIHeadless` standing in when there's no display. Keeping
//! the same split here is what lets the ported event listener live in its
//! own crate and drive either the egui window or nothing at all, instead of
//! depending on the GUI crate directly.
//!
//! ## Two deliberate differences from Java
//!
//! 1. **The VRChat lookup moved out of the UI.** Java's `playerJoin` calls
//!    `scarlet.vrc.getUser(id)` inline to fill account age, trust rank,
//!    pronouns and age-verification status — a blocking network call made
//!    from a synchronized UI method. Here those fields arrive already
//!    resolved on [`PlayerJoin`], because the caller (the event listener) is
//!    async and can do the lookup without blocking a UI frame. Same data,
//!    resolved one layer earlier.
//! 2. **No training view.** Java keeps a parallel `parkedPlayers` map so
//!    `ScarletSimulation`'s training mode can show fake rows while the real
//!    instance carries on underneath. Training mode is deliberately out of
//!    scope for this port (see PORTING.md's scope section), so there is one
//!    player map and `clear_instance` always clears it.

use chrono::NaiveDateTime;

use crate::trust_rank::TrustRank;

/// Java's `Integer.MIN_VALUE + 1`, the "no priority" sentinel every advisory
/// path starts from. Not `i32::MIN`: Java leaves room to negate it.
pub const NO_PRIORITY: i32 = i32::MIN + 1;

/// Port of `AvatarBundleInfo`, reduced to the fields the player table and
/// its "View" popup actually display.
///
/// The full Java type wraps a `VersionedFile`, a `ModelFile` and a
/// `FileAnalysis`; the remaining fields (upload sizes, platform, version
/// history) are only read by the avatar-hydration path, which is not ported
/// yet — see PORTING.md. Extending this struct is the right move when it
/// lands, rather than inventing a partial model now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AvatarBundleInfo {
    pub file_id: Option<String>,
    pub file_name: Option<String>,
    /// VRChat's own wording: "Excellent"/"Good"/"Medium"/"Poor"/"VeryPoor".
    pub performance_rating: Option<String>,
}

/// Port of the data half of `ScarletUI.ConnectedPlayer`.
///
/// The Swing `Action` fields (View/Open/Ban/Unban/Copy buttons) aren't data
/// and aren't ported here — those are behaviours the UI layer wires up
/// against a row, not state the event listener sets.
#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub id: String,
    pub name: String,
    pub pronouns: Option<String>,
    pub pronouns_flagged: bool,
    pub avatar_name: Option<String>,
    pub avatar_info: Option<AvatarBundleInfo>,
    /// Human-readable reason avatar info is missing, shown by "View" when
    /// [`ConnectedPlayer::avatar_info`] is `None`.
    pub avatar_info_note: Option<String>,
    /// Account age at join, as (years, months, days) — Java's `Period`.
    pub account_age: Option<(i32, i32, i32)>,
    pub joined: NaiveDateTime,
    pub left: Option<NaiveDateTime>,
    /// Raw VRChat `ageVerificationStatus` (e.g. "18+", "hidden").
    pub age_verification_status: Option<String>,
    pub trust_rank: TrustRank,

    /// Computed from the base/avatar pair below; never set directly.
    pub advisory: Option<String>,
    pub text_color: Option<(u8, u8, u8)>,
    pub priority: i32,

    base_advisory: Option<String>,
    base_text_color: Option<(u8, u8, u8)>,
    base_priority: i32,
    avatar_advisory: Option<String>,
    avatar_text_color: Option<(u8, u8, u8)>,
    avatar_priority: i32,

    pub is_rejoin_from_prev: bool,
}

impl ConnectedPlayer {
    pub fn new(id: impl Into<String>, name: impl Into<String>, joined: NaiveDateTime) -> Self {
        ConnectedPlayer {
            id: id.into(),
            name: name.into(),
            pronouns: None,
            pronouns_flagged: false,
            avatar_name: None,
            avatar_info: None,
            avatar_info_note: None,
            account_age: None,
            joined,
            left: None,
            age_verification_status: None,
            trust_rank: TrustRank::Unknown,
            advisory: None,
            text_color: None,
            priority: NO_PRIORITY,
            base_advisory: None,
            base_text_color: None,
            base_priority: NO_PRIORITY,
            avatar_advisory: None,
            avatar_text_color: None,
            avatar_priority: NO_PRIORITY,
            is_rejoin_from_prev: false,
        }
    }

    /// Port of `setBaseAdvisory`: the join-check advisory (watched user,
    /// watched group, new account, pronouns, trust rank).
    pub fn set_base_advisory(&mut self, advisory: Option<String>, text_color: Option<(u8, u8, u8)>, priority: i32) {
        self.base_advisory = advisory;
        self.base_text_color = text_color;
        self.base_priority = priority;
        self.rebuild_advisory_and_style();
    }

    /// Port of `setAvatarAdvisory`: the watched-avatar advisory, which is
    /// resolved separately and can outrank the base one.
    pub fn set_avatar_advisory(&mut self, advisory: Option<String>, text_color: Option<(u8, u8, u8)>, priority: i32) {
        self.avatar_advisory = advisory;
        self.avatar_text_color = text_color;
        self.avatar_priority = priority;
        self.rebuild_advisory_and_style();
    }

    pub fn base_advisory(&self) -> Option<&str> {
        self.base_advisory.as_deref()
    }

    pub fn avatar_advisory(&self) -> Option<&str> {
        self.avatar_advisory.as_deref()
    }

    /// Port of `rebuildAdvisoryAndStyle`.
    fn rebuild_advisory_and_style(&mut self) {
        self.advisory = join_advisory_parts(self.base_advisory.as_deref(), self.avatar_advisory.as_deref());
        if self.avatar_priority > self.base_priority {
            self.priority = self.avatar_priority;
            self.text_color = self.avatar_text_color;
        } else {
            self.priority = self.base_priority;
            self.text_color = self.base_text_color;
        }
    }
}

/// Port of `ScarletUI.joinAdvisoryParts` + `normalizeAdvisoryPart`.
pub fn join_advisory_parts(first: Option<&str>, second: Option<&str>) -> Option<String> {
    let first = normalize_advisory_part(first);
    let second = normalize_advisory_part(second);
    match (first, second) {
        (None, second) => second,
        (Some(first), None) => Some(first),
        (Some(first), Some(second)) if first == second => Some(first),
        (Some(first), Some(second)) => Some(format!("{first} / {second}")),
    }
}

fn normalize_advisory_part(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Port of `ScarletUI.COMPARE`, the player-table sort: still-present players
/// first, then highest priority, then earliest join.
pub fn compare_players(a: &ConnectedPlayer, b: &ConnectedPlayer) -> std::cmp::Ordering {
    a.left
        .is_some()
        .cmp(&b.left.is_some())
        .then_with(|| b.priority.cmp(&a.priority))
        .then_with(|| a.joined.cmp(&b.joined))
}

/// Everything `IScarletUI.playerJoin` receives, plus the profile fields
/// Java's implementation looked up inline (see the module docs).
#[derive(Debug, Clone)]
pub struct PlayerJoin {
    pub id: String,
    pub name: String,
    pub joined: NaiveDateTime,
    pub advisory: Option<String>,
    pub text_color: Option<(u8, u8, u8)>,
    pub priority: i32,
    pub is_rejoin_from_prev: bool,
    pub account_age: Option<(i32, i32, i32)>,
    pub trust_rank: TrustRank,
    pub pronouns: Option<String>,
    pub pronouns_flagged: bool,
    pub age_verification_status: Option<String>,
    pub avatar_name: Option<String>,
    pub avatar_info: Option<AvatarBundleInfo>,
}

impl PlayerJoin {
    pub fn new(id: impl Into<String>, name: impl Into<String>, joined: NaiveDateTime) -> Self {
        PlayerJoin {
            id: id.into(),
            name: name.into(),
            joined,
            advisory: None,
            text_color: None,
            priority: NO_PRIORITY,
            is_rejoin_from_prev: false,
            account_age: None,
            trust_rank: TrustRank::Unknown,
            pronouns: None,
            pronouns_flagged: false,
            age_verification_status: None,
            avatar_name: None,
            avatar_info: None,
        }
    }
}

/// The VRChat instance the local client is currently in.
///
/// Java tracks this as `ScarletEventListener.clientLocation` and never shows
/// it in the UI — the window only implies an instance exists by having rows
/// in the player table. Surfacing it is one of the "1:1 plus fixes" changes:
/// with no instance state of its own, the Rust UI had no way to distinguish
/// "no instance open" from "instance open, nobody logged yet", which is
/// exactly the failure this port was reported with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceState {
    /// The raw location, e.g.
    /// `wrld_...:09580~group(grp_...)~region(us)`.
    pub location: String,
    /// The world's display name, from the log's `Entering Room:` line.
    pub world_name: Option<String>,
    pub joined: NaiveDateTime,
    /// Whether this instance belongs to the configured VRChat group — the
    /// gate on every Discord emission in the event listener.
    pub is_group_instance: bool,
}

/// Port of `IScarletUI`, minus the Swing-specific members (`jframe`,
/// `getParentComponent`, `setUIScale`, `loadSettings`) which have no meaning
/// outside Swing and are handled by the egui app itself.
pub trait ScarletUi: Send + Sync {
    fn fire_sort(&self);
    fn clear_instance(&self);
    fn player_join(&self, initial_preamble: bool, player: PlayerJoin);
    fn player_update(&self, initial_preamble: bool, id: &str, update: &mut dyn FnMut(&mut ConnectedPlayer));
    fn player_leave(&self, initial_preamble: bool, id: &str, name: &str, left: NaiveDateTime);
    fn has_active_players(&self) -> bool;
    /// Appends to the CLI tab (Java's `appendCliOutput`).
    fn append_cli_output(&self, text: &str);
    /// Java's `refreshVrchatApiStatus`, which reads the status itself; here
    /// the caller passes the text it resolved.
    fn refresh_vrchat_api_status(&self, status: &str);
    /// Sets (or clears, with `None`) the instance the client is in.
    fn set_instance(&self, instance: Option<InstanceState>);
}

/// Port of `ScarletUIHeadless`: every call is a no-op, for runs with no
/// display (and for tests that only care about the listener's own state).
#[derive(Debug, Default, Clone, Copy)]
pub struct HeadlessUi;

impl ScarletUi for HeadlessUi {
    fn fire_sort(&self) {}
    fn clear_instance(&self) {}
    fn player_join(&self, _initial_preamble: bool, _player: PlayerJoin) {}
    fn player_update(&self, _initial_preamble: bool, _id: &str, _update: &mut dyn FnMut(&mut ConnectedPlayer)) {}
    fn player_leave(&self, _initial_preamble: bool, _id: &str, _name: &str, _left: NaiveDateTime) {}
    fn has_active_players(&self) -> bool {
        false
    }
    fn append_cli_output(&self, _text: &str) {}
    fn refresh_vrchat_api_status(&self, _status: &str) {}
    fn set_instance(&self, _instance: Option<InstanceState>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> NaiveDateTime {
        chrono::DateTime::from_timestamp(secs, 0).unwrap().naive_utc()
    }

    #[test]
    fn advisory_parts_join_dedupe_and_drop_blanks() {
        assert_eq!(join_advisory_parts(None, None), None);
        assert_eq!(join_advisory_parts(Some("  "), None), None);
        assert_eq!(join_advisory_parts(Some("Watched"), None).as_deref(), Some("Watched"));
        assert_eq!(join_advisory_parts(None, Some("Avatar")).as_deref(), Some("Avatar"));
        assert_eq!(join_advisory_parts(Some("Same"), Some("Same")).as_deref(), Some("Same"));
        assert_eq!(join_advisory_parts(Some("A"), Some("B")).as_deref(), Some("A / B"));
    }

    #[test]
    fn the_higher_priority_advisory_supplies_the_row_colour() {
        let mut player = ConnectedPlayer::new("usr_1", "One", at(0));
        player.set_base_advisory(Some("base".into()), Some((1, 1, 1)), 5);
        assert_eq!(player.priority, 5);
        assert_eq!(player.text_color, Some((1, 1, 1)));

        // A lower-priority avatar advisory still shows, but doesn't recolour.
        player.set_avatar_advisory(Some("avatar".into()), Some((2, 2, 2)), 1);
        assert_eq!(player.advisory.as_deref(), Some("base / avatar"));
        assert_eq!(player.priority, 5);
        assert_eq!(player.text_color, Some((1, 1, 1)));

        // A higher-priority one takes over the colour.
        player.set_avatar_advisory(Some("avatar".into()), Some((2, 2, 2)), 9);
        assert_eq!(player.priority, 9);
        assert_eq!(player.text_color, Some((2, 2, 2)));
    }

    #[test]
    fn sort_puts_present_players_first_then_priority_then_join_time() {
        let mut present_low = ConnectedPlayer::new("a", "a", at(100));
        present_low.set_base_advisory(None, None, 0);
        let mut present_high = ConnectedPlayer::new("b", "b", at(200));
        present_high.set_base_advisory(None, None, 10);
        let mut departed = ConnectedPlayer::new("c", "c", at(0));
        departed.set_base_advisory(None, None, 99);
        departed.left = Some(at(300));

        let mut all = vec![departed.clone(), present_low.clone(), present_high.clone()];
        all.sort_by(compare_players);
        assert_eq!(all.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["b", "a", "c"]);
    }
}
