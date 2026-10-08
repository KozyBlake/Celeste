//! Port target: `ScarletUI.java` (5,716 lines — the full Swing main window)
//! and `ScarletDiscordUI.java` (2,345 lines — Discord bot config panel).
//!
//! This covers the *shape* real-to-real: the toolbar row, the Instance/
//! Settings/CLI tab structure, the live player table with the same columns
//! as `propstable` in the Java source, and the status bar text format — all
//! driven by real state ([`crate::state::UiState`] for players/status,
//! [`celeste_core::settings`] for the Settings tab), not fixtures. What
//! The user-facing CLI commands, Create Instance, Import Groups, and Sync
//! Discord are wired to their
//! real stores/services. The table's
//! Ban/Unban/Invite actions call the shared VRChat client. TTS pause/resume, skip, and the
//! CLI `tts <text>` command are backed by the real platform TTS service.
//!
//! Toolkit: `egui`/`eframe` (see prior module docs in git history for the
//! rationale) with `egui_extras::TableBuilder` for the grid.
//!
//! **Not visually verified by me** — confirmed working by the repo owner
//! on their own machine (this container has no display server). If
//! something here looks wrong against a fresh run, that's expected until
//! iterated on further, not a regression from a "known good" state.

use std::path::PathBuf;
use std::sync::Arc;

use celeste_core::settings::file_valued::SettingDescriptor;
use celeste_core::settings::file_valued::{SettingCategory, SettingKind};
use celeste_core::settings::registry::Rectangle;
use celeste_core::settings::Settings;

use crate::state::UiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Instance,
    Clients,
    Settings,
    Cli,
}

/// A named run of tutorial steps.
///
/// The tour is split so a first-time operator can finish something
/// coherent without sitting through operator-only material. Essential
/// chapters run end to end; advanced ones are offered afterwards and can
/// also be launched individually from Help.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TutorialChapter {
    Basics,
    InstanceAndRoster,
    VoiceAndSettings,
    HelpAndTroubleshooting,
    DiscordModeration,
    VrChatIntegrations,
    ManagedClients,
    DataAndRecovery,
}

/// What a chapter needs configured before it describes anything real.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TutorialNeeds {
    Nothing,
    Discord,
    VrChat,
}

impl TutorialChapter {
    /// Running order. Essential chapters come first so the tour has a
    /// natural end for someone who does not want the advanced material.
    const ORDER: [TutorialChapter; 8] = [
        TutorialChapter::Basics,
        TutorialChapter::InstanceAndRoster,
        TutorialChapter::VoiceAndSettings,
        TutorialChapter::HelpAndTroubleshooting,
        TutorialChapter::DiscordModeration,
        TutorialChapter::VrChatIntegrations,
        TutorialChapter::ManagedClients,
        TutorialChapter::DataAndRecovery,
    ];

    fn title(self) -> &'static str {
        match self {
            TutorialChapter::Basics => "Basics",
            TutorialChapter::InstanceAndRoster => "Your instance and roster",
            TutorialChapter::VoiceAndSettings => "Voice and settings",
            TutorialChapter::HelpAndTroubleshooting => "Help and troubleshooting",
            TutorialChapter::DiscordModeration => "Discord and moderation",
            TutorialChapter::VrChatIntegrations => "VRChat integrations",
            TutorialChapter::ManagedClients => "Managed VRChat clients",
            TutorialChapter::DataAndRecovery => "Data, CLI, and recovery",
        }
    }

    fn essential(self) -> bool {
        matches!(
            self,
            TutorialChapter::Basics
                | TutorialChapter::InstanceAndRoster
                | TutorialChapter::VoiceAndSettings
                | TutorialChapter::HelpAndTroubleshooting
        )
    }

    fn needs(self) -> TutorialNeeds {
        match self {
            TutorialChapter::DiscordModeration => TutorialNeeds::Discord,
            TutorialChapter::VrChatIntegrations => TutorialNeeds::VrChat,
            TutorialChapter::ManagedClients => TutorialNeeds::VrChat,
            _ => TutorialNeeds::Nothing,
        }
    }

    /// Steps belonging to this chapter, as indices into [`TUTORIAL_STEPS`].
    fn steps(self) -> Vec<usize> {
        TUTORIAL_STEPS
            .iter()
            .enumerate()
            .filter(|(_, step)| step.chapter == self)
            .map(|(index, _)| index)
            .collect()
    }
}

#[derive(Clone, Copy)]
struct TutorialStep {
    chapter: TutorialChapter,
    tab: Tab,
    target: TutorialTarget,
    settings_category: Option<SettingCategory>,
    demo_data: bool,
    title: &'static str,
    body: &'static str,
    try_it: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialTarget {
    Header,
    TabBar,
    MainContent,
    InstanceCard,
    PlayerTable,
    ToolbarControls,
    SettingsSidebar,
    SettingsContent,
    StatusBar,
    Help,
}

#[derive(Default)]
struct TutorialLayout {
    header: Option<egui::Rect>,
    tab_bar: Option<egui::Rect>,
    toolbar_controls: Option<egui::Rect>,
    main_content: Option<egui::Rect>,
    instance_card: Option<egui::Rect>,
    player_table: Option<egui::Rect>,
    settings_sidebar: Option<egui::Rect>,
    settings_content: Option<egui::Rect>,
    status_bar: Option<egui::Rect>,
    help: Option<egui::Rect>,
}

struct TutorialDemo {
    state: UiState,
    directory: PathBuf,
    roster_mode: Option<TutorialRosterMode>,
    roster_scene: Option<usize>,
    roster_started: std::time::Instant,
    roster_event: usize,
    ambient_last_event: std::time::Instant,
    ambient_next_id: usize,
    ambient_present: Vec<String>,
    ambient_should_leave: bool,
    pending_scroll_target: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialRosterMode {
    Lightweight,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TutorialRosterCue {
    Speech(String),
    NuisanceAlert(String),
    MixedCharacterAlert(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TutorialScenarioPhase {
    WaitingForIntroduction,
    PlayingExample,
    WaitingForWrapUp,
    Complete,
}

impl TutorialRosterMode {
    fn title(self) -> &'static str {
        match self {
            Self::Lightweight => "Lightweight community instance",
            Self::Busy => "Busy instance with escalating risk",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Lightweight => {
                "About a dozen people are present. A community moderator, an ordinary watched-group member, and a mixed-character display name demonstrate routine informational and warning callouts."
            }
            Self::Busy => {
                "Roughly 35–40 people are present. A real VRChat Nuisance trust rank, a critical watched group, a risky new account, and departures appear without simulating a rapid raid."
            }
        }
    }
}

impl TutorialDemo {
    fn create() -> Option<Self> {
        use celeste_core::trust_rank::TrustRank;
        use celeste_core::ui::{AvatarBundleInfo, InstanceState, PlayerJoin};
        use celeste_core::watched_membership::WatchedMembership;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "celeste-guided-tour-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).ok()?;

        let fixture = serde_json::json!({
            "tutorialOnly": true,
            "instance": {
                "world": "Celeste Tutorial World",
                "location": "wrld_tutorial:42420~group(grp_tutorial)~region(us)"
            },
            "players": [
                {"id":"usr_tutorial_normal","name":"Example Visitor","kind":"ordinary"},
                {"id":"usr_tutorial_watched","name":"Example Watched User","kind":"watched"},
                {"id":"usr_tutorial_flagged","name":"Example Flagged Profile","kind":"flagged"},
                {"id":"usr_tutorial_left","name":"Example Departed Player","kind":"departed"}
            ]
        });
        let watched_users = serde_json::json!({
            "usr_tutorial_watched": {
                "message": "Tutorial: watched user and group match",
                "priority": 100
            }
        });
        let watched_groups = serde_json::json!({
            "grp_tutorial_moderators": {
                "type": "COMMUNITY",
                "message": "Tutorial community moderator — not a real moderation record"
            },
            "grp_tutorial_watched": {
                "type": "OTHER",
                "message": "Tutorial general watchlist — not a real moderation record"
            },
            "grp_tutorial_critical": {
                "type": "MALICIOUS",
                "message": "Tutorial critical watchlist — not a real moderation record"
            }
        });
        for (name, value) in [
            ("tutorial_instance.json", fixture),
            ("watched_users.json", watched_users),
            ("watched_groups.json", watched_groups),
        ] {
            let bytes = serde_json::to_vec_pretty(&value).ok()?;
            if std::fs::write(directory.join(name), bytes).is_err() {
                let _ = std::fs::remove_dir_all(&directory);
                return None;
            }
        }

        let state = UiState::new();
        let now = chrono::Local::now().naive_local();
        state.set_instance(Some(InstanceState {
            location: "wrld_tutorial:42420~group(grp_tutorial)~region(us)".to_string(),
            world_name: Some("Celeste Tutorial World — DEMO DATA".to_string()),
            joined: now - chrono::Duration::minutes(18),
            is_group_instance: true,
        }));
        state.refresh_vrchat_api_status("VRChat: tutorial data (not signed in)");

        let mut ordinary = PlayerJoin::new(
            "usr_tutorial_normal",
            "Example Visitor",
            now - chrono::Duration::minutes(12),
        );
        ordinary.pronouns = Some("they/them".to_string());
        ordinary.account_age = Some((1, 4, 8));
        ordinary.trust_rank = TrustRank::User;
        ordinary.age_verification_status = Some("18+".to_string());
        ordinary.avatar_name = Some("Tutorial Avatar".to_string());
        ordinary.avatar_info = Some(AvatarBundleInfo {
            performance_rating: Some("Good".to_string()),
            file_id: Some("file_tutorial_bundle".to_string()),
            file_name: Some("tutorial-avatar.vrca".to_string()),
            owner_id: Some("usr_tutorial_creator".to_string()),
            avatar_id: Some("avtr_tutorial".to_string()),
        });
        state.player_join(false, ordinary);

        let mut watched = PlayerJoin::new(
            "usr_tutorial_watched",
            "Example Watched User",
            now - chrono::Duration::minutes(9),
        );
        watched.pronouns = Some("she/her".to_string());
        watched.account_age = Some((0, 1, 12));
        watched.trust_rank = TrustRank::Known;
        watched.advisory = Some("DEMO: watched user; member of a watched group".to_string());
        watched.text_color = Some((255, 176, 76));
        watched.priority = 100;
        watched.watched_memberships = vec![WatchedMembership {
            group_id: "grp_tutorial_warning".to_string(),
            kind: "NUISANCE".to_string(),
            current_name: Some("Tutorial Watched Group".to_string()),
            message: Some("Demo only — inspect this membership during the tour".to_string()),
            hidden: false,
            critical: true,
            silent: false,
        }];
        state.player_join(false, watched);

        let mut flagged = PlayerJoin::new(
            "usr_tutorial_flagged",
            "Example Flagged Profile",
            now - chrono::Duration::minutes(7),
        );
        flagged.pronouns = Some("example.invalid/link".to_string());
        flagged.pronouns_flagged = true;
        flagged.account_age = Some((0, 0, 2));
        flagged.trust_rank = TrustRank::Visitor;
        flagged.advisory = Some("DEMO: new account and flagged profile text".to_string());
        flagged.text_color = Some((255, 208, 96));
        flagged.priority = 60;
        flagged.avatar_name = Some("Very Poor Tutorial Avatar".to_string());
        flagged.avatar_info = Some(AvatarBundleInfo {
            performance_rating: Some("VeryPoor".to_string()),
            ..Default::default()
        });
        state.player_join(false, flagged);

        let departed = PlayerJoin::new(
            "usr_tutorial_left",
            "Example Departed Player",
            now - chrono::Duration::minutes(15),
        );
        state.player_join(false, departed);
        state.player_leave(
            false,
            "usr_tutorial_left",
            "Example Departed Player",
            now - chrono::Duration::minutes(3),
        );

        Some(Self {
            state,
            directory,
            roster_mode: None,
            roster_scene: None,
            roster_started: std::time::Instant::now(),
            roster_event: 0,
            ambient_last_event: std::time::Instant::now(),
            ambient_next_id: 1,
            ambient_present: Vec::new(),
            ambient_should_leave: false,
            pending_scroll_target: None,
        })
    }

    fn set_roster_mode(&mut self, mode: TutorialRosterMode) {
        if self.roster_mode == Some(mode) {
            return;
        }
        self.roster_mode = Some(mode);
        self.roster_started = std::time::Instant::now();
        self.roster_event = 0;
        self.ambient_last_event = std::time::Instant::now();
        self.ambient_next_id = 1;
        self.ambient_present.clear();
        self.ambient_should_leave = false;
        self.pending_scroll_target = None;
        self.state.clear_instance();

        let now = chrono::Local::now().naive_local();
        self.state
            .set_instance(Some(celeste_core::ui::InstanceState {
                location: match mode {
                    TutorialRosterMode::Lightweight => {
                        "wrld_tutorial:42420~group(grp_tutorial)~region(us)"
                    }
                    TutorialRosterMode::Busy => {
                        "wrld_tutorial:42421~group(grp_tutorial)~region(us)"
                    }
                }
                .to_string(),
                world_name: Some(format!("{} — DEMO DATA", mode.title())),
                joined: now - chrono::Duration::minutes(18),
                is_group_instance: true,
            }));

        // Only the host exists when the scene opens. Generic visitors fill
        // the room one at a time while narration continues, instead of a
        // whole synthetic roster appearing in a single frame.
        let host_id = "usr_tutorial_ambient_host";
        self.join_ordinary(
            host_id,
            "Example Instance Host",
            12,
            celeste_core::trust_rank::TrustRank::Known,
        );
        self.ambient_present.push(host_id.to_string());
        self.pending_scroll_target = Some(host_id.to_string());
    }

    fn ambient_target(mode: TutorialRosterMode) -> usize {
        match mode {
            TutorialRosterMode::Lightweight => 9,
            TutorialRosterMode::Busy => 35,
        }
    }

    /// Keep the fake instance moving independently of narrated warnings.
    /// These are deliberately ordinary users: they change the roster but do
    /// not enqueue speech or sounds, just like uneventful real joins/leaves.
    fn tick_ambient_activity(&mut self, mode: TutorialRosterMode, scene: usize) {
        if self.roster_scene != Some(scene) {
            self.roster_scene = Some(scene);
            self.roster_mode = None;
        }
        self.set_roster_mode(mode);

        let target = Self::ambient_target(mode);
        let filling = self.ambient_present.len() < target;
        let interval = if filling {
            match mode {
                TutorialRosterMode::Lightweight => 1.35,
                TutorialRosterMode::Busy => 0.72,
            }
        } else {
            3.25
        };
        if self.ambient_last_event.elapsed().as_secs_f32() < interval {
            return;
        }
        self.ambient_last_event = std::time::Instant::now();

        if !filling && self.ambient_should_leave {
            self.depart_one_ambient();
            self.ambient_should_leave = false;
            return;
        }

        let number = self.ambient_next_id;
        self.ambient_next_id += 1;
        let id = format!("usr_tutorial_ambient_{number}");
        self.join_ordinary(
            &id,
            &format!("Example Visitor {number}"),
            0,
            celeste_core::trust_rank::TrustRank::User,
        );
        self.ambient_present.push(id.clone());
        self.pending_scroll_target = Some(id);
        if self.ambient_present.len() >= target {
            self.ambient_should_leave = true;
        }
    }

    fn depart_one_ambient(&mut self) {
        let Some(index) = self
            .ambient_present
            .iter()
            .position(|id| id != "usr_tutorial_ambient_host")
        else {
            return;
        };
        let id = self.ambient_present.remove(index);
        let number = id
            .rsplit('_')
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        self.state.player_leave(
            false,
            &id,
            &format!("Example Visitor {number}"),
            chrono::Local::now().naive_local(),
        );
        self.pending_scroll_target = Some(id);
    }

    fn join_ordinary(
        &self,
        id: &str,
        name: &str,
        minutes_ago: i64,
        rank: celeste_core::trust_rank::TrustRank,
    ) {
        let mut player = celeste_core::ui::PlayerJoin::new(
            id,
            name,
            chrono::Local::now().naive_local() - chrono::Duration::minutes(minutes_ago),
        );
        let seed = id.bytes().fold(0_usize, |value, byte| {
            value.wrapping_mul(31).wrapping_add(byte as usize)
        });
        let pronouns = [
            Some("he/him"),
            Some("she/her"),
            Some("they/them"),
            Some("he/they"),
            Some("she/they"),
            Some("any/all"),
            None,
        ];
        let ages = [
            (0, 2, 11),
            (0, 8, 4),
            (1, 1, 19),
            (2, 7, 3),
            (4, 0, 22),
            (6, 5, 14),
            (9, 2, 1),
        ];
        let avatars = [
            ("Neon Fox", "Good"),
            ("Cozy Robot", "Excellent"),
            ("Moonlit Cat", "Medium"),
            ("Retro Dragon", "Poor"),
            ("Tiny Astronaut", "Good"),
            ("Garden Moth", "Medium"),
            ("Fallback Tutorial Avatar", "VeryPoor"),
        ];
        let ranks = [
            celeste_core::trust_rank::TrustRank::Visitor,
            celeste_core::trust_rank::TrustRank::NewUser,
            celeste_core::trust_rank::TrustRank::User,
            celeste_core::trust_rank::TrustRank::Known,
            celeste_core::trust_rank::TrustRank::Trusted,
        ];
        player.pronouns = pronouns[seed % pronouns.len()].map(str::to_string);
        player.account_age = Some(ages[seed % ages.len()]);
        player.trust_rank = if id.ends_with("host") {
            rank
        } else {
            ranks[seed % ranks.len()]
        };
        player.age_verification_status = match seed % 4 {
            0 | 1 => Some("18+".to_string()),
            2 => Some("hidden".to_string()),
            _ => None,
        };
        let (avatar, performance) = avatars[seed % avatars.len()];
        player.avatar_name = Some(avatar.to_string());
        player.avatar_info = Some(celeste_core::ui::AvatarBundleInfo {
            performance_rating: Some(performance.to_string()),
            ..Default::default()
        });
        self.state.player_join(false, player);
    }

    fn join_nuisance_rank(&mut self) {
        let id = "usr_tutorial_nuisance_rank";
        let mut player = celeste_core::ui::PlayerJoin::new(
            id,
            "Example Nuisance-Rank User",
            chrono::Local::now().naive_local(),
        );
        player.pronouns = Some("they/she".to_string());
        player.account_age = Some((3, 9, 17));
        player.trust_rank = celeste_core::trust_rank::TrustRank::Nuisance;
        player.avatar_name = Some("Glitched Tutorial Avatar".to_string());
        player.avatar_info = Some(celeste_core::ui::AvatarBundleInfo {
            performance_rating: Some("Poor".to_string()),
            ..Default::default()
        });
        player.advisory = Some("DEMO: VRChat Nuisance trust rank".to_string());
        player.text_color = celeste_core::trust_rank::TrustRank::Nuisance.color_rgb();
        player.priority = 1000;
        self.state.player_join(false, player);
        self.pending_scroll_target = Some(id.to_string());
    }

    #[allow(clippy::too_many_arguments)]
    fn join_watched_group(
        &self,
        id: &str,
        name: &str,
        group_id: &str,
        group_name: &str,
        kind: &str,
        advisory: &str,
        critical: bool,
        priority: i32,
        color: (u8, u8, u8),
    ) {
        let mut player =
            celeste_core::ui::PlayerJoin::new(id, name, chrono::Local::now().naive_local());
        let (pronouns, age, rank, avatar, performance) = if critical {
            (
                "they/them",
                (7, 2, 18),
                celeste_core::trust_rank::TrustRank::Trusted,
                "Critical Demo Avatar",
                "VeryPoor",
            )
        } else if kind == "COMMUNITY" {
            (
                "she/her",
                (5, 6, 2),
                celeste_core::trust_rank::TrustRank::Known,
                "Community Helper Avatar",
                "Excellent",
            )
        } else {
            (
                "he/they",
                (2, 0, 27),
                celeste_core::trust_rank::TrustRank::User,
                "General Watchlist Avatar",
                "Medium",
            )
        };
        player.pronouns = Some(pronouns.to_string());
        player.account_age = Some(age);
        player.trust_rank = rank;
        player.avatar_name = Some(avatar.to_string());
        player.avatar_info = Some(celeste_core::ui::AvatarBundleInfo {
            performance_rating: Some(performance.to_string()),
            ..Default::default()
        });
        player.advisory = Some(advisory.to_string());
        player.text_color = Some(color);
        player.priority = priority;
        player.watched_memberships = vec![celeste_core::watched_membership::WatchedMembership {
            group_id: group_id.to_string(),
            kind: kind.to_string(),
            current_name: Some(group_name.to_string()),
            message: Some("Temporary guided-tour example".to_string()),
            hidden: false,
            critical,
            silent: false,
        }];
        self.state.player_join(false, player);
    }

    fn join_mixed_character_name(&mut self) {
        // The second `a` is Cyrillic U+0430. This is intentionally synthetic
        // and demonstrates the same mixed-script impersonation check as a
        // real local-log join without naming a real VRChat account.
        let mut player = celeste_core::ui::PlayerJoin::new(
            "usr_tutorial_mixed_name",
            "Exаmple Regular",
            chrono::Local::now().naive_local(),
        );
        player.account_age = Some((3, 1, 9));
        player.pronouns = Some("she/they".to_string());
        player.trust_rank = celeste_core::trust_rank::TrustRank::User;
        player.avatar_name = Some("Lookalike Demo Avatar".to_string());
        player.avatar_info = Some(celeste_core::ui::AvatarBundleInfo {
            performance_rating: Some("Good".to_string()),
            ..Default::default()
        });
        player.advisory = Some("DEMO: display name contains mixed character scripts".to_string());
        player.text_color = Some((255, 208, 96));
        player.priority = 70;
        self.state.player_join(false, player);
        self.pending_scroll_target = Some("usr_tutorial_mixed_name".to_string());
    }

    /// Advance one beat of the current tutorial scenario. Delays are relative
    /// to the preceding beat rather than to the start of the page, so a long
    /// callout cannot make the remaining rows rush in all at once.
    fn tick_roster(&mut self, mode: TutorialRosterMode, scene: usize) -> Option<TutorialRosterCue> {
        if self.roster_scene != Some(scene) {
            self.roster_scene = Some(scene);
            self.roster_mode = None;
        }
        self.set_roster_mode(mode);
        let elapsed = self.roster_started.elapsed().as_secs_f32();
        let delays: &[f32] = match mode {
            TutorialRosterMode::Lightweight => &[2.0, 3.0, 3.0, 3.0],
            TutorialRosterMode::Busy => &[2.0, 3.0, 3.5, 3.0],
        };
        if self.roster_event < delays.len() && elapsed >= delays[self.roster_event] {
            let event = self.roster_event;
            self.roster_event += 1;
            self.roster_started = std::time::Instant::now();
            match (mode, event) {
                (TutorialRosterMode::Lightweight, 0) => {
                    self.join_watched_group(
                        "usr_tutorial_mod",
                        "Example Community Moderator",
                        "grp_tutorial_moderators",
                        "DEMO Community Moderators",
                        "COMMUNITY",
                        "DEMO: recognized community moderator",
                        false,
                        35,
                        (86, 178, 120),
                    );
                    self.pending_scroll_target = Some("usr_tutorial_mod".to_string());
                    return Some(TutorialRosterCue::Speech(
                        "Community moderator joined from Demo Community Moderators.".to_string(),
                    ));
                }
                (TutorialRosterMode::Lightweight, 1) => {
                    self.join_watched_group(
                        "usr_tutorial_watched_group",
                        "Example Watched-Group Member",
                        "grp_tutorial_watched",
                        "DEMO General Watchlist",
                        "OTHER",
                        "DEMO: member of an ordinary watched group",
                        false,
                        60,
                        (220, 177, 78),
                    );
                    self.pending_scroll_target = Some("usr_tutorial_watched_group".to_string());
                    return Some(TutorialRosterCue::Speech(
                        "Watched-group member joined from Demo General Watchlist.".to_string(),
                    ));
                }
                (TutorialRosterMode::Lightweight, 2) => {
                    self.join_mixed_character_name();
                    return Some(TutorialRosterCue::MixedCharacterAlert(
                        "Warning. A user joined with a display name containing mixed character scripts."
                            .to_string(),
                    ));
                }
                (TutorialRosterMode::Lightweight, 3) => self.depart_one_ambient(),
                (TutorialRosterMode::Busy, 0) => {
                    self.join_nuisance_rank();
                    return Some(TutorialRosterCue::NuisanceAlert(
                        "Warning. A user with the VRChat Nuisance trust rank joined the busy tutorial instance."
                            .to_string(),
                    ));
                }
                (TutorialRosterMode::Busy, 1) => {
                    self.join_watched_group(
                        "usr_tutorial_critical",
                        "Example Critical Group Member",
                        "grp_tutorial_critical",
                        "DEMO Critical Crasher Watchlist",
                        "MALICIOUS",
                        "DEMO: critical crasher-group match",
                        true,
                        150,
                        (214, 73, 80),
                    );
                    self.pending_scroll_target = Some("usr_tutorial_critical".to_string());
                    return Some(TutorialRosterCue::Speech(
                        "Critical warning. Crasher watchlist member joined the busy tutorial instance."
                            .to_string(),
                    ));
                }
                (TutorialRosterMode::Busy, 2) => {
                    let mut player = celeste_core::ui::PlayerJoin::new(
                        "usr_tutorial_risky_new",
                        "Example New Risky Account",
                        chrono::Local::now().naive_local(),
                    );
                    player.account_age = Some((0, 0, 1));
                    player.pronouns = Some("any/all".to_string());
                    player.trust_rank = celeste_core::trust_rank::TrustRank::Visitor;
                    player.advisory =
                        Some("DEMO: one-day-old account and Very Poor avatar".to_string());
                    player.text_color = Some((255, 208, 96));
                    player.priority = 65;
                    player.avatar_name = Some("Very Poor Tutorial Avatar".to_string());
                    player.avatar_info = Some(celeste_core::ui::AvatarBundleInfo {
                        performance_rating: Some("VeryPoor".to_string()),
                        ..Default::default()
                    });
                    self.state.player_join(false, player);
                    self.pending_scroll_target = Some("usr_tutorial_risky_new".to_string());
                    return Some(TutorialRosterCue::Speech(
                        "New-account warning in the busy tutorial instance.".to_string(),
                    ));
                }
                (TutorialRosterMode::Busy, 3) => self.depart_one_ambient(),
                _ => {}
            }
        }
        None
    }

    fn roster_complete(&self, mode: TutorialRosterMode) -> bool {
        self.roster_mode == Some(mode)
            && self.roster_event
                >= match mode {
                    TutorialRosterMode::Lightweight => 4,
                    TutorialRosterMode::Busy => 4,
                }
    }
}

impl Drop for TutorialDemo {
    fn drop(&mut self) {
        // `directory` is a uniquely generated child of the OS temp folder;
        // it never points at CELESTE_HOME or at the user's chosen data path.
        if self
            .directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("celeste-guided-tour-"))
        {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

const TUTORIAL_STEPS: [TutorialStep; 17] = [
    TutorialStep {
        chapter: TutorialChapter::Basics,
        tab: Tab::Instance,
        target: TutorialTarget::Header,
        settings_category: None,
        demo_data: false,
        title: "Your Celeste setup",
        body: "Celeste can run as a local VRChat moderation logger, add VRChat API features, connect a Discord bot, or use everything together. The summary below reflects what is configured now; anything skipped during setup can be added later without losing existing data.",
        try_it: "The tour never sends commands, changes Discord, or moderates anyone. You can exit now and replay it later from Help.",
    },
    TutorialStep {
        chapter: TutorialChapter::Basics,
        tab: Tab::Instance,
        target: TutorialTarget::TabBar,
        settings_category: None,
        demo_data: false,
        title: "Finding your way around",
        body: "Instance is the live roster. Clients manages long-running VRChat coverage. Settings controls every available feature. CLI is a local maintenance console. The toolbar holds common actions, while the bottom bar reports the current local session.",
        try_it: "The tour selects each area automatically. You can use these tabs normally after the tour ends.",
    },
    TutorialStep {
        chapter: TutorialChapter::Basics,
        tab: Tab::Instance,
        target: TutorialTarget::Header,
        settings_category: None,
        demo_data: false,
        title: "Three separate connections",
        body: "Local log monitoring, VRChat API sign-in, and Discord are independent. The roster can work from VRChat's local logs without an API login or Discord. Profile, group, instance-creation, invite, ban, and audit features require the appropriate VRChat sign-in and group permissions. Discord posting, commands, and Discord voice require a connected bot.",
        try_it: "The badge reports VRChat API sign-in—not whether local log monitoring is working. Celeste explains missing requirements when an action cannot run.",
    },
    TutorialStep {
        chapter: TutorialChapter::InstanceAndRoster,
        tab: Tab::Instance,
        target: TutorialTarget::InstanceCard,
        settings_category: None,
        demo_data: true,
        title: "How Celeste detects an instance",
        body: "Celeste follows VRChat's local output log to learn the world, exact instance location, whether it belongs to the configured group, when it was joined, and how many players are present. Detection can take a moment immediately after VRChat joins or changes worlds.",
        try_it: "This highlighted instance and every player shown during the next lessons are clearly marked tutorial data and are never added to your real history.",
    },
    TutorialStep {
        chapter: TutorialChapter::InstanceAndRoster,
        tab: Tab::Instance,
        target: TutorialTarget::PlayerTable,
        settings_category: None,
        demo_data: true,
        title: "Reading the player roster",
        body: "This lightweight community-instance simulation starts with its host, fills naturally, and settles around 10–12 people as ordinary players continue to arrive and leave. Rows combine display name, user ID, pronouns, avatar and performance details, account age, join/leave times, age verification, trust rank, groups, and advisories. Present players lead the list; departed players remain dimmed so the session history stays useful.",
        try_it: "Watch a community moderator, an ordinary watched-group member, and a synthetic mixed-character name arrive without raid-like pacing. Their normal TTS callouts and the mixed-character warning sound play locally. None are real VRChat accounts or groups.",
    },
    TutorialStep {
        chapter: TutorialChapter::InstanceAndRoster,
        tab: Tab::Instance,
        target: TutorialTarget::PlayerTable,
        settings_category: None,
        demo_data: true,
        title: "Advisories, columns, and actions",
        body: "The tour now changes to a busy instance and gradually introduces a user carrying VRChat's Nuisance trust rank, a critical watched-group member, and a risky new account. Advisories can also come from watched users or avatars, suspicious names, pronoun validation, account age, and avatar performance. Click a sortable heading to order rows, drag headings to reorder columns, drag an edge to resize, and right-click a heading to hide or restore columns.",
        try_it: "Compare informational community rows with the Nuisance trust-rank warning and the red critical watched-group warning as they arrive. Open Profile, Ban, Unban, Invite, and similar actions require VRChat API sign-in and suitable group permissions, so external actions remain disabled on these fake rows.",
    },
    TutorialStep {
        chapter: TutorialChapter::InstanceAndRoster,
        tab: Tab::Instance,
        target: TutorialTarget::ToolbarControls,
        settings_category: None,
        demo_data: false,
        title: "Creating instances and importing groups",
        body: "Create instance accepts a world ID or URL and lets you choose access, region, queue, age verification, and an optional name. Launching or managing the result requires VRChat sign-in and the relevant group permissions. Import groups adds watched-group data from a supported file or URL; it is not the same as importing an old Scarlet installation.",
        try_it: "Tools opens the data folder, synchronizes Discord commands when a bot is configured, and clears only the current on-screen instance roster after confirmation.",
    },
    TutorialStep {
        chapter: TutorialChapter::VoiceAndSettings,
        tab: Tab::Instance,
        target: TutorialTarget::ToolbarControls,
        settings_category: None,
        demo_data: false,
        title: "Voice and notification controls",
        body: "Voice: On pauses or resumes callouts, and Skip voice stops the current announcement. TTS can use this computer's default audio device or Discord voice. Discord voice requires a configured bot and voice channel; local output does not. Notification settings decide which events become callouts, desktop notices, Discord posts, or roster advisories.",
        try_it: "Settings → Notifications can test TTS and open the operating system's sound controls if Celeste uses the wrong device.",
    },
    TutorialStep {
        chapter: TutorialChapter::VoiceAndSettings,
        tab: Tab::Settings,
        target: TutorialTarget::SettingsSidebar,
        settings_category: Some(SettingCategory::General),
        demo_data: false,
        title: "Finding and saving settings",
        body: "Categories organize the available settings and show how many controls they contain. Search looks across every category. Changes save as you make them; unavailable subsystems are hidden. Hover a setting's explanation to see the internal ID used by Discord's settings command.",
        try_it: "Leaving a category safely commits a value you were editing. Actions such as imports or credential resets remain explicit buttons rather than automatic settings.",
    },
    TutorialStep {
        chapter: TutorialChapter::VoiceAndSettings,
        tab: Tab::Settings,
        target: TutorialTarget::SettingsContent,
        settings_category: Some(SettingCategory::General),
        demo_data: false,
        title: "Make Celeste feel like yours",
        body: "General includes UI scale, accent colour, accent-coloured section headings, interface language, departed-player dimming, confirmation preferences, and CLI visibility. Appearance changes do not alter moderation decisions or stored moderation data.",
        try_it: "Choose accent colour and Set UI scale are in the General action card. You can change them again at any time.",
    },
    TutorialStep {
        chapter: TutorialChapter::HelpAndTroubleshooting,
        tab: Tab::Instance,
        target: TutorialTarget::StatusBar,
        settings_category: None,
        demo_data: false,
        title: "A quick troubleshooting order",
        body: "First check whether the bottom bar found the local instance. Then check the VRChat API badge, Discord connectivity if that feature is in use, and TTS output. A missing local instance usually concerns the VRChat log; a failed API action concerns sign-in or permissions; missing Discord output concerns the bot, channel, or routing settings.",
        try_it: "Give a fresh instance a moment to register. If it remains wrong, collect Diagnostics before restarting so the useful evidence is not lost.",
    },
    TutorialStep {
        chapter: TutorialChapter::HelpAndTroubleshooting,
        tab: Tab::Instance,
        target: TutorialTarget::Help,
        settings_category: None,
        demo_data: false,
        title: "Help and diagnostics",
        body: "Help can replay this tour, generate a diagnostic report, and open project, policy, and credit links. Diagnostics is the best starting point for a bug report because it captures the relevant paths and subsystem state without exposing stored passwords or tokens.",
        try_it: "Use Help → Diagnostics and Copy report when something goes wrong. The tour's temporary files are removed when you finish or exit.",
    },
    TutorialStep {
        chapter: TutorialChapter::DiscordModeration,
        tab: Tab::Settings,
        target: TutorialTarget::SettingsContent,
        settings_category: Some(SettingCategory::Moderation),
        demo_data: false,
        title: "Moderation, advisories, and Discord",
        body: "Moderation and Notifications control evidence, warnings, watched-list behavior, and which events become alerts. Discord controls bot behavior, permission delegation, event routing, voice channels, moderation tags, staff lists, and outstanding actions. Discord commands themselves must be tested inside your server and require the bot plus appropriate Discord or delegated Celeste permissions.",
        try_it: "Start conservatively: enable only the notices your staff needs, then expand the notification matrix after confirming where each event is routed.",
    },
    TutorialStep {
        chapter: TutorialChapter::VrChatIntegrations,
        tab: Tab::Settings,
        target: TutorialTarget::SettingsContent,
        settings_category: Some(SettingCategory::Integrations),
        demo_data: false,
        title: "VRChat and external integrations",
        body: "Integrations covers VRChat polling, avatar searches, instance following, and provider management. VRChat credentials and cached sessions are encrypted; 2FA is requested again when VRChat requires it. Group actions also need a group ID and sufficient permissions. Avatar search uses the configured providers and does not rely on World Balancer.",
        try_it: "System can reopen VRChat setup or clear credentials and the cached session if authentication becomes stuck. Clearing credentials means signing in again.",
    },
    TutorialStep {
        chapter: TutorialChapter::ManagedClients,
        tab: Tab::Clients,
        target: TutorialTarget::MainContent,
        settings_category: None,
        demo_data: false,
        title: "Managed VRChat clients",
        body: "Clients is an advanced feature for long-running moderation coverage. A slot maps one numbered Steam/VRChat profile to an expected identity and exact instance. Celeste can detect a stopped process, restart that profile through the normal Steam/EAC path, and rejoin its assigned location without storing that account's password.",
        try_it: "Ordinary single-client users can leave this empty. Multi-client operators should configure and verify one slot at a time before enabling automatic restart or replacement-instance creation.",
    },
    TutorialStep {
        chapter: TutorialChapter::DataAndRecovery,
        tab: Tab::Settings,
        target: TutorialTarget::SettingsContent,
        settings_category: Some(SettingCategory::System),
        demo_data: false,
        title: "Data, migration, cleanup, and recovery",
        body: "System shows Celeste's data folder and can open or move it, import legacy Scarlet data, clean old caches and logs, restart Celeste, rerun VRChat setup, or reset a broken VRChat session. On Linux it can also install or refresh Celeste in the desktop's application menu. Scarlet import is additive: existing Celeste records win, credentials and settings are skipped, and a backup is created so you can verify the result.",
        try_it: "Pronoun good/bad lists are managed from General and reloaded after editing. Cache cleanup protects active files and does not delete durable moderation lists or history. Linux users can rerun the launcher install after downloading a newer build.",
    },
    TutorialStep {
        chapter: TutorialChapter::DataAndRecovery,
        tab: Tab::Cli,
        target: TutorialTarget::MainContent,
        settings_category: None,
        demo_data: false,
        title: "Local CLI and action safety",
        body: "The CLI is Celeste's local maintenance console—not Discord chat. `help` lists commands for diagnostics, TTS tests, data-folder access, imports, ID linking, and restart. Read-only actions are immediate; destructive or moderation actions use confirmations where practical and still require the relevant connection and permissions.",
        try_it: "Try `help` safely. You can hide the CLI later from View → Show CLI tab without disabling any background moderation feature.",
    },
];

/// One column of the Instance tab's player table, matching `propstable`'s
/// `addProperty` call order in `ScarletUI.java`. Kept as an enum (rather
/// than just an index) so [`CelesteApp::column_order`] can be reordered by
/// dragging headers without the render code needing to track which index
/// currently means what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ColumnId {
    Name,
    Id,
    Pronouns,
    Avatar,
    Performance,
    AvatarStats,
    WatchedGroups,
    AccAge,
    Joined,
    Left,
    Advisory,
    Age18,
    Rank,
    Profile,
    CopyId,
    Ban,
    Unban,
    Invite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableSort {
    column: ColumnId,
    ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupAction {
    Ban,
    Unban,
    Invite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceKind {
    GroupPublic,
    GroupPlus,
    GroupMembers,
    Public,
    FriendsPlus,
    Friends,
    InvitePlus,
    Invite,
}

impl InstanceKind {
    const ALL: [Self; 8] = [
        Self::GroupPublic,
        Self::GroupPlus,
        Self::GroupMembers,
        Self::Public,
        Self::FriendsPlus,
        Self::Friends,
        Self::InvitePlus,
        Self::Invite,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::GroupPublic => "Group Public",
            Self::GroupPlus => "Group Plus",
            Self::GroupMembers => "Group Members",
            Self::Public => "Public",
            Self::FriendsPlus => "Friends+",
            Self::Friends => "Friends",
            Self::InvitePlus => "Invite+",
            Self::Invite => "Invite",
        }
    }
    fn group(self) -> bool {
        matches!(
            self,
            Self::GroupPublic | Self::GroupPlus | Self::GroupMembers
        )
    }
    fn group_access(self) -> Option<&'static str> {
        match self {
            Self::GroupPublic => Some("public"),
            Self::GroupPlus => Some("plus"),
            Self::GroupMembers => Some("members"),
            _ => None,
        }
    }
    fn instance_type(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::FriendsPlus => "hidden",
            Self::Friends => "friends",
            Self::InvitePlus | Self::Invite => "private",
            _ => "group",
        }
    }
}

#[derive(Debug, Clone)]
struct CreateInstanceForm {
    world: String,
    kind: InstanceKind,
    region: String,
    avatar_gate: String,
    display_name: String,
    queue_enabled: bool,
    age_gate: bool,
    content_drones: bool,
    content_emoji: bool,
    content_props: bool,
    content_pedestals: bool,
    content_prints: bool,
    content_stickers: bool,
}

impl Default for CreateInstanceForm {
    fn default() -> Self {
        Self {
            world: String::new(),
            kind: InstanceKind::GroupPublic,
            region: "us".into(),
            avatar_gate: String::new(),
            display_name: String::new(),
            queue_enabled: true,
            age_gate: false,
            content_drones: true,
            content_emoji: true,
            content_props: true,
            content_pedestals: true,
            content_prints: true,
            content_stickers: true,
        }
    }
}

#[derive(Debug, Clone)]
struct CreatedInstance {
    message: String,
    link: String,
}

fn resolve_world_id(input: &str) -> Option<String> {
    let start = input.find("wrld_")?;
    let candidate = input.get(start..start + 41)?;
    let uuid = candidate.strip_prefix("wrld_")?;
    (uuid.len() == 36
        && uuid.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        }))
    .then(|| candidate.to_string())
}

fn decode_url_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        output.push(if bytes[index] == b'+' {
            b' '
        } else {
            bytes[index]
        });
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn resolve_instance_location(input: &str) -> Option<String> {
    let input = input.trim();
    if celeste_vrchat::managed_clients::split_location(input).is_some() {
        return Some(input.to_string());
    }
    let world = resolve_world_id(input)?;
    let instance_start = input.find("instanceId=")? + "instanceId=".len();
    let encoded = &input[instance_start..];
    let encoded = encoded.split(['&', '#']).next()?;
    let instance = decode_url_component(encoded);
    (!instance.trim().is_empty()).then(|| format!("{world}:{instance}"))
}

/// Ask for either a live Scarlet folder or a portable archive. Archive data
/// is extracted to Celeste-owned staging immediately so the normal overlap
/// review can use the exact same import path as a folder.
fn pick_legacy_import_source(
    data_dir: &std::path::Path,
) -> Result<Option<(std::path::PathBuf, Option<std::path::PathBuf>)>, String> {
    let choice = crate::dialogs::message(
        "Celeste - choose Scarlet data",
        "Choose how to locate the legacy Scarlet data. Archives may be ZIP, 7Z, TAR, TAR.GZ, TAR.BZ2, or TAR.XZ.",
        rfd::MessageButtons::YesNoCancelCustom(
            "Choose folder".to_string(),
            "Choose archive".to_string(),
            "Cancel".to_string(),
        ),
        rfd::MessageLevel::Info,
    )?;
    let parent = Some(data_dir.parent().unwrap_or(data_dir));
    match choice {
        rfd::MessageDialogResult::Custom(label) if label.contains("folder") => {
            crate::dialogs::pick_folder("Choose the Scarlet data folder", parent)
                .map(|path| path.map(|path| (path, None)))
        }
        rfd::MessageDialogResult::Custom(label) if label.contains("archive") => {
            let Some(archive) =
                crate::dialogs::pick_archive("Choose a Scarlet data archive", parent)?
            else {
                return Ok(None);
            };
            let staged = celeste_core::legacy_archive::prepare(&archive, data_dir)
                .map_err(|error| format!("could not read {}: {error}", archive.display()))?;
            Ok(Some((staged, Some(archive))))
        }
        // Native backends are inconsistent about whether custom labels come
        // back as Custom/Yes/No. Preserve the visual left-to-right mapping.
        rfd::MessageDialogResult::Yes => {
            crate::dialogs::pick_folder("Choose the Scarlet data folder", parent)
                .map(|path| path.map(|path| (path, None)))
        }
        rfd::MessageDialogResult::No => {
            let Some(archive) =
                crate::dialogs::pick_archive("Choose a Scarlet data archive", parent)?
            else {
                return Ok(None);
            };
            let staged = celeste_core::legacy_archive::prepare(&archive, data_dir)
                .map_err(|error| format!("could not read {}: {error}", archive.display()))?;
            Ok(Some((staged, Some(archive))))
        }
        _ => Ok(None),
    }
}

fn validate_instance_display_name(name: &str) -> Result<Option<String>, String> {
    let name = name.trim();
    if name.len() > 32 || name.contains(['.', '/', '\\']) {
        return Err("Instance names can be blank or up to 32 characters, and cannot contain '.', '/', or '\\'.".into());
    }
    Ok((!name.is_empty()).then(|| name.to_string()))
}

fn create_instance_request(
    form: &CreateInstanceForm,
    world_id: &str,
    owner_id: &str,
) -> Result<serde_json::Value, String> {
    let mut request = serde_json::json!({
        "worldId": world_id, "ownerId": owner_id, "type": form.kind.instance_type(), "region": form.region,
        "queueEnabled": form.queue_enabled, "hardClose": false, "ageGate": form.age_gate,
        "contentSettings": { "drones": form.content_drones, "emoji": form.content_emoji, "props": form.content_props, "pedestals": form.content_pedestals, "prints": form.content_prints, "stickers": form.content_stickers }
    });
    let object = request
        .as_object_mut()
        .expect("instance request is an object");
    if let Some(access) = form.kind.group_access() {
        object.insert("groupAccessType".into(), serde_json::json!(access));
    }
    if form.kind == InstanceKind::InvitePlus {
        object.insert("canRequestInvite".into(), serde_json::json!(true));
    }
    if let Some(name) = validate_instance_display_name(&form.display_name)? {
        object.insert("displayName".into(), serde_json::json!(name));
    }
    if !form.avatar_gate.is_empty() {
        object.insert(
            "minimumAvatarPerformance".into(),
            serde_json::json!(form.avatar_gate),
        );
    }
    Ok(request)
}

fn canonicalize_cli_command(op: &str) -> String {
    let low = op.trim().to_ascii_lowercase();
    const CANONICAL: [&str; 26] = [
        "info",
        "help",
        "check",
        "logout",
        "exit",
        "halt",
        "quit",
        "stop",
        "reboot",
        "restart",
        "langlint",
        "simulate",
        "explore",
        "tts",
        "link",
        "importgroups",
        "importgroupsjson",
        "translate-advisories",
        "restore-advisories",
        "repair-secure-store",
        "vrchatapi-test",
        "popup",
        "popups",
        "popup-test",
        "clear",
        "",
    ];
    if CANONICAL.contains(&low.as_str()) {
        return low;
    }
    for (canonical, key) in [
        ("help", "cli.alias.help"),
        ("logout", "cli.alias.logout"),
        ("exit", "cli.alias.exit"),
        ("reboot", "cli.alias.reboot"),
        ("langlint", "cli.alias.langlint"),
        ("simulate", "cli.alias.simulate"),
        ("explore", "cli.alias.explore"),
        ("tts", "cli.alias.tts"),
        ("link", "cli.alias.link"),
        ("importgroups", "cli.alias.importgroups"),
        ("importgroupsjson", "cli.alias.importgroupsjson"),
    ] {
        let aliases = celeste_core::i18n::tr(key, &[]);
        if aliases != key
            && aliases
                .split(|c: char| c == ',' || c.is_whitespace())
                .any(|alias| alias.eq_ignore_ascii_case(&low))
        {
            return canonical.into();
        }
    }
    low
}

type PendingGroupFileImport = Result<Option<(String, Vec<u8>, String)>, String>;

fn load_column_layout(
    settings: Option<&Settings>,
) -> (Vec<ColumnId>, std::collections::HashMap<ColumnId, f32>) {
    let mut widths = ColumnId::ALL
        .into_iter()
        .map(|column| (column, column.initial_width()))
        .collect::<std::collections::HashMap<_, _>>();
    let Some(saved) = settings.and_then(|settings| {
        settings
            .store
            .get_object::<std::collections::HashMap<String, UiPropsInfo>>("ui_instance_columns")
    }) else {
        return (ColumnId::DEFAULT_VISIBLE.to_vec(), widths);
    };
    if saved.is_empty() {
        return (ColumnId::DEFAULT_VISIBLE.to_vec(), widths);
    }

    let mut visible = saved
        .iter()
        .filter_map(|(id, info)| {
            let column = ColumnId::from_id(id)?;
            if info.width > 0 {
                widths.insert(column, info.width as f32);
            }
            (info.index >= 0).then_some((info.index, column))
        })
        .collect::<Vec<_>>();
    visible.sort_by_key(|(index, _)| *index);
    let mut order = visible
        .into_iter()
        .map(|(_, column)| column)
        .collect::<Vec<_>>();
    // A layout written by an older version has no entries for newly added
    // default-visible columns. Java leaves those columns at their defaults.
    for column in ColumnId::DEFAULT_VISIBLE {
        if !saved.contains_key(column.id()) && !order.contains(&column) {
            order.push(column);
        }
    }
    if order.is_empty() {
        order.push(ColumnId::Name);
    }
    (order, widths)
}

impl ColumnId {
    const ALL: [ColumnId; 18] = [
        ColumnId::Name,
        ColumnId::Id,
        ColumnId::Pronouns,
        ColumnId::Avatar,
        ColumnId::Performance,
        ColumnId::AvatarStats,
        ColumnId::AccAge,
        ColumnId::Joined,
        ColumnId::Left,
        ColumnId::Advisory,
        ColumnId::WatchedGroups,
        ColumnId::Age18,
        ColumnId::Rank,
        ColumnId::Profile,
        ColumnId::CopyId,
        ColumnId::Ban,
        ColumnId::Unban,
        ColumnId::Invite,
    ];

    const DEFAULT_VISIBLE: [ColumnId; 16] = [
        ColumnId::Name,
        ColumnId::Id,
        ColumnId::Pronouns,
        ColumnId::Avatar,
        ColumnId::Performance,
        ColumnId::AvatarStats,
        ColumnId::AccAge,
        ColumnId::Joined,
        ColumnId::Left,
        ColumnId::Advisory,
        ColumnId::WatchedGroups,
        ColumnId::Age18,
        ColumnId::Rank,
        ColumnId::Profile,
        ColumnId::CopyId,
        ColumnId::Ban,
    ];

    fn label(&self) -> &'static str {
        match self {
            ColumnId::Name => "Name",
            ColumnId::Id => "Id",
            ColumnId::Pronouns => "Pronouns",
            ColumnId::Avatar => "Avatar",
            ColumnId::Performance => "Performance",
            ColumnId::AvatarStats => "Avatar Stats",
            ColumnId::WatchedGroups => "Groups",
            ColumnId::AccAge => "Acc-Age",
            ColumnId::Joined => "Joined",
            ColumnId::Left => "Left",
            ColumnId::Advisory => "Advisory",
            ColumnId::Age18 => "18+",
            ColumnId::Rank => "Rank",
            ColumnId::Profile => "Profile",
            ColumnId::CopyId => "Copy Id",
            ColumnId::Ban => "Ban",
            ColumnId::Unban => "Unban",
            ColumnId::Invite => "Invite",
        }
    }

    fn id(self) -> &'static str {
        self.label()
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.id() == id)
    }

    /// Starting column width. These are sized so the value Java shows in
    /// each column fits on one line at the default font — a full
    /// `usr_`-prefixed id, a `2026-09-27 19:29` timestamp, an `8y 8m 8d`
    /// account age — because a column narrower than its content is what
    /// turned every row into a stacked block. Columns stay resizable, so
    /// these are a starting point rather than a cap.
    fn initial_width(&self) -> f32 {
        match self {
            ColumnId::Name => 130.0,
            ColumnId::Id => 300.0,
            ColumnId::Pronouns => 80.0,
            ColumnId::Avatar => 160.0,
            ColumnId::Performance => 90.0,
            ColumnId::AvatarStats => 60.0,
            ColumnId::WatchedGroups => 86.0,
            ColumnId::AccAge => 90.0,
            ColumnId::Joined => 125.0,
            ColumnId::Left => 125.0,
            ColumnId::Advisory => 160.0,
            ColumnId::Age18 => 55.0,
            ColumnId::Rank => 80.0,
            ColumnId::Profile => 55.0,
            ColumnId::CopyId => 55.0,
            ColumnId::Ban | ColumnId::Unban | ColumnId::Invite => 55.0,
        }
    }

    /// How far a column can be dragged in before its header stops being
    /// readable.
    fn min_width(&self) -> f32 {
        match self {
            ColumnId::Age18
            | ColumnId::Ban
            | ColumnId::Unban
            | ColumnId::Invite
            | ColumnId::Profile
            | ColumnId::CopyId
            | ColumnId::AvatarStats
            | ColumnId::WatchedGroups => 40.0,
            _ => 55.0,
        }
    }

    /// Smallest useful on-screen width when the roster has to fit inside a
    /// narrow window. This is deliberately smaller than `min_width`: saved
    /// widths describe the user's preferred layout, while the rendered table
    /// is allowed to compress those preferences so it never needs a
    /// horizontal scrollbar. Labels and values truncate with an ellipsis.
    fn fitted_min_width(&self) -> f32 {
        match self {
            ColumnId::AvatarStats
            | ColumnId::Profile
            | ColumnId::CopyId
            | ColumnId::Ban
            | ColumnId::Unban
            | ColumnId::Invite => 42.0,
            ColumnId::Age18 => 30.0,
            _ => 36.0,
        }
    }

    fn sortable(self) -> bool {
        !matches!(
            self,
            ColumnId::AvatarStats
                | ColumnId::Profile
                | ColumnId::CopyId
                | ColumnId::Ban
                | ColumnId::Unban
                | ColumnId::Invite
        )
    }
}

/// Fit every visible column into `available_width` while preserving the
/// user's saved widths as relative preferences. Long values are truncated by
/// the cell renderer, so the roster stays entirely inside its panel rather
/// than introducing a second, horizontal scrolling direction.
fn fitted_column_widths(
    columns: &[ColumnId],
    preferred: &std::collections::HashMap<ColumnId, f32>,
    available_width: f32,
) -> Vec<f32> {
    if columns.is_empty() {
        return Vec::new();
    }

    let floors = columns
        .iter()
        .map(|column| column.fitted_min_width())
        .collect::<Vec<_>>();
    let floor_total = floors.iter().sum::<f32>();
    if available_width <= floor_total {
        let scale = (available_width / floor_total).max(0.0);
        return floors.into_iter().map(|width| width * scale).collect();
    }

    let flexible = columns
        .iter()
        .zip(&floors)
        .map(|(column, floor)| {
            preferred
                .get(column)
                .copied()
                .unwrap_or_else(|| column.initial_width())
                .max(*floor)
                - *floor
        })
        .collect::<Vec<_>>();
    let flexible_total = flexible.iter().sum::<f32>();
    let extra = available_width - floor_total;
    if flexible_total <= f32::EPSILON {
        let extra_per_column = extra / columns.len() as f32;
        return floors
            .into_iter()
            .map(|width| width + extra_per_column)
            .collect();
    }

    floors
        .into_iter()
        .zip(flexible)
        .map(|(floor, preference)| floor + extra * preference / flexible_total)
        .collect()
}

/// Convert a width chosen in the rendered, fitted table back into the
/// stored preference that will reproduce it. The table has a fixed total
/// width, so enlarging one column necessarily takes flexible space from the
/// others. Applying the pointer delta directly to the stored preference is
/// incorrect whenever the table is compressed and makes a drag appear to
/// snap back on the following frame.
fn preference_for_fitted_width(
    columns: &[ColumnId],
    preferred: &std::collections::HashMap<ColumnId, f32>,
    available_width: f32,
    resized: ColumnId,
    desired_fitted_width: f32,
) -> f32 {
    let floor_total = columns.iter().map(ColumnId::fitted_min_width).sum::<f32>();
    let extra = (available_width - floor_total).max(0.0);
    let floor = resized.fitted_min_width();
    if extra <= f32::EPSILON {
        return resized.min_width();
    }

    let other_flexible = columns
        .iter()
        .copied()
        .filter(|column| *column != resized)
        .map(|column| {
            preferred
                .get(&column)
                .copied()
                .unwrap_or_else(|| column.initial_width())
                .max(column.fitted_min_width())
                - column.fitted_min_width()
        })
        .sum::<f32>();
    if other_flexible <= f32::EPSILON {
        return preferred
            .get(&resized)
            .copied()
            .unwrap_or_else(|| resized.initial_width());
    }

    // Keep at least a sliver of flexible width for the other columns. Their
    // hard floors are already protected by `fitted_column_widths`.
    let target_flexible = (desired_fitted_width - floor).clamp(0.0, extra - 0.5);
    let resized_flexible = target_flexible * other_flexible / (extra - target_flexible);
    (floor + resized_flexible).max(resized.min_width())
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct UiPropsInfo {
    index: i32,
    width: i32,
}

/// Width of the settings column's controls, so checkboxes, drop-downs and
/// text fields line up down the page instead of each sitting wherever its
/// label ended.
const CONTROL_WIDTH: f32 = 260.0;

/// What the avatar-stats window shows, snapshotted when it opens so it
/// doesn't hold a borrow of the player table.
#[derive(Debug, Clone)]
pub struct AvatarStatsView {
    player_name: String,
    avatar_name: Option<String>,
    info: Option<celeste_core::ui::AvatarBundleInfo>,
    note: Option<String>,
    /// Pre-rendered report block, built when the window opens so the
    /// button is a clipboard write and nothing else.
    evidence: Option<String>,
}

/// Every watched group a player is in, for the panel that shows them
/// all at once.
pub struct WatchedGroupsView {
    player_name: String,
    user_id: String,
    memberships: Vec<celeste_core::watched_membership::WatchedMembership>,
}

struct PendingCacheCleanup {
    files: Vec<PathBuf>,
    bytes: u64,
    retention_days: i32,
}

#[derive(Clone)]
struct AvatarProviderChoice {
    url: String,
    enabled: bool,
}

pub struct CelesteApp {
    state: UiState,
    settings: Option<Arc<Settings>>,
    data_dir: PathBuf,
    staff_mode: bool,
    runtime_handle: tokio::runtime::Handle,
    /// The first-run setup wizard (vibe-coded notice, Discord bot token,
    /// VRChat login — see `crate::setup`). `None` once it's finished (or if
    /// there's no settings store to run it against at all), at which point
    /// `update()` renders the normal toolbar/tabs/table instead.
    wizard: Option<crate::setup::SetupWizard>,
    /// Settings-only page opened from setup's finish screen. It deliberately
    /// keeps the wizard alive and hides the normal toolbar/tabs/status UI.
    setup_settings_review: bool,
    tutorial_step: Option<usize>,
    /// Showing the chapter chooser rather than a step.
    tutorial_menu: bool,
    /// Settings asked to run the full setup wizard; handled where the
    /// wizard can be constructed.
    open_full_setup: bool,
    /// Ignore the setup profile and show every chapter ("Show anyway").
    tutorial_show_all: bool,
    tutorial_layout: TutorialLayout,
    tutorial_demo: Option<TutorialDemo>,
    /// Optional local narration for the current guided-tour session. This
    /// is deliberately independent of moderation callout pause/routing:
    /// tour speech is opt-in, always local, and never sent to Discord.
    tutorial_narration: bool,
    tutorial_voice: String,
    tutorial_narrated_step: Option<usize>,
    tutorial_scenario_phase: Option<(usize, TutorialScenarioPhase)>,
    /// The live roster example normally collapses the guide so it cannot
    /// cover the rows it is demonstrating. This opt-out is scoped to one
    /// step and is only set when the user explicitly asks to see it.
    tutorial_revealed_example_guide: Option<usize>,
    tutorial_tts_notice: Option<String>,
    resume_tutorial_step: Option<usize>,
    tutorial_setup_had_discord: bool,
    /// The authenticated VRChat client, shared with `celeste-discord`'s
    /// command handlers via its `TypeMap` (see `celeste_discord::data`).
    /// `None` until a login succeeds — from the wizard this session, or an
    /// unattended background login using already-stored credentials.
    vrchat_slot: celeste_discord::data::SharedVrChatClient,
    /// See `celeste_discord::data::SharedHttp` — shared with the event
    /// listener's audit emitter so both see the same connection.
    discord_http: celeste_discord::data::SharedHttp,
    avatar_search: Arc<celeste_discord::avatar_search::AvatarSearch>,
    discord_config: Arc<celeste_discord::data::DiscordConfigStore>,
    health_reporter: Arc<celeste_discord::health::OpsHealthReporter>,
    voice_manager: Arc<songbird::Songbird>,
    process_controller: Arc<celeste_discord::data::ProcessController>,
    /// Set when a background VRChat login stopped to ask for a two-factor
    /// code; drained in `update` to open the code prompt.
    two_factor_tx:
        std::sync::mpsc::Sender<(Arc<celeste_vrchat::client::VrChatClient>, Vec<String>)>,
    two_factor_rx:
        std::sync::mpsc::Receiver<(Arc<celeste_vrchat::client::VrChatClient>, Vec<String>)>,
    celeste_data: Arc<celeste_core::data::CelesteData>,
    staff_list: Arc<celeste_core::staff_list::StaffList>,
    secret_staff_list: Arc<celeste_core::staff_list::StaffList>,
    moderation_tags: Arc<celeste_core::moderation_tags::ModerationTags>,
    watched_groups: Arc<celeste_core::watched_groups::WatchedGroups>,
    watched_users: Arc<celeste_core::watched_entities::WatchedEntities>,
    watched_avatars: Arc<celeste_core::watched_entities::WatchedEntities>,
    pronoun_lists: Arc<celeste_core::pronoun_lists::PronounLists>,
    tts: Arc<crate::tts::TtsService>,
    tts_route: Arc<crate::tts::TtsSinkAdapter>,
    managed_clients: Arc<celeste_vrchat::managed_clients::ManagedClientStore>,
    tab: Tab,
    /// Java uses `DO_NOTHING_ON_CLOSE` and asks before quitting
    /// (`ScarletUI.java:1281-1288`, `uiModalExit` at `:1391-1401`), so the
    /// window's X can't kill a live moderation session by accident. `true`
    /// while that prompt is up; `exit_confirmed` lets the second close
    /// request through.
    exit_prompt_open: bool,
    exit_confirmed: bool,
    cli_input: String,
    settings_edit_buffers: std::collections::HashMap<String, String>,
    /// Current left-to-right order of the Instance tab's columns. Starts as
    /// `ColumnId::ALL` (matching `propstable`'s declared order) and can be
    /// changed by dragging a header, matching the original Swing
    /// `JTable`'s column-reordering behavior. NOTE: Java *does* persist the
    /// layout (`ScarletUI.java:1338-1372` writes each column's display
    /// index, width and enabled flag to `ui_instance_columns` every 10s and
    /// restores it at startup); this resets on restart. Tracked as B6 in
    /// AUDIT-2026-09-28.md — the claim that Java doesn't persist it was
    /// simply wrong.
    column_order: Vec<ColumnId>,
    column_widths: std::collections::HashMap<ColumnId, f32>,
    /// Active JTable-style header sort. `None` uses Java's default player
    /// order (present, advisory priority, join time).
    table_sort: Option<TableSort>,
    /// Last native window rectangle written to Java's `uiBounds` registry
    /// key. Keeping the previous value avoids rewriting registry.json on
    /// every immediate-mode frame when the window has not moved.
    last_window_bounds: Option<Rectangle>,
    /// Which sidebar category the Settings tab currently shows, matching
    /// the reference screenshots' left-hand category list (General/
    /// Moderation/Notifications/Discord/Integrations/System) instead of one
    /// long flat list of ~90 settings.
    settings_category: SettingCategory,
    /// The "Import Groups (URL)" prompt — Java asks for the URL with a
    /// modal input box; this is the same question as an egui window.
    settings_search: String,
    /// The player whose avatar stats are being shown (port of
    /// `ScarletUI.infoStats`), and the pending Ban confirmation (port of the
    /// row's Ban action, which Java also confirms before acting).
    avatar_stats_open: Option<AvatarStatsView>,
    watched_groups_open: Option<WatchedGroupsView>,
    group_action_confirm: Option<(GroupAction, String, String)>,
    create_instance_open: bool,
    create_instance_form: CreateInstanceForm,
    create_instance_error: Option<String>,
    create_instance_rx: Option<std::sync::mpsc::Receiver<Result<CreatedInstance, String>>>,
    created_instance: Option<CreatedInstance>,
    import_groups_url_open: bool,
    import_groups_url: String,
    import_groups_rx: Option<(String, std::sync::mpsc::Receiver<Result<String, String>>)>,
    import_groups_file_rx: Option<std::sync::mpsc::Receiver<PendingGroupFileImport>>,
    cache_cleanup_confirm: Option<PendingCacheCleanup>,
    cache_cleanup_rx: Option<std::sync::mpsc::Receiver<celeste_core::cache_cleanup::CleanupReport>>,
    cache_cleanup_notice: Option<String>,
    settings_notice: Option<String>,
    legacy_import_result: Option<(bool, String)>,
    ui_scale_open: bool,
    ui_scale_percent: f32,
    ui_scale: f32,
    accent_open: bool,
    accent: egui::Color32,
    accent_edit: [u8; 3],
    avatar_providers_open: bool,
    avatar_provider_choices: Vec<AvatarProviderChoice>,
    avatar_provider_input: String,
    avatar_provider_error: Option<String>,
    clear_instance_confirm: bool,
    diagnostics_open: bool,
    credits_open: bool,
    visual_style_installed: bool,
    /// egui ships a deliberately compact font set. System fallbacks fill in
    /// arrows used by Celeste and, on demand, scripts appearing in VRChat
    /// display names without bundling a large world font into every build.
    font_fallbacks_installed: bool,
    font_fallback_paths: Vec<PathBuf>,
    font_fallback_attempted_chars: std::collections::HashSet<char>,
    managed_client_edit: Option<celeste_vrchat::managed_clients::ManagedClientSlot>,
    managed_client_error: Option<String>,
    /// One throttled process-table snapshot shared by every enabled coverage
    /// slot. Disabled slots deliberately do not monitor the machine.
    managed_profile_pids: std::collections::HashMap<u32, Vec<u32>>,
    managed_profile_scan_at: Option<std::time::Instant>,
}

impl CelesteApp {
    /// `shared` carries the data objects every subsystem must agree on —
    /// the same `CelesteData`, watched lists and VRChat session the Discord
    /// commands and the ported event listener use. They are built once by
    /// the caller and handed in; this used to open its own copies here,
    /// which meant the GUI and the event listener would have been looking
    /// at two different in-memory views of the same files.
    pub fn new(
        state: UiState,
        shared: celeste_discord::data::SharedState,
        data_dir: PathBuf,
        runtime_handle: tokio::runtime::Handle,
        tts: Arc<crate::tts::TtsService>,
        tts_route: Arc<crate::tts::TtsSinkAdapter>,
        pronoun_lists: Arc<celeste_core::pronoun_lists::PronounLists>,
        two_factor_tx: std::sync::mpsc::Sender<(
            Arc<celeste_vrchat::client::VrChatClient>,
            Vec<String>,
        )>,
        two_factor_rx: std::sync::mpsc::Receiver<(
            Arc<celeste_vrchat::client::VrChatClient>,
            Vec<String>,
        )>,
        managed_clients: Arc<celeste_vrchat::managed_clients::ManagedClientStore>,
    ) -> Self {
        let celeste_data = shared.celeste_data.clone();
        let staff_list = shared.staff_list.clone();
        let secret_staff_list = shared.secret_staff_list.clone();
        let moderation_tags = shared.moderation_tags.clone();
        let watched_groups = shared.watched_groups.clone();
        let watched_users = shared.watched_users.clone();
        let watched_avatars = shared.watched_avatars.clone();
        let vrchat_slot = shared.vrchat_client.clone();
        let discord_http = shared.http.clone();
        let avatar_search = shared.avatar_search.clone();
        let discord_config = shared.discord_config.clone();
        let health_reporter = shared.health_reporter.clone();
        let voice_manager = shared.voice_manager.clone();
        let process_controller = shared.process_controller.clone();
        let settings = Some(shared.settings.clone());
        let last_window_bounds = settings
            .as_ref()
            .and_then(|settings| settings.ui_bounds.get_or_null());
        let ui_scale = settings
            .as_ref()
            .and_then(|settings| settings.store.get_object::<f32>("ui_scale"))
            .unwrap_or(1.0)
            .clamp(0.5, 4.0);
        let accent = settings
            .as_ref()
            .and_then(|settings| settings.store.get_object::<Vec<u8>>("ui_accent_color"))
            .filter(|rgb| rgb.len() == 3)
            .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            // Java's `Swing.DEFAULT_ACCENT` is crimson; the port had its own blue.
            .unwrap_or(crate::theme::DEFAULT_ACCENT);
        let settings_notice = tts.engine_name().is_none().then(||
            "Local text-to-speech is unavailable because no supported speech engine was found. Install eSpeak NG/eSpeak, Speech Dispatcher, or Flite on Linux; use macOS 'say'; or ensure PowerShell is available on Windows. Discord voice TTS can still work when configured.".to_string()
        );
        let legacy_import_result = celeste_core::legacy_import::take_result(&data_dir);
        if let Some((_success, message)) = &legacy_import_result {
            state.append_cli_output(message.as_str());
        }

        let wizard = settings
            .as_ref()
            .map(|s| crate::setup::SetupWizard::new(s, runtime_handle.clone()));

        // If setup already ran in an earlier session, the wizard is
        // constructed already-`done` and won't render at all -- but Discord
        // still needs to connect with whatever token is already stored,
        // which is normally the wizard's own job when it finishes *this*
        // session (see the `JustFinished` handling in `update()`).
        let staff_mode = match (&wizard, &settings) {
            (Some(wizard), Some(settings)) if wizard.is_done() => {
                let token = wizard.stored_discord_token();
                let staff_mode = token.as_deref().map(str::trim).unwrap_or("").is_empty();

                // No interactive login happened this session (the wizard
                // was already done) -- attempt an unattended login with
                // whatever VRChat credentials are stored, so Discord
                // commands still get a working client. Matches Java's own
                // unattended-re-login behavior, minus stored-TOTP-secret
                // support (see `login_with_stored_credentials`'s docs).
                spawn_unattended_vrchat_login(
                    &runtime_handle,
                    vrchat_slot.clone(),
                    settings.clone(),
                    two_factor_tx.clone(),
                );

                runtime_handle.spawn(celeste_discord::client::connect_and_serve(
                    token,
                    celeste_discord::data::SharedState {
                        vrchat_client: vrchat_slot.clone(),
                        avatar_search: avatar_search.clone(),
                        http: discord_http.clone(),
                        discord_config: discord_config.clone(),
                        health_reporter: health_reporter.clone(),
                        voice_manager: voice_manager.clone(),
                        process_controller: process_controller.clone(),
                        celeste_data: celeste_data.clone(),
                        settings: settings.clone(),
                        staff_list: staff_list.clone(),
                        secret_staff_list: secret_staff_list.clone(),
                        moderation_tags: moderation_tags.clone(),
                        watched_groups: watched_groups.clone(),
                        watched_users: watched_users.clone(),
                        watched_avatars: watched_avatars.clone(),
                    },
                ));
                staff_mode
            }
            // Unknown until the wizard finishes (or there's no settings
            // store to check at all); the window title updates once it does.
            _ => true,
        };

        let (column_order, column_widths) = load_column_layout(settings.as_deref());
        let installed_tutorial_voices = celeste_core::tts_voices::installed();
        let tutorial_voice = settings
            .as_ref()
            .and_then(|settings| settings.store.get_object::<String>("tutorial_tts_voice"))
            .or_else(|| {
                settings.as_ref().and_then(|settings| {
                    let voice = settings.tts_voice_name.get();
                    (!voice.trim().is_empty()).then_some(voice)
                })
            })
            .and_then(|voice| {
                celeste_core::tts_voices::resolve_legacy(&voice, &installed_tutorial_voices)
            })
            .unwrap_or_default();

        CelesteApp {
            state,
            settings,
            data_dir,
            staff_mode,
            runtime_handle,
            wizard,
            setup_settings_review: false,
            tutorial_step: None,
            tutorial_menu: false,
            open_full_setup: false,
            tutorial_show_all: false,
            tutorial_layout: TutorialLayout::default(),
            tutorial_demo: None,
            tutorial_narration: false,
            tutorial_voice,
            tutorial_narrated_step: None,
            tutorial_scenario_phase: None,
            tutorial_revealed_example_guide: None,
            tutorial_tts_notice: None,
            resume_tutorial_step: None,
            tutorial_setup_had_discord: false,
            vrchat_slot,
            avatar_search,
            discord_http,
            discord_config,
            health_reporter,
            voice_manager,
            process_controller,
            two_factor_tx,
            two_factor_rx,
            celeste_data,
            staff_list,
            secret_staff_list,
            moderation_tags,
            watched_groups,
            watched_users,
            watched_avatars,
            pronoun_lists,
            tts,
            tts_route,
            managed_clients,
            tab: Tab::Instance,
            exit_prompt_open: false,
            exit_confirmed: false,
            cli_input: String::new(),
            settings_edit_buffers: std::collections::HashMap::new(),
            column_order,
            column_widths,
            table_sort: None,
            last_window_bounds,
            settings_category: SettingCategory::General,
            settings_search: String::new(),
            avatar_stats_open: None,
            watched_groups_open: None,
            group_action_confirm: None,
            create_instance_open: false,
            create_instance_form: CreateInstanceForm::default(),
            create_instance_error: None,
            create_instance_rx: None,
            created_instance: None,
            import_groups_url_open: false,
            import_groups_url: String::new(),
            import_groups_rx: None,
            import_groups_file_rx: None,
            cache_cleanup_confirm: None,
            cache_cleanup_rx: None,
            cache_cleanup_notice: None,
            settings_notice,
            legacy_import_result,
            ui_scale_open: false,
            ui_scale_percent: ui_scale * 100.0,
            ui_scale,
            accent_open: false,
            accent,
            accent_edit: [accent.r(), accent.g(), accent.b()],
            avatar_providers_open: false,
            avatar_provider_choices: Vec::new(),
            avatar_provider_input: String::new(),
            avatar_provider_error: None,
            clear_instance_confirm: false,
            diagnostics_open: false,
            credits_open: false,
            visual_style_installed: false,
            font_fallbacks_installed: false,
            font_fallback_paths: Vec::new(),
            font_fallback_attempted_chars: std::collections::HashSet::new(),
            managed_client_edit: None,
            managed_client_error: None,
            managed_profile_pids: std::collections::HashMap::new(),
            managed_profile_scan_at: None,
        }
    }

    fn install_visual_style(&mut self, ctx: &egui::Context) {
        if self.visual_style_installed {
            return;
        }
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(crate::theme::PANEL_PAD);
        style.visuals.panel_fill = crate::theme::INK;
        style.visuals.window_fill = crate::theme::PANEL;
        style.visuals.extreme_bg_color = crate::theme::WELL;
        style.visuals.faint_bg_color = crate::theme::PANEL;
        style.visuals.selection.bg_fill = self.accent.gamma_multiply(0.62);
        style.visuals.selection.stroke.color = crate::theme::TEXT;
        style.visuals.window_rounding = egui::Rounding::same(crate::theme::RADIUS_PANEL);
        style.visuals.menu_rounding = egui::Rounding::same(crate::theme::RADIUS_PANEL);
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            // Only things you can press are rounded; panels stay square.
            widget.rounding = egui::Rounding::same(crate::theme::RADIUS_CONTROL);
        }
        style.visuals.widgets.inactive.weak_bg_fill = crate::theme::RAISED;
        style.visuals.widgets.hovered.weak_bg_fill = crate::theme::RULE_STRONG;
        style.visuals.widgets.active.weak_bg_fill = self.accent.gamma_multiply(0.72);

        // Setting the fills above without also setting the text and border
        // colours leaves egui's defaults on top of them. Those defaults are
        // tuned for egui's own darker, desaturated widgets, so on the
        // saturated blue pressed/selected fill the label stayed a dim grey
        // and the border kept a bright stock outline -- which is what made a
        // pressed or selected control look wrong rather than merely
        // different. A fill and the text on it have to be chosen together.
        let text_default = crate::theme::TEXT;
        let text_emphasis = crate::theme::TEXT;
        let border_subtle = crate::theme::RULE;
        style.visuals.widgets.noninteractive.fg_stroke =
            egui::Stroke::new(1.0_f32, crate::theme::TEXT_DIM);
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border_subtle);
        style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, text_default);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, border_subtle);
        style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, text_emphasis);
        style.visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0_f32, crate::theme::RULE_STRONG);
        // On the blue, text has to be near-white to stay readable, and the
        // border has to be a lighter blue rather than a grey outline.
        style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, text_emphasis);
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, self.accent);
        style.visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, text_emphasis);
        style.visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, border_subtle);
        // `selectable_value` (the tab bar, the settings sidebar, the
        // create-instance pickers) fills with `selection.bg_fill` and draws
        // its label with `selection.stroke`, so those two must agree with
        // each other for a selected tab to read as selected rather than
        // smudged.
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, text_emphasis);

        ctx.set_style(style);
        self.visual_style_installed = true;
    }

    fn ensure_font_fallbacks(&mut self, ctx: &egui::Context) {
        if !self.font_fallbacks_installed {
            self.font_fallback_paths = base_system_font_paths();
            apply_font_fallbacks(ctx, &self.font_fallback_paths);
            self.font_fallbacks_installed = true;
            ctx.request_repaint();
            return;
        }

        // Only pay for large language fonts when live VRChat data actually
        // needs them. The ordinary fallback is under 2 MiB; a CJK font can
        // be around 20 MiB, which matters for an app expected to sit open
        // beside VRChat for days.
        let players = self.state.ordered_players();
        let mut characters = std::collections::BTreeSet::new();
        for player in &players {
            for text in [
                Some(player.name.as_str()),
                player.pronouns.as_deref(),
                player.avatar_name.as_deref(),
                player.advisory.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                characters.extend(text.chars().filter(|character| !character.is_control()));
            }
        }
        characters.retain(|character| {
            !self.font_fallback_attempted_chars.contains(character)
                && !ctx.fonts(|fonts| {
                    fonts.has_glyph(
                        &egui::FontId::proportional(crate::theme::TEXT_BODY),
                        *character,
                    )
                })
        });
        if characters.is_empty() {
            return;
        }

        self.font_fallback_attempted_chars
            .extend(characters.iter().copied());
        let mut changed = false;
        for path in system_font_paths_for(&characters) {
            if !self.font_fallback_paths.contains(&path) {
                tracing::info!(path = %path.display(), "Adding a system font fallback for VRChat text");
                self.font_fallback_paths.push(path);
                changed = true;
            }
        }
        if changed {
            apply_font_fallbacks(ctx, &self.font_fallback_paths);
            ctx.request_repaint();
        }
    }

    /// Shared visual primitives. Keeping these here prevents each newly
    /// ported screen from inventing its own shade, border and corner radius.
    fn card_frame() -> egui::Frame {
        egui::Frame::none()
            .fill(crate::theme::PANEL)
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::RULE))
            .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
            .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD))
    }

    fn inset_frame() -> egui::Frame {
        egui::Frame::none()
            .fill(crate::theme::WELL)
            .stroke(egui::Stroke::new(1.0_f32, crate::theme::RULE))
            .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
            .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD))
    }

    fn primary_button(&self, label: impl Into<egui::WidgetText>) -> egui::Button<'_> {
        egui::Button::new(label)
            .fill(self.accent.gamma_multiply(0.78))
            .stroke(egui::Stroke::new(1.0_f32, self.accent))
    }

    /// The window title.
    ///
    /// Was "KozyBlake/Celeste <version> — Fork by KozyBlake — Windows &
    /// Linux", which is Scarlet-era framing: it described the project's
    /// relationship to its upstream rather than telling the operator
    /// anything about what they are looking at.
    ///
    /// Now it names the group being watched, which on a second monitor is
    /// the one question the title can usefully answer. The group comes from
    /// the `vrchat_group_id` setting and is resolved at runtime, so
    /// switching groups retitles the window on the next frame — nothing
    /// here is fixed at build time. Before a group is configured, or while
    /// its name is still resolving, it falls back to the name and version
    /// alone. Staff mode keeps its marker either way.
    fn window_title(&self) -> String {
        let base = if self.staff_mode {
            format!(
                "{} {} (staff mode)",
                celeste_core::identity::NAME,
                celeste_core::identity::VERSION
            )
        } else {
            format!(
                "{} {}",
                celeste_core::identity::NAME,
                celeste_core::identity::VERSION
            )
        };
        match self.state.group_name() {
            Some(group) if !group.trim().is_empty() => {
                format!("{base} \u{2014} {group}")
            }
            _ => base,
        }
    }

    fn persist_window_bounds(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().minimized == Some(true)) {
            return;
        }
        let Some(bounds) = ctx.input(|input| {
            let viewport = input.viewport();
            let outer = viewport.outer_rect?;
            let inner = viewport.inner_rect?;
            Some(Rectangle {
                x: outer.min.x.round() as i32,
                y: outer.min.y.round() as i32,
                // ViewportBuilder accepts an inner size. Persisting the
                // inner dimensions prevents the decoration border from
                // being added again on every restart.
                width: inner.width().round() as i32,
                height: inner.height().round() as i32,
            })
        }) else {
            return;
        };
        if bounds.width <= 0 || bounds.height <= 0 || self.last_window_bounds == Some(bounds) {
            return;
        }
        if let Some(settings) = &self.settings {
            settings.ui_bounds.set(Some(bounds));
            self.last_window_bounds = Some(bounds);
        }
    }

    fn save_column_layout(&self) {
        let Some(settings) = &self.settings else {
            return;
        };
        let layout = ColumnId::ALL
            .into_iter()
            .map(|column| {
                let index = self
                    .column_order
                    .iter()
                    .position(|visible| *visible == column)
                    .map_or(-1, |index| index as i32);
                let width = self
                    .column_widths
                    .get(&column)
                    .copied()
                    .unwrap_or_else(|| column.initial_width())
                    .round() as i32;
                (column.id().to_string(), UiPropsInfo { index, width })
            })
            .collect::<std::collections::HashMap<_, _>>();
        settings.store.set_object("ui_instance_columns", &layout);
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let header = ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(celeste_core::identity::NAME)
                        .strong()
                        .size(crate::theme::TEXT_HEADING),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Version {}  |  Windows & Linux",
                        celeste_core::identity::VERSION
                    ))
                    .small()
                    .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let status = self.state.connection_status();
                let signed_in = status.contains("signed in as");
                let colour = if signed_in {
                    crate::theme::OK
                } else {
                    crate::theme::WARN
                };
                let label = if status.is_empty() {
                    "VRChat: connecting"
                } else {
                    &status
                };
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(
                        colour.r(),
                        colour.g(),
                        colour.b(),
                        28,
                    ))
                    .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
                    .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                    .show(ui, |ui| {
                        ui.colored_label(colour, label);
                    });
            });
        });
        self.tutorial_layout.header = Some(header.response.rect);

        ui.add_space(9.0);
        ui.separator();
        ui.add_space(3.0);
        let controls = ui.horizontal_wrapped(|ui| {
            let tabs = egui::Frame::none()
                .fill(crate::theme::WELL)
                .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
                .inner_margin(egui::Margin::same(crate::theme::GAP_TIGHT))
                .show(ui, |ui| self.tab_bar(ui));
            self.tutorial_layout.tab_bar = Some(tabs.response.rect);
            ui.separator();

            if ui
                .add(self.primary_button(egui::RichText::new("+ Create instance").strong()))
                .clicked()
            {
                self.create_instance_open = true;
                self.create_instance_error = None;
            }

            ui.menu_button("Import groups", |ui| {
                if ui.button("From URL...").clicked() {
                    self.import_groups_url_open = true;
                    self.import_groups_url.clear();
                    ui.close_menu();
                }
                if ui.button("From file...").clicked() {
                    self.import_groups_from_file();
                    ui.close_menu();
                }
            });

            ui.menu_button("Tools", |ui| {
                if ui
                    .button("Open data folder")
                    .on_hover_text("Open the KozyBlake/Celeste data directory")
                    .clicked()
                {
                    if let Err(err) = open::that(&self.data_dir) {
                        self.state
                            .append_cli_output(format!("Could not open data folder: {err}"));
                    }
                    ui.close_menu();
                }
                if ui.button("Sync Discord").clicked() {
                    self.sync_discord_commands();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Clear current instance").clicked() {
                    self.clear_instance_confirm = true;
                    ui.close_menu();
                }
            });
            ui.separator();

            let tts_on = self.state.tts_enabled();
            let voice_colour = if tts_on {
                crate::theme::OK
            } else {
                ui.visuals().weak_text_color()
            };
            let button = egui::Button::new(
                egui::RichText::new(if tts_on { "Voice: On" } else { "Voice: Off" })
                    .color(voice_colour),
            );
            if ui.add(button).clicked() {
                self.state.set_tts_enabled(!tts_on);
                self.tts.set_paused(tts_on);
            }
            if ui
                .button("Skip voice")
                .on_hover_text("Skip the current TTS announcement")
                .clicked()
            {
                // Routed, not local-only: with announcements going to
                // Discord there is nothing playing on this machine to kill.
                self.state.append_cli_output(if self.tts_route.skip() {
                    "Skipped the current TTS announcement."
                } else {
                    "No TTS announcement is currently playing."
                });
            }
            ui.separator();

            ui.menu_button("View", |ui| {
                let mut show_cli = self.cli_tab_visible();
                if ui.checkbox(&mut show_cli, "Show CLI tab").changed() {
                    if let Some(settings) = &self.settings {
                        settings.show_cli_tab.set(Some(show_cli), "ui-view-menu");
                    }
                    if show_cli {
                        self.tab = Tab::Cli;
                    } else if self.tab == Tab::Cli {
                        self.tab = Tab::Instance;
                    }
                }
            });

            let help = ui.menu_button("Help", |ui| {
                let resume_at = self.tutorial_resume_step();
                if ui
                    .button(if resume_at.is_some() {
                        "Resume guided tour"
                    } else {
                        "Guided tour"
                    })
                    .clicked()
                {
                    self.start_tutorial(ui.ctx(), resume_at.unwrap_or(0));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    ui.close_menu();
                }
                if resume_at.is_some() && ui.button("Start tour over").clicked() {
                    if let Some(settings) = &self.settings {
                        settings.tutorial_progress_step.set(Some(-1), "guided-tour");
                    }
                    self.start_tutorial(ui.ctx(), 0);
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    ui.close_menu();
                }
                if ui.button("Tour chapters…").clicked() {
                    self.stop_tutorial_narration();
                    self.tutorial_step = None;
                    self.tutorial_menu = true;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                    ui.close_menu();
                }
                if ui.button("Diagnostics").clicked() {
                    self.diagnostics_open = true;
                    ui.close_menu();
                }
                if ui.button("Credits / About Celeste").clicked() {
                    self.credits_open = true;
                    ui.close_menu();
                }
                ui.separator();
                for (label, url) in [
                    ("GitHub repository", celeste_core::identity::GITHUB_URL),
                    ("License", celeste_core::identity::LICENSE_URL),
                    ("Community API docs", "https://vrchat.community/"),
                    ("VRChat Terms of Service", "https://hello.vrchat.com/legal"),
                    (
                        "VRChat Privacy Policy",
                        "https://hello.vrchat.com/privacy#7",
                    ),
                    (
                        "VRChat Community Guidelines",
                        "https://hello.vrchat.com/community-guidelines",
                    ),
                ] {
                    if ui.button(label).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        ui.close_menu();
                    }
                }
            });
            self.tutorial_layout.help = Some(help.response.rect);
        });
        self.tutorial_layout.toolbar_controls = Some(controls.response.rect);
    }

    /// Port of `ScarletUI.discordUpdateCommandList` ->
    /// `ScarletDiscordJDA.updateCommandList`. The gateway publishes its HTTP
    /// handle into `SharedHttp`, so the desktop action can perform the same
    /// global command reconciliation without owning serenity's `Context`.
    fn sync_discord_commands(&self) {
        let http = self.discord_http.clone();
        let state = self.state.clone();
        state.append_cli_output("Sync Discord: pushing the current command list...");
        self.runtime_handle.spawn(async move {
            let http = http.read().await.clone();
            let Some(http) = http else {
                state.append_cli_output("Sync Discord failed: the Discord bot is not connected.");
                return;
            };
            match celeste_discord::commands::register_global_commands_http(&http).await {
                Ok(count) => state.append_cli_output(format!(
                    "Sync Discord complete: registered {count} global commands."
                )),
                Err(error) => state.append_cli_output(format!("Sync Discord failed: {error}")),
            }
        });
    }

    /// Whether the CLI tab is shown, per `ui_show_cli_tab`. Java's View menu
    /// toggles this live (`ScarletUI.setCliTabVisible`, `:1374-1385`); the
    /// setting existed here but nothing read it, so the toggle did nothing.
    fn cli_tab_visible(&self) -> bool {
        self.settings
            .as_ref()
            .is_none_or(|settings| settings.show_cli_tab.get())
    }

    /// Port of `ScarletUI`'s `DO_NOTHING_ON_CLOSE` + `uiModalExit`
    /// (`:1281-1288`, `:1391-1401`). eframe's equivalent is to cancel the
    /// close, ask, and only then let a second close request through — so the
    /// X can't end a live session by accident, and shutdown is ordered
    /// rather than a kill.
    fn exit_confirm_window(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.exit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.exit_prompt_open = true;
        }
        if !self.exit_prompt_open {
            return;
        }
        let mut quit = false;
        let mut stay = false;
        egui::Window::new(celeste_core::i18n::tr("dialog.exit.title", &[]))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(celeste_core::i18n::tr("dialog.exit.message", &[]));
                // A session with people still in the instance is the case
                // this prompt is really for; say so rather than making the
                // user remember.
                let (present, _, _) = self.state.status_counts();
                if present > 0 {
                    ui.add_space(4.0);
                    ui.weak(format!(
                        "{present} player{} still in the instance.",
                        if present == 1 { "" } else { "s" }
                    ));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Quit").clicked() {
                        quit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        stay = true;
                    }
                });
            });
        if quit {
            self.exit_prompt_open = false;
            self.exit_confirmed = true;
            self.shutdown();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if stay {
            self.exit_prompt_open = false;
        }
    }

    /// Java's `scarlet.stop()` (`Scarlet.java:945-947`): flush what an
    /// orderly exit must not lose before the process goes away. The
    /// high-cardinality stores write through, while global.json/live.json
    /// retain Java's dirty-tracked behavior and require this explicit save.
    fn shutdown(&mut self) {
        self.flush_pending_setting_edits();
        if let Some(settings) = self.settings.as_ref() {
            settings.store.save();
        }
        self.celeste_data.save_all();
        tracing::info!("Celeste is shutting down");
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let show_cli = self.cli_tab_visible();
        // Java hides the tab and falls back to the first one rather than
        // leaving a selected-but-invisible tab behind.
        // Same reason as the category switch: leaving the tab stops the
        // fields being drawn, with no focus-loss event.
        let leaving_settings = self.tab == Tab::Settings;
        if !show_cli && self.tab == Tab::Cli {
            self.tab = Tab::Instance;
        }
        let tabs = [
            (Tab::Instance, "Instance"),
            (Tab::Clients, "Clients"),
            (Tab::Settings, "Settings"),
            (Tab::Cli, "CLI"),
        ];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, label) in tabs {
                if tab == Tab::Cli && !show_cli {
                    continue;
                }
                let selected = self.tab == tab;
                let text = if selected {
                    egui::RichText::new(label)
                        .strong()
                        .color(crate::theme::TEXT)
                } else {
                    egui::RichText::new(label)
                };
                let response = ui.add_sized(
                    [82.0, 30.0],
                    egui::Button::new(text)
                        .fill(if selected {
                            self.accent.gamma_multiply(0.62)
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE),
                );
                if response.clicked() {
                    if leaving_settings && tab != Tab::Settings {
                        // Leaving the tab stops the settings fields being
                        // drawn, and egui delivers no focus-loss event for
                        // a widget that simply stops existing -- so a
                        // half-typed value would be discarded.
                        self.flush_pending_setting_edits();
                    }
                    self.tab = tab;
                }
            }
        });
    }

    /// Renders one player's value for `column` into the current cell.
    /// Split out from `instance_tab` so the drag-to-reorder header can
    /// change which column an index maps to without duplicating the
    /// per-column render logic.
    /// A single-line cell: clipped to the column with the full value on
    /// hover, so a long id or avatar name can't make the row taller.
    fn text_cell(ui: &mut egui::Ui, text: &str) {
        let response = ui.add(egui::Label::new(text).truncate());
        if !text.is_empty() {
            response.on_hover_text(text);
        }
    }

    /// Table actions need much less horizontal padding than normal toolbar
    /// buttons, especially after the columns compress with the window.
    fn table_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
        ui.scope(|ui| {
            ui.spacing_mut().button_padding.x = 2.0;
            ui.add_sized(
                [ui.available_width(), 22.0],
                egui::Button::new(egui::RichText::new(label).small()),
            )
        })
        .inner
    }

    fn render_cell(
        &mut self,
        ui: &mut egui::Ui,
        player: &celeste_core::ui::ConnectedPlayer,
        column: ColumnId,
        display_state: &UiState,
        external_actions_enabled: bool,
    ) {
        match column {
            ColumnId::Name => {
                Self::text_cell(ui, &player.name);
            }
            ColumnId::Id => {
                Self::text_cell(ui, &player.id);
            }
            ColumnId::Pronouns => {
                Self::text_cell(ui, player.pronouns.as_deref().unwrap_or(""));
            }
            ColumnId::Avatar => {
                Self::text_cell(ui, player.avatar_name.as_deref().unwrap_or(""));
            }
            ColumnId::Performance => {
                Self::text_cell(
                    ui,
                    player
                        .avatar_info
                        .as_ref()
                        .and_then(|info| info.performance_rating.as_deref())
                        .unwrap_or(""),
                );
            }
            ColumnId::AvatarStats => {
                if Self::table_button(ui, "View").clicked() {
                    let location = display_state.instance().map(|instance| instance.location);
                    self.avatar_stats_open = Some(AvatarStatsView {
                        player_name: player.name.clone(),
                        avatar_name: player.avatar_name.clone(),
                        info: player.avatar_info.clone(),
                        note: player.avatar_info_note.clone(),
                        evidence: celeste_core::report_evidence::has_avatar_evidence(player).then(
                            || celeste_core::report_evidence::render(player, location.as_deref()),
                        ),
                    });
                }
            }
            ColumnId::WatchedGroups => {
                let total = player.watched_memberships.len();
                if total == 0 {
                    Self::text_cell(ui, "");
                } else {
                    // The count is the signal moderators act on; the
                    // button is how they see which ones.
                    let worst = player
                        .watched_memberships
                        .iter()
                        .any(|m| m.kind == "MALICIOUS" || m.critical);
                    let label = if worst {
                        format!("⚠ {total}")
                    } else {
                        total.to_string()
                    };
                    if Self::table_button(ui, &label).clicked() {
                        self.watched_groups_open = Some(WatchedGroupsView {
                            player_name: player.name.clone(),
                            user_id: player.id.clone(),
                            memberships: player.watched_memberships.clone(),
                        });
                    }
                }
            }
            ColumnId::AccAge => {
                let age = player
                    .account_age
                    .map(format_account_age)
                    .unwrap_or_default();
                Self::text_cell(ui, &age);
            }
            ColumnId::Joined => {
                Self::text_cell(ui, &player.joined.format("%Y-%m-%d %H:%M:%S").to_string());
            }
            ColumnId::Left => {
                let left = player
                    .left
                    .map(|l| l.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                Self::text_cell(ui, &left);
            }
            ColumnId::Advisory => {
                Self::text_cell(ui, player.advisory.as_deref().unwrap_or(""));
            }
            ColumnId::Age18 => {
                Self::text_cell(ui, player.age_verification_status.as_deref().unwrap_or(""));
            }
            ColumnId::Rank => {
                let color = player
                    .trust_rank
                    .color_rgb()
                    .map(|(r, g, b)| egui::Color32::from_rgb(r, g, b))
                    .unwrap_or(egui::Color32::GRAY);
                ui.add(
                    egui::Label::new(egui::RichText::new(player.trust_rank.label()).color(color))
                        .truncate(),
                );
            }
            ColumnId::Profile => {
                let response = ui
                    .add_enabled_ui(external_actions_enabled, |ui| {
                        Self::table_button(ui, "Open")
                    })
                    .inner
                    .on_disabled_hover_text("Disabled for temporary tutorial data");
                if response.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::same_tab(format!(
                        "https://vrchat.com/home/user/{}",
                        player.id
                    )));
                }
            }
            ColumnId::CopyId => {
                if Self::table_button(ui, "Copy").clicked() {
                    ui.ctx().copy_text(player.id.clone());
                }
            }
            ColumnId::Ban => {
                let response = ui
                    .add_enabled_ui(external_actions_enabled, |ui| Self::table_button(ui, "Ban"))
                    .inner
                    .on_disabled_hover_text("Disabled for temporary tutorial data");
                if response.clicked() {
                    // Java confirms before banning from the table; so does
                    // this, because a stray click in a row list is easy.
                    self.group_action_confirm =
                        Some((GroupAction::Ban, player.id.clone(), player.name.clone()));
                }
            }
            ColumnId::Unban => {
                let response = ui
                    .add_enabled_ui(external_actions_enabled, |ui| {
                        Self::table_button(ui, "Unban")
                    })
                    .inner
                    .on_disabled_hover_text("Disabled for temporary tutorial data");
                if response.clicked() {
                    self.group_action_confirm =
                        Some((GroupAction::Unban, player.id.clone(), player.name.clone()));
                }
            }
            ColumnId::Invite => {
                let response = ui
                    .add_enabled_ui(external_actions_enabled, |ui| {
                        Self::table_button(ui, "Invite")
                    })
                    .inner
                    .on_disabled_hover_text("Disabled for temporary tutorial data");
                if response.clicked() {
                    let confirm = self
                        .settings
                        .as_ref()
                        .is_some_and(|settings| settings.confirm_group_invite.get());
                    if confirm {
                        self.group_action_confirm =
                            Some((GroupAction::Invite, player.id.clone(), player.name.clone()));
                    } else {
                        self.spawn_group_invite(player.id.clone(), player.name.clone());
                    }
                }
            }
        }
    }

    fn instance_tab(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let display_state = self.tutorial_state();
        let tutorial_demo = self.tutorial_uses_demo_data();
        let tutorial_scroll_target = tutorial_demo
            .then(|| {
                self.tutorial_demo
                    .as_mut()
                    .and_then(|demo| demo.pending_scroll_target.take())
            })
            .flatten();
        let mut players = display_state.ordered_players();
        sort_players(&mut players, self.table_sort);
        let tutorial_scroll_row = tutorial_scroll_target
            .as_deref()
            .and_then(|target| players.iter().position(|player| player.id == target));
        if players.is_empty() {
            return self.instance_empty_state(ui);
        }
        let (present, left, total) = display_state.status_counts();
        let compact_height = ui.available_height() < 300.0;
        let instance = display_state.instance();
        let instance_name = instance
            .as_ref()
            .and_then(|instance| instance.world_name.clone())
            .unwrap_or_else(|| "Current instance".to_string());
        let instance_card = Self::card_frame()
            .inner_margin(if compact_height {
                egui::Margin::symmetric(12.0, 7.0)
            } else {
                egui::Margin::symmetric(14.0, 11.0)
            })
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    egui::RichText::new("Current instance")
                        .small()
                        .color(crate::theme::INFO),
                );
                ui.label(
                    egui::RichText::new(&instance_name)
                        .strong()
                        .size(crate::theme::TEXT_HEADING),
                );
                if !compact_height {
                    if let Some(instance) = &instance {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&instance.location)
                                    .small()
                                    .weak()
                                    .monospace(),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&instance.location);
                    }
                }
                if let Some(instance) = &instance {
                    ui.add_space(7.0);
                    ui.separator();
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        let kind = if instance.is_group_instance {
                            "Group instance"
                        } else {
                            "Public or private instance"
                        };
                        ui.label(egui::RichText::new(kind).small().strong());
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!(
                                "Joined {}",
                                instance.joined.format("%b %-d at %H:%M")
                            ))
                            .small()
                            .weak(),
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("{present} present"))
                                .color(crate::theme::OK),
                        );
                        if left > 0 {
                            ui.label(
                                egui::RichText::new(format!("{left} left"))
                                    .color(crate::theme::TEXT_MUTED),
                            );
                        }
                        ui.label(egui::RichText::new(format!("{total} total")).small().weak());
                        if !compact_height {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(
                                    "Drag headers to reorder  |  Right-click for columns",
                                )
                                .small()
                                .weak(),
                            );
                        }
                    });
                }
            });
        self.tutorial_layout.instance_card = Some(instance_card.response.rect);
        ui.add_space(8.0);
        let column_order = self.column_order.clone();
        let column_widths = self.column_widths.clone();
        // `ui_left_player_dim_percent` is how much to dim a player who has
        // left (0 = no dimming), so the fade is the user's setting rather
        // than a number baked in here.
        let dim_opacity = self
            .settings
            .as_ref()
            .map(|settings| {
                1.0 - (settings.ui_left_player_dim_percent.get() as f32 / 100.0).clamp(0.0, 0.9)
            })
            .unwrap_or(0.65);

        // Header cells are drag sources *and* drop zones at once: dropping
        // one column's drag payload (its current index) onto another
        // column's header swaps the two — matching the original Swing
        // `JTableHeader`'s drag-to-reorder behavior instead of egui_extras'
        // fixed column order.
        let mut swap: Option<(usize, usize)> = None;
        let mut clicked_sort = None;
        let mut reset_sort = false;
        let mut toggle_column = None;
        let mut width_update = None;
        let active_sort = self.table_sort;
        let player_panel_height = ui.available_height();

        let player_table = Self::card_frame().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height((player_panel_height - 24.0).max(0.0));
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Player details")
                        .strong()
                        .size(crate::theme::TEXT_HEADING),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("{} visible columns", column_order.len()))
                        .small()
                        .weak(),
                );
            });
            ui.add_space(8.0);
            let table_panel_height = ui.available_height();
            Self::inset_frame()
                .inner_margin(egui::Margin::same(crate::theme::GAP))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.set_min_height((table_panel_height - 12.0).max(0.0));
                    let table_view_width = ui.available_width();
                    let table_view_height = ui.available_height();
                    // `TableBuilder` places `item_spacing.x` between each pair of
                    // columns and reserves the scrollbar width separately from the
                    // declared column widths. Budget for both or the last columns
                    // still land outside the panel even when the widths themselves
                    // add up to `table_view_width`.
                    let inter_column_spacing =
                        ui.spacing().item_spacing.x * column_order.len().saturating_sub(1) as f32;
                    let scrollbar_width = ui.spacing().scroll.allocated_width();
                    let content_width =
                        (table_view_width - inter_column_spacing - scrollbar_width).max(0.0);
                    // Keep the entire roster inside the inner panel. Saved widths
                    // remain the user's preferences, but the on-screen widths are
                    // proportionally compressed to the current viewport. Cells and
                    // headings use ellipses, with full cell values available on
                    // hover.
                    let fitted_widths =
                        fitted_column_widths(&column_order, &column_widths, content_width);
                    let mut table = TableBuilder::new(ui)
                        .striped(true)
                        .min_scrolled_height(table_view_height)
                        .max_scroll_height(table_view_height)
                        .auto_shrink([false, false]);
                    if let Some(row) = tutorial_scroll_row {
                        table = table.scroll_to_row(row, Some(egui::Align::Center));
                    }
                    for width in &fitted_widths {
                        table = table.column(Column::exact(*width).clip(true));
                    }

                    table
                        .header(28.0, |mut header| {
                            for (index, column) in column_order.iter().enumerate() {
                                header.col(|ui| {
                                    // A plain label is the drop target. egui's stock
                                    // `dnd_drop_zone` always paints every target as a
                                    // filled widget, which produced the row of little
                                    // grey boxes around otherwise ordinary headings.
                                    let response = ui
                                        .dnd_drag_source(
                                            egui::Id::new("instance_column_header").with(index),
                                            index,
                                            |ui| {
                                                let marker = match active_sort {
                                                    Some(TableSort {
                                                        column: sorted,
                                                        ascending: true,
                                                    }) if sorted == *column => " [asc]",
                                                    Some(TableSort {
                                                        column: sorted,
                                                        ascending: false,
                                                    }) if sorted == *column => " [desc]",
                                                    _ => "",
                                                };
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(format!(
                                                            "{}{marker}",
                                                            column.label()
                                                        ))
                                                        .strong()
                                                        .color(crate::theme::TEXT_DIM),
                                                    )
                                                    .truncate()
                                                    .sense(egui::Sense::click_and_drag()),
                                                );
                                            },
                                        )
                                        .response;
                                    if column.sortable() && response.clicked() {
                                        clicked_sort = Some(*column);
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button("Default order").clicked() {
                                            reset_sort = true;
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        ui.strong("Columns");
                                        for candidate in ColumnId::ALL {
                                            let mut shown = column_order.contains(&candidate);
                                            if ui.checkbox(&mut shown, candidate.label()).changed()
                                            {
                                                toggle_column = Some(candidate);
                                            }
                                        }
                                    });
                                    let resize_id =
                                        egui::Id::new("instance_column_resize").with(*column);
                                    let edge = ui.max_rect().right();
                                    let resize_rect = egui::Rect::from_min_max(
                                        egui::pos2(edge - 4.0, ui.max_rect().top()),
                                        egui::pos2(edge + 4.0, ui.max_rect().bottom()),
                                    );
                                    let resize = ui
                                        .interact(resize_rect, resize_id, egui::Sense::drag())
                                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                                    if resize.drag_started() {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(
                                                resize_id,
                                                fitted_widths
                                                    .get(index)
                                                    .copied()
                                                    .unwrap_or_else(|| column.initial_width()),
                                            );
                                        });
                                    }
                                    if resize.dragged() || resize.drag_stopped() {
                                        let start = ui.ctx().data(|data| {
                                            data.get_temp::<f32>(resize_id).unwrap_or_else(|| {
                                                fitted_widths
                                                    .get(index)
                                                    .copied()
                                                    .unwrap_or_else(|| column.initial_width())
                                            })
                                        });
                                        let desired_fitted = (start + resize.drag_delta().x)
                                            .max(column.fitted_min_width());
                                        width_update = Some((
                                            *column,
                                            preference_for_fitted_width(
                                                &column_order,
                                                &column_widths,
                                                content_width,
                                                *column,
                                                desired_fitted,
                                            ),
                                            resize.drag_stopped(),
                                        ));
                                    }
                                    if let Some(dragged_from) =
                                        response.dnd_release_payload::<usize>()
                                    {
                                        swap = Some((*dragged_from, index));
                                    }
                                });
                            }
                        })
                        .body(|mut body| {
                            for player in &players {
                                body.row(27.0, |mut row| {
                                    let departed = player.left.is_some();
                                    for column in &column_order {
                                        row.col(|ui| {
                                            // One line per row: anything too long is
                                            // clipped with an ellipsis and readable
                                            // in full on hover, instead of wrapping
                                            // and pushing the row's height out.
                                            ui.style_mut().wrap_mode =
                                                Some(egui::TextWrapMode::Truncate);
                                            // Java repaints the whole row from the
                                            // advisory (`ScarletUI.java:762-813`):
                                            // amber background for flagged pronouns,
                                            // and the advisory's own colour — or
                                            // amber — as the foreground. Both were
                                            // being computed in core and then never
                                            // read, so a watched user looked exactly
                                            // like everyone else.
                                            if let Some(background) = row_background(player) {
                                                let rect = ui.available_rect_before_wrap();
                                                ui.painter().rect_filled(rect, 0.0, background);
                                            }
                                            if let Some(foreground) = row_foreground(player) {
                                                ui.visuals_mut().override_text_color =
                                                    Some(foreground);
                                            }
                                            if departed {
                                                // Someone who has left stays listed
                                                // (their history is the point) but
                                                // reads as past tense: sorted below
                                                // everyone present, and faded by the
                                                // amount `ui_left_player_dim_percent`
                                                // asks for.
                                                ui.set_opacity(dim_opacity);
                                            }
                                            self.render_cell(
                                                ui,
                                                player,
                                                *column,
                                                &display_state,
                                                !tutorial_demo,
                                            );
                                        });
                                    }
                                });
                            }
                        });
                });
        });
        self.tutorial_layout.player_table = Some(player_table.response.rect);

        if let Some((from, to)) = swap {
            if from != to && from < self.column_order.len() && to < self.column_order.len() {
                self.column_order.swap(from, to);
                self.save_column_layout();
            }
        }
        if let Some((column, width, finished)) = width_update {
            self.column_widths.insert(column, width);
            if finished {
                self.save_column_layout();
            }
        }
        if let Some(column) = toggle_column {
            if let Some(index) = self
                .column_order
                .iter()
                .position(|visible| *visible == column)
            {
                if self.column_order.len() > 1 {
                    self.column_order.remove(index);
                }
            } else {
                self.column_order.push(column);
            }
            self.save_column_layout();
        }
        if reset_sort {
            self.table_sort = None;
        } else if let Some(column) = clicked_sort {
            self.table_sort = Some(match self.table_sort {
                Some(TableSort {
                    column: active,
                    ascending,
                }) if active == column => TableSort {
                    column,
                    ascending: !ascending,
                },
                _ => TableSort {
                    column,
                    ascending: true,
                },
            });
        }
    }

    /// What the Instance tab shows before anyone has been logged, including
    /// which of the two reasons applies — no instance open, or an
    /// instance open that nobody has joined yet. Java shows a plain "No
    /// instance loaded" card here.
    fn instance_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(62.0);
            match self.state.instance() {
                Some(instance) => {
                    let where_ = instance.world_name.clone().unwrap_or_else(|| instance.location.clone());
                    ui.label(egui::RichText::new("Instance ready").small().color(crate::theme::INFO));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(where_).strong().size(crate::theme::TEXT_TITLE));
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(egui::RichText::new(&instance.location).small().weak().monospace()).truncate())
                        .on_hover_text(&instance.location);
                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("Waiting for players").strong().size(crate::theme::TEXT_HEADING));
                    ui.label(egui::RichText::new("The roster and moderation details will appear automatically as people join.").weak());
                }
                None => {
                    ui.label(egui::RichText::new("Waiting for VRChat").small().color(crate::theme::WARN));
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("No instance detected").strong().size(crate::theme::TEXT_TITLE));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(celeste_core::i18n::tr("ui.playerDataWillAppear", &[])).weak());
                }
            }
        });
    }

    /// Port of `ScarletUI.infoStats`: what Celeste knows about the avatar a
    /// player is wearing, and — when it knows nothing — the honest reason
    /// why, which is the part that matters for a private avatar.
    fn avatar_stats_window(&mut self, ctx: &egui::Context) {
        let Some(view) = self.avatar_stats_open.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Avatar - {}", view.player_name))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("avatar_stats_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Player");
                    ui.label(&view.player_name);
                    ui.end_row();

                    ui.label("Avatar");
                    ui.label(view.avatar_name.clone().unwrap_or_else(|| "(unknown)".to_string()));
                    ui.end_row();

                    match &view.info {
                        Some(info) => {
                            ui.label("Performance");
                            ui.label(info.performance_rating.clone().unwrap_or_else(|| "Unknown".to_string()));
                            ui.end_row();

                            if let Some(file_id) = &info.file_id {
                                ui.label("Asset bundle");
                                ui.add(egui::Label::new(egui::RichText::new(file_id).monospace()).truncate())
                                    .on_hover_text(file_id);
                                ui.end_row();
                            }
                            if let Some(name) = &info.file_name {
                                ui.label("File");
                                ui.add(egui::Label::new(name).truncate()).on_hover_text(name);
                                ui.end_row();
                            }
                            // The account that published the bundle. For an
                            // avatar you only saw, this is the identifier
                            // that still exists: VRChat does not expose
                            // another player's avtr_ id.
                            if let Some(owner_id) = &info.owner_id {
                                ui.label("Uploaded by");
                                ui.add(egui::Label::new(egui::RichText::new(owner_id).monospace()).truncate())
                                    .on_hover_text(owner_id);
                                ui.end_row();
                            }
                            if let Some(avatar_id) = &info.avatar_id {
                                ui.label("Avatar ID");
                                ui.add(egui::Label::new(egui::RichText::new(avatar_id).monospace()).truncate())
                                    .on_hover_text(avatar_id);
                                ui.end_row();
                            }
                        }
                        None => {
                            ui.label("Performance");
                            ui.weak("not resolved");
                            ui.end_row();
                        }
                    }
                });

                if let Some(evidence) = &view.evidence {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    if ui
                        .button("Copy report evidence")
                        .on_hover_text(
                            "Copies this player, the instance, and the asset bundle Celeste saw them \
                             wearing - including the account that uploaded it - ready to paste into a \
                             VRChat report.",
                        )
                        .clicked()
                    {
                        ui.ctx().copy_text(evidence.clone());
                        self.state.append_cli_output(&format!(
                            "Copied avatar report evidence for {}.",
                            view.player_name
                        ));
                    }
                }

                if let Some(note) = &view.note {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(note).weak());
                }
                if view.info.is_none() && view.note.is_none() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Avatar statuses come from VRChat's own API-request log lines, which the client only \
                             writes when it was launched with debug logging. Without them Celeste falls back to the \
                             avatar databases, which can't see private avatars.",
                        )
                        .small()
                        .weak(),
                    );
                }
            });
        if !open {
            self.avatar_stats_open = None;
        }
    }

    /// Every watched group a player is in, grouped by type.
    ///
    /// The advisory column joins the messages and truncates them, so for
    /// the accounts that matter most — the ones in a dozen watched groups
    /// — almost everything is invisible. This is the whole set, laid out
    /// by type with the worst first, rather than one run-on string.
    fn watched_groups_window(&mut self, ctx: &egui::Context) {
        let Some(view) = self.watched_groups_open.as_ref() else {
            return;
        };
        let sections = celeste_core::watched_membership::group_by_kind(&view.memberships);
        let total = view.memberships.len();
        let player_name = view.player_name.clone();
        let user_id = view.user_id.clone();
        let mut open = true;
        let mut copy: Option<String> = None;

        egui::Window::new(format!("Watched groups - {player_name}"))
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .max_height(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{player_name} is in {total} watched group{}",
                        if total == 1 { "" } else { "s" }
                    ))
                    .strong()
                    .size(crate::theme::TEXT_TITLE),
                );
                ui.add(egui::Label::new(egui::RichText::new(&user_id).monospace().weak()).truncate());
                ui.add_space(crate::theme::GAP);

                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    for (kind, entries) in &sections {
                        let accent = match kind.as_str() {
                            "MALICIOUS" => crate::theme::DANGER,
                            "NUISANCE" => crate::theme::WARN,
                            "COMMUNITY" => crate::theme::OK,
                            "AFFILIATED" => crate::theme::INFO,
                            _ => crate::theme::TEXT_DIM,
                        };
                        ui.add_space(crate::theme::GAP_TIGHT);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(kind).color(accent).strong());
                            ui.label(
                                egui::RichText::new(format!("({})", entries.len()))
                                    .color(crate::theme::TEXT_MUTED),
                            );
                        });
                        ui.separator();

                        for entry in entries {
                            ui.add_space(2.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(entry.label()).strong());
                                if entry.critical {
                                    ui.label(egui::RichText::new("critical").small().color(crate::theme::DANGER));
                                }
                                if entry.hidden {
                                    ui.label(
                                        egui::RichText::new("hidden on profile")
                                            .small()
                                            .color(crate::theme::WARN),
                                    )
                                    .on_hover_text(
                                        "This membership is hidden on their VRChat profile - checking \
                                         the profile by hand will not show it.",
                                    );
                                }
                                if entry.silent {
                                    ui.label(egui::RichText::new("silent").small().weak());
                                }
                            });
                            if let Some(message) = entry.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                                ui.label(egui::RichText::new(message).color(crate::theme::TEXT_DIM));
                            }
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&entry.group_id).monospace().small().weak(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&entry.group_id);
                            ui.add_space(crate::theme::GAP_TIGHT);
                        }
                    }
                });

                ui.add_space(crate::theme::GAP);
                ui.separator();
                ui.add_space(crate::theme::GAP_TIGHT);
                if ui.button("Copy all").clicked() {
                    let mut text = format!("{player_name} ({user_id}) - {total} watched groups\n");
                    for (kind, entries) in &sections {
                        text.push_str(&format!("\n{kind} ({})\n", entries.len()));
                        for entry in entries {
                            text.push_str(&format!("  {} [{}]", entry.label(), entry.group_id));
                            if entry.hidden {
                                text.push_str(" (hidden on profile)");
                            }
                            text.push('\n');
                            if let Some(message) = entry.message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
                                text.push_str(&format!("    {message}\n"));
                            }
                        }
                    }
                    copy = Some(text);
                }
            });

        if let Some(text) = copy {
            ctx.copy_text(text);
            self.state
                .append_cli_output(&format!("Copied {player_name}'s watched groups."));
        }
        if !open {
            self.watched_groups_open = None;
        }
    }

    /// Shown when a VRChat session ended without shutting down cleanly.
    ///
    /// The last avatar to finish loading is the one worth looking at: a
    /// crasher avatar takes the client down as it loads, so it is the last
    /// thing the log records. That is a lead, not a verdict, and the
    /// window says so — the same avatar loading harmlessly a moment before
    /// an unrelated freeze looks identical from here.
    fn crash_report_window(&mut self, ctx: &egui::Context) {
        let Some(report) = self.state.crash_report() else {
            return;
        };
        let mut open = true;
        egui::Window::new("VRChat stopped unexpectedly")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("VRChat ended without shutting down cleanly.")
                        .strong()
                        .size(crate::theme::TEXT_TITLE),
                );
                ui.add_space(crate::theme::GAP);
                match &report.last_avatar_load {
                    Some(load) => {
                        ui.label("Last avatar to finish loading before it stopped:");
                        ui.add_space(crate::theme::GAP_TIGHT);
                        egui::Grid::new("crash_report_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                            ui.label("Player");
                            ui.label(egui::RichText::new(&load.display_name).strong());
                            ui.end_row();
                            if let Some(user_id) = &load.user_id {
                                ui.label("User ID");
                                ui.add(egui::Label::new(egui::RichText::new(user_id).monospace()).truncate())
                                    .on_hover_text(user_id);
                                ui.end_row();
                            }
                            if let Some(avatar) = &load.avatar_name {
                                ui.label("Avatar");
                                ui.add(egui::Label::new(avatar).truncate()).on_hover_text(avatar);
                                ui.end_row();
                            }
                            // The wearer is who you kick; the author is
                            // who you report. They are usually different.
                            if let Some(author) = &load.avatar_author {
                                ui.label("Avatar uploaded by");
                                ui.add(egui::Label::new(egui::RichText::new(author).strong()).truncate())
                                    .on_hover_text(author);
                                ui.end_row();
                            }
                            if let Some(file_id) = &load.bundle_file_id {
                                ui.label("Asset bundle");
                                ui.add(egui::Label::new(egui::RichText::new(file_id).monospace()).truncate())
                                    .on_hover_text(file_id);
                                ui.end_row();
                            }
                            // The question a moderator asks next. Always
                            // shown, with three distinct answers: the
                            // counts, "none" (evidence against the lead),
                            // and "not checked" (Celeste does not know).
                            // Collapsing the last two would present an
                            // absence of information as an absence of
                            // doubt.
                            ui.label("Watched groups");
                            match &load.watched_groups {
                                Some(profile) => {
                                    let text = egui::RichText::new(profile.describe());
                                    let text =
                                        if profile.total > 0 { text.strong() } else { text.weak() };
                                    ui.add(egui::Label::new(text).truncate())
                                        .on_hover_text(profile.describe());
                                }
                                None => {
                                    ui.add(egui::Label::new(
                                        egui::RichText::new("not checked").weak().italics(),
                                    ))
                                    .on_hover_text(
                                        "Celeste could not read this user's groups - no VRChat \
                                         session, or the lookup failed. This is not the same as \
                                         them being in none.",
                                    );
                                }
                            }
                            ui.end_row();
                            ui.label("Loaded at");
                            ui.label(load.at.format("%Y-%m-%d %H:%M:%S").to_string());
                            ui.end_row();
                            if let Some(location) = &report.location {
                                ui.label("Instance");
                                ui.add(egui::Label::new(egui::RichText::new(location).monospace()).truncate())
                                    .on_hover_text(location);
                                ui.end_row();
                            }
                        });
                    }
                    None => {
                        ui.label("No avatar finished loading beforehand, so there is nobody to point at.");
                    }
                }
                ui.add_space(crate::theme::GAP);
                if let Some(load) = &report.last_avatar_load {
                    if ui
                        .button("Copy crash details")
                        .on_hover_text("Copy this report, ready to paste into a VRChat report.")
                        .clicked()
                    {
                        let mut text = String::from("VRChat client crash\n");
                        text.push_str(&format!("Last avatar load: {}\n", load.display_name));
                        if let Some(user_id) = &load.user_id {
                            text.push_str(&format!("User ID: {user_id}\n"));
                        }
                        if let Some(avatar) = &load.avatar_name {
                            text.push_str(&format!("Avatar: {avatar}\n"));
                        }
                        if let Some(author) = &load.avatar_author {
                            text.push_str(&format!("Avatar uploaded by: {author}\n"));
                        }
                        if let Some(file_id) = &load.bundle_file_id {
                            text.push_str(&format!("Asset bundle file ID: {file_id}\n"));
                        }
                        text.push_str(&match &load.watched_groups {
                            Some(profile) => format!("Watched groups: {}\n", profile.describe()),
                            None => "Watched groups: not checked (lookup unavailable)\n".to_string(),
                        });
                        if let Some(location) = &report.location {
                            text.push_str(&format!("Instance: {location}\n"));
                        }
                        text.push_str(&format!("Loaded at (local): {}\n", load.at.format("%Y-%m-%d %H:%M:%S")));
                        ui.ctx().copy_text(text);
                        self.state.append_cli_output("Copied the crash report to the clipboard.");
                    }
                    ui.add_space(crate::theme::GAP_TIGHT);
                }
                ui.separator();
                ui.add_space(crate::theme::GAP_TIGHT);
                ui.label(
                    egui::RichText::new(
                        "A crasher avatar takes the client down while it loads, so it tends to be \
                         the last thing in the log. This is a lead, not proof - an ordinary avatar \
                         loading just before an unrelated freeze looks exactly the same from here.",
                    )
                    .weak(),
                );
            });
        if !open {
            self.state.set_crash_report(None);
        }
    }

    /// The table's Ban action. Same call the `/vrchat-user-ban` command
    /// makes, run on the shared VRChat session.
    fn group_action_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((action, user_id, display_name)) = self.group_action_confirm.clone() else {
            return;
        };
        let mut open = true;
        let mut decided = false;
        let verb = match action {
            GroupAction::Ban => "Ban",
            GroupAction::Unban => "Unban",
            GroupAction::Invite => "Invite",
        };
        let preposition = if action == GroupAction::Invite {
            "to"
        } else {
            "from"
        };
        egui::Window::new(format!("{verb} VRChat group user"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{verb} {display_name} {preposition} the configured VRChat group?"
                ));
                ui.add(egui::Label::new(
                    egui::RichText::new(&user_id).small().monospace().weak(),
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(verb).clicked() {
                        decided = true;
                        match action {
                            GroupAction::Ban => {
                                self.spawn_group_ban(user_id.clone(), display_name.clone())
                            }
                            GroupAction::Unban => {
                                self.spawn_group_unban(user_id.clone(), display_name.clone())
                            }
                            GroupAction::Invite => {
                                self.spawn_group_invite(user_id.clone(), display_name.clone())
                            }
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        decided = true;
                    }
                });
            });
        if decided || !open {
            self.group_action_confirm = None;
        }
    }

    fn spawn_group_ban(&mut self, user_id: String, display_name: String) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let Some(group_id) = settings
            .store
            .get_string("vrchat_group_id")
            .filter(|id| !id.trim().is_empty())
        else {
            self.state
                .append_cli_output("Ban failed: no VRChat group is configured.");
            return;
        };
        let slot = self.vrchat_slot.clone();
        let state = self.state.clone();
        self.state
            .append_cli_output(format!("Banning {display_name}..."));
        self.runtime_handle.spawn(async move {
            let client = slot.read().await.clone();
            let Some(client) = client else {
                state.append_cli_output("Ban failed: not signed in to VRChat.");
                return;
            };
            match client.ban_from_group(&group_id, &user_id).await {
                Ok(_) => state.append_cli_output(format!("Banned {display_name}.")),
                Err(err) => state.append_cli_output(format!("Failed to ban {display_name}: {err}")),
            }
        });
    }

    fn spawn_group_unban(&mut self, user_id: String, display_name: String) {
        self.spawn_group_action(user_id, display_name, GroupAction::Unban);
    }

    fn spawn_group_invite(&mut self, user_id: String, display_name: String) {
        self.spawn_group_action(user_id, display_name, GroupAction::Invite);
    }

    fn spawn_group_action(&mut self, user_id: String, display_name: String, action: GroupAction) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let Some(group_id) = settings
            .store
            .get_string("vrchat_group_id")
            .filter(|id| !id.trim().is_empty())
        else {
            self.state.append_cli_output(format!(
                "{} failed: no VRChat group is configured.",
                match action {
                    GroupAction::Unban => "Unban",
                    GroupAction::Invite => "Invite",
                    GroupAction::Ban => "Ban",
                }
            ));
            return;
        };
        let slot = self.vrchat_slot.clone();
        let state = self.state.clone();
        let verb = match action {
            GroupAction::Unban => "Unbanning",
            GroupAction::Invite => "Inviting",
            GroupAction::Ban => "Banning",
        };
        self.state
            .append_cli_output(format!("{verb} {display_name}..."));
        self.runtime_handle.spawn(async move {
            let client = slot.read().await.clone();
            let Some(client) = client else {
                state.append_cli_output(format!("{verb} failed: not signed in to VRChat."));
                return;
            };
            let result = match action {
                GroupAction::Unban => client
                    .unban_from_group(&group_id, &user_id)
                    .await
                    .map(|_| ()),
                GroupAction::Invite => client.create_group_invite(&group_id, &user_id, true).await,
                GroupAction::Ban => client.ban_from_group(&group_id, &user_id).await.map(|_| ()),
            };
            match result {
                Ok(()) => state.append_cli_output(format!(
                    "{} {display_name}.",
                    match action {
                        GroupAction::Unban => "Unbanned",
                        GroupAction::Invite => "Invited",
                        GroupAction::Ban => "Banned",
                    }
                )),
                Err(err) => state.append_cli_output(format!(
                    "Failed to {} {display_name}: {err}",
                    match action {
                        GroupAction::Unban => "unban",
                        GroupAction::Invite => "invite",
                        GroupAction::Ban => "ban",
                    }
                )),
            }
        });
    }

    /// Desktop port of `ScarletUI.uiCreateGroupInstance`: the same picker,
    /// validation, self-permission checks, and `/instances` request.
    fn create_instance_window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.create_instance_rx {
            match rx.try_recv() {
                Ok(Ok(created)) => {
                    self.create_instance_rx = None;
                    self.create_instance_open = false;
                    self.state.append_cli_output(created.message.clone());
                    self.created_instance = Some(created);
                }
                Ok(Err(error)) => {
                    self.create_instance_rx = None;
                    self.create_instance_error = Some(error.clone());
                    self.state
                        .append_cli_output(format!("Create instance failed: {error}"));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.create_instance_rx = None;
                    self.create_instance_error =
                        Some("The instance creation task stopped unexpectedly.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if self.create_instance_open {
            let mut open = true;
            let mut submit = false;
            let mut cancel = false;
            let busy = self.create_instance_rx.is_some();
            egui::Window::new("Create VRChat instance")
                .collapsible(false).resizable(true).default_width(560.0).open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Create an instance owned by the configured group or by the signed-in VRChat account.");
                    ui.add_space(6.0);
                    egui::Grid::new("create_instance_fields").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                        ui.label("World URL or ID");
                        ui.add(egui::TextEdit::singleline(&mut self.create_instance_form.world).desired_width(390.0).hint_text("wrld_… or a VRChat world URL"));
                        ui.end_row();
                        ui.label("Who can join");
                        egui::ComboBox::from_id_salt("create_instance_kind").selected_text(self.create_instance_form.kind.label()).show_ui(ui, |ui| {
                            for kind in InstanceKind::ALL { ui.selectable_value(&mut self.create_instance_form.kind, kind, kind.label()); }
                        });
                        ui.end_row();
                        ui.label("Region");
                        let region_label = match self.create_instance_form.region.as_str() { "use" => "US East", "eu" => "Europe", "jp" => "Japan", _ => "US West" };
                        egui::ComboBox::from_id_salt("create_instance_region").selected_text(region_label).show_ui(ui, |ui| {
                            for (value, label) in [("us", "US West"), ("use", "US East"), ("eu", "Europe"), ("jp", "Japan")] {
                                ui.selectable_value(&mut self.create_instance_form.region, value.into(), label);
                            }
                        });
                        ui.end_row();
                        ui.label("Avatar gate");
                        let gate_label = match self.create_instance_form.avatar_gate.as_str() { "Poor" => "Poor or better", "Medium" => "Medium or better", "Good" => "Good only", _ => "None" };
                        egui::ComboBox::from_id_salt("create_instance_avatar_gate").selected_text(gate_label).show_ui(ui, |ui| {
                            for (value, label) in [("", "None"), ("Poor", "Poor or better"), ("Medium", "Medium or better"), ("Good", "Good only")] {
                                ui.selectable_value(&mut self.create_instance_form.avatar_gate, value.into(), label);
                            }
                        });
                        ui.end_row();
                        ui.label("Instance name");
                        ui.add(egui::TextEdit::singleline(&mut self.create_instance_form.display_name).desired_width(260.0).char_limit(33).hint_text("Optional, up to 32 characters"));
                        ui.end_row();
                    });
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.create_instance_form.queue_enabled, "Enable queue when full");
                        ui.checkbox(&mut self.create_instance_form.age_gate, "Require age-verified 18+ users");
                    });
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Allowed content").strong());
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.create_instance_form.content_drones, "Drones");
                        ui.checkbox(&mut self.create_instance_form.content_emoji, "Emoji");
                        ui.checkbox(&mut self.create_instance_form.content_props, "Items");
                        ui.checkbox(&mut self.create_instance_form.content_pedestals, "Pedestals");
                        ui.checkbox(&mut self.create_instance_form.content_prints, "Prints");
                        ui.checkbox(&mut self.create_instance_form.content_stickers, "Stickers");
                    });
                    if let Some(error) = &self.create_instance_error {
                        ui.add_space(8.0);
                        ui.colored_label(crate::theme::DANGER, error);
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new(if busy { "Creating…" } else { "Create instance" })).clicked() { submit = true; }
                        if ui.button("Cancel").clicked() { cancel = true; }
                    });
                });
            if cancel {
                open = false;
            }
            self.create_instance_open = open;
            if submit {
                self.start_create_instance();
            }
        }

        if let Some(created) = self.created_instance.clone() {
            let mut open = true;
            egui::Window::new("VRChat instance created")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(&created.message);
                    ui.hyperlink_to("Open instance on VRChat", &created.link);
                    ui.horizontal(|ui| {
                        if ui.button("Open in browser").clicked() {
                            if let Err(error) = open::that(&created.link) {
                                self.state.append_cli_output(format!(
                                    "Could not open instance link: {error}"
                                ));
                            }
                        }
                        if ui.button("Copy link").clicked() {
                            ui.ctx().copy_text(created.link.clone());
                        }
                    });
                });
            if !open {
                self.created_instance = None;
            }
        }
    }

    fn start_create_instance(&mut self) {
        self.create_instance_error = None;
        let Some(world_id) = resolve_world_id(&self.create_instance_form.world) else {
            self.create_instance_error = Some("Paste a valid VRChat world URL or wrld_ ID.".into());
            return;
        };
        if let Err(error) = validate_instance_display_name(&self.create_instance_form.display_name)
        {
            self.create_instance_error = Some(error);
            return;
        }
        let group_id = if self.create_instance_form.kind.group() {
            let Some(id) = self
                .settings
                .as_ref()
                .and_then(|s| s.store.get_string("vrchat_group_id"))
                .filter(|id| !id.trim().is_empty())
            else {
                self.create_instance_error =
                    Some("No VRChat group ID is configured in Settings.".into());
                return;
            };
            Some(id)
        } else {
            None
        };

        let form = self.create_instance_form.clone();
        let slot = self.vrchat_slot.clone();
        let data_dir = self.data_dir.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.create_instance_rx = Some(rx);
        self.runtime_handle.spawn(async move {
            let result = async {
                let client = slot.read().await.clone().ok_or_else(|| "Celeste is not signed in to VRChat.".to_string())?;
                let current = client.get_current_user().await.map_err(|e| format!("Could not identify the signed-in VRChat account: {e}"))?;
                if let Some(group_id) = group_id.as_deref() {
                    let permission = match form.kind {
                        InstanceKind::GroupPlus => celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_PLUS_CREATE,
                        InstanceKind::GroupMembers => celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_OPEN_CREATE,
                        _ => celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE,
                    };
                    for permission in std::iter::once(permission).chain(form.age_gate.then_some(celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE)) {
                        match client.self_has_group_permission(group_id, &current.id, permission).await {
                            Ok(true) => {}
                            Ok(false) => return Err(format!("The signed-in VRChat account needs the `{permission}` group permission.")),
                            Err(e) => return Err(format!("Could not check VRChat group permissions: {e}")),
                        }
                    }
                }
                let world = client.get_world(&world_id).await.map_err(|e| format!("Could not find world {world_id}: {e}"))?;
                let request = create_instance_request(&form, &world_id, group_id.as_deref().unwrap_or(&current.id))?;
                let instance = client.create_instance(&request).await.map_err(|e| format!("VRChat rejected the instance: {e}"))?;
                let returned_world = instance.world_id.as_deref().unwrap_or(&world_id);
                let raw_id = instance.instance_id.as_deref().or(instance.id.as_deref()).ok_or_else(|| "VRChat created the instance but did not return its ID.".to_string())?;
                let instance_id = raw_id.strip_prefix(&format!("{returned_world}:")).unwrap_or(raw_id);
                // Java records the creator before the matching group audit
                // event arrives so the event can be attributed correctly.
                if let Err(error) = celeste_discord::pending_moderation::PendingModeration::open(&data_dir)
                    .add_pending("group.instance.create", raw_id, &current.id)
                {
                    tracing::warn!(%error, target = raw_id, "Could not save instance-creation attribution");
                }
                Ok(CreatedInstance {
                    message: format!("Created a new instance of {}.", world.name),
                    link: format!("https://vrchat.com/home/launch?worldId={returned_world}&instanceId={instance_id}"),
                })
            }.await;
            let _ = tx.send(result);
        });
    }

    /// Port of `ScarletUI.importWG(true)`: pick a `.csv` or `.json` file
    /// and import it over the current watched groups.
    fn import_groups_from_file(&mut self) {
        if self.import_groups_file_rx.is_some() {
            self.state
                .append_cli_output("Import groups: a file picker is already open.");
            return;
        }

        self.state
            .append_cli_output("Import groups: opening file picker...");
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime_handle.spawn(async move {
            // The synchronous rfd API drives the XDG portal from egui's UI
            // thread. A portal panic there closes the entire application.
            // Keep the dialog and file I/O on the async runtime instead.
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Import watched groups")
                .add_filter("CSV or JSON", &["csv", "json"])
                .pick_file()
                .await;
            let result = match picked {
                None => Ok(None),
                Some(file) => {
                    let path = file.path().to_path_buf();
                    match tokio::fs::read(&path).await {
                        Ok(contents) => {
                            let extension = path
                                .extension()
                                .and_then(|extension| extension.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            Ok(Some((extension, contents, path.display().to_string())))
                        }
                        Err(err) => Err(format!("could not read {}: {err}", path.display())),
                    }
                }
            };
            let _ = tx.send(result);
        });
        self.import_groups_file_rx = Some(rx);
    }

    /// Port of `ScarletUI.importWG(false)`: fetch the CSV/JSON from a URL
    /// and import it. The fetch goes through the same public-only guard as
    /// every other user-supplied URL (Java's `HttpURLInputStream.PUBLIC_ONLY`),
    /// so it can't be pointed at something on the local network.
    fn import_groups_from_url(&mut self, url: String) {
        // Java's own branch picks the format from the URL text.
        let lower = url.to_ascii_lowercase();
        let extension = if lower.contains(".csv") {
            "csv"
        } else if lower.contains(".json") {
            "json"
        } else {
            self.state.append_cli_output(
                "Import groups: unrecognized file type (expected .csv or .json in the URL).",
            );
            return;
        };

        self.state
            .append_cli_output(format!("Import groups: fetching {url}..."));
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime_handle.spawn(async move {
            // 8 MiB is far beyond any real watched-group list and keeps a
            // hostile response from exhausting memory.
            let result =
                celeste_discord::url_guard::download_public_text(&url, 8 * 1024 * 1024).await;
            let _ = tx.send(result);
        });
        self.import_groups_rx = Some((extension.to_string(), rx));
    }

    /// Applies a fetched or read import, whichever format it is.
    fn apply_group_import(&mut self, extension: &str, contents: &[u8], source: &str) {
        let result = match extension {
            // Java imports with overwrite = true from both paths.
            "csv" => self.watched_groups.import_legacy_csv(contents, true),
            "json" => self.watched_groups.import_json(contents, true),
            other => {
                self.state.append_cli_output(format!(
                    "Import groups: unrecognized file type \"{other}\"."
                ));
                return;
            }
        };
        match result {
            Ok(count) => self
                .state
                .append_cli_output(format!("Imported {count} watched group(s) from {source}.")),
            Err(err) => self
                .state
                .append_cli_output(format!("Import groups failed ({source}): {err}")),
        }
    }

    /// The URL question, and the result of a fetch started by it.
    fn import_groups_windows(&mut self, ctx: &egui::Context) {
        if self.import_groups_url_open {
            let mut open = true;
            let mut submit = false;
            egui::Window::new("Import watched groups from a URL")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("URL of a CSV or JSON watched-group list:");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.import_groups_url)
                            .desired_width(420.0)
                            .hint_text("https://..."),
                    );
                    ui.horizontal(|ui| {
                        submit = ui.button("Import").clicked()
                            || (response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        if ui.button("Cancel").clicked() {
                            self.import_groups_url.clear();
                            self.import_groups_url_open = false;
                        }
                    });
                });
            if !open {
                self.import_groups_url_open = false;
            }
            if submit {
                let url = self.import_groups_url.trim().to_string();
                self.import_groups_url_open = false;
                self.import_groups_url.clear();
                if url.is_empty() {
                    self.state.append_cli_output("Import groups: canceled.");
                } else {
                    self.import_groups_from_url(url);
                }
            }
        }

        if let Some(rx) = &self.import_groups_file_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.import_groups_file_rx = None;
                    match result {
                        Ok(Some((extension, contents, source))) => {
                            self.apply_group_import(&extension, &contents, &source);
                        }
                        Ok(None) => self.state.append_cli_output("Import groups: canceled."),
                        Err(err) => self
                            .state
                            .append_cli_output(format!("Import groups failed: {err}")),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.import_groups_file_rx = None;
                    self.state.append_cli_output(
                        "Import groups failed: the system file picker stopped unexpectedly.",
                    );
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some((extension, rx)) = &self.import_groups_rx {
            match rx.try_recv() {
                Ok(result) => {
                    let extension = extension.clone();
                    self.import_groups_rx = None;
                    match result {
                        Ok(text) => self.apply_group_import(&extension, text.as_bytes(), "the URL"),
                        Err(err) => self
                            .state
                            .append_cli_output(format!("Import groups failed: {err}")),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.import_groups_rx = None;
                    self.state.append_cli_output(
                        "Import groups failed: the URL download stopped unexpectedly.",
                    );
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Settings that belong to subsystems this port deliberately does not
    /// have (see PORTING.md's scope section). They stay registered, because
    /// removing them would drop the values from a `settings.json` shared
    /// with the Java build — but showing a "Mobile relay endpoint" field in
    /// an app with no mobile companion is just clutter with no way to act
    /// on it.
    fn is_dropped_subsystem(id: &str) -> bool {
        const DROPPED_PREFIXES: [&str; 4] = [
            "mobile_",             // mobile companion / FCM relay
            "evidence_",           // evidence submission
            "advisory_translate_", // LibreTranslate advisory translation
            "vrchat_report_",      // report templates
        ];
        const DROPPED_IDS: [&str; 1] = [
            "training_mode_enabled", // training / simulation mode
        ];
        DROPPED_PREFIXES.iter().any(|prefix| id.starts_with(prefix)) || DROPPED_IDS.contains(&id)
    }

    /// Intentional sections based on Java's `SETTINGS_SECTIONS`, while the
    /// six-category sidebar remains the more compact Rust navigation.
    fn setting_group(id: &str) -> &'static str {
        if matches!(
            id,
            "ui_language" | "ui_left_player_dim_percent" | "ui_accent_headers"
        ) {
            "Appearance"
        } else if id.starts_with("ui_")
            || matches!(
                id,
                "vibecoded_notice_acknowledged"
                    | "setup_profile_prompted"
                    | "discord_setup_prompted"
                    | "vrchat_setup_prompted"
            )
        {
            "Interface"
        } else if id.starts_with("enforce_instances")
            || matches!(
                id,
                "vrchat_client_launch_on_instance_create" | "instance_follow_fast_poll_seconds"
            )
        {
            "Instance enforcement"
        } else if id.starts_with("advisory_") {
            "Advisories"
        } else if id.starts_with("tts_") {
            "Text to speech"
        } else if id.starts_with("toast_") || id.starts_with("notify_") {
            "Desktop notifications"
        } else if id.starts_with("discord_ping_outstanding_") {
            "Outstanding moderation"
        } else if matches!(
            id,
            "verified_role_snowflake"
                | "members_role_snowflake"
                | "auto_invite_group_on_verify"
                | "link_vrchat_manual_verify_message"
                | "auto_invite_group_id"
                | "announce_verify_complete"
        ) {
            "Verification"
        } else if id.starts_with("discord_") || id.starts_with("moderation_summary_") {
            "Discord"
        } else if id.starts_with("attempt_avatar_") || id.starts_with("custom_avatar_") {
            "Avatar search"
        } else if id.starts_with("cache_") {
            "Cache cleanup"
        } else if id.starts_with("audit_")
            || id.starts_with("moderation_")
            || id.starts_with("heuristic")
            || id.starts_with("outstanding")
            || matches!(id, "timed_bans_enabled" | "auto_close_bot_instance_minutes")
        {
            "Moderation"
        } else {
            "General"
        }
    }

    fn notification_matrix_id(id: &str) -> bool {
        const IDS: [&str; 16] = [
            "tts_announce_watched_users",
            "toast_notify_watched_users",
            "tts_announce_watched_groups",
            "toast_notify_watched_groups",
            "tts_announce_watched_avatars",
            "toast_notify_watched_avatars",
            "tts_announce_new_players",
            "toast_notify_new_players",
            "tts_announce_mixed_character_names",
            "toast_notify_mixed_character_names",
            "tts_announce_votes_to_kick",
            "toast_notify_votes_to_kick",
            "tts_announce_suspicious_pronouns",
            "toast_notify_suspicious_pronouns",
            "toast_notify_moderation",
            "toast_notify_staff",
        ];
        IDS.contains(&id)
    }

    /// A sidebar click is navigation, even if the user previously searched
    /// for a setting. Leaving the global query active made the heading and
    /// selected button change while the same cross-category search results
    /// stayed on screen, which made every category button appear broken.
    fn activate_settings_category(
        current: &mut SettingCategory,
        search: &mut String,
        selected: SettingCategory,
    ) {
        *current = selected;
        search.clear();
    }

    /// The action cards contain buttons rather than registry-backed values,
    /// so they need their own search metadata. Without this, searching for an
    /// action such as "import" hid the very button the user was looking for.
    fn settings_action_matches(category: SettingCategory, query: &str) -> bool {
        let searchable = match category {
            SettingCategory::General => {
                "appearance ui scale accent colour color pronoun validation good bad list reload"
            }
            SettingCategory::System => {
                "system data folder open move import scarlet legacy migration cache cleanup restart celeste reset vrchat credentials session linux desktop application launcher shortcut menu install"
            }
            SettingCategory::Notifications => {
                "audio sound output device speaker headphones mixer control panel windows settings pavucontrol pipewire pulse tts"
            }
            SettingCategory::Discord => {
                "discord reset connection token guild server setup"
            }
            SettingCategory::Integrations => {
                "integrations avatar search database provider manage"
            }
            _ => return false,
        };
        searchable.contains(query)
    }

    fn has_settings_actions(category: SettingCategory) -> bool {
        matches!(
            category,
            SettingCategory::General
                | SettingCategory::System
                | SettingCategory::Notifications
                | SettingCategory::Discord
                | SettingCategory::Integrations
        )
    }

    fn render_notification_matrix(
        ui: &mut egui::Ui,
        descriptors: &[std::sync::Arc<dyn SettingDescriptor>],
    ) {
        const ROWS: [(&str, Option<&str>, Option<&str>); 9] = [
            (
                "Watched user",
                Some("tts_announce_watched_users"),
                Some("toast_notify_watched_users"),
            ),
            (
                "Watched group",
                Some("tts_announce_watched_groups"),
                Some("toast_notify_watched_groups"),
            ),
            (
                "Watched avatar",
                Some("tts_announce_watched_avatars"),
                Some("toast_notify_watched_avatars"),
            ),
            (
                "New player",
                Some("tts_announce_new_players"),
                Some("toast_notify_new_players"),
            ),
            (
                "Mixed-character name",
                Some("tts_announce_mixed_character_names"),
                Some("toast_notify_mixed_character_names"),
            ),
            (
                "Vote to kick",
                Some("tts_announce_votes_to_kick"),
                Some("toast_notify_votes_to_kick"),
            ),
            (
                "Suspicious pronouns",
                Some("tts_announce_suspicious_pronouns"),
                Some("toast_notify_suspicious_pronouns"),
            ),
            ("Moderation action", None, Some("toast_notify_moderation")),
            ("Staff activity", None, Some("toast_notify_staff")),
        ];
        let find = |id: &str| descriptors.iter().find(|descriptor| descriptor.id() == id);
        Self::card_frame().show(ui, |ui| {
            ui.label(
                egui::RichText::new("Event notifications")
                    .strong()
                    .size(crate::theme::TEXT_HEADING),
            );
            ui.label(
                egui::RichText::new("Choose how each event should get your attention.")
                    .small()
                    .weak(),
            );
            ui.add_space(8.0);
            egui::Grid::new("notification_matrix")
                .num_columns(3)
                .spacing([24.0, 7.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("Event");
                    ui.strong("Voice");
                    ui.strong("Desktop");
                    ui.end_row();
                    for (label, voice_id, desktop_id) in ROWS {
                        ui.label(label);
                        for id in [voice_id, desktop_id] {
                            if let Some(descriptor) = id.and_then(find) {
                                let mut enabled = descriptor.get_as_string() == "true";
                                if ui.checkbox(&mut enabled, "").changed() {
                                    descriptor.set_from_string(&enabled.to_string(), "ui");
                                }
                            } else {
                                ui.weak("--");
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn managed_clients_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Managed VRChat clients");
                ui.label(
                    egui::RichText::new(
                        "Keep VRChat processes in exact group instances without signing those accounts into Celeste.",
                    )
                    .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(self.primary_button("+ Add coverage slot"))
                    .clicked()
                {
                    let used = self
                        .managed_clients
                        .list()
                        .into_iter()
                        .map(|slot| slot.profile)
                        .collect::<std::collections::HashSet<_>>();
                    // Start at 1: profile 0 belongs to a manually launched
                    // client and to the legacy instance follow, and the slot
                    // validator rejects it.
                    let profile = (1..).find(|profile| !used.contains(profile)).unwrap_or(1);
                    self.managed_client_edit = Some(
                        celeste_vrchat::managed_clients::ManagedClientSlot {
                            profile,
                            ..Default::default()
                        },
                    );
                    self.managed_client_error = None;
                }
            });
        });
        ui.add_space(8.0);
        Self::inset_frame().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(crate::theme::INFO, "Normal Steam/EAC launch only");
                ui.separator();
                ui.label("Celeste identifies each client by its numeric VRChat --profile value.");
                ui.separator();
                ui.label(
                    "A VRChat-wide API outage pauses recovery instead of restarting every client.",
                );
                ui.separator();
                ui.label("Automatic instance replacement is opt-in per slot.");
            });
        });
        ui.add_space(10.0);

        if self.managed_client_edit.is_some() {
            let mut save_requested = false;
            let mut cancel_requested = false;
            Self::card_frame().show(ui, |ui| {
                let slot = self.managed_client_edit.as_mut().expect("checked above");
                ui.label(egui::RichText::new(if self.managed_clients.list().iter().any(|saved| saved.id == slot.id) {
                    "Edit coverage slot"
                } else {
                    "New coverage slot"
                }).strong().size(crate::theme::TEXT_HEADING));
                ui.add_space(7.0);
                egui::Grid::new("managed_client_fields")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Slot name");
                        ui.add(egui::TextEdit::singleline(&mut slot.label).desired_width(420.0));
                        ui.end_row();
                        ui.label("Local profile");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut slot.profile).range(1..=999).speed(1));
                            ui.weak("VRChat --profile number; each slot must be unique (0 is reserved)");
                        });
                        ui.end_row();
                        ui.label("Expected account ID (optional)");
                        ui.add(
                            egui::TextEdit::singleline(&mut slot.expected_user_id)
                                .desired_width(420.0)
                                .hint_text("usr_…; no password or account sign-in required"),
                        );
                        ui.end_row();
                        ui.label("Account label");
                        ui.add(
                            egui::TextEdit::singleline(&mut slot.expected_display_name)
                                .desired_width(420.0)
                                .hint_text("Optional display name for operators"),
                        );
                        ui.end_row();
                        ui.label("Assigned instance");
                        ui.add(
                            egui::TextEdit::singleline(&mut slot.desired_location)
                                .desired_width(420.0)
                                .hint_text("wrld_…:instance… or VRChat launch URL"),
                        );
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new(
                        "Without an account ID, Celeste can restart a stopped profile into its assigned instance, but it will not move a running process because it cannot safely confirm who or where it is.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut slot.enabled, "Manage this slot");
                    ui.checkbox(&mut slot.auto_restart, "Restart after crash/disconnect");
                    ui.checkbox(
                        &mut slot.auto_recreate_instance,
                        "Create a replacement when its instance expires",
                    );
                });

                if slot.auto_recreate_instance {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(7.0);
                    ui.label(egui::RichText::new("Replacement instance template").strong());
                    ui.weak("Celeste uses this only after the exact assigned group instance disappears.");
                    ui.add_space(5.0);
                    egui::Grid::new("managed_client_template")
                        .num_columns(2)
                        .spacing([16.0, 8.0])
                        .show(ui, |ui| {
                            ui.label("World URL or ID");
                            ui.add(egui::TextEdit::singleline(&mut slot.template.world_id).desired_width(420.0));
                            ui.end_row();
                            ui.label("Who can join");
                            egui::ComboBox::from_id_salt("managed_access")
                                .selected_text(match slot.template.group_access_type.as_str() {
                                    "plus" => "Group Plus",
                                    "members" => "Group Members",
                                    _ => "Group Public",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut slot.template.group_access_type, "public".into(), "Group Public");
                                    ui.selectable_value(&mut slot.template.group_access_type, "plus".into(), "Group Plus");
                                    ui.selectable_value(&mut slot.template.group_access_type, "members".into(), "Group Members");
                                });
                            ui.end_row();
                            ui.label("Region");
                            egui::ComboBox::from_id_salt("managed_region")
                                .selected_text(match slot.template.region.as_str() {
                                    "use" => "US East",
                                    "eu" => "Europe",
                                    "jp" => "Japan",
                                    _ => "US West",
                                })
                                .show_ui(ui, |ui| {
                                    for (value, label) in [("us", "US West"), ("use", "US East"), ("eu", "Europe"), ("jp", "Japan")] {
                                        ui.selectable_value(&mut slot.template.region, value.into(), label);
                                    }
                                });
                            ui.end_row();
                            ui.label("Instance name");
                            ui.add(egui::TextEdit::singleline(&mut slot.template.display_name).char_limit(32).desired_width(300.0));
                            ui.end_row();
                        });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut slot.template.queue_enabled, "Enable queue when full");
                        ui.checkbox(&mut slot.template.age_gate, "Require age-verified 18+");
                    });
                }

                if let Some(error) = &self.managed_client_error {
                    ui.add_space(7.0);
                    ui.colored_label(crate::theme::DANGER, error);
                }
                ui.add_space(9.0);
                ui.horizontal(|ui| {
                    if ui.button("Save slot").clicked() {
                        save_requested = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_requested = true;
                    }
                });
            });
            ui.add_space(10.0);

            if save_requested {
                let mut slot = self.managed_client_edit.clone().expect("edit exists");
                if !slot.desired_location.trim().is_empty() {
                    match resolve_instance_location(&slot.desired_location) {
                        Some(location) => slot.desired_location = location,
                        None => {
                            self.managed_client_error = Some(
                                "Paste a full wrld_…:instance… location or VRChat launch URL."
                                    .into(),
                            );
                            return;
                        }
                    }
                }
                if !slot.template.world_id.trim().is_empty() {
                    match resolve_world_id(&slot.template.world_id) {
                        Some(world) => slot.template.world_id = world,
                        None => {
                            self.managed_client_error = Some(
                                "The replacement template needs a valid wrld_ ID or world URL."
                                    .into(),
                            );
                            return;
                        }
                    }
                }
                match self.managed_clients.upsert(slot) {
                    Ok(()) => {
                        self.managed_client_edit = None;
                        self.managed_client_error = None;
                    }
                    Err(error) => self.managed_client_error = Some(error.to_string()),
                }
            } else if cancel_requested {
                self.managed_client_edit = None;
                self.managed_client_error = None;
            }
        }

        let slots = self.managed_clients.list();
        if slots.is_empty() {
            self.managed_profile_pids.clear();
            self.managed_profile_scan_at = None;
            Self::card_frame().show(ui, |ui| {
                ui.set_min_height(120.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(24.0);
                    ui.label(egui::RichText::new("No managed clients yet").strong().size(crate::theme::TEXT_HEADING));
                    ui.label("Add one slot for each numeric VRChat profile and instance Celeste should keep covered.");
                });
            });
            return;
        }

        let enabled_profiles = slots
            .iter()
            .filter(|slot| slot.enabled)
            .map(|slot| slot.profile)
            .collect::<Vec<_>>();
        if enabled_profiles.is_empty() {
            self.managed_profile_pids.clear();
            self.managed_profile_scan_at = None;
        } else {
            let scan_due = self
                .managed_profile_scan_at
                .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(3));
            if scan_due {
                self.managed_profile_pids =
                    celeste_vrchat::launch::profile_pids_for(&enabled_profiles);
                self.managed_profile_scan_at = Some(std::time::Instant::now());
            }
        }

        let mut edit = None;
        let mut toggle = None;
        let mut stop = None;
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for slot in slots {
                Self::card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&slot.label)
                                    .strong()
                                    .size(crate::theme::TEXT_HEADING),
                            );
                            let account = if slot.expected_display_name.trim().is_empty() {
                                slot.expected_user_id.as_str()
                            } else {
                                slot.expected_display_name.as_str()
                            };
                            ui.label(
                                egui::RichText::new(format!(
                                    "Profile {}  |  {}",
                                    slot.profile,
                                    if account.is_empty() {
                                        "identity check disabled"
                                    } else {
                                        account
                                    }
                                ))
                                .small()
                                .weak(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.colored_label(
                                if slot.enabled {
                                    crate::theme::OK
                                } else {
                                    crate::theme::TEXT_MUTED
                                },
                                if slot.enabled { "Managed" } else { "Disabled" },
                            );
                        });
                    });
                    ui.add_space(7.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(&slot.last_status).strong());
                        ui.separator();
                        if slot.enabled {
                            let pids = self
                                .managed_profile_pids
                                .get(&slot.profile)
                                .map(Vec::as_slice)
                                .unwrap_or_default();
                            ui.label(if pids.is_empty() {
                                "Process stopped".into()
                            } else {
                                format!(
                                    "PID {}",
                                    pids.iter()
                                        .map(u32::to_string)
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            });
                        } else {
                            ui.weak("Process monitoring off");
                        }
                    });
                    if !slot.desired_location.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&slot.desired_location)
                                    .monospace()
                                    .small(),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&slot.desired_location);
                    }
                    if !slot.last_error.is_empty() {
                        ui.colored_label(crate::theme::WARN, &slot.last_error);
                    }
                    ui.add_space(7.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Edit").clicked() {
                            edit = Some(slot.clone());
                        }
                        if ui
                            .button(if slot.enabled { "Disable" } else { "Enable" })
                            .clicked()
                        {
                            toggle = Some(slot.clone());
                        }
                        if ui
                            .button("Stop & disable")
                            .on_hover_text("Stops only this numeric VRChat profile")
                            .clicked()
                        {
                            stop = Some(slot.clone());
                        }
                        if ui
                            .button("Forget slot")
                            .on_hover_text("Stops managing it; does not close VRChat")
                            .clicked()
                        {
                            remove = Some(slot.id.clone());
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
        if let Some(slot) = edit {
            self.managed_client_edit = Some(slot);
            self.managed_client_error = None;
        }
        if let Some(mut slot) = toggle {
            slot.enabled = !slot.enabled;
            if let Err(error) = self.managed_clients.upsert(slot) {
                self.managed_client_error = Some(error.to_string());
            } else {
                self.managed_profile_scan_at = None;
            }
        }
        if let Some(mut slot) = stop {
            slot.enabled = false;
            if let Err(error) = self.managed_clients.upsert(slot.clone()) {
                self.managed_client_error = Some(error.to_string());
            } else if let Err(error) = celeste_vrchat::launch::stop_profile(slot.profile) {
                self.managed_client_error = Some(error.to_string());
            }
        }
        if let Some(id) = remove {
            if let Err(error) = self.managed_clients.remove(&id) {
                self.managed_client_error = Some(error.to_string());
            }
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let Some(settings) = &self.settings else {
            ui.label("Settings unavailable (no settings store attached).");
            return;
        };
        let all = settings.registry.all();
        let query = self.settings_search.trim().to_lowercase();

        // Everything the current filter shows, so the sidebar counts and the
        // "nothing here" message agree with the list.
        let visible: Vec<_> = all
            .iter()
            .filter(|descriptor| !Self::is_dropped_subsystem(descriptor.id()))
            .filter(|descriptor| {
                query.is_empty()
                    || descriptor.name().to_lowercase().contains(&query)
                    || descriptor.id().to_lowercase().contains(&query)
            })
            .collect();

        let settings_sidebar = egui::SidePanel::left("settings_categories")
            .resizable(false)
            .exact_width(176.0)
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD)),
            )
            .show_inside(ui, |ui| {
                ui.label(egui::RichText::new("SETTINGS").strong().small().weak());
                ui.add_space(5.0);
                for category in SettingCategory::ALL {
                    let mut count = visible.iter().filter(|d| d.category() == category).count();
                    // Count the category's action card as an available result.
                    // This avoids labels such as "System (0)" while that page
                    // contains important controls including legacy import.
                    if (query.is_empty() && Self::has_settings_actions(category))
                        || (!query.is_empty() && Self::settings_action_matches(category, &query))
                    {
                        count += 1;
                    }
                    let label = if count == 0 {
                        egui::RichText::new(format!("{}  ({count})", category.label())).weak()
                    } else {
                        egui::RichText::new(format!("{}  ({count})", category.label()))
                    };
                    let selected = self.settings_category == category;
                    let response = ui.add_sized(
                        [ui.available_width(), 32.0],
                        egui::Button::new(label)
                            .selected(selected)
                            .stroke(egui::Stroke::NONE),
                    );
                    if response.clicked() {
                        // Leaving the category destroys every text field in
                        // it, and egui delivers no focus-loss event for a
                        // widget that simply stops being drawn — so anything
                        // typed and not yet committed would vanish. Write it
                        // before navigating away.
                        self.flush_pending_setting_edits();
                        Self::activate_settings_category(
                            &mut self.settings_category,
                            &mut self.settings_search,
                            category,
                        );
                    }
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(5.0);
                ui.label(
                    egui::RichText::new("Unavailable features are hidden.")
                        .small()
                        .weak(),
                );
            });
        self.tutorial_layout.settings_sidebar = Some(settings_sidebar.response.rect);

        let settings_content = egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::INK)
                    .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD)),
            )
            .show_inside(ui, |ui| {
                let category = self.settings_category;

                ui.horizontal(|ui| {
                    ui.heading(category.label());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // The field needs a stable id of its own. egui derives
                        // one from the widget's position in the layout
                        // otherwise, and the "Clear" button below is
                        // conditional: typing the first character makes it
                        // appear, which shifts the field, which changes its
                        // derived id — so egui saw a different widget and took
                        // the keyboard focus away after exactly one keystroke.
                        //
                        // Drawn before the button as well, so that in this
                        // right-to-left layout the field keeps the same
                        // position whether or not the button is there, and
                        // doesn't jump sideways as you type.
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings_search)
                                .id(settings_search_id())
                                .desired_width(220.0)
                                .hint_text("Search all settings"),
                        );
                        if !self.settings_search.is_empty()
                            && ui.button("Clear").on_hover_text("Clear search").clicked()
                        {
                            self.settings_search.clear();
                            // Hand focus back rather than leaving the caret
                            // nowhere after the button disappears.
                            ui.memory_mut(|memory| memory.request_focus(settings_search_id()));
                        }
                    });
                });
                ui.separator();

                // A search looks across every category; otherwise this is the
                // selected one.
                let mut rows: Vec<_> = if query.is_empty() {
                    visible
                        .iter()
                        .filter(|d| d.category() == category)
                        .copied()
                        .collect()
                } else {
                    visible.clone()
                };
                let show_notification_matrix =
                    query.is_empty() && category == SettingCategory::Notifications;
                let matching_action_categories: Vec<_> = if query.is_empty() {
                    Vec::new()
                } else {
                    SettingCategory::ALL
                        .into_iter()
                        .filter(|category| Self::settings_action_matches(*category, &query))
                        .collect()
                };
                if show_notification_matrix {
                    rows.retain(|descriptor| !Self::notification_matrix_id(descriptor.id()));
                }

                const SECTION_ORDER: [&str; 15] = [
                    "Appearance",
                    "Interface",
                    "Instance enforcement",
                    "Moderation",
                    "Advisories",
                    "Text to speech",
                    "Desktop notifications",
                    "Discord",
                    "Verification",
                    "Outstanding moderation",
                    "Avatar search",
                    "Cache cleanup",
                    "VRChat credentials",
                    "CLI",
                    "General",
                ];
                rows.sort_by_key(|descriptor| {
                    SECTION_ORDER
                        .iter()
                        .position(|section| *section == Self::setting_group(descriptor.id()))
                        .unwrap_or(SECTION_ORDER.len())
                });

                if rows.is_empty()
                    && !show_notification_matrix
                    && matching_action_categories.is_empty()
                {
                    ui.add_space(12.0);
                    ui.weak(if query.is_empty() {
                        "No settings in this category.".to_string()
                    } else {
                        format!("Nothing matches \"{}\".", self.settings_search.trim())
                    });
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if show_notification_matrix {
                            ui.add_space(10.0);
                            Self::render_notification_matrix(ui, &all);
                            ui.add_space(12.0);
                        }

                        let mut start = 0;
                        while start < rows.len() {
                            let group = Self::setting_group(rows[start].id());
                            let mut end = start + 1;
                            while end < rows.len() && Self::setting_group(rows[end].id()) == group {
                                end += 1;
                            }
                            Self::card_frame().show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(group)
                                        .strong()
                                        .size(crate::theme::TEXT_HEADING),
                                );
                                ui.add_space(5.0);
                                for (offset, descriptor) in rows[start..end].iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(
                                                (ui.available_width() - CONTROL_WIDTH).max(120.0),
                                            );
                                            ui.label(descriptor.name());
                                            // The raw id used to be printed here.
                                            // That tells you how the setting is
                                            // stored — the one thing someone
                                            // deciding whether to change it does
                                            // not need — while leaving "what
                                            // happens if I turn this on?"
                                            // unanswered. The description answers
                                            // it; the id stays on hover, where it
                                            // is still useful for `/settings edit`.
                                            let hover = format!(
                                                "{}\nUse this id with /settings edit in Discord",
                                                descriptor.id()
                                            );
                                            if let Some(description) =
                                                celeste_core::settings::help::describe(
                                                    descriptor.id(),
                                                )
                                            {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(description)
                                                            .size(crate::theme::TEXT_SM)
                                                            .color(crate::theme::TEXT_MUTED),
                                                    )
                                                    .wrap(),
                                                )
                                                .on_hover_text(hover);
                                            }
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                self.render_setting_control(ui, descriptor);
                                            },
                                        );
                                    });
                                    if offset + 1 < end - start {
                                        ui.add_space(3.0);
                                        ui.separator();
                                        ui.add_space(3.0);
                                    }
                                }
                            });
                            ui.add_space(10.0);
                            start = end;
                        }
                        if query.is_empty() {
                            self.render_settings_actions(ui, category);
                        } else {
                            for action_category in matching_action_categories {
                                self.render_settings_actions(ui, action_category);
                            }
                        }
                        ui.add_space(4.0);
                    });
            });
        self.tutorial_layout.settings_content = Some(settings_content.response.rect);
    }

    /// Java registers these as `FileValuedVoid` rows. They are UI actions,
    /// not stored values, so keep them beside the registry-backed controls.
    fn render_settings_actions(&mut self, ui: &mut egui::Ui, category: SettingCategory) {
        if !matches!(
            category,
            SettingCategory::General
                | SettingCategory::System
                | SettingCategory::Notifications
                | SettingCategory::Discord
                | SettingCategory::Integrations
        ) {
            return;
        }
        ui.add_space(2.0);
        Self::card_frame().show(ui, |ui| {
            ui.label(egui::RichText::new("Actions").strong().size(crate::theme::TEXT_HEADING));
            ui.add_space(5.0);
            match category {
                SettingCategory::General => {
                    ui.label("Appearance");
                    ui.horizontal(|ui| {
                        if ui.button("Set UI scale").clicked() {
                            self.ui_scale_percent = self.ui_scale * 100.0;
                            self.ui_scale_open = true;
                        }
                        if ui.button("Choose accent colour").clicked() {
                            self.accent_edit = [self.accent.r(), self.accent.g(), self.accent.b()];
                            self.accent_open = true;
                        }
                    });
                    ui.add_space(8.0);
                    ui.label("Pronoun validation lists");
                    ui.horizontal(|ui| {
                        if ui.button("Edit good list").clicked() {
                            if let Err(error) = open::that(&self.pronoun_lists.good_file) { self.state.append_cli_output(format!("Could not open good_pronoun.json: {error}")); }
                        }
                        if ui.button("Edit bad list").clicked() {
                            if let Err(error) = open::that(&self.pronoun_lists.bad_file) { self.state.append_cli_output(format!("Could not open bad_pronoun.json: {error}")); }
                        }
                        if ui.button("Reload lists").clicked() {
                            self.pronoun_lists.load();
                            let (good, bad) = self.pronoun_lists.counts();
                            let message = format!("Pronoun lists reloaded: {good} good, {bad} bad entries.");
                            self.state.append_cli_output(&message);
                            self.settings_notice = Some(message);
                        }
                    });
                    ui.label(egui::RichText::new("Edit either file, then reload it here. New player checks use the updated lists immediately.").small().weak());
                }
                SettingCategory::System => {
                    // Where the data lives is the one setting you cannot
                    // find from inside the app without being told, because
                    // it is resolved before any settings file is read.
                    ui.label("Data folder");
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.data_dir.display().to_string())
                                .size(crate::theme::TEXT_SM)
                                .color(crate::theme::TEXT_DIM),
                        )
                        .truncate(),
                    )
                    .on_hover_text(self.data_dir.display().to_string());
                    ui.horizontal(|ui| {
                        if ui.button("Open folder").clicked() {
                            if let Err(error) = open::that(&self.data_dir) {
                                self.state.append_cli_output(format!(
                                    "Could not open the data folder: {error}"
                                ));
                            }
                        }
                        if ui.button("Move data folder...").clicked() {
                            self.begin_move_data_dir();
                        }
                        if ui.button("Import Scarlet data...").clicked() {
                            self.begin_legacy_import();
                        }
                    });
                    ui.label(
                        egui::RichText::new(
                            "Settings, logins, staff lists and watched entities live here. \
                             Moving copies them to the new folder and remembers it for next \
                             launch; the old folder is left in place so you can check the \
                             move worked before deleting it. Importing Scarlet is additive: \
                             existing Celeste records win, while settings and credentials are skipped.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(8.0);
                    let cleanup_running = self.cache_cleanup_rx.is_some();
                    if ui
                        .add_enabled(!cleanup_running, egui::Button::new(if cleanup_running {
                            "Cache cleanup running..."
                        } else {
                            "Run cache cleanup now"
                        }))
                        .clicked()
                    {
                        self.prepare_cache_cleanup();
                    }
                    if ui.button("Restart Celeste").clicked() {
                        self.state.append_cli_output(if self.process_controller.restart() { "Restarting Celeste..." } else { "Restart is already queued" });
                    }
                    if ui.button("Open data folder").clicked() {
                        if let Err(error) = open::that(&self.data_dir) { self.state.append_cli_output(format!("Could not open data folder: {error}")); }
                    }
                    #[cfg(target_os = "linux")]
                    {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label("Linux application launcher");
                        ui.label(
                            egui::RichText::new(
                                "Installs a private copy for this user and adds Celeste to KDE, GNOME and other Freedesktop application menus. Run this again after replacing Celeste with a newer build.",
                            )
                            .small()
                            .weak(),
                        );
                        if ui.button("Install or refresh application launcher").clicked() {
                            let message = match crate::platform::install_desktop_launcher() {
                                Ok(message) => message,
                                Err(error) => error,
                            };
                            self.state.append_cli_output(&message);
                            self.settings_notice = Some(message);
                        }
                    }
                }
                SettingCategory::Notifications => {
                    ui.label("Audio output troubleshooting");
                    ui.label(
                        egui::RichText::new(
                            "Celeste uses the operating system's current default output device. If callouts play through the wrong speakers or headset, choose the intended default device here, then test TTS again.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(5.0);
                    if ui.button("Test TTS output").clicked() {
                        self.tts_route.submit_routed(
                            "settings-test",
                            "Celeste text to speech is using this audio output.",
                        );
                        self.state.append_cli_output("Submitted an audio-output test.");
                    }
                    ui.horizontal_wrapped(|ui| {
                        #[cfg(target_os = "windows")]
                        {
                            if ui.button("Open Windows Sound Settings").clicked() {
                                self.report_audio_settings_result(crate::platform::open_audio_settings(
                                    crate::platform::AudioSettingsPanel::System,
                                ));
                            }
                            if ui.button("Open classic Sound panel").clicked() {
                                self.report_audio_settings_result(crate::platform::open_audio_settings(
                                    crate::platform::AudioSettingsPanel::Classic,
                                ));
                            }
                        }
                        #[cfg(target_os = "linux")]
                        if ui.button("Open system sound controls").clicked() {
                            self.report_audio_settings_result(crate::platform::open_audio_settings(
                                crate::platform::AudioSettingsPanel::System,
                            ));
                        }
                    });
                    #[cfg(target_os = "linux")]
                    ui.label(
                        egui::RichText::new(
                            "Celeste tries your desktop's sound settings first, then pavucontrol and pwvucontrol. It will tell you if none are installed.",
                        )
                        .small()
                        .weak(),
                    );
                }
                SettingCategory::Discord => {
                    if ui.button("Reset Discord connection settings").on_hover_text("Clears the stored bot token and guild ID; setup will ask again next launch.").clicked() {
                        if let Some(settings) = &self.settings {
                            use celeste_core::settings::registry::registry_string_encrypted;
                            registry_string_encrypted(settings.encrypted.clone(), "discord.token").clear();
                            registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id").clear();
                            if let Err(error) = self.discord_config.set_string("guildSf", None) {
                                tracing::error!(%error, "Failed to clear the Java-compatible Discord guild setting");
                            }
                            settings.discord_setup_prompted.set(Some(false), "ui");
                            self.state.append_cli_output("Cleared Discord token and guild ID. Restart Celeste to configure them again.");
                        }
                    }
                }
                SettingCategory::Integrations => {
                    ui.label("Avatar search databases");
                    if ui.button("Manage avatar search providers").clicked() {
                        self.open_avatar_provider_manager();
                    }
                    ui.label(egui::RichText::new("Provider changes apply to new lookups immediately. World Balancer is always excluded.").small().weak());
                }
                _ => {}
            }
            if category == SettingCategory::System {
                ui.separator();
                // Changing your mind about an integration was previously
                // only possible from inside the guided tour, which is not
                // where anyone looks for it a month later.
                if ui.button("Set up Discord and VRChat").clicked() {
                    self.open_full_setup = true;
                }
                ui.label(
                    egui::RichText::new(
                        "Adds an integration you skipped at first run. Existing credentials are left alone.",
                    )
                    .small()
                    .weak(),
                );
                if ui.button("Set up VRChat on next launch").clicked() {
                    if let Some(settings) = &self.settings {
                        settings.vrchat_setup_prompted.set(Some(false), "ui");
                        self.state.append_cli_output(
                            "VRChat setup will open the next time Celeste starts. Existing credentials were left unchanged.",
                        );
                    }
                }
                if ui.button("Reset VRChat credentials and session").clicked() {
                    if let Some(settings) = &self.settings {
                        use celeste_core::settings::registry::registry_string_encrypted;
                        registry_string_encrypted(settings.encrypted.clone(), celeste_vrchat::client::USERNAME_KEY).clear();
                        registry_string_encrypted(settings.encrypted.clone(), celeste_vrchat::client::PASSWORD_KEY).clear();
                        registry_string_encrypted(settings.encrypted.clone(), celeste_vrchat::client::TOTP_SECRET_KEY).clear();
                        celeste_vrchat::client::clear_session(settings);
                        settings.vrchat_setup_prompted.set(Some(false), "ui");
                        let slot = self.vrchat_slot.clone();
                        self.runtime_handle.spawn(async move { *slot.write().await = None; });
                        self.state.refresh_vrchat_api_status("VRChat: credentials reset");
                        self.state.append_cli_output("Cleared VRChat credentials, cached 2FA secret, and session. Restart Celeste to sign in again.");
                    }
                }
            }
        });
        ui.add_space(10.0);
    }

    fn report_audio_settings_result(&mut self, result: Result<&'static str, String>) {
        let message = match result {
            Ok(panel) => {
                format!("Opened {panel}. Choose the output device you want Celeste to use.")
            }
            Err(error) => error,
        };
        self.state.append_cli_output(&message);
        self.settings_notice = Some(message);
    }

    fn open_avatar_provider_manager(&mut self) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let builtins: Vec<String> = celeste_discord::avatar_search::BUILTIN_PROVIDERS
            .iter()
            .map(|url| (*url).to_string())
            .collect();
        let selected: std::collections::HashSet<String> =
            if settings.avatar_search_providers_enabled.get() {
                settings.avatar_search_providers.get().into_iter().collect()
            } else {
                builtins.iter().cloned().collect()
            };
        let mut known = builtins;
        for url in settings.avatar_search_providers.get() {
            if !known.contains(&url) && !url.to_ascii_lowercase().contains("worldbalancer") {
                known.push(url);
            }
        }
        self.avatar_provider_choices = known
            .into_iter()
            .filter(|url| !url.to_ascii_lowercase().contains("worldbalancer"))
            .map(|url| AvatarProviderChoice {
                enabled: selected.contains(&url),
                url,
            })
            .collect();
        self.avatar_provider_input.clear();
        self.avatar_provider_error = None;
        self.avatar_providers_open = true;
    }

    fn save_avatar_provider_choices(&mut self) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let selected: Vec<String> = self
            .avatar_provider_choices
            .iter()
            .filter(|choice| choice.enabled)
            .map(|choice| choice.url.clone())
            .collect();
        let defaults: std::collections::HashSet<&str> =
            celeste_discord::avatar_search::BUILTIN_PROVIDERS
                .iter()
                .copied()
                .collect();
        let selected_set: std::collections::HashSet<&str> =
            selected.iter().map(String::as_str).collect();
        let uses_defaults = selected_set == defaults;
        settings
            .avatar_search_providers
            .set(if uses_defaults { None } else { Some(selected) }, "ui");
        settings
            .avatar_search_providers_enabled
            .set(Some(!uses_defaults), "ui");
        let search = self.avatar_search.clone();
        self.runtime_handle
            .spawn(async move { search.clear_provider_state().await });
        self.settings_notice = Some(if uses_defaults {
            "Avatar search providers restored to the built-in defaults.".to_string()
        } else {
            "Avatar search providers saved. New avatar lookups use this selection immediately."
                .to_string()
        });
    }

    fn settings_action_windows(&mut self, ctx: &egui::Context) {
        if self.ui_scale_open {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("UI scale")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(format!("Scale: {:.0}%", self.ui_scale_percent));
                    ui.add(egui::Slider::new(&mut self.ui_scale_percent, 50.0..=400.0).suffix("%"));
                    ui.horizontal(|ui| {
                        apply = ui.button("Apply").clicked();
                        if ui.button("Cancel").clicked() {
                            self.ui_scale_open = false;
                        }
                    });
                });
            if !open {
                self.ui_scale_open = false;
            }
            if apply {
                self.ui_scale = (self.ui_scale_percent / 100.0).clamp(0.5, 4.0);
                if let Some(settings) = &self.settings {
                    settings.store.set_object("ui_scale", &self.ui_scale);
                }
                ctx.set_zoom_factor(self.ui_scale);
                self.ui_scale_open = false;
                self.settings_notice =
                    Some(format!("UI scale set to {:.0}%.", self.ui_scale * 100.0));
            }
        }

        if self.accent_open {
            let mut open = true;
            let mut save = false;
            egui::Window::new("Accent colour")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Choose a preset or a custom colour.");
                    for (name, value) in [
                        ("Celeste blue", [38, 104, 170]),
                        ("Crimson", [200, 55, 65]),
                        ("Cobalt", [59, 125, 216]),
                        ("Forest", [46, 158, 91]),
                        ("Amber", [212, 130, 26]),
                        ("Violet", [124, 77, 184]),
                        ("Rose", [212, 68, 128]),
                    ] {
                        if ui
                            .selectable_label(self.accent_edit == value, name)
                            .clicked()
                        {
                            self.accent_edit = value;
                        }
                    }
                    ui.add_space(6.0);
                    ui.color_edit_button_srgb(&mut self.accent_edit);
                    ui.horizontal(|ui| {
                        save = ui.button("Apply").clicked();
                        if ui.button("Cancel").clicked() {
                            self.accent_open = false;
                        }
                    });
                });
            if !open {
                self.accent_open = false;
            }
            if save {
                self.accent = egui::Color32::from_rgb(
                    self.accent_edit[0],
                    self.accent_edit[1],
                    self.accent_edit[2],
                );
                if let Some(settings) = &self.settings {
                    settings
                        .store
                        .set_object("ui_accent_color", &self.accent_edit.to_vec());
                }
                self.visual_style_installed = false;
                self.accent_open = false;
                self.settings_notice = Some("Accent colour updated.".to_string());
            }
        }

        if self.avatar_providers_open {
            let mut open = true;
            let mut save = false;
            let mut restore = false;
            let mut add = false;
            egui::Window::new("Avatar search providers")
                .collapsible(false)
                .resizable(true)
                .default_width(620.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("Choose the VRCX-compatible databases Celeste may query.");
                    ui.label(
                        egui::RichText::new("World Balancer entries are intentionally ignored.")
                            .small()
                            .weak(),
                    );
                    ui.add_space(6.0);
                    // A database is a thing you recognise by name, not a URL
                    // you proof-read. Named services show their name with the
                    // host underneath; an endpoint the user added themselves has
                    // no name we can honestly give it, so it shows its URL.
                    for choice in &mut self.avatar_provider_choices {
                        let name = celeste_discord::avatar_search::provider_name(&choice.url);
                        ui.horizontal(|ui| {
                            let label = name.unwrap_or("Custom endpoint");
                            ui.checkbox(&mut choice.enabled, label);
                            let detail = match name {
                                Some(_) => {
                                    celeste_discord::avatar_search::provider_host(&choice.url)
                                }
                                None => choice.url.as_str(),
                            };
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(detail)
                                        .size(crate::theme::TEXT_SM)
                                        .color(crate::theme::TEXT_MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&choice.url);
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.avatar_provider_input)
                                .desired_width(430.0)
                                .hint_text("https://..."),
                        );
                        add = ui.button("Add").clicked();
                    });
                    if let Some(error) = &self.avatar_provider_error {
                        ui.colored_label(crate::theme::DANGER, error);
                    }
                    ui.horizontal(|ui| {
                        restore = ui.button("Restore defaults").clicked();
                        save = ui.button("Save").clicked();
                        if ui.button("Cancel").clicked() {
                            self.avatar_providers_open = false;
                        }
                    });
                });
            if add {
                let url = self.avatar_provider_input.trim().to_string();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    self.avatar_provider_error = Some("Enter a full http(s) URL.".to_string());
                } else if url.to_ascii_lowercase().contains("worldbalancer") {
                    self.avatar_provider_error =
                        Some("World Balancer is excluded from avatar searches.".to_string());
                } else {
                    if let Some(choice) = self
                        .avatar_provider_choices
                        .iter_mut()
                        .find(|choice| choice.url == url)
                    {
                        choice.enabled = true;
                    } else {
                        self.avatar_provider_choices
                            .push(AvatarProviderChoice { url, enabled: true });
                    }
                    self.avatar_provider_input.clear();
                    self.avatar_provider_error = None;
                }
            }
            if restore {
                let defaults: std::collections::HashSet<&str> =
                    celeste_discord::avatar_search::BUILTIN_PROVIDERS
                        .iter()
                        .copied()
                        .collect();
                for choice in &mut self.avatar_provider_choices {
                    choice.enabled = defaults.contains(choice.url.as_str());
                }
            }
            if !open {
                self.avatar_providers_open = false;
            }
            if save {
                self.save_avatar_provider_choices();
                self.avatar_providers_open = false;
            }
        }

        if self.settings_notice.is_some() {
            let message = self.settings_notice.clone().unwrap();
            let mut close = false;
            egui::Window::new("Celeste")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(message);
                    ui.add_space(8.0);
                    close = ui.button("Close").clicked();
                });
            if close {
                self.settings_notice = None;
            }
        }
    }

    fn legacy_import_result_page(&mut self, ctx: &egui::Context) {
        if let Some((success, message)) = self.legacy_import_result.clone() {
            let title = if success {
                "Scarlet import completed"
            } else {
                "Scarlet import failed"
            };
            let mut close = false;
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(crate::theme::INK)
                        .inner_margin(egui::Margin::same(32.0)),
                )
                .show(ctx, |ui| {
                    ui.add_space(36.0);
                    let width = ui.available_width().min(720.0);
                    let side = ((ui.available_width() - width) / 2.0).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            ui.label(
                                egui::RichText::new(if success {
                                    "MIGRATION COMPLETE"
                                } else {
                                    "MIGRATION NEEDS ATTENTION"
                                })
                                .small()
                                .strong()
                                .color(if success {
                                    crate::theme::OK
                                } else {
                                    crate::theme::WARN
                                }),
                            );
                            ui.heading(title);
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(16.0);
                            ui.label(message);
                            if success {
                                ui.add_space(18.0);
                                ui.colored_label(
                                    crate::theme::WARN,
                                    "Credentials were intentionally not transferred. Re-enter VRChat or Discord credentials if prompted, then double-check the imported lists and history before relying on them.",
                                );
                            }
                            ui.add_space(24.0);
                            close = ui
                                .add(self.primary_button(if success {
                                    "I’ve checked the result"
                                } else {
                                    "Continue"
                                }))
                                .clicked();
                        });
                    });
                });
            if close {
                self.legacy_import_result = None;
            }
        }
    }

    /// Check what is working and what is not, in terms the operator can
    /// act on.
    ///
    /// Deliberately separate from `diagnostics_report`, which lists state.
    /// State tells you what Celeste is configured with and leaves you to
    /// work out which part is wrong; this names the problem and the fix.
    fn readiness_report(&self) -> celeste_core::readiness::Report {
        use celeste_core::readiness::{Finding, Report};
        let mut report = Report::default();

        match self.tts.engine_name() {
            Some(engine) => {
                report.push(Finding::ok("Text to speech", format!("Using {engine}.")))
            }
            None => report.push(Finding::blocked(
                "Text to speech",
                "No speech engine is installed, so announcements cannot be spoken.",
                if cfg!(target_os = "linux") {
                    "Install eSpeak NG (`espeak-ng`), Flite, or speech-dispatcher."
                } else if cfg!(target_os = "windows") {
                    "Windows SAPI voices are missing; add one under Settings, Time & language, Speech."
                } else {
                    "macOS provides `say` as standard; check it is on PATH."
                },
            )),
        }

        // Speech can fall back to letting the engine play for itself, so a
        // missing player is not fatal - but it silently costs the chimes.
        match self.tts.alert_player_name() {
            Some(player) => report.push(Finding::ok(
                "Alert chimes",
                format!("Playing through {player}."),
            )),
            None => report.push(Finding::degraded(
                "Alert chimes",
                "No program was found to play a sound file, so alert chimes are silent. Spoken announcements still work.",
                if cfg!(target_os = "linux") {
                    "Install pipewire for `pw-play`, or libpulse for `paplay`. `CELESTE_TTS_PLAYER` overrides the choice."
                } else {
                    "PowerShell is required to play the bundled chimes."
                },
            )),
        }

        // Whether a missing integration is a fault depends on what the
        // operator asked for at setup. Someone who chose the local logger
        // has not lost anything by having no Discord bot; someone who
        // chose full setup and has no bot has a broken install. Without a
        // stored choice there is nothing to hold them to, so an absent
        // integration is reported as a choice rather than a fault.
        let profile = self
            .settings
            .as_ref()
            .and_then(|settings| settings.store.get_object::<String>("setup_profile"));
        let wanted = |integration: &str| -> bool {
            match profile.as_deref() {
                Some("full") => true,
                Some("vrchat_only") => integration == "vrchat",
                Some("discord_only") => integration == "discord",
                Some("local_logger") => false,
                // No recorded choice: do not invent an expectation.
                _ => false,
            }
        };
        const SWITCH: &str =
            "Settings, System, \"Set up Discord and VRChat\" adds an integration you skipped.";

        match self.vrchat_slot.try_read() {
            Ok(slot) if slot.is_some() => {
                report.push(Finding::ok("VRChat", "Signed in."))
            }
            Ok(_) if wanted("vrchat") => report.push(Finding::blocked(
                "VRChat",
                "Not signed in, so group bans, audit polling, and profile checks are unavailable.",
                "Sign in from Settings, or re-run VRChat setup if the stored session expired.",
            )),
            Ok(_) => report.push(Finding::ok(
                "VRChat",
                format!("Not signed in; reading local logs only. {SWITCH}"),
            )),
            // A locked slot means a sign-in is in flight; calling that a
            // failure would make the check flap during startup.
            Err(_) => report.push(Finding::ok("VRChat", "Signing in.")),
        }

        let token_configured = self.integration_setup_status().0;
        match (self.discord_http.try_read().map(|http| http.is_some()), token_configured) {
            (Ok(true), _) => report.push(Finding::ok("Discord", "Connected.")),
            (Ok(false), true) => report.push(Finding::blocked(
                "Discord",
                "A bot token is stored but Celeste is not connected, so no commands or event posts will work.",
                "Check the token is still valid and that the bot has not been removed from the server.",
            )),
            (Ok(false), false) if wanted("discord") => report.push(Finding::blocked(
                "Discord",
                "This install was set up to use Discord, but no bot token is stored, so commands and event posts will not work.",
                SWITCH,
            )),
            (Ok(false), false) => report.push(Finding::ok(
                "Discord",
                format!("Not configured; running as a local logger. {SWITCH}"),
            )),
            (Err(_), _) => report.push(Finding::ok("Discord", "Connecting.")),
        }

        if let Some(settings) = &self.settings {
            let group = settings
                .store
                .get_string(celeste_vrchat::client::GROUP_ID_KEY)
                .unwrap_or_default();
            if group.trim().is_empty() {
                report.push(Finding::degraded(
                    "VRChat group",
                    "No group is configured, so group moderation and audit events are unavailable.",
                    "Set the group in Settings, or re-run setup.",
                ));
            } else {
                report.push(Finding::ok("VRChat group", format!("Watching {group}.")));
            }
        }

        report.push(Finding::ok(
            "Logs",
            format!("Writing to {}.", self.data_dir.join("logs").display()),
        ));

        report
    }

    fn diagnostics_report(&self) -> String {
        let vrchat = self
            .vrchat_slot
            .try_read()
            .map(|slot| {
                if slot.is_some() {
                    "connected"
                } else {
                    "not connected"
                }
            })
            .unwrap_or("busy");
        let discord = self
            .discord_http
            .try_read()
            .map(|http| {
                if http.is_some() {
                    "connected"
                } else {
                    "not connected / staff mode"
                }
            })
            .unwrap_or("busy");
        let mut report = self.readiness_report().to_text();
        report.push('\n');
        report.push_str(&format!(
            "Celeste {}\nData folder: {}\n\nVRChat\n  Session: {}\n  Status: {}\n\nDiscord\n  Gateway/HTTP: {}\n\nText to speech\n  Local engine: {}\n",
            celeste_core::identity::VERSION,
            self.data_dir.display(),
            vrchat,
            self.state.connection_status(),
            discord,
            self.tts.engine_name().unwrap_or("not installed"),
        ));
        if let Some(settings) = &self.settings {
            let last_audit = settings
                .last_audit_query
                .get_or_null()
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| "never".to_string());
            report.push_str(&format!(
                "\nAudit log\n  Polling interval: {}s\n  Last query cursor: {}\n",
                settings.audit_polling_interval.get(),
                last_audit
            ));
            let providers: Vec<String> = if settings.avatar_search_providers_enabled.get() {
                settings.avatar_search_providers.get()
            } else {
                celeste_discord::avatar_search::BUILTIN_PROVIDERS
                    .iter()
                    .map(|url| (*url).to_string())
                    .collect()
            };
            let backoffs = self.avatar_search.provider_backoffs();
            report.push_str("\nAvatar search providers\n");
            for provider in providers
                .into_iter()
                .filter(|url| !url.to_ascii_lowercase().contains("worldbalancer"))
            {
                let status = backoffs
                    .as_ref()
                    .and_then(|states| states.get(&provider))
                    .filter(|seconds| **seconds > 0)
                    .map(|seconds| format!("backoff {seconds}s"))
                    .unwrap_or_else(|| "ready".to_string());
                report.push_str(&format!("  [{status}] {provider}\n"));
            }
            if backoffs.is_none() {
                report.push_str("  (provider status is busy; no request was delayed)\n");
            }
        }
        report
    }

    fn start_tutorial(&mut self, ctx: &egui::Context, step: usize) {
        // Give the tour the whole screen. The spotlight points at real UI,
        // and in a small window the areas it describes are cramped or
        // scrolled out of view before the tour can highlight them.
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        let step = step.min(TUTORIAL_STEPS.len().saturating_sub(1));
        self.tutorial_step = Some(step);
        self.tutorial_narrated_step = None;
        self.tutorial_revealed_example_guide = None;
        self.tutorial_tts_notice = None;
        self.record_tutorial_progress(step);
        if self.tutorial_demo.is_none() {
            self.tutorial_demo = TutorialDemo::create();
            if self.tutorial_demo.is_none() {
                tracing::warn!("Could not create isolated guided-tour demonstration data");
            }
        }
    }

    fn tutorial_voice_label(voice: &str) -> String {
        if voice.trim().is_empty() {
            return "System default".to_string();
        }
        let parsed = celeste_core::tts_voices::VoiceId::parse(voice);
        if parsed.engine.is_empty() {
            parsed.voice
        } else {
            format!("{} ({})", parsed.voice, parsed.engine)
        }
    }

    fn tutorial_spoken_text(step: TutorialStep) -> String {
        let mut text = format!("{}. {}", step.title, step.body);
        if step.demo_data {
            text.push_str(
                " The information shown is temporary demonstration data, not your Celeste records.",
            );
        }
        if !step.try_it.trim().is_empty() {
            text.push_str(" Try or notice this. ");
            text.push_str(step.try_it);
        }
        text
    }

    fn tutorial_scenario_introduction(step: TutorialStep, mode: TutorialRosterMode) -> String {
        let opening = match mode {
            TutorialRosterMode::Lightweight => {
                "Here is how a normal, moderated, but light instance looks and sounds."
            }
            TutorialRosterMode::Busy => {
                "Now here is how a busy instance looks as its risk level changes."
            }
        };
        format!(
            "{opening} {} I will pause while the example plays. Watch each row appear and listen for Celeste's normal local callouts and alert sounds.",
            step.body
        )
    }

    fn tutorial_scenario_wrap_up(step: TutorialStep, mode: TutorialRosterMode) -> String {
        format!(
            "That completes the {} example. {} The information you just saw was temporary demonstration data, not your Celeste records.",
            match mode {
                TutorialRosterMode::Lightweight => "lightly populated instance",
                TutorialRosterMode::Busy => "busy and higher-risk instance",
            },
            step.try_it
        )
    }

    fn speak_tutorial_step(&mut self, index: usize) {
        let Some(step) = TUTORIAL_STEPS.get(index).copied() else {
            return;
        };
        self.tts.skip();
        self.tutorial_revealed_example_guide = None;
        let spoken = if let Some(mode) = Self::tutorial_roster_mode(index) {
            self.tutorial_scenario_phase =
                Some((index, TutorialScenarioPhase::WaitingForIntroduction));
            if let Some(demo) = self.tutorial_demo.as_mut() {
                // Force the live example to restart only after its spoken
                // introduction has actually completed.
                demo.roster_scene = None;
                demo.roster_mode = None;
            }
            Self::tutorial_scenario_introduction(step, mode)
        } else {
            self.tutorial_scenario_phase = None;
            Self::tutorial_spoken_text(step)
        };
        self.tutorial_tts_notice = self
            .tts
            .submit_accessibility(spoken, self.tutorial_voice.clone())
            .err();
        self.tutorial_narrated_step = Some(index);
    }

    fn stop_tutorial_narration(&mut self) {
        self.tts.skip();
        self.tutorial_narrated_step = None;
        self.tutorial_scenario_phase = None;
        self.tutorial_revealed_example_guide = None;
    }

    /// Remember the furthest step reached, so leaving the tour does not
    /// throw the progress away. Deliberately a high-water mark: stepping
    /// back to re-read something and then quitting should not rewind what
    /// Help offers to resume.
    fn record_tutorial_progress(&self, step: usize) {
        let Some(settings) = &self.settings else {
            return;
        };
        let step = step as i32;
        if settings.tutorial_progress_step.get() < step {
            settings
                .tutorial_progress_step
                .set(Some(step), "guided-tour");
        }
    }

    /// The step Help should resume at, if the tour was left part-way and
    /// is not already finished.
    fn tutorial_resume_step(&self) -> Option<usize> {
        let settings = self.settings.as_ref()?;
        let reached = settings.tutorial_progress_step.get();
        if reached < 0 {
            return None;
        }
        let reached = reached as usize;
        (reached + 1 < TUTORIAL_STEPS.len()).then_some(reached)
    }

    /// Whether this step is shown, honouring the "Show anyway" override.
    fn tutorial_step_applies(&self, index: usize) -> bool {
        self.tutorial_show_all
            || TUTORIAL_STEPS
                .get(index)
                .is_some_and(|step| self.tutorial_chapter_applies(step.chapter))
    }

    /// Next shown step after `from`, skipping chapters that do not apply.
    fn tutorial_next_step(&self, from: usize) -> Option<usize> {
        ((from + 1)..TUTORIAL_STEPS.len()).find(|index| self.tutorial_step_applies(*index))
    }

    /// Previous shown step before `from`.
    fn tutorial_prev_step(&self, from: usize) -> Option<usize> {
        (0..from)
            .rev()
            .find(|index| self.tutorial_step_applies(*index))
    }

    /// True when `from` is the last step of the essential path, so the tour
    /// has somewhere natural to stop rather than running an operator-only
    /// chapter at a first-time user.
    fn tutorial_ends_essentials(&self, from: usize) -> bool {
        let Some(step) = TUTORIAL_STEPS.get(from) else {
            return false;
        };
        if !step.chapter.essential() {
            return false;
        }
        self.tutorial_next_step(from)
            .and_then(|next| TUTORIAL_STEPS.get(next))
            .is_none_or(|next| !next.chapter.essential())
    }

    /// Whether a chapter describes something this install has configured.
    ///
    /// Blake's rule: go by what the operator actually chose at setup, and
    /// if they never chose, show everything. So a chapter is only held back
    /// when a setup profile was explicitly stored and excludes it - a
    /// detected-but-unchosen state is not treated as a decision, and the
    /// tour never silently hides a feature from someone who never said
    /// they did not want it.
    fn tutorial_chapter_applies(&self, chapter: TutorialChapter) -> bool {
        let needs = chapter.needs();
        if needs == TutorialNeeds::Nothing {
            return true;
        }
        let Some(settings) = &self.settings else {
            return true;
        };
        let Some(profile) = settings.store.get_object::<String>("setup_profile") else {
            // No profile was ever chosen: show everything.
            return true;
        };
        match (profile.as_str(), needs) {
            ("local_logger", _) => false,
            ("vrchat_only", TutorialNeeds::Discord) => false,
            ("discord_only", TutorialNeeds::VrChat) => false,
            _ => true,
        }
    }

    fn tutorial_uses_demo_data(&self) -> bool {
        self.tutorial_step
            .and_then(|index| TUTORIAL_STEPS.get(index))
            .is_some_and(|step| step.demo_data)
            && self.tutorial_demo.is_some()
    }

    fn tutorial_roster_mode(index: usize) -> Option<TutorialRosterMode> {
        let step = TUTORIAL_STEPS.get(index)?;
        if step.chapter != TutorialChapter::InstanceAndRoster || !step.demo_data {
            return None;
        }
        let within_chapter = step
            .chapter
            .steps()
            .iter()
            .position(|step| *step == index)?;
        match within_chapter {
            // The instance-detection lesson uses the stable fixture. The two
            // roster lessons are the narrated, live examples.
            1 => Some(TutorialRosterMode::Lightweight),
            2 => Some(TutorialRosterMode::Busy),
            _ => None,
        }
    }

    fn tutorial_example_is_playing(&self, index: usize) -> bool {
        let Some(mode) = Self::tutorial_roster_mode(index) else {
            return false;
        };
        if self.tutorial_narration {
            return matches!(
                self.tutorial_scenario_phase,
                Some((step, TutorialScenarioPhase::PlayingExample)) if step == index
            );
        }
        self.tutorial_demo
            .as_ref()
            .is_some_and(|demo| !demo.roster_complete(mode))
    }

    fn tutorial_state(&self) -> UiState {
        if self.tutorial_uses_demo_data() {
            self.tutorial_demo
                .as_ref()
                .map(|demo| demo.state.clone())
                .unwrap_or_else(|| self.state.clone())
        } else {
            self.state.clone()
        }
    }

    fn integration_setup_status(&self) -> (bool, bool, &'static str) {
        let Some(settings) = &self.settings else {
            return (false, false, "Local moderation logger");
        };
        let secret = |key| {
            celeste_core::settings::registry::registry_string_encrypted(
                settings.encrypted.clone(),
                key,
            )
            .get_or_null()
            .is_some_and(|value| !value.trim().is_empty())
        };
        let discord = secret("discord.token");
        let vrchat = secret("global:username") && secret("global:password");
        let stored = settings.store.get_object::<String>("setup_profile");
        let label = match stored.as_deref() {
            Some("full") => "Full setup",
            Some("vrchat_only") => "VRChat only",
            Some("discord_only") => "Discord only",
            Some("local_logger") => "Local moderation logger",
            _ if discord && vrchat => "Full setup (detected)",
            _ if vrchat => "VRChat only (detected)",
            _ if discord => "Discord only (detected)",
            _ => "Local moderation logger (detected)",
        };
        (discord, vrchat, label)
    }

    /// Where to put the tour window so it does not cover what it is
    /// pointing at.
    ///
    /// The window was anchored to the top right regardless of the
    /// spotlight, so any step describing something on the right - the
    /// toolbar, the rightmost columns, Help - was explaining a thing it
    /// was sitting on top of. This picks the opposite side of the screen
    /// from the target instead. A target spanning most of the width (the
    /// player table) cannot be dodged sideways, so those are dodged
    /// vertically.
    fn tutorial_window_anchor(
        ctx: &egui::Context,
        target_kind: TutorialTarget,
        target: Option<egui::Rect>,
    ) -> (egui::Align2, [f32; 2]) {
        const SIDE: f32 = 24.0;
        // Clears the header and tab bar when the window sits at the top.
        const TOP: f32 = 118.0;
        const BOTTOM: f32 = -24.0;

        // These areas are deliberately broad. Geometry alone cannot find a
        // non-overlapping side, but their useful controls and sample rows
        // are concentrated toward the top/left. The quiet bottom-right
        // corner keeps the explanation visible without covering the exact
        // buttons, table values or action card the text asks the user to
        // inspect.
        if let Some(anchor) = Self::tutorial_forced_anchor(target_kind) {
            return (anchor, [-SIDE, BOTTOM]);
        }

        let _ = TOP;
        Self::tutorial_anchor_for(ctx.screen_rect(), target)
    }

    fn tutorial_forced_anchor(target: TutorialTarget) -> Option<egui::Align2> {
        matches!(
            target,
            TutorialTarget::PlayerTable
                | TutorialTarget::ToolbarControls
                | TutorialTarget::SettingsContent
        )
        .then_some(egui::Align2::RIGHT_BOTTOM)
    }

    /// Geometry half of [`Self::tutorial_window_anchor`], split out so the
    /// placement rules can be tested without an egui context.
    fn tutorial_anchor_for(
        screen: egui::Rect,
        target: Option<egui::Rect>,
    ) -> (egui::Align2, [f32; 2]) {
        const SIDE: f32 = 24.0;
        const TOP: f32 = 118.0;
        const BOTTOM: f32 = -24.0;

        let Some(target) = target else {
            return (egui::Align2::RIGHT_TOP, [-SIDE, TOP]);
        };

        if target.width() > screen.width() * 0.6 {
            // Too wide to sit beside: go above or below it instead.
            if target.center().y < screen.center().y {
                (egui::Align2::RIGHT_BOTTOM, [-SIDE, BOTTOM])
            } else {
                (egui::Align2::RIGHT_TOP, [-SIDE, TOP])
            }
        } else if target.center().x > screen.center().x {
            (egui::Align2::LEFT_TOP, [SIDE, TOP])
        } else {
            (egui::Align2::RIGHT_TOP, [-SIDE, TOP])
        }
    }

    /// The chapter chooser: shown after the essential path, and reachable
    /// from Help so a single chapter can be revisited without replaying
    /// the whole tour.
    fn tutorial_menu_window(&mut self, ctx: &egui::Context) {
        if !self.tutorial_menu {
            return;
        }
        let mut start: Option<usize> = None;
        let mut close = false;
        let mut show_all = self.tutorial_show_all;
        let any_hidden = TutorialChapter::ORDER
            .iter()
            .any(|chapter| !self.tutorial_chapter_applies(*chapter));

        egui::Window::new("Guided tour — chapters")
            .id(egui::Id::new("celeste-guided-tour-menu"))
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(egui::Align2::RIGHT_TOP, [-24.0, 118.0])
            .show(ctx, |ui| {
                ui.label("Pick a chapter to run on its own. Essentials cover everything needed to moderate; the advanced chapters are for operators.");
                ui.add_space(10.0);
                for chapter in TutorialChapter::ORDER {
                    let applies = self.tutorial_chapter_applies(chapter);
                    let steps = chapter.steps();
                    if steps.is_empty() {
                        continue;
                    }
                    Self::inset_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(chapter.title()).strong());
                            if !chapter.essential() {
                                ui.colored_label(crate::theme::INFO, "advanced");
                            }
                            if !applies {
                                ui.colored_label(crate::theme::WARN, "not part of your setup");
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.weak(format!("{} step{}", steps.len(), if steps.len() == 1 { "" } else { "s" }));
                            if (applies || show_all) && ui.button("Run chapter").clicked() {
                                start = steps.first().copied();
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
                if any_hidden {
                    ui.add_space(4.0);
                    ui.checkbox(&mut show_all, "Show chapters that are not part of my setup");
                }
                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });

        self.tutorial_show_all = show_all;
        if let Some(step) = start {
            self.tutorial_menu = false;
            self.start_tutorial(ctx, step);
            ctx.request_repaint();
        } else if close {
            self.tutorial_menu = false;
            self.tutorial_demo = None;
            self.tutorial_narration = false;
            self.stop_tutorial_narration();
            ctx.request_repaint();
        }
    }

    fn tutorial_window(&mut self, ctx: &egui::Context) {
        self.tutorial_menu_window(ctx);
        let Some(index) = self.tutorial_step else {
            return;
        };
        let Some(step) = TUTORIAL_STEPS.get(index).copied() else {
            self.tutorial_step = None;
            return;
        };
        if self.tutorial_narration && self.tutorial_narrated_step != Some(index) {
            self.speak_tutorial_step(index);
        }

        // Open the area being described rather than asking a new user to
        // find it from prose alone.
        self.tab = step.tab;
        let target = match step.target {
            TutorialTarget::Header => self.tutorial_layout.header,
            TutorialTarget::TabBar => self.tutorial_layout.tab_bar,
            TutorialTarget::MainContent => self.tutorial_layout.main_content,
            TutorialTarget::InstanceCard => self.tutorial_layout.instance_card,
            TutorialTarget::PlayerTable => self.tutorial_layout.player_table,
            TutorialTarget::ToolbarControls => self.tutorial_layout.toolbar_controls,
            TutorialTarget::SettingsSidebar => self.tutorial_layout.settings_sidebar,
            TutorialTarget::SettingsContent => self.tutorial_layout.settings_content,
            TutorialTarget::StatusBar => self.tutorial_layout.status_bar,
            TutorialTarget::Help => self.tutorial_layout.help,
        };

        if let Some(target) = target {
            let screen = ctx.screen_rect();
            let target = target.expand(6.0).intersect(screen.shrink(2.0));
            let shade = egui::Color32::from_black_alpha(205);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("celeste-guided-tour-spotlight"),
            ));
            for rect in [
                egui::Rect::from_min_max(screen.min, egui::pos2(screen.max.x, target.min.y)),
                egui::Rect::from_min_max(egui::pos2(screen.min.x, target.max.y), screen.max),
                egui::Rect::from_min_max(
                    egui::pos2(screen.min.x, target.min.y),
                    egui::pos2(target.min.x, target.max.y),
                ),
                egui::Rect::from_min_max(
                    egui::pos2(target.max.x, target.min.y),
                    egui::pos2(screen.max.x, target.max.y),
                ),
            ] {
                if rect.is_positive() {
                    painter.rect_filled(rect, 0.0, shade);
                }
            }
            painter.rect_stroke(
                target.expand(2.0),
                crate::theme::RADIUS_PANEL + 2.0,
                egui::Stroke::new(5.0_f32, self.accent.gamma_multiply(0.28)),
            );
            painter.rect_stroke(
                target,
                crate::theme::RADIUS_PANEL,
                egui::Stroke::new(2.0_f32, self.accent),
            );
            painter.text(
                target.left_top() + egui::vec2(2.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                "LOOK HERE",
                egui::FontId::proportional(12.0),
                self.accent,
            );
        }

        let example_playing = self.tutorial_example_is_playing(index);
        if example_playing && self.tutorial_revealed_example_guide != Some(index) {
            let mut reveal = false;
            let mut exit = false;
            let panel = crate::theme::PANEL;
            egui::Window::new("Tutorial example playing")
                .id(egui::Id::new("celeste-guided-tour-example-strip"))
                .order(egui::Order::Foreground)
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 164.0])
                .frame(
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgba_unmultiplied(
                            panel.r(),
                            panel.g(),
                            panel.b(),
                            218,
                        ))
                        .stroke(egui::Stroke::new(1.0_f32, self.accent.gamma_multiply(0.7)))
                        .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Live example playing — follow the roster");
                        reveal = ui.small_button("Show guide").clicked();
                        exit = ui.small_button("Exit tour").clicked();
                    });
                });
            if reveal {
                self.tutorial_revealed_example_guide = Some(index);
            }
            if exit {
                self.tutorial_narration = false;
                self.stop_tutorial_narration();
                self.tutorial_step = None;
                self.tutorial_menu = false;
                self.tutorial_demo = None;
            }
            return;
        }

        let mut previous = false;
        let mut next = false;
        let mut close = false;
        let mut complete_full_setup = false;
        let mut narration_enabled = self.tutorial_narration;
        let mut selected_voice = self.tutorial_voice.clone();
        let mut test_voice = false;
        let mut replay_step = false;
        let mut hide_example_guide = false;
        let tts_available = self.tts.engine_name().is_some();
        let mut voices = celeste_core::tts_voices::installed();
        voices.insert(0, String::new());
        let (discord_configured, vrchat_configured, setup_label) = self.integration_setup_status();
        let chapter = step.chapter;
        let ends_essentials = self.tutorial_ends_essentials(index);
        let last_step = self.tutorial_next_step(index).is_none();
        let has_previous = self.tutorial_prev_step(index).is_some();
        let chapter_steps = chapter.steps();
        let within = chapter_steps
            .iter()
            .position(|i| *i == index)
            .map(|i| i + 1)
            .unwrap_or(1);
        let (anchor_align, anchor_offset) = Self::tutorial_window_anchor(ctx, step.target, target);
        egui::Window::new(format!(
            "Guided tour — {} ({} of {}){}",
            chapter.title(),
            within,
            chapter_steps.len(),
            if chapter.essential() { "" } else { "  ·  advanced" }
        ))
            .id(egui::Id::new("celeste-guided-tour"))
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .anchor(anchor_align, anchor_offset)
            .show(ctx, |ui| {
                ui.heading(step.title);
                ui.add_space(8.0);
                ui.label(step.body);
                if index == 0 {
                    ui.add_space(10.0);
                    Self::inset_frame().show(ui, |ui| {
                        ui.label(egui::RichText::new(setup_label).strong());
                        ui.label(format!(
                            "Discord bot: {}",
                            if discord_configured { "configured" } else { "not configured" }
                        ));
                        ui.label(format!(
                            "VRChat API: {}",
                            if vrchat_configured { "configured" } else { "not configured" }
                        ));
                        ui.label("Local VRChat log monitoring: available without either sign-in");
                    });
                    if (!discord_configured || !vrchat_configured)
                        && ui.button("Complete full setup…").clicked()
                    {
                        complete_full_setup = true;
                    }
                }
                if step.demo_data {
                    ui.add_space(8.0);
                    ui.colored_label(
                        crate::theme::WARN,
                        "DEMO DATA — temporary tutorial JSON, not your Celeste records",
                    );
                    if let Some(mode) = Self::tutorial_roster_mode(index) {
                        ui.add_space(6.0);
                        Self::inset_frame().show(ui, |ui| {
                            ui.label(egui::RichText::new(mode.title()).strong());
                            ui.label(mode.description());
                            ui.label(
                                egui::RichText::new(
                                    "With narration enabled, Celeste introduces the scene, pauses for the live example, plays the same local callout order plus the nuisance and mixed-character alert clips used during monitoring, then resumes the tour.",
                                )
                                .small()
                                .color(crate::theme::TEXT_MUTED),
                            );
                            if let Some((phase_step, phase)) = self.tutorial_scenario_phase {
                                if phase_step == index && self.tutorial_narration {
                                    ui.colored_label(
                                        crate::theme::INFO,
                                        match phase {
                                            TutorialScenarioPhase::WaitingForIntroduction => {
                                                "Narrator is introducing this example…"
                                            }
                                            TutorialScenarioPhase::PlayingExample => {
                                                "Example is playing — watch the roster and listen…"
                                            }
                                            TutorialScenarioPhase::WaitingForWrapUp => {
                                                "Example finished — narrator is continuing…"
                                            }
                                            TutorialScenarioPhase::Complete => {
                                                "Narrated example complete"
                                            }
                                        },
                                    );
                                }
                            }
                        });
                    }
                }
                ui.add_space(12.0);
                Self::inset_frame().show(ui, |ui| {
                    ui.label(egui::RichText::new("Try or notice this").strong());
                    ui.label(step.try_it);
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Tour narration").strong());
                ui.add_enabled_ui(tts_available, |ui| {
                    ui.checkbox(&mut narration_enabled, "Read each tour step aloud");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Voice:");
                        egui::ComboBox::from_id_salt("guided-tour-voice")
                            .selected_text(Self::tutorial_voice_label(&selected_voice))
                            .width(190.0)
                            .show_ui(ui, |ui| {
                                for voice in &voices {
                                    ui.selectable_value(
                                        &mut selected_voice,
                                        voice.clone(),
                                        Self::tutorial_voice_label(voice),
                                    );
                                }
                            });
                        test_voice = ui.button("Test voice").clicked();
                        replay_step = ui
                            .add_enabled(narration_enabled, egui::Button::new("Replay step"))
                            .clicked();
                    });
                });
                if !tts_available {
                    ui.colored_label(
                        crate::theme::WARN,
                        "No supported local TTS engine was detected. The written tour remains available.",
                    );
                } else {
                    ui.label(
                        egui::RichText::new(
                            "Narration uses this computer's audio output and is never sent to Discord.",
                        )
                        .small()
                        .color(crate::theme::TEXT_MUTED),
                    );
                }
                if let Some(notice) = &self.tutorial_tts_notice {
                    ui.colored_label(crate::theme::WARN, notice);
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    previous = ui
                        .add_enabled(has_previous, egui::Button::new("Back"))
                        .clicked();
                    next = ui
                        .button(if ends_essentials {
                            "Finish essentials"
                        } else if last_step {
                            "Finish tour"
                        } else {
                            "Next"
                        })
                        .clicked();
                    if ui.button("Exit tour").clicked() {
                        close = true;
                    }
                    if example_playing && ui.button("Hide guide").clicked() {
                        hide_example_guide = true;
                    }
                });
            });

        if hide_example_guide {
            self.tutorial_revealed_example_guide = None;
            ctx.request_repaint();
            return;
        }

        let narration_changed = narration_enabled != self.tutorial_narration;
        let voice_changed = selected_voice != self.tutorial_voice;
        self.tutorial_narration = narration_enabled;
        if voice_changed {
            self.tutorial_voice = selected_voice;
            if let Some(settings) = &self.settings {
                settings
                    .store
                    .set_object("tutorial_tts_voice", &self.tutorial_voice);
            }
        }
        if !self.tutorial_narration {
            if narration_changed {
                self.stop_tutorial_narration();
            }
        } else if test_voice {
            self.tts.skip();
            self.tutorial_tts_notice = self
                .tts
                .submit_accessibility(
                    "Hello. This is how I will sound while reading Celeste's guided tour.",
                    self.tutorial_voice.clone(),
                )
                .err();
        } else if replay_step || narration_changed || voice_changed {
            self.speak_tutorial_step(index);
        }

        if complete_full_setup {
            if let Some(settings) = self.settings.clone() {
                let mut wizard = crate::setup::SetupWizard::for_full_setup(
                    &settings,
                    self.runtime_handle.clone(),
                );
                if !wizard.is_done() {
                    wizard.resize_for_current_stage_next_frame();
                    self.resume_tutorial_step = Some(index);
                    self.tutorial_setup_had_discord = discord_configured;
                    self.stop_tutorial_narration();
                    self.tutorial_step = None;
                    self.tutorial_demo = None;
                    self.wizard = Some(wizard);
                    ctx.request_repaint();
                    return;
                }
            }
        }

        if close {
            self.tutorial_narration = false;
            self.stop_tutorial_narration();
            self.tutorial_step = None;
            self.tutorial_menu = false;
            self.tutorial_demo = None;
        } else if next {
            self.stop_tutorial_narration();
            // Finishing the essential path opens the chapter chooser rather
            // than dropping a first-time user straight into operator-only
            // material - or ending the tour before they know it exists.
            match self.tutorial_next_step(index) {
                Some(following) if !self.tutorial_ends_essentials(index) => {
                    self.tutorial_step = Some(following);
                    self.record_tutorial_progress(following);
                }
                _ => {
                    self.tutorial_step = None;
                    self.tutorial_menu = true;
                    self.record_tutorial_progress(TUTORIAL_STEPS.len().saturating_sub(1));
                }
            }
            ctx.request_repaint();
        } else if previous {
            self.stop_tutorial_narration();
            if let Some(earlier) = self.tutorial_prev_step(index) {
                self.tutorial_step = Some(earlier);
            }
            ctx.request_repaint();
        }
    }

    /// Honour a Settings request to run the full setup wizard.
    ///
    /// Settings only raises a flag: the wizard needs the runtime handle
    /// and replaces the whole view, which is not something to do from
    /// inside the settings list's closure.
    fn open_full_setup_if_requested(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.open_full_setup) {
            return;
        }
        let Some(settings) = self.settings.clone() else { return };
        let mut wizard =
            crate::setup::SetupWizard::for_full_setup(&settings, self.runtime_handle.clone());
        if wizard.is_done() {
            self.state
                .append_cli_output("Discord and VRChat are already set up; nothing to add.");
            return;
        }
        wizard.resize_for_current_stage_next_frame();
        self.wizard = Some(wizard);
        ctx.request_repaint();
    }

    fn utility_windows(&mut self, ctx: &egui::Context) {
        self.open_full_setup_if_requested(ctx);
        if self.clear_instance_confirm {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Clear current instance?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Clear every player row from the current Instance view?");
                    ui.label(
                        egui::RichText::new(
                            "New VRChat log events will continue to populate the table.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        confirm = ui.button("Clear instance").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if confirm {
                self.state.clear_instance();
                self.state.append_cli_output("Instance cleared.");
                self.clear_instance_confirm = false;
            } else if cancel {
                self.clear_instance_confirm = false;
            }
        }

        if self.diagnostics_open {
            let report = self.diagnostics_report();
            let mut open = true;
            egui::Window::new("Diagnostics")
                .default_width(680.0)
                .default_height(440.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Copy report").clicked() {
                            ui.ctx().copy_text(report.clone());
                        }
                        if ui.button("Open data folder").clicked() {
                            if let Err(error) = open::that(&self.data_dir) {
                                self.state.append_cli_output(format!(
                                    "Could not open data folder: {error}"
                                ));
                            }
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.monospace(&report);
                    });
                });
            if !open {
                self.diagnostics_open = false;
            }
        }

        if self.credits_open {
            let credits: Vec<serde_json::Value> =
                serde_json::from_str(include_str!("../resources/credits.json")).unwrap_or_default();
            let mut open = true;
            egui::Window::new("Credits / About Celeste")
                .default_width(650.0)
                .default_height(420.0)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.heading(format!("Celeste {}", celeste_core::identity::VERSION));
                    ui.label(celeste_core::identity::FORK_NOTE);
                    ui.label("A VRChat group moderation logger.");
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for credit in &credits {
                            let name = credit
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("Unknown");
                            let role = credit
                                .get("role")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("");
                            let url = credit
                                .get("url")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("");
                            ui.horizontal_wrapped(|ui| {
                                if url.is_empty() {
                                    ui.strong(name);
                                } else if ui.link(name).clicked() {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                                }
                                ui.label(format!("— {role}"));
                            });
                        }
                    });
                });
            if !open {
                self.credits_open = false;
            }
        }
    }

    /// Asks for a new data directory, copies the current one into it and
    /// remembers the choice.
    ///
    /// Copy rather than move, and no deletion: the files being relocated
    /// include the only copy of the user's VRChat session and Discord
    /// token, some of the stores hold them open, and a half-finished move
    /// of a live data directory is unrecoverable. Leaving the old folder
    /// alone costs disk and nothing else — the user deletes it once the
    /// new location has started cleanly.
    fn begin_move_data_dir(&mut self) {
        let target = match crate::dialogs::pick_folder(
            "Choose a folder for Celeste's data",
            Some(&self.data_dir),
        ) {
            Ok(Some(target)) => target,
            Ok(None) => return,
            Err(error) => {
                let message = format!("Could not open the folder picker: {error}");
                self.state.append_cli_output(&message);
                self.settings_notice = Some(message);
                return;
            }
        };
        if target == self.data_dir {
            self.settings_notice =
                Some("That is already the data folder; nothing was moved.".to_string());
            return;
        }
        match celeste_core::paths::copy_data_dir(&self.data_dir, &target) {
            Ok(copied) => {
                let remembered = celeste_core::paths::remember_data_dir(&target);
                let message = match remembered {
                    Ok(()) => format!(
                        "Copied {copied} item(s) to {}. Restart Celeste to use it. The old \
                         folder is untouched at {} — delete it once the new one starts cleanly.",
                        target.display(),
                        self.data_dir.display()
                    ),
                    Err(error) => format!(
                        "Copied {copied} item(s) to {}, but could not record it as the data \
                         folder ({error}), so the next launch will use the old one.",
                        target.display()
                    ),
                };
                self.state.append_cli_output(&message);
                self.settings_notice = Some(message);
            }
            Err(error) => {
                let message = format!("Could not move the data folder: {error}");
                self.state.append_cli_output(&message);
                self.settings_notice = Some(message);
            }
        }
    }

    fn begin_legacy_import(&mut self) {
        let detected = celeste_core::paths::find_legacy_data_dirs(&self.data_dir)
            .into_iter()
            .next();
        let selection = if let Some(detected) = detected {
            let result = crate::dialogs::message(
                "Celeste - Scarlet data found",
                format!(
                    "Celeste found a legacy Scarlet folder:\n\n{}\n\nUse this folder?\n\nYes: use it\nNo: choose a folder or archive\nCancel: do nothing",
                    detected.display(),
                ),
                rfd::MessageButtons::YesNoCancel,
                rfd::MessageLevel::Info,
            );
            match result {
                Err(error) => {
                    let message = format!("Could not open the Scarlet import question: {error}");
                    self.state.append_cli_output(&message);
                    self.settings_notice = Some(message);
                    return;
                }
                Ok(rfd::MessageDialogResult::Yes) => Some((detected, None)),
                Ok(rfd::MessageDialogResult::No) => match pick_legacy_import_source(&self.data_dir)
                {
                    Ok(selection) => selection,
                    Err(error) => {
                        let message = format!("Could not choose Scarlet data: {error}");
                        self.state.append_cli_output(&message);
                        self.settings_notice = Some(message);
                        return;
                    }
                },
                Ok(_) => None,
            }
        } else {
            match pick_legacy_import_source(&self.data_dir) {
                Ok(selection) => selection,
                Err(error) => {
                    let message = format!("Could not choose Scarlet data: {error}");
                    self.state.append_cli_output(&message);
                    self.settings_notice = Some(message);
                    return;
                }
            }
        };
        let Some((source, archive)) = selection else {
            return;
        };
        if !celeste_core::paths::settings_file_usable(&source) {
            let message = format!(
                "{} does not contain a readable, non-empty settings.json, so it was not imported.",
                source.display()
            );
            let _ = crate::dialogs::message(
                "Celeste - not a Scarlet data folder",
                &message,
                rfd::MessageButtons::Ok,
                rfd::MessageLevel::Error,
            );
            self.settings_notice = Some(message);
            if archive.is_some() {
                celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
            }
            return;
        }
        let plan = match celeste_core::legacy_import::inspect(&source, &self.data_dir) {
            Ok(plan) => plan,
            Err(error) => {
                let message = format!("Could not inspect Scarlet data: {error}");
                self.state.append_cli_output(&message);
                self.settings_notice = Some(message);
                if archive.is_some() {
                    celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                }
                return;
            }
        };
        let security_warning = "SECURITY REMINDER: VRChat and Discord credentials are never transferred. Make sure you have them available to enter again, and double-check that the imported data is correct after Celeste restarts.";
        let mode = if plan.overlapping_files.is_empty() {
            let confirmed = crate::dialogs::message(
                "Celeste - import Scarlet data",
                format!(
                    "Add reusable data from:\n{}\n\ninto Celeste:\n{}\n\n{security_warning}\n\nA backup is created first. Settings and credentials are skipped, the Scarlet folder is never changed, and Celeste must restart to import safely.",
                    archive.as_deref().unwrap_or(&source).display(),
                    self.data_dir.display(),
                ),
                rfd::MessageButtons::OkCancel,
                rfd::MessageLevel::Warning,
            );
            match confirmed {
                Ok(rfd::MessageDialogResult::Ok) => celeste_core::legacy_import::ImportMode::Merge,
                Ok(_) => {
                    if archive.is_some() {
                        celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                    }
                    return;
                }
                Err(error) => {
                    let message =
                        format!("Could not open the Scarlet import confirmation: {error}");
                    self.state.append_cli_output(&message);
                    self.settings_notice = Some(message);
                    if archive.is_some() {
                        celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                    }
                    return;
                }
            }
        } else {
            let overlaps = plan
                .overlapping_files
                .iter()
                .map(|file| format!("• {file}"))
                .collect::<Vec<_>>()
                .join("\n");
            let choice = crate::dialogs::message(
                "Celeste - overlapping Scarlet data",
                format!(
                    "Celeste found {} overlapping recognized data location(s):\n\n{overlaps}\n\nChoose Merge to add records Scarlet has while preserving existing Celeste values. Choose Replace to copy Scarlet's recognized data over those overlaps. A full backup is created first.\n\n{security_warning}\n\nSettings and credentials are excluded in both modes.",
                    plan.overlapping_files.len(),
                ),
                rfd::MessageButtons::YesNoCancelCustom(
                    "Merge (recommended)".to_string(),
                    "Replace overlaps".to_string(),
                    "Cancel".to_string(),
                ),
                rfd::MessageLevel::Warning,
            );
            match choice {
                Ok(rfd::MessageDialogResult::Custom(label)) if label.starts_with("Merge") => {
                    celeste_core::legacy_import::ImportMode::Merge
                }
                Ok(rfd::MessageDialogResult::Custom(label)) if label.starts_with("Replace") => {
                    celeste_core::legacy_import::ImportMode::Replace
                }
                // Some native backends return semantic results for custom
                // buttons rather than their labels.
                Ok(rfd::MessageDialogResult::Yes) => celeste_core::legacy_import::ImportMode::Merge,
                Ok(rfd::MessageDialogResult::No) => {
                    celeste_core::legacy_import::ImportMode::Replace
                }
                Ok(_) => {
                    if archive.is_some() {
                        celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                    }
                    return;
                }
                Err(error) => {
                    let message = format!("Could not open the Scarlet import choice: {error}");
                    self.state.append_cli_output(&message);
                    self.settings_notice = Some(message);
                    if archive.is_some() {
                        celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                    }
                    return;
                }
            }
        };
        let queued = if let Some(archive) = archive.as_deref() {
            celeste_core::legacy_import::queue_prepared_archive(
                &self.data_dir,
                &source,
                archive,
                mode,
            )
        } else {
            celeste_core::legacy_import::queue(&self.data_dir, &source, mode)
        };
        match queued {
            Ok(()) => {
                let message = "Scarlet import queued. Restarting Celeste to apply it safely...";
                self.state.append_cli_output(message);
                self.settings_notice = Some(message.to_string());
                if !self.process_controller.restart() {
                    self.settings_notice = Some(
                        "Scarlet import is queued, but a restart is already pending. It will run on the next launch."
                            .to_string(),
                    );
                }
            }
            Err(error) => {
                let message = format!("Could not queue the Scarlet import: {error}");
                self.state.append_cli_output(&message);
                self.settings_notice = Some(message);
                if archive.is_some() {
                    celeste_core::legacy_archive::cleanup_staging(&source, &self.data_dir);
                }
            }
        }
    }

    fn prepare_cache_cleanup(&mut self) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let days = settings.cache_cleanup_days.get().clamp(1, 30);
        let cutoff = std::time::SystemTime::now()
            - std::time::Duration::from_secs(days as u64 * 24 * 60 * 60);
        let files = celeste_core::cache_cleanup::collect_eligible(
            &self.data_dir,
            &celeste_vrchat::log_tail::resolve_vrc_app_data_dir(),
            cutoff,
        );
        if files.is_empty() {
            self.cache_cleanup_notice = Some(format!(
                "Nothing to clean. No disposable files older than {days} day(s) were found."
            ));
            return;
        }
        let bytes = files
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        self.cache_cleanup_confirm = Some(PendingCacheCleanup {
            files,
            bytes,
            retention_days: days,
        });
    }

    fn cache_cleanup_windows(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.cache_cleanup_rx {
            match rx.try_recv() {
                Ok(report) => {
                    self.cache_cleanup_rx = None;
                    let message = format!(
                        "Cache cleanup complete: {} file(s) deleted, {} failed, {:.1} MiB freed.",
                        report.deleted,
                        report.failed,
                        report.bytes_freed as f64 / (1024.0 * 1024.0),
                    );
                    self.state.append_cli_output(&message);
                    self.cache_cleanup_notice = Some(message);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.cache_cleanup_rx = None;
                    self.cache_cleanup_notice = Some("Cache cleanup stopped unexpectedly.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(pending) = self.cache_cleanup_confirm.as_ref() {
            let count = pending.files.len();
            let bytes = pending.bytes;
            let days = pending.retention_days;
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Confirm cache cleanup")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{count} disposable file(s) older than {days} day(s) will be permanently deleted."
                    ));
                    ui.label(format!("Estimated space: {:.1} MiB", bytes as f64 / (1024.0 * 1024.0)));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        confirm = ui.button("Delete old files").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if cancel {
                self.cache_cleanup_confirm = None;
            } else if confirm {
                let pending = self.cache_cleanup_confirm.take().unwrap();
                let (tx, rx) = std::sync::mpsc::channel();
                self.cache_cleanup_rx = Some(rx);
                if let Some(settings) = self.settings.clone() {
                    self.runtime_handle.spawn_blocking(move || {
                        let report = celeste_core::cache_cleanup::delete_files(&pending.files);
                        settings.last_cache_cleanup.set(Some(chrono::Utc::now()));
                        let _ = tx.send(report);
                    });
                }
            }
        }

        if self.cache_cleanup_notice.is_some() {
            let message = self.cache_cleanup_notice.clone().unwrap();
            let mut close = false;
            egui::Window::new("Cache cleanup")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(message);
                    ui.add_space(8.0);
                    close = ui.button("Close").clicked();
                });
            if close {
                self.cache_cleanup_notice = None;
            }
        }
    }

    /// The control for one setting, sized so the column lines up.
    /// Commits any text setting whose buffer differs from what is stored.
    ///
    /// Called before anything that stops the settings list being drawn.
    /// Without it, typing a value and then clicking a different category
    /// (or leaving the tab, or closing the window) throws the value away
    /// with no indication that it was not saved.
    fn flush_pending_setting_edits(&mut self) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let mut rejected = Vec::new();
        for descriptor in settings.registry.all() {
            let Some(buffer) = self.settings_edit_buffers.get(descriptor.id()) else {
                continue;
            };
            let stored = descriptor.get_as_string();
            if *buffer == stored {
                continue;
            }
            if !descriptor.set_from_string(buffer, "ui") {
                rejected.push(descriptor.id().to_string());
            }
        }
        for id in rejected {
            if let Some(descriptor) = settings.registry.all().into_iter().find(|d| d.id() == id) {
                self.settings_edit_buffers
                    .insert(id.clone(), descriptor.get_as_string());
            }
            self.state
                .append_cli_output(format!("Setting {id}: value rejected (didn't validate)."));
        }
    }

    fn render_setting_control(
        &mut self,
        ui: &mut egui::Ui,
        descriptor: &std::sync::Arc<dyn SettingDescriptor>,
    ) {
        ui.set_width(CONTROL_WIDTH);
        match descriptor.kind() {
            SettingKind::Boolean { default } => {
                let mut value = descriptor.get_as_string() == "true";
                let label = if value { "On" } else { "Off" };
                if ui.checkbox(&mut value, label).changed() {
                    descriptor.set_from_string(&value.to_string(), "ui");
                }
                if value != *default {
                    ui.scope(|ui| {
                        if ui
                            .small_button("Reset")
                            .on_hover_text("Reset to default")
                            .clicked()
                        {
                            descriptor.set_from_string(&default.to_string(), "ui");
                        }
                    });
                }
            }
            SettingKind::IntegerRange { default, min, max } => {
                let mut value: i32 = descriptor.get_as_string().parse().unwrap_or(*default);
                if ui
                    .add(
                        egui::DragValue::new(&mut value)
                            .range(*min..=*max)
                            .speed(1.0),
                    )
                    .changed()
                {
                    descriptor.set_from_string(&value.to_string(), "ui");
                }
                if value != *default {
                    ui.scope(|ui| {
                        if ui
                            .small_button("Reset")
                            .on_hover_text("Reset to default")
                            .clicked()
                        {
                            descriptor.set_from_string(&default.to_string(), "ui");
                        }
                    });
                }
            }
            SettingKind::Enum { variants, .. } => {
                let mut current = descriptor.get_as_string();
                egui::ComboBox::from_id_salt(descriptor.id())
                    .selected_text(current.clone())
                    .width(CONTROL_WIDTH - 24.0)
                    .show_ui(ui, |ui| {
                        for variant in variants {
                            if ui
                                .selectable_value(&mut current, variant.clone(), variant)
                                .changed()
                            {
                                descriptor.set_from_string(variant, "ui");
                            }
                        }
                    });
            }
            SettingKind::StringChoice { valid_values } => {
                let mut current = descriptor.get_as_string();
                let values = valid_values();
                let selected = if current.is_empty() {
                    "System default".to_string()
                } else {
                    current.clone()
                };
                egui::ComboBox::from_id_salt(descriptor.id())
                    .selected_text(selected)
                    .width(CONTROL_WIDTH - 24.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut current, String::new(), "System default")
                            .changed()
                        {
                            descriptor.set_from_string("", "ui");
                        }
                        if values.is_empty() {
                            ui.weak("(nothing installed)");
                        }
                        for value in values {
                            if ui
                                .selectable_value(&mut current, value.clone(), &value)
                                .changed()
                            {
                                descriptor.set_from_string(&value, "ui");
                            }
                        }
                    });
            }
            SettingKind::Void { task } => {
                if ui.button("Run").clicked() {
                    task();
                }
            }
            // A list setting is not a line of text. It used to fall through
            // to the single-line box below, which cannot contain a newline
            // at all -- so the avatar databases showed as one URL in a
            // field too narrow to read, and a multi-entry list could not be
            // edited in the window.
            SettingKind::StringArrayPattern { .. } => {
                let current: Vec<String> = descriptor
                    .get_as_string()
                    .lines()
                    .map(|line| line.trim().to_string())
                    .filter(|line| !line.is_empty())
                    .collect();
                match known_choices_for(descriptor.id()) {
                    // Settings whose values come from a known catalogue are
                    // a choice between named things, not free text.
                    Some(catalogue) => {
                        let mut selected = current.clone();
                        let total = catalogue.len();
                        let summary = match selected.len() {
                            0 => "None selected".to_string(),
                            n => format!("{n} of {total} selected"),
                        };
                        egui::ComboBox::from_id_salt(descriptor.id())
                            .selected_text(summary)
                            .width(CONTROL_WIDTH - 24.0)
                            .show_ui(ui, |ui| {
                                for (label, value) in &catalogue {
                                    let mut on = selected.iter().any(|item| item == value);
                                    if ui.checkbox(&mut on, label).changed() {
                                        if on {
                                            selected.push(value.clone());
                                        } else {
                                            selected.retain(|item| item != value);
                                        }
                                        descriptor.set_from_string(&selected.join("\n"), "ui");
                                    }
                                }
                            });
                    }
                    // Everything else is a genuinely free list, so give it
                    // a box that can hold more than one line.
                    None => {
                        let id = descriptor.id().to_string();
                        let widget_id = egui::Id::new(("setting-list", &id));
                        let stored = descriptor.get_as_string();
                        let focused = ui.memory(|memory| memory.has_focus(widget_id));
                        let buf = self
                            .settings_edit_buffers
                            .entry(id.clone())
                            .or_insert_with(|| stored.clone());
                        if !focused && *buf != stored {
                            *buf = stored.clone();
                        }
                        let response = ui.add(
                            egui::TextEdit::multiline(buf)
                                .id(widget_id)
                                .desired_width(CONTROL_WIDTH - 12.0)
                                .desired_rows(3)
                                .hint_text("One per line"),
                        );
                        if response.lost_focus() && !descriptor.set_from_string(buf, "ui") {
                            *buf = descriptor.get_as_string();
                            self.state.append_cli_output(format!(
                                "Setting {id}: value rejected (didn't validate)."
                            ));
                        }
                    }
                }
            }
            _ => {
                // Text-ish settings are edited in a buffer and committed,
                // rather than writing on every keystroke, so a half-typed
                // value is never stored. Two things that has to get right,
                // both of which it previously got wrong:
                //
                // 1. The buffer was created once with `or_insert_with` and
                //    never resynced, so a value changed anywhere else --
                //    `/settings edit` in Discord, a reset, a reload -- left
                //    a stale string in the box that the next focus loss
                //    would write straight back over the newer value.
                // 2. It only committed on `lost_focus`. Clicking a
                //    different settings category destroys the widget
                //    without that ever being delivered, so anything typed
                //    and not followed by a click *inside* the settings list
                //    was silently discarded. That is the "settings don't
                //    save sometimes" case.
                let id = descriptor.id().to_string();
                let widget_id = egui::Id::new(("setting-edit", &id));
                let stored = descriptor.get_as_string();
                let focused = ui.memory(|memory| memory.has_focus(widget_id));
                let buf = self
                    .settings_edit_buffers
                    .entry(id.clone())
                    .or_insert_with(|| stored.clone());
                // While the field is idle it mirrors the store, so it can
                // neither show nor write back something stale.
                if !focused && *buf != stored {
                    *buf = stored.clone();
                }
                let response = ui.add(
                    egui::TextEdit::singleline(buf)
                        .id(widget_id)
                        .desired_width(CONTROL_WIDTH - 12.0),
                );
                let entered =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if commit_text_setting(response.lost_focus(), entered) {
                    let accepted = descriptor.set_from_string(buf, "ui");
                    if !accepted {
                        // Put the stored value back rather than leaving a
                        // rejected string sitting in the box looking saved.
                        *buf = descriptor.get_as_string();
                        self.state.append_cli_output(format!(
                            "Setting {id}: value rejected (didn't validate)."
                        ));
                    }
                }
            }
        }
    }

    fn cli_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Command console");
                ui.label(
                    egui::RichText::new(
                        "Run local Celeste maintenance commands and inspect their output.",
                    )
                    .small()
                    .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Clear output").clicked() {
                    self.state.clear_cli_log();
                }
            });
        });
        ui.add_space(8.0);

        let available_height = (ui.available_height() - 50.0).max(0.0);
        Self::inset_frame().show(ui, |ui| {
            ui.set_min_height((available_height - 20.0).max(80.0));
            egui::ScrollArea::vertical()
                .max_height(available_height.max(80.0))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in self.state.cli_log() {
                        ui.label(
                            egui::RichText::new(&line.0)
                                .monospace()
                                .color(crate::theme::TEXT_DIM),
                        );
                    }
                });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(">").monospace().strong());
            let width = (ui.available_width() - 78.0).max(120.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.cli_input)
                    .desired_width(width)
                    .hint_text("Type help for available commands"),
            );
            let run = ui.add(self.primary_button("Run")).clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if run && !self.cli_input.trim().is_empty() {
                self.run_cli_command(ui.ctx(), self.cli_input.trim().to_string());
                self.cli_input.clear();
            }
        });
    }

    /// Port of Java's `Scarlet.rawCommand`. Commands whose backing subsystem
    /// is deliberately out of scope are recognized and say so explicitly.
    fn run_cli_command(&mut self, ctx: &egui::Context, command: String) {
        self.state.append_cli_output(format!("> {command}"));
        let (raw_op, rest) = command
            .split_once(char::is_whitespace)
            .map_or((command.as_str(), ""), |(op, rest)| (op, rest.trim()));
        let op = canonicalize_cli_command(raw_op);
        match op.as_str() {
            "check" => {
                let report = self.readiness_report();
                self.state.append_cli_output(&report.to_text());
            }
            "help" | "info" => self.state.append_cli_output(
                "Celeste CLI commands:\n  help/info                 show this list\n  check                    check what is working and what is not\n  logout                   log out of VRChat\n  exit/halt/quit/stop       stop Celeste\n  reboot/restart            restart Celeste\n  explore                  open the data folder\n  tts <text>               speak text\n  link <usr_id> <snowflake> link VRChat and Discord users\n  importgroups <file|URL>   import watched groups CSV\n  importgroupsjson <file|URL> import watched groups JSON\n  clear                    clear this output",
            ),
            "clear" => self.state.clear_cli_log(),
            "explore" => {
                match open::that(&self.data_dir) {
                    Ok(()) => self.state.append_cli_output(format!("Opening data folder: {}", self.data_dir.display())),
                    Err(error) => self.state.append_cli_output(format!("[error] Could not open data folder: {error}")),
                }
            }
            "tts" => {
                if rest.is_empty() { return; }
                self.tts_route.submit_routed("cli", rest);
                let route = if self.settings.as_ref().is_some_and(|settings| settings.tts_use_default_audio_device.get()) {
                    "local audio"
                } else {
                    "Discord voice"
                };
                self.state.append_cli_output(format!("Submitting TTS to {route}: `{rest}`"));
            }
            "reboot" | "restart" => {
                self.state.append_cli_output(if self.process_controller.restart() { "Restarting Celeste..." } else { "Restart is already queued" });
            }
            "exit" | "halt" | "quit" | "stop" => {
                self.state.append_cli_output("Stopping Celeste...");
                self.exit_confirmed = true;
                self.shutdown();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "logout" => {
                let slot = self.vrchat_slot.clone();
                let settings = self.settings.clone();
                let state = self.state.clone();
                self.runtime_handle.spawn(async move {
                    let client = slot.read().await.clone();
                    let result = match client { Some(client) => client.logout().await.map_err(|e| e.to_string()), None => Ok(()) };
                    if let Some(settings) = settings { celeste_vrchat::client::clear_session(&settings); }
                    *slot.write().await = None;
                    match result {
                        Ok(()) => { state.refresh_vrchat_api_status("VRChat: signed out"); state.append_cli_output("Logout success: true"); }
                        Err(error) => state.append_cli_output(format!("[error] Logout failed: {error}")),
                    }
                });
                // Java deliberately falls through from `logout` into
                // `exit`, ensuring the next launch starts a fresh session.
                self.exit_confirmed = true;
                self.shutdown();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            "link" => {
                let mut args = rest.split_whitespace();
                let (Some(user_id), Some(snowflake), None) = (args.next(), args.next(), args.next()) else {
                    self.state.append_cli_output("Usage: link <usr_id> <discord snowflake>");
                    return;
                };
                let user_id = user_id.to_string();
                let snowflake = snowflake.to_string();
                let slot = self.vrchat_slot.clone();
                let data = self.celeste_data.clone();
                let state = self.state.clone();
                self.runtime_handle.spawn(async move {
                    let Some(client) = slot.read().await.clone() else { state.append_cli_output("[warn] Celeste is not signed in to VRChat."); return; };
                    match client.get_user(&user_id).await {
                        Ok(user) => {
                            data.link_id_to_snowflake(&user_id, &snowflake);
                            state.append_cli_output(format!("Linking VRChat user {} ({user_id}) to Discord user <@{snowflake}>", user.display_name.unwrap_or_else(|| user_id.clone())));
                        }
                        Err(_) => state.append_cli_output(format!("[warn] Unknown VRChat user: {user_id}")),
                    }
                });
            }
            "importgroups" => self.cli_import_groups(rest, "csv"),
            "importgroupsjson" => self.cli_import_groups(rest, "json"),
            "translate-advisories" | "restore-advisories" => self.state.append_cli_output("This command belongs to the intentionally omitted advisory-translation integration."),
            "simulate" => self.state.append_cli_output("Simulation/training mode is intentionally omitted from the Rust port."),
            "repair-secure-store" | "langlint" | "vrchatapi-test" | "popup" | "popups" | "popup-test" => self.state.append_cli_output(format!("The `{op}` diagnostic command is not available in this build.")),
            other => self.state.append_cli_output(format!("Unknown command: {other} (type 'help')")),
        }
    }

    fn cli_import_groups(&mut self, source: &str, extension: &str) {
        if source.is_empty() {
            self.state.append_cli_output(format!(
                "Usage: importgroups{} <file|URL>",
                if extension == "json" { "json" } else { "" }
            ));
            return;
        }
        if source.starts_with("http://") || source.starts_with("https://") {
            self.state.append_cli_output(format!(
                "Importing watched groups {extension} from URL: {source}"
            ));
            let url = source.to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            self.runtime_handle.spawn(async move {
                let _ = tx.send(
                    celeste_discord::url_guard::download_public_text(&url, 8 * 1024 * 1024).await,
                );
            });
            self.import_groups_rx = Some((extension.to_string(), rx));
        } else {
            match std::fs::read(source) {
                Ok(contents) => self.apply_group_import(extension, &contents, source),
                Err(error) => self.state.append_cli_output(format!(
                    "[error] Exception importing watched groups from file {source}: {error}"
                )),
            }
        }
    }
}

impl eframe::App for CelesteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
        ctx.set_zoom_factor(self.ui_scale);
        self.ensure_font_fallbacks(ctx);
        self.install_visual_style(ctx);
        self.persist_window_bounds(ctx);

        // Restarts requested by setup, Settings, CLI, or Discord close the
        // renderer first. The binary is relaunched by main only after
        // `run_native` has returned, preventing EGL/WGL shutdown races.
        if self.process_controller.restart_queued() {
            if !self.exit_confirmed {
                self.exit_confirmed = true;
                self.shutdown();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // A migration result is a blocking handoff: show it before either
        // the remaining first-run wizard or the normal application. That
        // guarantees the user sees the credential reminder and verifies the
        // result instead of having it buried behind another setup page.
        if self.legacy_import_result.is_some() {
            self.legacy_import_result_page(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }

        if self.setup_settings_review {
            let mut back = false;
            egui::TopBottomPanel::top("setup_settings_header")
                .frame(
                    egui::Frame::none()
                        .fill(crate::theme::PANEL)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Back to setup").clicked() {
                            back = true;
                        }
                        ui.separator();
                        ui.heading("Review settings");
                    });
                    ui.label(
                        egui::RichText::new(
                            "Changes save immediately. Celeste will not start its integrations until you return and choose Start Celeste.",
                        )
                        .small()
                        .weak(),
                    );
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(crate::theme::INK))
                .show(ctx, |ui| self.settings_tab(ui));
            if back {
                self.flush_pending_setting_edits();
                self.setup_settings_review = false;
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.resize_for_current_stage_next_frame();
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }

        // The setup wizard, while running, replaces the entire window
        // (toolbar/tabs/table included) the same way a blocking Swing
        // dialog sequence would have — nothing else is shown until it's
        // done. `.map()` here (rather than an `if let` held across the
        // match) keeps the borrow of `self.wizard` scoped to just this
        // call, so the match arms below are free to touch other `self`
        // fields (staff_mode, runtime_handle, wizard itself).
        // A background login that stopped at the two-factor step opens the
        // code prompt, even though setup itself finished long ago.
        if self.wizard.is_none() {
            if let Ok((client, methods)) = self.two_factor_rx.try_recv() {
                if let Some(settings) = self.settings.clone() {
                    self.wizard = crate::setup::SetupWizard::for_two_factor_only(
                        &settings,
                        self.runtime_handle.clone(),
                        client,
                        &methods,
                    );
                    if self.wizard.is_none() {
                        tracing::warn!(
                            "VRChat asked for an unsupported two-factor method: {methods:?}"
                        );
                    }
                }
            }
        }

        let (outcome, wizard_vrchat_client) = match &mut self.wizard {
            Some(wizard) => {
                let outcome = wizard.render(ctx);
                let vrchat_client =
                    if matches!(outcome, crate::setup::SetupOutcome::JustFinished { .. }) {
                        wizard.vrchat_client()
                    } else {
                        None
                    };
                (Some(outcome), vrchat_client)
            }
            None => (None, None),
        };
        // A code-only prompt finishing means a login happened and nothing
        // else: publish the client, but don't re-run setup's finishing work
        // (which would connect a second Discord client).
        let two_factor_only = self
            .wizard
            .as_ref()
            .map(crate::setup::SetupWizard::is_two_factor_only)
            .unwrap_or(false);
        let outcome = if two_factor_only {
            match outcome {
                Some(crate::setup::SetupOutcome::JustFinished { .. }) => {
                    if let Some(client) = wizard_vrchat_client.clone() {
                        let slot = self.vrchat_slot.clone();
                        self.runtime_handle.spawn(async move {
                            *slot.write().await = Some(client);
                        });
                    }
                    self.wizard = None;
                    // Treated as "nothing more to do" below, so the normal
                    // window renders on this same frame.
                    Some(crate::setup::SetupOutcome::AlreadyDone)
                }
                other => other,
            }
        } else {
            outcome
        };

        match outcome {
            Some(crate::setup::SetupOutcome::JustFinished {
                discord_token,
                start_tutorial,
            }) => {
                let resuming_tutorial = self.resume_tutorial_step.is_some();
                self.staff_mode = discord_token
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty();

                match wizard_vrchat_client {
                    // The wizard already logged in this session -- reuse
                    // that client rather than logging in again from
                    // scratch (which could needlessly re-trigger 2FA).
                    Some(client) => {
                        let slot = self.vrchat_slot.clone();
                        self.runtime_handle.spawn(async move {
                            *slot.write().await = Some(client);
                        });
                    }
                    // VRChat was skipped in the wizard (or setup was
                    // already done and there was no wizard step to reuse
                    // from) -- fall back to an unattended login attempt.
                    None if !resuming_tutorial => {
                        if let Some(settings) = self.settings.clone() {
                            spawn_unattended_vrchat_login(
                                &self.runtime_handle,
                                self.vrchat_slot.clone(),
                                settings,
                                self.two_factor_tx.clone(),
                            );
                        }
                    }
                    None => {}
                }

                // `if cond && let Some(x) = ...` is a let chain, which needs
                // edition 2024; this workspace is on 2021. `Option::filter`
                // expresses the same guard without re-indenting the block.
                let tutorial_allows_discord =
                    !resuming_tutorial || !self.tutorial_setup_had_discord;
                if let Some(settings) = self.settings.clone().filter(|_| tutorial_allows_discord) {
                    self.runtime_handle
                        .spawn(celeste_discord::client::connect_and_serve(
                            discord_token,
                            celeste_discord::data::SharedState {
                                vrchat_client: self.vrchat_slot.clone(),
                                avatar_search: self.avatar_search.clone(),
                                http: self.discord_http.clone(),
                                discord_config: self.discord_config.clone(),
                                health_reporter: self.health_reporter.clone(),
                                voice_manager: self.voice_manager.clone(),
                                process_controller: self.process_controller.clone(),
                                celeste_data: self.celeste_data.clone(),
                                settings,
                                staff_list: self.staff_list.clone(),
                                secret_staff_list: self.secret_staff_list.clone(),
                                moderation_tags: self.moderation_tags.clone(),
                                watched_groups: self.watched_groups.clone(),
                                watched_users: self.watched_users.clone(),
                                watched_avatars: self.watched_avatars.clone(),
                            },
                        ));
                }
                self.wizard = None;
                if let Some(step) = self.resume_tutorial_step.take() {
                    self.start_tutorial(ctx, step);
                    self.tutorial_setup_had_discord = false;
                } else if start_tutorial {
                    self.start_tutorial(ctx, 0);
                }
                // Fall through to render the normal UI below on this same frame.
            }
            Some(crate::setup::SetupOutcome::ReviewSettings) => {
                self.setup_settings_review = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize([960.0, 700.0].into()));
                ctx.request_repaint();
                return;
            }
            Some(crate::setup::SetupOutcome::TestTts) => {
                self.tts_route
                    .submit_routed("setup-test", "Celeste text to speech is working.");
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            Some(crate::setup::SetupOutcome::AlreadyDone) => {
                self.wizard = None;
            }
            Some(crate::setup::SetupOutcome::RestartForImport) => {
                if self.process_controller.restart() {
                    self.state.append_cli_output(
                        "Scarlet import queued. Restarting Celeste before setup continues...",
                    );
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            Some(crate::setup::SetupOutcome::InProgress) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            None => {}
        }

        if let Some(step) = self
            .tutorial_step
            .and_then(|index| TUTORIAL_STEPS.get(index))
            .copied()
        {
            self.tab = step.tab;
            if let Some(category) = step.settings_category {
                if self.settings_category != category || !self.settings_search.is_empty() {
                    self.flush_pending_setting_edits();
                    self.settings_category = category;
                    self.settings_search.clear();
                }
            }
        }
        if let Some((scene, mode)) = self
            .tutorial_step
            .and_then(|index| Self::tutorial_roster_mode(index).map(|mode| (index, mode)))
        {
            if let Some(demo) = self.tutorial_demo.as_mut() {
                demo.tick_ambient_activity(mode, scene);
            }
            if !self.tutorial_narration {
                if let Some(demo) = self.tutorial_demo.as_mut() {
                    let _ = demo.tick_roster(mode, scene);
                }
            } else {
                match self.tutorial_scenario_phase {
                    Some((step, TutorialScenarioPhase::WaitingForIntroduction))
                        if step == scene && self.tts.is_idle() =>
                    {
                        self.tutorial_scenario_phase =
                            Some((scene, TutorialScenarioPhase::PlayingExample));
                    }
                    Some((step, TutorialScenarioPhase::PlayingExample))
                        if step == scene && self.tts.is_idle() =>
                    {
                        let (cue, complete) = self
                            .tutorial_demo
                            .as_mut()
                            .map(|demo| {
                                let cue = demo.tick_roster(mode, scene);
                                let complete = demo.roster_complete(mode);
                                (cue, complete)
                            })
                            .unwrap_or((None, true));
                        if let Some(cue) = cue {
                            let result = match cue {
                                TutorialRosterCue::Speech(text) => self
                                    .tts
                                    .submit_accessibility(text, self.tutorial_voice.clone()),
                                TutorialRosterCue::NuisanceAlert(text) => {
                                    self.tts.submit_accessibility_nuisance_alert(
                                        text,
                                        self.tutorial_voice.clone(),
                                    )
                                }
                                TutorialRosterCue::MixedCharacterAlert(text) => {
                                    self.tts.submit_accessibility_mixed_character_alert(
                                        text,
                                        self.tutorial_voice.clone(),
                                    )
                                }
                            };
                            if let Err(error) = result {
                                self.tutorial_tts_notice = Some(error);
                            }
                        }
                        if complete {
                            if let Some(step) = TUTORIAL_STEPS.get(scene).copied() {
                                self.tutorial_scenario_phase =
                                    Some((scene, TutorialScenarioPhase::WaitingForWrapUp));
                                self.tutorial_tts_notice = self
                                    .tts
                                    .submit_accessibility(
                                        Self::tutorial_scenario_wrap_up(step, mode),
                                        self.tutorial_voice.clone(),
                                    )
                                    .err();
                            }
                        }
                    }
                    Some((step, TutorialScenarioPhase::WaitingForWrapUp))
                        if step == scene && self.tts.is_idle() =>
                    {
                        self.tutorial_scenario_phase =
                            Some((scene, TutorialScenarioPhase::Complete));
                    }
                    _ => {}
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        self.tutorial_layout = TutorialLayout::default();

        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0)),
            )
            .show(ctx, |ui| {
                self.toolbar(ui);
            });

        let status_state = self.tutorial_state();
        let status_bar = egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
                let (present, left, total) = status_state.status_counts();
                // Java's `updateStatusBar`, through the same message catalogue
                // so the wording (and its translations) stay in one place.
                let text = if total == 0 {
                    celeste_core::i18n::tr("ui.statusNoInstance", &[])
                } else {
                    celeste_core::i18n::tr(
                        "ui.statusBar",
                        &[&present.to_string(), &left.to_string(), &total.to_string()],
                    )
                };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(text).small());
                    // Which instance those players are in. Java never showed
                    // this; without it there is no way to tell "no instance
                    // open" from "instance open, nobody joined yet".
                    match status_state.instance() {
                        Some(instance) => {
                            ui.separator();
                            let label = instance
                                .world_name
                                .clone()
                                .unwrap_or_else(|| instance.location.clone());
                            let label = if instance.is_group_instance {
                                format!("{label}  (group)")
                            } else {
                                label
                            };
                            ui.label(egui::RichText::new(label).small().strong())
                                .on_hover_text(instance.location.clone());
                        }
                        None => {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(
                                    celeste_core::i18n::tr("ui.statusNoInstance", &[]).trim(),
                                )
                                .small()
                                .weak(),
                            );
                        }
                    }
                    // A dropped TTS announcement is otherwise silent in every
                    // sense: nothing plays and nothing is shown.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(problem) = status_state.tts_status() {
                            ui.colored_label(crate::theme::WARN, "Voice warning")
                                .on_hover_text(&problem);
                        }
                    });
                });
            });
        self.tutorial_layout.status_bar = Some(status_bar.response.rect);

        self.watched_groups_window(ctx);
        self.crash_report_window(ctx);
        self.exit_confirm_window(ctx);
        self.create_instance_window(ctx);
        self.import_groups_windows(ctx);
        self.avatar_stats_window(ctx);
        self.group_action_confirm_window(ctx);
        self.cache_cleanup_windows(ctx);
        self.settings_action_windows(ctx);
        self.utility_windows(ctx);

        let main_content = egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::INK)
                    .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD)),
            )
            .show(ctx, |ui| match self.tab {
                Tab::Instance => self.instance_tab(ui),
                Tab::Clients => self.managed_clients_tab(ui),
                Tab::Settings => self.settings_tab(ui),
                Tab::Cli => self.cli_tab(ui),
            });
        self.tutorial_layout.main_content = Some(main_content.response.rect);

        // Render this last so the explanation remains above the area it is
        // describing. Changing steps selects the corresponding tab for the
        // next frame without performing any action in that tab.
        self.tutorial_window(ctx);

        // Immediate-mode redraw: request another frame so live updates from
        // background polling tasks (mutating `UiState` from other threads)
        // show up promptly instead of waiting for the next input event.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// Port of `showMainWindow`/the graphical half of `IScarletUI.create`.
/// Blocks the calling thread for the lifetime of the window — call it from
/// wherever the app decides to run graphically (e.g. the last step of
/// `main`, with Discord/VRChat running on background tasks that share
/// `state`), not concurrently with other blocking work on the same thread.
/// `runtime_handle` is used both by the setup wizard (to run the VRChat
/// login/TOTP calls without blocking the UI thread) and, once setup
/// resolves a Discord token, to spawn the Discord client itself.
fn format_account_age((years, months, days): (i32, i32, i32)) -> String {
    let mut parts = Vec::new();
    if years != 0 {
        parts.push(format!("{years}y"));
    }
    if months != 0 {
        parts.push(format!("{months}m"));
    }
    if days != 0 {
        parts.push(format!("{days}d"));
    }
    if parts.is_empty() {
        "0D".to_string()
    } else {
        parts.join(" ")
    }
}

fn sort_players(players: &mut [celeste_core::ui::ConnectedPlayer], sort: Option<TableSort>) {
    let Some(sort) = sort else {
        return;
    };
    fn text(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
        left.map(str::to_lowercase)
            .cmp(&right.map(str::to_lowercase))
    }
    players.sort_by(|left, right| {
        let ordering = match sort.column {
            ColumnId::Name => text(Some(&left.name), Some(&right.name)),
            ColumnId::Id => text(Some(&left.id), Some(&right.id)),
            ColumnId::Pronouns => text(left.pronouns.as_deref(), right.pronouns.as_deref()),
            ColumnId::Avatar => text(left.avatar_name.as_deref(), right.avatar_name.as_deref()),
            ColumnId::Performance => text(
                left.avatar_info
                    .as_ref()
                    .and_then(|info| info.performance_rating.as_deref()),
                right
                    .avatar_info
                    .as_ref()
                    .and_then(|info| info.performance_rating.as_deref()),
            ),
            ColumnId::AccAge => left.account_age.cmp(&right.account_age),
            ColumnId::Joined => left.joined.cmp(&right.joined),
            ColumnId::Left => left.left.cmp(&right.left),
            ColumnId::Advisory => text(left.advisory.as_deref(), right.advisory.as_deref()),
            // Sorting by count puts the accounts worth looking at
            // together, which is the point of the column.
            ColumnId::WatchedGroups => left
                .watched_memberships
                .len()
                .cmp(&right.watched_memberships.len()),
            ColumnId::Age18 => text(
                left.age_verification_status.as_deref(),
                right.age_verification_status.as_deref(),
            ),
            ColumnId::Rank => left.trust_rank.cmp(&right.trust_rank),
            // Java marks Action-valued columns non-sortable.
            ColumnId::AvatarStats
            | ColumnId::Profile
            | ColumnId::CopyId
            | ColumnId::Ban
            | ColumnId::Unban
            | ColumnId::Invite => std::cmp::Ordering::Equal,
        };
        if sort.ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

/// Whether a text setting's buffer should be written to the store.
///
/// Split out so the rule is testable and so the dead expression it
/// replaces cannot come back: it read
/// `lost_focus() && enter_pressed() || lost_focus()`, which — since `&&`
/// binds tighter than `||` — is just `lost_focus()`. The Enter branch had
/// no effect at all.
/// A stable id for the settings search field.
///
/// Without one, egui derives the id from where the widget lands in the
/// layout, and anything that changes the layout around it — here, a
/// conditional button appearing as soon as the query is non-empty — silently
/// becomes a *different widget* as far as focus is concerned.
/// The named values a list setting can hold, when they come from a known
/// catalogue rather than being free text. `(label, stored value)`.
///
/// Returning `None` means "this really is a free list" and the setting gets
/// a multi-line editor instead.
fn known_choices_for(id: &str) -> Option<Vec<(String, String)>> {
    match id {
        "custom_avatar_search_providers" => Some(
            celeste_discord::avatar_search::BUILTIN_PROVIDERS
                .iter()
                .map(|url| {
                    let label = celeste_discord::avatar_search::provider_name(url)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            celeste_discord::avatar_search::provider_host(url).to_string()
                        });
                    (label, (*url).to_string())
                })
                .collect(),
        ),
        _ => None,
    }
}

const BUNDLED_WINDOW_ICON: &[u8] = include_bytes!("../../../assets/icons/celeste-256.png");

fn base_system_font_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    let candidates = [
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect::<Vec<_>>();

    #[cfg(target_os = "windows")]
    let candidates = {
        let fonts = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        ["segoeui.ttf", "seguisym.ttf", "arial.ttf"]
            .into_iter()
            .map(|name| fonts.join(name))
            .collect::<Vec<_>>()
    };

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let candidates = Vec::<PathBuf>::new();

    let mut found = Vec::new();
    for path in candidates {
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

fn system_font_paths_for(characters: &std::collections::BTreeSet<char>) -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mut found = Vec::new();
        for character in characters.iter().take(16) {
            let pattern = format!("sans-serif:charset={:x}", *character as u32);
            let Ok(output) = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", &pattern])
                .output()
            else {
                break;
            };
            if !output.status.success() {
                continue;
            }
            let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            // ab_glyph supports TrueType/OpenType faces (including a face
            // selected from TTC), but not bitmap-only colour emoji fonts.
            if path.is_file()
                && ["ttf", "otf", "ttc"].contains(&extension.to_ascii_lowercase().as_str())
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains("ColorEmoji"))
                && !found.contains(&path)
            {
                found.push(path);
            }
        }
        found
    }

    #[cfg(target_os = "windows")]
    {
        let fonts = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        // These cover Japanese, Korean and Simplified/Traditional Chinese.
        // They are loaded only after egui reports a missing live-name glyph.
        [
            "YuGothR.ttc",
            "meiryo.ttc",
            "malgun.ttf",
            "msyh.ttc",
            "msjh.ttc",
        ]
        .into_iter()
        .map(|name| fonts.join(name))
        .filter(|path| path.is_file())
        .collect()
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = characters;
        Vec::new()
    }
}

fn apply_font_fallbacks(ctx: &egui::Context, paths: &[PathBuf]) {
    let mut definitions = egui::FontDefinitions::default();
    for (index, path) in paths.iter().enumerate() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let name = format!("celeste-system-fallback-{index}");
        definitions
            .font_data
            .insert(name.clone(), egui::FontData::from_owned(bytes));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            if let Some(fonts) = definitions.families.get_mut(&family) {
                fonts.push(name.clone());
            }
        }
    }
    ctx.set_fonts(definitions);
}

fn decode_window_icon(bytes: &[u8]) -> Result<egui::IconData, image::ImageError> {
    let image = image::load_from_memory(bytes)?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// The window and taskbar icon. A user-supplied `icon.png` in the data folder
/// or beside the executable remains the first choice; Celeste's own artwork
/// is embedded as the dependable fallback so a standalone binary never falls
/// back to the platform's generic application icon.
fn load_window_icon(data_dir: &std::path::Path) -> Option<egui::IconData> {
    let beside_binary = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("icon.png")));
    let candidates = [Some(data_dir.join("icon.png")), beside_binary];
    for path in candidates.into_iter().flatten() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match decode_window_icon(&bytes) {
            Ok(icon) => {
                tracing::info!("Using window icon {}", path.display());
                return Some(icon);
            }
            Err(error) => {
                tracing::warn!(
                    "Ignoring unreadable window icon {}: {error}",
                    path.display()
                );
            }
        }
    }

    match decode_window_icon(BUNDLED_WINDOW_ICON) {
        Ok(icon) => {
            tracing::info!("Using bundled Celeste window icon");
            Some(icon)
        }
        Err(error) => {
            tracing::error!("Bundled Celeste window icon is unreadable: {error}");
            None
        }
    }
}

fn settings_search_id() -> egui::Id {
    egui::Id::new("settings-search")
}

fn commit_text_setting(lost_focus: bool, enter_pressed: bool) -> bool {
    lost_focus || enter_pressed
}

fn row_background(player: &celeste_core::ui::ConnectedPlayer) -> Option<egui::Color32> {
    (player.pronouns_flagged && player.left.is_none() && player.text_color.is_none())
        .then(|| crate::theme::PRONOUNS_FLAGGED_FILL)
}

/// Port of `getOverrideForegroundColor` in `ScarletUI.java:787-813`. The
/// "player has left" dimming Java does here is applied in the table body
/// instead, via `ui.set_opacity`, because egui composites the whole cell.
fn row_foreground(player: &celeste_core::ui::ConnectedPlayer) -> Option<egui::Color32> {
    if let Some((r, g, b)) = player.text_color {
        return Some(egui::Color32::from_rgb(r, g, b));
    }
    player
        .pronouns_flagged
        .then(|| crate::theme::PRONOUNS_FLAGGED_TEXT)
}

pub fn run(
    state: UiState,
    shared: celeste_discord::data::SharedState,
    data_dir: PathBuf,
    runtime_handle: tokio::runtime::Handle,
    tts: Arc<crate::tts::TtsService>,
    tts_route: Arc<crate::tts::TtsSinkAdapter>,
    pronoun_lists: Arc<celeste_core::pronoun_lists::PronounLists>,
    two_factor_tx: std::sync::mpsc::Sender<(
        Arc<celeste_vrchat::client::VrChatClient>,
        Vec<String>,
    )>,
    two_factor_rx: std::sync::mpsc::Receiver<(
        Arc<celeste_vrchat::client::VrChatClient>,
        Vec<String>,
    )>,
    managed_clients: Arc<celeste_vrchat::managed_clients::ManagedClientStore>,
) -> eframe::Result<()> {
    let bounds = shared
        .settings
        .ui_bounds
        .get_or_null()
        .filter(|bounds| bounds.width > 0 && bounds.height > 0);
    // A data-folder or beside-the-binary `icon.png` can still override the
    // bundled Celeste mark without a rebuild.
    let mut viewport =
        egui::ViewportBuilder::default().with_app_id(celeste_core::identity::DESKTOP_APP_ID);
    if let Some(icon) = load_window_icon(&data_dir) {
        viewport = viewport.with_icon(icon);
    }
    if let Some(bounds) = bounds {
        viewport = viewport
            .with_position([bounds.x as f32, bounds.y as f32])
            .with_inner_size([bounds.width as f32, bounds.height as f32]);
    } else {
        viewport = viewport
            .with_position([100.0, 100.0])
            .with_inner_size([600.0, 400.0]);
    }
    viewport = viewport.with_min_inner_size([560.0, 360.0]);
    let options = eframe::NativeOptions {
        viewport,
        renderer: crate::platform::native_renderer(),
        ..Default::default()
    };
    eframe::run_native(
        celeste_core::identity::APP_NAME,
        options,
        Box::new(move |cc| {
            if let Some(render_state) = &cc.wgpu_render_state {
                let adapter = render_state.adapter.get_info();
                tracing::info!(
                    name = %adapter.name,
                    backend = ?adapter.backend,
                    device_type = ?adapter.device_type,
                    driver = %adapter.driver,
                    driver_info = %adapter.driver_info,
                    "Celeste graphics adapter selected"
                );
            }
            Ok(Box::new(CelesteApp::new(
                state,
                shared,
                data_dir,
                runtime_handle,
                tts,
                tts_route,
                pronoun_lists,
                two_factor_tx,
                two_factor_rx,
                managed_clients,
            )))
        }),
    )
}

/// Spawns a background, unattended VRChat login using whatever credentials
/// are already stored, filling `slot` on success. Used whenever there's no
/// freshly-authenticated client from the setup wizard to reuse (setup
/// already completed in an earlier session, or the VRChat step was
/// skipped) — see `celeste_vrchat::client::login_with_stored_credentials`
/// for what this can't do (no stored-TOTP-secret support, so an account
/// requiring 2FA needs the interactive wizard instead).
fn spawn_unattended_vrchat_login(
    runtime_handle: &tokio::runtime::Handle,
    slot: celeste_discord::data::SharedVrChatClient,
    settings: Arc<Settings>,
    two_factor_tx: std::sync::mpsc::Sender<(
        Arc<celeste_vrchat::client::VrChatClient>,
        Vec<String>,
    )>,
) {
    runtime_handle.spawn(async move {
        match celeste_vrchat::client::login_with_stored_credentials_detailed(&settings).await {
            celeste_vrchat::client::StoredLogin::LoggedIn(client) => {
                *slot.write().await = Some(Arc::new(client));
            }
            // The account has 2FA: hand the half-authenticated client to the
            // UI thread so it can ask for the code, the way Java prompts at
            // startup. Dropping it here (which is what used to happen) left
            // the app permanently signed out after first-run setup.
            celeste_vrchat::client::StoredLogin::RequiresTwoFactor { client, methods } => {
                let _ = two_factor_tx.send((Arc::new(client), methods));
            }
            celeste_vrchat::client::StoredLogin::Unavailable => {}
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_window_icon_is_valid_and_square() {
        let icon = decode_window_icon(BUNDLED_WINDOW_ICON).expect("bundled icon must decode");
        assert_eq!(icon.width, 256);
        assert_eq!(icon.height, 256);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }

    /// The avatar databases are a choice between named services, so the
    /// setting row offers them as such. Everything else is a genuinely
    /// free list and gets a multi-line editor instead of a single-line box
    /// that cannot hold a list at all.
    #[test]
    fn list_settings_with_a_catalogue_offer_named_choices() {
        let catalogue = known_choices_for("custom_avatar_search_providers")
            .expect("the avatar providers have a catalogue");
        assert_eq!(
            catalogue.len(),
            celeste_discord::avatar_search::BUILTIN_PROVIDERS.len()
        );
        for (label, value) in &catalogue {
            assert!(!label.is_empty(), "every choice needs a name to pick it by");
            assert!(!label.starts_with("http"), "a URL is not a name: {label}");
            assert!(
                value.starts_with("http"),
                "the stored value is still the URL"
            );
        }
        // A free list has no catalogue, so it must not be rendered as one.
        assert!(known_choices_for("enforce_instances_world_list").is_none());
    }

    /// The expression this replaced was
    /// `lost_focus() && enter_pressed() || lost_focus()`. Rust binds `&&`
    /// tighter than `||`, so it collapsed to `lost_focus()` and the Enter
    /// branch never ran.
    #[test]
    fn a_text_setting_commits_on_focus_loss_or_enter() {
        assert!(commit_text_setting(true, false), "clicking away commits");
        assert!(commit_text_setting(false, true), "pressing Enter commits");
        assert!(commit_text_setting(true, true));
        assert!(!commit_text_setting(false, false), "typing alone does not");
    }

    /// Moving the data folder must carry the things that cannot be
    /// regenerated — the login, the settings, the lists — and must not
    /// drag tens of megabytes of old logs and caches along with them.
    #[test]
    fn moving_the_data_folder_copies_config_and_skips_regenerable_bulk() {
        let root = std::env::temp_dir().join(format!(
            "celeste-move-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let from = root.join("old");
        let to = root.join("new");
        std::fs::create_dir_all(from.join("logs")).unwrap();
        std::fs::create_dir_all(from.join("caches")).unwrap();
        std::fs::create_dir_all(from.join("usr")).unwrap();
        std::fs::write(from.join("settings.json"), "{}").unwrap();
        std::fs::write(from.join("secure-prefs.json"), "{}").unwrap();
        std::fs::write(from.join("usr").join("usr_1.json"), "{}").unwrap();
        std::fs::write(from.join("logs").join("celeste_log.txt"), "noise").unwrap();
        std::fs::write(from.join("caches").join("junk.json"), "noise").unwrap();

        let copied = celeste_core::paths::copy_data_dir(&from, &to).unwrap();

        assert!(
            to.join("settings.json").is_file(),
            "settings must come across"
        );
        assert!(
            to.join("secure-prefs.json").is_file(),
            "the login must come across"
        );
        assert!(
            to.join("usr").join("usr_1.json").is_file(),
            "per-user data must come across"
        );
        assert!(
            !to.join("logs").exists(),
            "logs regenerate; don't carry them"
        );
        assert!(
            !to.join("caches").exists(),
            "caches regenerate; don't carry them"
        );
        assert_eq!(copied, 3);
        // The original is left intact: a half-finished move of a live data
        // directory is unrecoverable.
        assert!(from.join("settings.json").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// These two were computed in core and never read by the renderer, so
    /// watched users, groups and avatars didn't stand out at all — the
    /// advisory pipeline's whole visible output was missing.
    #[test]
    fn a_watched_players_advisory_colour_reaches_the_renderer() {
        let mut player = celeste_core::ui::ConnectedPlayer::new(
            "usr_1",
            "Someone",
            chrono::Utc::now().naive_utc(),
        );
        assert_eq!(row_foreground(&player), None);
        assert_eq!(row_background(&player), None);

        player.text_color = Some((200, 30, 30));
        assert_eq!(
            row_foreground(&player),
            Some(egui::Color32::from_rgb(200, 30, 30))
        );
    }

    #[test]
    fn flagged_pronouns_get_javas_amber_pair() {
        let mut player = celeste_core::ui::ConnectedPlayer::new(
            "usr_1",
            "Someone",
            chrono::Utc::now().naive_utc(),
        );
        player.pronouns_flagged = true;
        assert_eq!(
            row_foreground(&player),
            Some(egui::Color32::from_rgb(235, 190, 80))
        );
        assert_eq!(
            row_background(&player),
            Some(egui::Color32::from_rgb(80, 55, 10))
        );
    }

    /// Java's background condition is narrow on purpose: an advisory colour
    /// wins, and a departed player never gets the amber highlight.
    #[test]
    fn an_advisory_colour_or_a_departure_suppresses_the_amber_background() {
        let mut player = celeste_core::ui::ConnectedPlayer::new(
            "usr_1",
            "Someone",
            chrono::Utc::now().naive_utc(),
        );
        player.pronouns_flagged = true;
        player.text_color = Some((200, 30, 30));
        assert_eq!(row_background(&player), None);
        assert_eq!(
            row_foreground(&player),
            Some(egui::Color32::from_rgb(200, 30, 30))
        );

        let mut departed =
            celeste_core::ui::ConnectedPlayer::new("usr_2", "Gone", chrono::Utc::now().naive_utc());
        departed.pronouns_flagged = true;
        departed.left = Some(chrono::Utc::now().naive_utc());
        assert_eq!(row_background(&departed), None);
    }

    use super::*;

    #[test]
    fn column_order_starts_as_all_columns_with_no_duplicates() {
        // Guards the drag-to-reorder column list against accidental
        // duplication/omission: each column exactly once.
        //
        // Java's `propstable` had 17. Celeste has 18: `Groups`, which
        // opens the full watched-group panel, has no Java counterpart
        // because Java only ever showed the truncated advisory string.
        let mut seen = std::collections::HashSet::new();
        for column in ColumnId::ALL {
            assert!(
                seen.insert(column),
                "duplicate column in ColumnId::ALL: {:?}",
                column
            );
        }
        assert_eq!(ColumnId::ALL.len(), 18);
        assert_eq!(ColumnId::DEFAULT_VISIBLE.len(), 16);
        assert!(ColumnId::DEFAULT_VISIBLE.contains(&ColumnId::WatchedGroups));
        assert!(!ColumnId::DEFAULT_VISIBLE.contains(&ColumnId::Unban));
        assert!(!ColumnId::DEFAULT_VISIBLE.contains(&ColumnId::Invite));
    }

    #[test]
    fn every_column_has_a_non_empty_label_and_positive_width() {
        for column in ColumnId::ALL {
            assert!(!column.label().is_empty());
            assert!(column.min_width() > 0.0);
        }
    }

    #[test]
    fn fitted_columns_always_use_exactly_the_available_width() {
        let preferred = ColumnId::ALL
            .into_iter()
            .map(|column| (column, column.initial_width()))
            .collect::<std::collections::HashMap<_, _>>();

        for available in [420.0, 900.0, 1800.0] {
            let widths = fitted_column_widths(&ColumnId::DEFAULT_VISIBLE, &preferred, available);
            assert_eq!(widths.len(), ColumnId::DEFAULT_VISIBLE.len());
            assert!((widths.iter().sum::<f32>() - available).abs() < 0.01);
            assert!(widths.iter().all(|width| *width > 0.0));
        }
    }

    #[test]
    fn fitted_columns_preserve_more_room_for_long_identifiers() {
        let preferred = ColumnId::ALL
            .into_iter()
            .map(|column| (column, column.initial_width()))
            .collect::<std::collections::HashMap<_, _>>();
        let columns = [ColumnId::Name, ColumnId::Id, ColumnId::Age18, ColumnId::Ban];
        let widths = fitted_column_widths(&columns, &preferred, 600.0);

        assert!(widths[1] > widths[0]);
        assert!(widths[0] > widths[2]);
        assert!(widths[2] > 0.0);
        assert!(widths[3] > 0.0);
    }

    #[test]
    fn a_dragged_fitted_column_keeps_the_width_the_user_chose() {
        let columns = [
            ColumnId::Name,
            ColumnId::Id,
            ColumnId::WatchedGroups,
            ColumnId::Advisory,
            ColumnId::Ban,
        ];
        let mut preferred = ColumnId::ALL
            .into_iter()
            .map(|column| (column, column.initial_width()))
            .collect::<std::collections::HashMap<_, _>>();
        let available = 620.0;
        let before = fitted_column_widths(&columns, &preferred, available);
        let groups_index = columns
            .iter()
            .position(|column| *column == ColumnId::WatchedGroups)
            .unwrap();
        let desired = before[groups_index] + 70.0;

        let stored = preference_for_fitted_width(
            &columns,
            &preferred,
            available,
            ColumnId::WatchedGroups,
            desired,
        );
        preferred.insert(ColumnId::WatchedGroups, stored);
        let after = fitted_column_widths(&columns, &preferred, available);

        assert!(
            (after[groups_index] - desired).abs() < 0.01,
            "requested {desired}, got {}",
            after[groups_index]
        );
        assert!((after.iter().sum::<f32>() - available).abs() < 0.01);
    }

    #[test]
    fn dragging_a_header_swaps_the_two_columns() {
        // Exercises the same swap logic `instance_tab` applies on a
        // successful header drop, without needing a full egui context.
        let mut order = ColumnId::ALL.to_vec();
        let (from, to) = (0usize, 3usize);
        order.swap(from, to);
        assert_eq!(order[to], ColumnId::Name);
        assert_eq!(order[from], ColumnId::Avatar);
    }

    #[test]
    fn sortable_headers_order_rows_and_can_reverse() {
        let at = chrono::Utc::now().naive_utc();
        let mut players = vec![
            celeste_core::ui::ConnectedPlayer::new("usr_2", "Zulu", at),
            celeste_core::ui::ConnectedPlayer::new("usr_1", "alpha", at),
        ];
        sort_players(
            &mut players,
            Some(TableSort {
                column: ColumnId::Name,
                ascending: true,
            }),
        );
        assert_eq!(players[0].name, "alpha");
        sort_players(
            &mut players,
            Some(TableSort {
                column: ColumnId::Name,
                ascending: false,
            }),
        );
        assert_eq!(players[0].name, "Zulu");
    }

    #[test]
    fn action_headers_are_not_sortable_like_java() {
        assert!(!ColumnId::AvatarStats.sortable());
        assert!(!ColumnId::Profile.sortable());
        assert!(!ColumnId::CopyId.sortable());
        assert!(!ColumnId::Ban.sortable());
        assert!(!ColumnId::Unban.sortable());
        assert!(!ColumnId::Invite.sortable());
        assert!(ColumnId::AccAge.sortable());
    }

    #[test]
    fn column_layout_reads_javas_index_width_and_hidden_shape() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        let saved = std::collections::HashMap::from([
            (
                "Name".to_string(),
                UiPropsInfo {
                    index: 1,
                    width: 222,
                },
            ),
            (
                "Id".to_string(),
                UiPropsInfo {
                    index: 0,
                    width: 333,
                },
            ),
            (
                "Ban".to_string(),
                UiPropsInfo {
                    index: -1,
                    width: 70,
                },
            ),
        ]);
        settings.store.set_object("ui_instance_columns", &saved);

        let (order, widths) = load_column_layout(Some(&settings));
        assert_eq!(&order[..2], &[ColumnId::Id, ColumnId::Name]);
        assert!(!order.contains(&ColumnId::Ban));
        assert!(!order.contains(&ColumnId::Unban));
        assert_eq!(widths[&ColumnId::Name], 222.0);
        assert_eq!(widths[&ColumnId::Id], 333.0);
    }

    #[test]
    fn settings_use_intentional_sections_instead_of_prefix_guessing() {
        assert_eq!(CelesteApp::setting_group("ui_language"), "Appearance");
        assert_eq!(
            CelesteApp::setting_group("ui_confirm_group_invite"),
            "Interface"
        );
        assert_eq!(
            CelesteApp::setting_group("verified_role_snowflake"),
            "Verification"
        );
        assert_eq!(
            CelesteApp::setting_group("discord_ping_outstanding_user_ban"),
            "Outstanding moderation"
        );
        assert_eq!(
            CelesteApp::setting_group("custom_avatar_search_providers"),
            "Avatar search"
        );
    }

    #[test]
    fn category_navigation_exits_global_search_mode() {
        let mut current = SettingCategory::General;
        let mut search = "voice channel".to_string();

        CelesteApp::activate_settings_category(&mut current, &mut search, SettingCategory::Discord);

        assert_eq!(current, SettingCategory::Discord);
        assert!(search.is_empty());
    }

    #[test]
    fn settings_search_finds_non_registry_actions() {
        assert!(CelesteApp::settings_action_matches(
            SettingCategory::System,
            "import"
        ));
        assert!(CelesteApp::settings_action_matches(
            SettingCategory::System,
            "scarlet"
        ));
        assert!(!CelesteApp::settings_action_matches(
            SettingCategory::General,
            "import"
        ));
        assert!(CelesteApp::has_settings_actions(SettingCategory::System));
        assert!(!CelesteApp::has_settings_actions(
            SettingCategory::Moderation
        ));
    }

    #[test]
    fn notification_matrix_owns_shared_event_toggles() {
        assert!(CelesteApp::notification_matrix_id(
            "tts_announce_watched_users"
        ));
        assert!(CelesteApp::notification_matrix_id("toast_notify_staff"));
        assert!(!CelesteApp::notification_matrix_id("tts_voice_name"));
        assert!(!CelesteApp::notification_matrix_id(
            "notify_desktop_enabled"
        ));
    }

    #[test]
    fn world_id_resolver_accepts_an_id_or_vrchat_url() {
        let id = "wrld_01234567-89ab-cdef-0123-456789abcdef";
        assert_eq!(resolve_world_id(id).as_deref(), Some(id));
        assert_eq!(
            resolve_world_id(&format!("https://vrchat.com/home/world/{id}/info")).as_deref(),
            Some(id)
        );
        assert_eq!(resolve_world_id("wrld_not-a-uuid"), None);
    }

    #[test]
    fn instance_location_resolver_accepts_raw_and_launch_urls() {
        let world = "wrld_01234567-89ab-cdef-0123-456789abcdef";
        let location = format!("{world}:12345~group(grp_1)~region(us)");
        assert_eq!(
            resolve_instance_location(&location).as_deref(),
            Some(location.as_str())
        );
        let url = format!(
            "https://vrchat.com/home/launch?worldId={world}&instanceId=12345%7Egroup%28grp_1%29%7Eregion%28us%29"
        );
        assert_eq!(
            resolve_instance_location(&url).as_deref(),
            Some(location.as_str())
        );
    }

    #[test]
    fn instance_name_validation_matches_java() {
        assert_eq!(validate_instance_display_name("  ").unwrap(), None);
        assert_eq!(
            validate_instance_display_name(" Movie night ")
                .unwrap()
                .as_deref(),
            Some("Movie night")
        );
        assert!(validate_instance_display_name("has/slash").is_err());
        assert!(validate_instance_display_name(&"x".repeat(33)).is_err());
    }

    #[test]
    fn group_instance_request_matches_java_wire_shape() {
        let mut form = CreateInstanceForm::default();
        form.kind = InstanceKind::GroupPlus;
        form.age_gate = true;
        form.avatar_gate = "Good".into();
        form.display_name = "Movie night".into();
        let request = create_instance_request(
            &form,
            "wrld_01234567-89ab-cdef-0123-456789abcdef",
            "grp_owner",
        )
        .unwrap();
        assert_eq!(request["type"], "group");
        assert_eq!(request["groupAccessType"], "plus");
        assert_eq!(request["minimumAvatarPerformance"], "Good");
        assert_eq!(request["displayName"], "Movie night");
        assert_eq!(request["hardClose"], false);
    }

    #[test]
    fn invite_plus_request_is_personal_and_requestable() {
        let mut form = CreateInstanceForm::default();
        form.kind = InstanceKind::InvitePlus;
        let request = create_instance_request(
            &form,
            "wrld_01234567-89ab-cdef-0123-456789abcdef",
            "usr_owner",
        )
        .unwrap();
        assert_eq!(request["type"], "private");
        assert_eq!(request["canRequestInvite"], true);
        assert!(request.get("groupAccessType").is_none());
    }

    #[test]
    fn cli_canonical_commands_and_exit_synonyms_match_java() {
        assert_eq!(canonicalize_cli_command("HELP"), "help");
        assert_eq!(canonicalize_cli_command("halt"), "halt");
        assert_eq!(canonicalize_cli_command("restart"), "restart");
        assert_eq!(canonicalize_cli_command("not-a-command"), "not-a-command");
    }

    #[test]
    fn account_age_omits_zero_components_like_javas_period_renderer() {
        assert_eq!(format_account_age((4, 0, 3)), "4y 3d");
        assert_eq!(format_account_age((0, 7, 0)), "7m");
        assert_eq!(format_account_age((0, 0, 0)), "0D");
    }

    /// Each step must belong to a chapter that actually runs, or it would
    /// be unreachable from both the linear path and the chooser.
    #[test]
    fn every_step_belongs_to_a_listed_chapter() {
        for step in TUTORIAL_STEPS.iter() {
            assert!(
                TutorialChapter::ORDER.contains(&step.chapter),
                "{:?} is not in TutorialChapter::ORDER",
                step.chapter
            );
        }
        for chapter in TutorialChapter::ORDER {
            assert!(!chapter.steps().is_empty(), "{:?} has no steps", chapter);
        }
    }

    #[test]
    fn tutorial_voice_labels_are_human_readable() {
        assert_eq!(CelesteApp::tutorial_voice_label(""), "System default");
        assert_eq!(
            CelesteApp::tutorial_voice_label("espeak-ng/en-us"),
            "en-us (espeak-ng)"
        );
        assert_eq!(
            CelesteApp::tutorial_voice_label("Legacy Voice"),
            "Legacy Voice"
        );
    }

    #[test]
    fn narrated_tour_steps_include_the_visible_guidance() {
        let step = TUTORIAL_STEPS
            .iter()
            .copied()
            .find(|step| step.demo_data)
            .expect("the tour has a demonstration-data step");
        let spoken = CelesteApp::tutorial_spoken_text(step);
        assert!(spoken.contains(step.title));
        assert!(spoken.contains(step.body));
        assert!(spoken.contains(step.try_it));
        assert!(spoken.contains("temporary demonstration data"));
    }

    #[test]
    fn only_the_two_roster_lessons_run_live_scenarios() {
        let chapter = TutorialChapter::InstanceAndRoster.steps();
        assert_eq!(CelesteApp::tutorial_roster_mode(chapter[0]), None);
        assert_eq!(
            CelesteApp::tutorial_roster_mode(chapter[1]),
            Some(TutorialRosterMode::Lightweight)
        );
        assert_eq!(
            CelesteApp::tutorial_roster_mode(chapter[2]),
            Some(TutorialRosterMode::Busy)
        );
    }

    #[test]
    fn roster_narration_brackets_the_example_instead_of_describing_it_afterward() {
        let step = TUTORIAL_STEPS[TutorialChapter::InstanceAndRoster.steps()[1]];
        let intro =
            CelesteApp::tutorial_scenario_introduction(step, TutorialRosterMode::Lightweight);
        let wrap = CelesteApp::tutorial_scenario_wrap_up(step, TutorialRosterMode::Lightweight);
        assert!(intro.starts_with("Here is how a normal, moderated, but light instance"));
        assert!(intro.contains("pause while the example plays"));
        assert!(!intro.contains(step.try_it));
        assert!(wrap.contains(step.try_it));
        assert!(wrap.contains("temporary demonstration data"));
    }

    #[test]
    fn roster_tour_progresses_from_lightweight_to_busy_risk_examples() {
        let mut demo = TutorialDemo::create().expect("tutorial fixture");

        demo.tick_ambient_activity(TutorialRosterMode::Lightweight, 1);
        assert_eq!(demo.state.status_counts(), (1, 0, 1));
        for _ in 1..TutorialDemo::ambient_target(TutorialRosterMode::Lightweight) {
            demo.ambient_last_event = std::time::Instant::now() - std::time::Duration::from_secs(2);
            demo.tick_ambient_activity(TutorialRosterMode::Lightweight, 1);
        }
        assert_eq!(demo.state.status_counts(), (9, 0, 9));
        let light_players = demo.state.ordered_players();
        assert!(
            light_players
                .iter()
                .filter_map(|player| player.account_age)
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 3
        );
        assert!(
            light_players
                .iter()
                .filter_map(|player| player.pronouns.as_deref())
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 3
        );
        assert!(
            light_players
                .iter()
                .filter_map(|player| player.avatar_name.as_deref())
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 3
        );
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(3);
        assert!(matches!(
            demo.tick_roster(TutorialRosterMode::Lightweight, 1),
            Some(TutorialRosterCue::Speech(_))
        ));
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(4);
        assert!(matches!(
            demo.tick_roster(TutorialRosterMode::Lightweight, 1),
            Some(TutorialRosterCue::Speech(_))
        ));
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(4);
        assert!(matches!(
            demo.tick_roster(TutorialRosterMode::Lightweight, 1),
            Some(TutorialRosterCue::MixedCharacterAlert(_))
        ));
        assert_eq!(demo.state.status_counts(), (12, 0, 12));
        assert!(demo.state.ordered_players().iter().any(|player| player
            .advisory
            .as_deref()
            .is_some_and(|text| text.contains("mixed character"))));
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(4);
        assert_eq!(demo.tick_roster(TutorialRosterMode::Lightweight, 1), None);
        assert_eq!(demo.state.status_counts(), (11, 1, 12));
        assert!(demo.roster_complete(TutorialRosterMode::Lightweight));

        demo.tick_ambient_activity(TutorialRosterMode::Busy, 2);
        assert_eq!(demo.state.status_counts(), (1, 0, 1));
        for _ in 1..TutorialDemo::ambient_target(TutorialRosterMode::Busy) {
            demo.ambient_last_event = std::time::Instant::now() - std::time::Duration::from_secs(1);
            demo.tick_ambient_activity(TutorialRosterMode::Busy, 2);
        }
        assert_eq!(demo.state.status_counts(), (35, 0, 35));
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(3);
        assert!(matches!(
            demo.tick_roster(TutorialRosterMode::Busy, 2),
            Some(TutorialRosterCue::NuisanceAlert(_))
        ));
        let nuisance = demo
            .state
            .ordered_players()
            .into_iter()
            .find(|player| player.id == "usr_tutorial_nuisance_rank")
            .expect("nuisance-rank tutorial user");
        assert_eq!(
            nuisance.trust_rank,
            celeste_core::trust_rank::TrustRank::Nuisance
        );
        assert!(nuisance.watched_memberships.is_empty());
        for _ in 0..2 {
            demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(4);
            assert!(matches!(
                demo.tick_roster(TutorialRosterMode::Busy, 2),
                Some(TutorialRosterCue::Speech(_))
            ));
        }
        assert_eq!(demo.state.status_counts(), (38, 0, 38));
        assert!(demo.state.ordered_players().iter().any(|player| player
            .advisory
            .as_deref()
            .is_some_and(|text| text.contains("critical"))));
        demo.roster_started = std::time::Instant::now() - std::time::Duration::from_secs(4);
        assert_eq!(demo.tick_roster(TutorialRosterMode::Busy, 2), None);
        assert_eq!(demo.state.status_counts(), (37, 1, 38));
        assert!(demo.roster_complete(TutorialRosterMode::Busy));
    }

    /// A chapter's steps must be contiguous. The window numbers steps as
    /// "3 of 4" within a chapter and Back/Next walk the flat array, so a
    /// split chapter would count wrongly and appear to jump elsewhere
    /// mid-chapter.
    #[test]
    fn chapters_are_contiguous_runs() {
        for chapter in TutorialChapter::ORDER {
            let steps = chapter.steps();
            let first = steps[0];
            for (offset, index) in steps.iter().enumerate() {
                assert_eq!(
                    *index,
                    first + offset,
                    "{:?} is split across the step array",
                    chapter
                );
            }
        }
    }

    /// Essentials must come first, or "Finish essentials" would strand
    /// essential material behind an advanced chapter.
    #[test]
    fn the_essential_path_runs_before_the_advanced_one() {
        let mut seen_advanced = false;
        for step in TUTORIAL_STEPS.iter() {
            if step.chapter.essential() {
                assert!(
                    !seen_advanced,
                    "essential chapter {:?} follows an advanced one",
                    step.chapter
                );
            } else {
                seen_advanced = true;
            }
        }
        assert!(
            seen_advanced,
            "there should be at least one advanced chapter"
        );
    }

    /// Chapters that need an integration are the advanced ones; an
    /// essential chapter must never be skippable, or a local-only user
    /// would lose part of the basic path.
    #[test]
    fn essential_chapters_never_depend_on_an_integration() {
        for chapter in TutorialChapter::ORDER {
            if chapter.essential() {
                assert_eq!(
                    chapter.needs(),
                    TutorialNeeds::Nothing,
                    "{:?} is essential but gated on an integration",
                    chapter
                );
            }
        }
    }

    /// The window must never sit on top of the thing it is explaining.
    /// This was the reported bug: a fixed top-right anchor covered the
    /// toolbar, the rightmost columns, and Help.
    #[test]
    fn the_tour_window_dodges_its_own_spotlight() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let rect = |x, y, w, h| {
            Some(egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(w, h),
            ))
        };

        // Something on the right (toolbar, Help): window goes left.
        let (align, offset) =
            CelesteApp::tutorial_anchor_for(screen, rect(1200.0, 60.0, 300.0, 40.0));
        assert_eq!(align, egui::Align2::LEFT_TOP);
        assert!(offset[0] > 0.0, "a left anchor offsets inward");

        // Something on the left (settings sidebar): window stays right.
        let (align, offset) =
            CelesteApp::tutorial_anchor_for(screen, rect(20.0, 200.0, 240.0, 400.0));
        assert_eq!(align, egui::Align2::RIGHT_TOP);
        assert!(offset[0] < 0.0, "a right anchor offsets inward");

        // Full-width target in the upper half (the player table) cannot be
        // dodged sideways, so the window drops below it.
        let (align, _) = CelesteApp::tutorial_anchor_for(screen, rect(0.0, 100.0, 1600.0, 300.0));
        assert_eq!(align, egui::Align2::RIGHT_BOTTOM);

        // Full-width target low down (the status bar): window goes up.
        let (align, _) = CelesteApp::tutorial_anchor_for(screen, rect(0.0, 820.0, 1600.0, 40.0));
        assert_eq!(align, egui::Align2::RIGHT_TOP);

        // No measured target yet: fall back to the original corner.
        let (align, _) = CelesteApp::tutorial_anchor_for(screen, None);
        assert_eq!(align, egui::Align2::RIGHT_TOP);
    }

    #[test]
    fn broad_tour_targets_use_the_quiet_bottom_right_corner() {
        for target in [
            TutorialTarget::PlayerTable,
            TutorialTarget::ToolbarControls,
            TutorialTarget::SettingsContent,
        ] {
            assert_eq!(
                CelesteApp::tutorial_forced_anchor(target),
                Some(egui::Align2::RIGHT_BOTTOM),
                "{target:?} should not cover the controls it explains"
            );
        }
        assert_eq!(
            CelesteApp::tutorial_forced_anchor(TutorialTarget::SettingsSidebar),
            None
        );
    }

    #[test]
    fn public_credits_contain_only_the_requested_people_and_communities() {
        let credits: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../resources/credits.json")).unwrap();
        let names = credits
            .iter()
            .filter_map(|credit| credit.get("name").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "VRChat",
                "VRChat API",
                "KozyBlake",
                "Claude",
                "OpenAI Codex",
                "The Synapse",
                "VRC Kmart",
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_font_fallbacks_cover_ui_arrows_and_cjk_names_when_available() {
        let mut paths = base_system_font_paths();
        assert!(
            !paths.is_empty(),
            "at least one normal Linux font is available"
        );
        let cjk = ['界']
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        for path in system_font_paths_for(&cjk) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }

        let ctx = egui::Context::default();
        apply_font_fallbacks(&ctx, &paths);
        ctx.begin_pass(egui::RawInput::default());
        ctx.fonts(|fonts| {
            let font = egui::FontId::proportional(crate::theme::TEXT_BODY);
            assert!(fonts.has_glyph(&font, '→'));
            if paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("CJK"))
            }) {
                assert!(fonts.has_glyph(&font, '界'));
            }
        });
        let _ = ctx.end_pass();
    }
}
