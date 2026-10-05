//! Port target: `ScarletUI.java` (5,716 lines — the full Swing main window)
//! and `ScarletDiscordUI.java` (2,345 lines — Discord bot config panel).
//!
//! This covers the *shape* real-to-real: the toolbar row, the Instance/
//! Settings/CLI tab structure, the live player table with the same columns
//! as `propstable` in the Java source, and the status bar text format — all
//! driven by real state ([`crate::state::UiState`] for players/status,
//! [`scarlet_core::settings`] for the Settings tab), not fixtures. What
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

use scarlet_core::settings::file_valued::SettingDescriptor;
use scarlet_core::settings::file_valued::{SettingCategory, SettingKind};
use scarlet_core::settings::registry::Rectangle;
use scarlet_core::settings::Settings;

use crate::state::UiState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Instance,
    Settings,
    Cli,
}

/// One column of the Instance tab's player table, matching `propstable`'s
/// `addProperty` call order in `ScarletUI.java`. Kept as an enum (rather
/// than just an index) so [`ScarletApp::column_order`] can be reordered by
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
enum InstanceKind { GroupPublic, GroupPlus, GroupMembers, Public, FriendsPlus, Friends, InvitePlus, Invite }

impl InstanceKind {
    const ALL: [Self; 8] = [Self::GroupPublic, Self::GroupPlus, Self::GroupMembers, Self::Public, Self::FriendsPlus, Self::Friends, Self::InvitePlus, Self::Invite];
    fn label(self) -> &'static str { match self { Self::GroupPublic => "Group Public", Self::GroupPlus => "Group Plus", Self::GroupMembers => "Group Members", Self::Public => "Public", Self::FriendsPlus => "Friends+", Self::Friends => "Friends", Self::InvitePlus => "Invite+", Self::Invite => "Invite" } }
    fn group(self) -> bool { matches!(self, Self::GroupPublic | Self::GroupPlus | Self::GroupMembers) }
    fn group_access(self) -> Option<&'static str> { match self { Self::GroupPublic => Some("public"), Self::GroupPlus => Some("plus"), Self::GroupMembers => Some("members"), _ => None } }
    fn instance_type(self) -> &'static str { match self { Self::Public => "public", Self::FriendsPlus => "hidden", Self::Friends => "friends", Self::InvitePlus | Self::Invite => "private", _ => "group" } }
}

#[derive(Debug, Clone)]
struct CreateInstanceForm {
    world: String, kind: InstanceKind, region: String, avatar_gate: String, display_name: String,
    queue_enabled: bool, age_gate: bool, content_drones: bool, content_emoji: bool,
    content_props: bool, content_pedestals: bool, content_prints: bool, content_stickers: bool,
}

impl Default for CreateInstanceForm {
    fn default() -> Self { Self { world: String::new(), kind: InstanceKind::GroupPublic, region: "us".into(), avatar_gate: String::new(), display_name: String::new(), queue_enabled: true, age_gate: false, content_drones: true, content_emoji: true, content_props: true, content_pedestals: true, content_prints: true, content_stickers: true } }
}

#[derive(Debug, Clone)]
struct CreatedInstance { message: String, link: String }

fn resolve_world_id(input: &str) -> Option<String> {
    let start = input.find("wrld_")?;
    let candidate = input.get(start..start + 41)?;
    let uuid = candidate.strip_prefix("wrld_")?;
    (uuid.len() == 36 && uuid.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })).then(|| candidate.to_string())
}

fn validate_instance_display_name(name: &str) -> Result<Option<String>, String> {
    let name = name.trim();
    if name.len() > 32 || name.contains(['.', '/', '\\']) {
        return Err("Instance names can be blank or up to 32 characters, and cannot contain '.', '/', or '\\'.".into());
    }
    Ok((!name.is_empty()).then(|| name.to_string()))
}

fn create_instance_request(form: &CreateInstanceForm, world_id: &str, owner_id: &str) -> Result<serde_json::Value, String> {
    let mut request = serde_json::json!({
        "worldId": world_id, "ownerId": owner_id, "type": form.kind.instance_type(), "region": form.region,
        "queueEnabled": form.queue_enabled, "hardClose": false, "ageGate": form.age_gate,
        "contentSettings": { "drones": form.content_drones, "emoji": form.content_emoji, "props": form.content_props, "pedestals": form.content_pedestals, "prints": form.content_prints, "stickers": form.content_stickers }
    });
    let object = request.as_object_mut().expect("instance request is an object");
    if let Some(access) = form.kind.group_access() { object.insert("groupAccessType".into(), serde_json::json!(access)); }
    if form.kind == InstanceKind::InvitePlus { object.insert("canRequestInvite".into(), serde_json::json!(true)); }
    if let Some(name) = validate_instance_display_name(&form.display_name)? { object.insert("displayName".into(), serde_json::json!(name)); }
    if !form.avatar_gate.is_empty() { object.insert("minimumAvatarPerformance".into(), serde_json::json!(form.avatar_gate)); }
    Ok(request)
}

fn canonicalize_cli_command(op: &str) -> String {
    let low = op.trim().to_ascii_lowercase();
    const CANONICAL: [&str; 25] = [
        "info", "help", "logout", "exit", "halt", "quit", "stop", "reboot", "restart", "langlint",
        "simulate", "explore", "tts", "link", "importgroups", "importgroupsjson", "translate-advisories",
        "restore-advisories", "repair-secure-store", "vrchatapi-test", "popup", "popups", "popup-test", "clear", "",
    ];
    if CANONICAL.contains(&low.as_str()) { return low; }
    for (canonical, key) in [
        ("help", "cli.alias.help"), ("logout", "cli.alias.logout"), ("exit", "cli.alias.exit"),
        ("reboot", "cli.alias.reboot"), ("langlint", "cli.alias.langlint"), ("simulate", "cli.alias.simulate"),
        ("explore", "cli.alias.explore"), ("tts", "cli.alias.tts"), ("link", "cli.alias.link"),
        ("importgroups", "cli.alias.importgroups"), ("importgroupsjson", "cli.alias.importgroupsjson"),
    ] {
        let aliases = scarlet_core::i18n::tr(key, &[]);
        if aliases != key && aliases.split(|c: char| c == ',' || c.is_whitespace()).any(|alias| alias.eq_ignore_ascii_case(&low)) {
            return canonical.into();
        }
    }
    low
}

type PendingGroupFileImport = Result<Option<(String, Vec<u8>, String)>, String>;

fn load_column_layout(
    settings: Option<&Settings>,
) -> (
    Vec<ColumnId>,
    std::collections::HashMap<ColumnId, f32>,
) {
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
    let mut order = visible.into_iter().map(|(_, column)| column).collect::<Vec<_>>();
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
    const ALL: [ColumnId; 17] = [
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
        ColumnId::Age18,
        ColumnId::Rank,
        ColumnId::Profile,
        ColumnId::CopyId,
        ColumnId::Ban,
        ColumnId::Unban,
        ColumnId::Invite,
    ];

    const DEFAULT_VISIBLE: [ColumnId; 15] = [
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
            | ColumnId::AvatarStats => 40.0,
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
        return floors.into_iter().map(|width| width + extra_per_column).collect();
    }

    floors
        .into_iter()
        .zip(flexible)
        .map(|(floor, preference)| floor + extra * preference / flexible_total)
        .collect()
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
    info: Option<scarlet_core::ui::AvatarBundleInfo>,
    note: Option<String>,
    /// Pre-rendered report block, built when the window opens so the
    /// button is a clipboard write and nothing else.
    evidence: Option<String>,
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

pub struct ScarletApp {
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
    /// The authenticated VRChat client, shared with `scarlet-discord`'s
    /// command handlers via its `TypeMap` (see `scarlet_discord::data`).
    /// `None` until a login succeeds — from the wizard this session, or an
    /// unattended background login using already-stored credentials.
    vrchat_slot: scarlet_discord::data::SharedVrChatClient,
    /// See `scarlet_discord::data::SharedHttp` — shared with the event
    /// listener's audit emitter so both see the same connection.
    discord_http: scarlet_discord::data::SharedHttp,
    avatar_search: Arc<scarlet_discord::avatar_search::AvatarSearch>,
    discord_config: Arc<scarlet_discord::data::DiscordConfigStore>,
    voice_manager: Arc<songbird::Songbird>,
    process_controller: Arc<scarlet_discord::data::ProcessController>,
    /// Set when a background VRChat login stopped to ask for a two-factor
    /// code; drained in `update` to open the code prompt.
    two_factor_tx: std::sync::mpsc::Sender<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
    two_factor_rx: std::sync::mpsc::Receiver<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
    scarlet_data: Arc<scarlet_core::data::ScarletData>,
    staff_list: Arc<scarlet_core::staff_list::StaffList>,
    secret_staff_list: Arc<scarlet_core::staff_list::StaffList>,
    moderation_tags: Arc<scarlet_core::moderation_tags::ModerationTags>,
    watched_groups: Arc<scarlet_core::watched_groups::WatchedGroups>,
    watched_users: Arc<scarlet_core::watched_entities::WatchedEntities>,
    watched_avatars: Arc<scarlet_core::watched_entities::WatchedEntities>,
    pronoun_lists: Arc<scarlet_core::pronoun_lists::PronounLists>,
    tts: Arc<crate::tts::TtsService>,
    tts_route: Arc<crate::tts::TtsSinkAdapter>,
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
    cache_cleanup_rx: Option<std::sync::mpsc::Receiver<scarlet_core::cache_cleanup::CleanupReport>>,
    cache_cleanup_notice: Option<String>,
    settings_notice: Option<String>,
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
}

impl ScarletApp {
    /// `shared` carries the data objects every subsystem must agree on —
    /// the same `ScarletData`, watched lists and VRChat session the Discord
    /// commands and the ported event listener use. They are built once by
    /// the caller and handed in; this used to open its own copies here,
    /// which meant the GUI and the event listener would have been looking
    /// at two different in-memory views of the same files.
    pub fn new(
        state: UiState,
        shared: scarlet_discord::data::SharedState,
        data_dir: PathBuf,
        runtime_handle: tokio::runtime::Handle,
        tts: Arc<crate::tts::TtsService>,
        tts_route: Arc<crate::tts::TtsSinkAdapter>,
        pronoun_lists: Arc<scarlet_core::pronoun_lists::PronounLists>,
        two_factor_tx: std::sync::mpsc::Sender<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
        two_factor_rx: std::sync::mpsc::Receiver<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
    ) -> Self {
        let scarlet_data = shared.scarlet_data.clone();
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
        let voice_manager = shared.voice_manager.clone();
        let process_controller = shared.process_controller.clone();
        let settings = Some(shared.settings.clone());
        let last_window_bounds = settings.as_ref().and_then(|settings| settings.ui_bounds.get_or_null());
        let ui_scale = settings.as_ref()
            .and_then(|settings| settings.store.get_object::<f32>("ui_scale"))
            .unwrap_or(1.0)
            .clamp(0.5, 4.0);
        let accent = settings.as_ref()
            .and_then(|settings| settings.store.get_object::<Vec<u8>>("ui_accent_color"))
            .filter(|rgb| rgb.len() == 3)
            .map(|rgb| egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
            // Java's `Swing.DEFAULT_ACCENT` is crimson; the port had its own blue.
            .unwrap_or(crate::theme::DEFAULT_ACCENT);
        let settings_notice = tts.engine_name().is_none().then(||
            "Local text-to-speech is unavailable because no supported speech engine was found. Install eSpeak NG/eSpeak, Speech Dispatcher, or Flite on Linux; use macOS 'say'; or ensure PowerShell is available on Windows. Discord voice TTS can still work when configured.".to_string()
        );

        let wizard = settings.as_ref().map(|s| crate::setup::SetupWizard::new(s, runtime_handle.clone()));

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

                runtime_handle.spawn(scarlet_discord::client::connect_and_serve(
                    token,
                    scarlet_discord::data::SharedState {
                        vrchat_client: vrchat_slot.clone(),
                        avatar_search: avatar_search.clone(),
                        http: discord_http.clone(),
                        discord_config: discord_config.clone(),
                        voice_manager: voice_manager.clone(),
                        process_controller: process_controller.clone(),
                        scarlet_data: scarlet_data.clone(),
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

        ScarletApp {
            state,
            settings,
            data_dir,
            staff_mode,
            runtime_handle,
            wizard,
            vrchat_slot,
            avatar_search,
            discord_http,
            discord_config,
            voice_manager,
            process_controller,
            two_factor_tx,
            two_factor_rx,
            scarlet_data,
            staff_list,
            secret_staff_list,
            moderation_tags,
            watched_groups,
            watched_users,
            watched_avatars,
            pronoun_lists,
            tts,
            tts_route,
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
        style.visuals.widgets.active.bg_stroke =
            egui::Stroke::new(1.0_f32, self.accent);
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

    /// Shared visual primitives. Keeping these here prevents each newly
    /// ported screen from inventing its own shade, border and corner radius.
    fn card_frame() -> egui::Frame {
        egui::Frame::none()
            .fill(crate::theme::PANEL)
            .stroke(egui::Stroke::new(
                1.0_f32,
                crate::theme::RULE,
            ))
            .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
            .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD))
    }

    fn inset_frame() -> egui::Frame {
        egui::Frame::none()
            .fill(crate::theme::WELL)
            .stroke(egui::Stroke::new(
                1.0_f32,
                crate::theme::RULE,
            ))
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
                scarlet_core::identity::NAME,
                scarlet_core::identity::VERSION
            )
        } else {
            format!("{} {}", scarlet_core::identity::NAME, scarlet_core::identity::VERSION)
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
        let Some(settings) = &self.settings else { return };
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(scarlet_core::identity::NAME).strong().size(crate::theme::TEXT_HEADING));
                ui.label(
                    egui::RichText::new(format!("Version {}  |  Windows & Linux", scarlet_core::identity::VERSION))
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
                let label = if status.is_empty() { "VRChat: connecting" } else { &status };
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 28))
                    .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
                    .inner_margin(egui::Margin::symmetric(10.0, 5.0))
                    .show(ui, |ui| {
                        ui.colored_label(colour, label);
                    });
            });
        });

        ui.add_space(9.0);
        ui.separator();
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            egui::Frame::none()
                .fill(crate::theme::WELL)
                .rounding(egui::Rounding::same(crate::theme::RADIUS_PANEL))
                .inner_margin(egui::Margin::same(crate::theme::GAP_TIGHT))
                .show(ui, |ui| self.tab_bar(ui));
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
                if ui.button("Open data folder").on_hover_text("Open the KozyBlake/Celeste data directory").clicked() {
                    if let Err(err) = open::that(&self.data_dir) {
                        self.state.append_cli_output(format!("Could not open data folder: {err}"));
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
                egui::RichText::new(if tts_on { "Voice: On" } else { "Voice: Off" }).color(voice_colour),
            );
            if ui.add(button).clicked() {
                self.state.set_tts_enabled(!tts_on);
                self.tts.set_paused(tts_on);
            }
            if ui.button("Skip voice").on_hover_text("Skip the current TTS announcement").clicked() {
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

            ui.menu_button("Help", |ui| {
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
                    ("GitHub repository", scarlet_core::identity::GITHUB_URL),
                    ("VRChat group", "https://vrchat.com/home/group/grp_eb2eb120-67bd-404e-9665-498f567cb56d"),
                    ("License", scarlet_core::identity::LICENSE_URL),
                    ("Community API docs", "https://vrchat.community/"),
                    ("VRChat Terms of Service", "https://hello.vrchat.com/legal"),
                    ("VRChat Privacy Policy", "https://hello.vrchat.com/privacy#7"),
                    ("VRChat Community Guidelines", "https://hello.vrchat.com/community-guidelines"),
                ] {
                    if ui.button(label).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        ui.close_menu();
                    }
                }
            });
        });
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
                state.append_cli_output(
                    "Sync Discord failed: the Discord bot is not connected.",
                );
                return;
            };
            match scarlet_discord::commands::register_global_commands_http(&http).await {
                Ok(count) => state.append_cli_output(format!(
                    "Sync Discord complete: registered {count} global commands."
                )),
                Err(error) => state.append_cli_output(format!(
                    "Sync Discord failed: {error}"
                )),
            }
        });
    }

    /// Whether the CLI tab is shown, per `ui_show_cli_tab`. Java's View menu
    /// toggles this live (`ScarletUI.setCliTabVisible`, `:1374-1385`); the
    /// setting existed here but nothing read it, so the toggle did nothing.
    fn cli_tab_visible(&self) -> bool {
        self.settings.as_ref().is_none_or(|settings| settings.show_cli_tab.get())
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
        egui::Window::new(scarlet_core::i18n::tr("dialog.exit.title", &[]))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(scarlet_core::i18n::tr("dialog.exit.message", &[]));
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
        self.scarlet_data.save_all();
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
        let tabs = [(Tab::Instance, "Instance"), (Tab::Settings, "Settings"), (Tab::Cli, "CLI")];
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

    fn render_cell(&mut self, ui: &mut egui::Ui, player: &scarlet_core::ui::ConnectedPlayer, column: ColumnId) {
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
                    player.avatar_info.as_ref().and_then(|info| info.performance_rating.as_deref()).unwrap_or(""),
                );
            }
            ColumnId::AvatarStats => {
                if Self::table_button(ui, "View").clicked() {
                    let location = self.state.instance().map(|instance| instance.location);
                    self.avatar_stats_open = Some(AvatarStatsView {
                        player_name: player.name.clone(),
                        avatar_name: player.avatar_name.clone(),
                        info: player.avatar_info.clone(),
                        note: player.avatar_info_note.clone(),
                        evidence: scarlet_core::report_evidence::has_avatar_evidence(player)
                            .then(|| scarlet_core::report_evidence::render(player, location.as_deref())),
                    });
                }
            }
            ColumnId::AccAge => {
                let age = player.account_age.map(format_account_age).unwrap_or_default();
                Self::text_cell(ui, &age);
            }
            ColumnId::Joined => {
                Self::text_cell(ui, &player.joined.format("%Y-%m-%d %H:%M:%S").to_string());
            }
            ColumnId::Left => {
                let left = player.left.map(|l| l.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
                Self::text_cell(ui, &left);
            }
            ColumnId::Advisory => {
                Self::text_cell(ui, player.advisory.as_deref().unwrap_or(""));
            }
            ColumnId::Age18 => {
                Self::text_cell(ui, player.age_verification_status.as_deref().unwrap_or(""));
            }
            ColumnId::Rank => {
                let color =
                    player.trust_rank.color_rgb().map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::GRAY);
                ui.add(egui::Label::new(egui::RichText::new(player.trust_rank.label()).color(color)).truncate());
            }
            ColumnId::Profile => {
                if Self::table_button(ui, "Open").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::same_tab(format!("https://vrchat.com/home/user/{}", player.id)));
                }
            }
            ColumnId::CopyId => {
                if Self::table_button(ui, "Copy").clicked() {
                    ui.ctx().copy_text(player.id.clone());
                }
            }
            ColumnId::Ban => {
                if Self::table_button(ui, "Ban").clicked() {
                    // Java confirms before banning from the table; so does
                    // this, because a stray click in a row list is easy.
                    self.group_action_confirm = Some((GroupAction::Ban, player.id.clone(), player.name.clone()));
                }
            }
            ColumnId::Unban => {
                if Self::table_button(ui, "Unban").clicked() {
                    self.group_action_confirm = Some((GroupAction::Unban, player.id.clone(), player.name.clone()));
                }
            }
            ColumnId::Invite => {
                if Self::table_button(ui, "Invite").clicked() {
                    let confirm = self
                        .settings
                        .as_ref()
                        .is_some_and(|settings| settings.confirm_group_invite.get());
                    if confirm {
                        self.group_action_confirm = Some((GroupAction::Invite, player.id.clone(), player.name.clone()));
                    } else {
                        self.spawn_group_invite(player.id.clone(), player.name.clone());
                    }
                }
            }
        }
    }

    fn instance_tab(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let mut players = self.state.ordered_players();
        sort_players(&mut players, self.table_sort);
        if players.is_empty() {
            return self.instance_empty_state(ui);
        }
        let (present, left, total) = self.state.status_counts();
        let compact_height = ui.available_height() < 300.0;
        let instance = self.state.instance();
        let instance_name = instance
            .as_ref()
            .and_then(|instance| instance.world_name.clone())
            .unwrap_or_else(|| "Current instance".to_string());
        Self::card_frame()
            .inner_margin(if compact_height {
                egui::Margin::symmetric(12.0, 7.0)
            } else {
                egui::Margin::symmetric(14.0, 11.0)
            })
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(egui::RichText::new("Current instance").small().color(crate::theme::INFO));
                ui.label(egui::RichText::new(&instance_name).strong().size(crate::theme::TEXT_HEADING));
                if !compact_height {
                    if let Some(instance) = &instance {
                    ui.add(egui::Label::new(egui::RichText::new(&instance.location).small().weak().monospace()).truncate())
                        .on_hover_text(&instance.location);
                    }
                }
                if let Some(instance) = &instance {
                    ui.add_space(7.0);
                    ui.separator();
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        let kind = if instance.is_group_instance { "Group instance" } else { "Public or private instance" };
                        ui.label(egui::RichText::new(kind).small().strong());
                        ui.separator();
                        ui.label(egui::RichText::new(format!("Joined {}", instance.joined.format("%b %-d at %H:%M"))).small().weak());
                        ui.separator();
                        ui.label(egui::RichText::new(format!("{present} present")).color(crate::theme::OK));
                        if left > 0 {
                            ui.label(egui::RichText::new(format!("{left} left")).color(crate::theme::TEXT_MUTED));
                        }
                        ui.label(egui::RichText::new(format!("{total} total")).small().weak());
                        if !compact_height {
                            ui.separator();
                            ui.label(egui::RichText::new("Drag headers to reorder  |  Right-click for columns").small().weak());
                        }
                    });
                }
            });
        ui.add_space(8.0);
        let column_order = self.column_order.clone();
        let column_widths = self.column_widths.clone();
        // `ui_left_player_dim_percent` is how much to dim a player who has
        // left (0 = no dimming), so the fade is the user's setting rather
        // than a number baked in here.
        let dim_opacity = self
            .settings
            .as_ref()
            .map(|settings| 1.0 - (settings.ui_left_player_dim_percent.get() as f32 / 100.0).clamp(0.0, 0.9))
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

        Self::card_frame()
            .show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.set_min_height((player_panel_height - 24.0).max(0.0));
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Player details").strong().size(crate::theme::TEXT_HEADING));
            ui.separator();
            ui.label(egui::RichText::new(format!("{} visible columns", column_order.len())).small().weak());
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
            let inter_column_spacing = ui.spacing().item_spacing.x
                * column_order.len().saturating_sub(1) as f32;
            let scrollbar_width = ui.spacing().scroll.allocated_width();
            let content_width = (table_view_width - inter_column_spacing - scrollbar_width).max(0.0);
            // Keep the entire roster inside the inner panel. Saved widths
            // remain the user's preferences, but the on-screen widths are
            // proportionally compressed to the current viewport. Cells and
            // headings use ellipses, with full cell values available on
            // hover.
            let fitted_widths = fitted_column_widths(&column_order, &column_widths, content_width);
            let mut table = TableBuilder::new(ui)
                .striped(true)
                .min_scrolled_height(table_view_height)
                .max_scroll_height(table_view_height)
                .auto_shrink([false, false]);
            for width in fitted_widths {
                table = table.column(Column::exact(width).clip(true));
            }

            table
                .header(28.0, |mut header| {
                    for (index, column) in column_order.iter().enumerate() {
                        header.col(|ui| {
                            // A plain label is the drop target. egui's stock
                            // `dnd_drop_zone` always paints every target as a
                            // filled widget, which produced the row of little
                            // grey boxes around otherwise ordinary headings.
                            let response = ui.dnd_drag_source(
                                egui::Id::new("instance_column_header").with(index),
                                index,
                                |ui| {
                                    let marker = match active_sort {
                                        Some(TableSort { column: sorted, ascending: true }) if sorted == *column => " [asc]",
                                        Some(TableSort { column: sorted, ascending: false }) if sorted == *column => " [desc]",
                                        _ => "",
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("{}{marker}", column.label()))
                                                .strong()
                                                .color(crate::theme::TEXT_DIM),
                                        )
                                        .truncate()
                                        .sense(egui::Sense::click_and_drag()),
                                    );
                                },
                            ).response;
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
                                    if ui.checkbox(&mut shown, candidate.label()).changed() {
                                        toggle_column = Some(candidate);
                                    }
                                }
                            });
                            let resize_id = egui::Id::new("instance_column_resize").with(*column);
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
                                    data.insert_temp(resize_id, column_widths.get(column).copied().unwrap_or_else(|| column.initial_width()));
                                });
                            }
                            if resize.dragged() || resize.drag_stopped() {
                                let start = ui.ctx().data(|data| {
                                    data.get_temp::<f32>(resize_id)
                                        .unwrap_or_else(|| column_widths.get(column).copied().unwrap_or_else(|| column.initial_width()))
                                });
                                width_update = Some((*column, (start + resize.drag_delta().x).max(column.min_width()), resize.drag_stopped()));
                            }
                            if let Some(dragged_from) = response.dnd_release_payload::<usize>() {
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
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
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
                                        ui.visuals_mut().override_text_color = Some(foreground);
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
                                    self.render_cell(ui, player, *column);
                                });
                            }
                        });
                    }
                });
            });
            });

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
            if let Some(index) = self.column_order.iter().position(|visible| *visible == column) {
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
                Some(TableSort { column: active, ascending }) if active == column => {
                    TableSort { column, ascending: !ascending }
                }
                _ => TableSort { column, ascending: true },
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
                    ui.label(egui::RichText::new(scarlet_core::i18n::tr("ui.playerDataWillAppear", &[])).weak());
                }
            }
        });
    }

    /// Port of `ScarletUI.infoStats`: what Celeste knows about the avatar a
    /// player is wearing, and — when it knows nothing — the honest reason
    /// why, which is the part that matters for a private avatar.
    fn avatar_stats_window(&mut self, ctx: &egui::Context) {
        let Some(view) = self.avatar_stats_open.clone() else { return };
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

    /// Shown when a VRChat session ended without shutting down cleanly.
    ///
    /// The last avatar to finish loading is the one worth looking at: a
    /// crasher avatar takes the client down as it loads, so it is the last
    /// thing the log records. That is a lead, not a verdict, and the
    /// window says so — the same avatar loading harmlessly a moment before
    /// an unrelated freeze looks identical from here.
    fn crash_report_window(&mut self, ctx: &egui::Context) {
        let Some(report) = self.state.crash_report() else { return };
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
        let Some((action, user_id, display_name)) = self.group_action_confirm.clone() else { return };
        let mut open = true;
        let mut decided = false;
        let verb = match action {
            GroupAction::Ban => "Ban",
            GroupAction::Unban => "Unban",
            GroupAction::Invite => "Invite",
        };
        let preposition = if action == GroupAction::Invite { "to" } else { "from" };
        egui::Window::new(format!("{verb} VRChat group user"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("{verb} {display_name} {preposition} the configured VRChat group?"));
                ui.add(egui::Label::new(egui::RichText::new(&user_id).small().monospace().weak()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(verb).clicked() {
                        decided = true;
                        match action {
                            GroupAction::Ban => self.spawn_group_ban(user_id.clone(), display_name.clone()),
                            GroupAction::Unban => self.spawn_group_unban(user_id.clone(), display_name.clone()),
                            GroupAction::Invite => self.spawn_group_invite(user_id.clone(), display_name.clone()),
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
        let Some(settings) = self.settings.clone() else { return };
        let Some(group_id) = settings.store.get_string("vrchat_group_id").filter(|id| !id.trim().is_empty()) else {
            self.state.append_cli_output("Ban failed: no VRChat group is configured.");
            return;
        };
        let slot = self.vrchat_slot.clone();
        let state = self.state.clone();
        self.state.append_cli_output(format!("Banning {display_name}..."));
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
        let Some(settings) = self.settings.clone() else { return };
        let Some(group_id) = settings.store.get_string("vrchat_group_id").filter(|id| !id.trim().is_empty()) else {
            self.state.append_cli_output(format!("{} failed: no VRChat group is configured.", match action { GroupAction::Unban => "Unban", GroupAction::Invite => "Invite", GroupAction::Ban => "Ban" }));
            return;
        };
        let slot = self.vrchat_slot.clone();
        let state = self.state.clone();
        let verb = match action { GroupAction::Unban => "Unbanning", GroupAction::Invite => "Inviting", GroupAction::Ban => "Banning" };
        self.state.append_cli_output(format!("{verb} {display_name}..."));
        self.runtime_handle.spawn(async move {
            let client = slot.read().await.clone();
            let Some(client) = client else {
                state.append_cli_output(format!("{verb} failed: not signed in to VRChat."));
                return;
            };
            let result = match action {
                GroupAction::Unban => client.unban_from_group(&group_id, &user_id).await.map(|_| ()),
                GroupAction::Invite => client.create_group_invite(&group_id, &user_id, true).await,
                GroupAction::Ban => client.ban_from_group(&group_id, &user_id).await.map(|_| ()),
            };
            match result {
                Ok(()) => state.append_cli_output(format!("{} {display_name}.", match action { GroupAction::Unban => "Unbanned", GroupAction::Invite => "Invited", GroupAction::Ban => "Banned" })),
                Err(err) => state.append_cli_output(format!("Failed to {} {display_name}: {err}", match action { GroupAction::Unban => "unban", GroupAction::Invite => "invite", GroupAction::Ban => "ban" })),
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
                    self.state.append_cli_output(format!("Create instance failed: {error}"));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.create_instance_rx = None;
                    self.create_instance_error = Some("The instance creation task stopped unexpectedly.".into());
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
            if cancel { open = false; }
            self.create_instance_open = open;
            if submit { self.start_create_instance(); }
        }

        if let Some(created) = self.created_instance.clone() {
            let mut open = true;
            egui::Window::new("VRChat instance created").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label(&created.message);
                ui.hyperlink_to("Open instance on VRChat", &created.link);
                ui.horizontal(|ui| {
                    if ui.button("Open in browser").clicked() {
                        if let Err(error) = open::that(&created.link) { self.state.append_cli_output(format!("Could not open instance link: {error}")); }
                    }
                    if ui.button("Copy link").clicked() { ui.ctx().copy_text(created.link.clone()); }
                });
            });
            if !open { self.created_instance = None; }
        }
    }

    fn start_create_instance(&mut self) {
        self.create_instance_error = None;
        let Some(world_id) = resolve_world_id(&self.create_instance_form.world) else {
            self.create_instance_error = Some("Paste a valid VRChat world URL or wrld_ ID.".into());
            return;
        };
        if let Err(error) = validate_instance_display_name(&self.create_instance_form.display_name) {
            self.create_instance_error = Some(error);
            return;
        }
        let group_id = if self.create_instance_form.kind.group() {
            let Some(id) = self.settings.as_ref().and_then(|s| s.store.get_string("vrchat_group_id")).filter(|id| !id.trim().is_empty()) else {
                self.create_instance_error = Some("No VRChat group ID is configured in Settings.".into());
                return;
            };
            Some(id)
        } else { None };

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
                        InstanceKind::GroupPlus => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_PLUS_CREATE,
                        InstanceKind::GroupMembers => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_OPEN_CREATE,
                        _ => scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_PUBLIC_CREATE,
                    };
                    for permission in std::iter::once(permission).chain(form.age_gate.then_some(scarlet_vrchat::client::PERMISSION_GROUP_INSTANCE_AGE_GATED_CREATE)) {
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
                if let Err(error) = scarlet_discord::pending_moderation::PendingModeration::open(&data_dir)
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
            self.state.append_cli_output("Import groups: a file picker is already open.");
            return;
        }

        self.state.append_cli_output("Import groups: opening file picker...");
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
            self.state.append_cli_output("Import groups: unrecognized file type (expected .csv or .json in the URL).");
            return;
        };

        self.state.append_cli_output(format!("Import groups: fetching {url}..."));
        let (tx, rx) = std::sync::mpsc::channel();
        self.runtime_handle.spawn(async move {
            // 8 MiB is far beyond any real watched-group list and keeps a
            // hostile response from exhausting memory.
            let result = scarlet_discord::url_guard::download_public_text(&url, 8 * 1024 * 1024).await;
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
                self.state.append_cli_output(format!("Import groups: unrecognized file type \"{other}\"."));
                return;
            }
        };
        match result {
            Ok(count) => self.state.append_cli_output(format!("Imported {count} watched group(s) from {source}.")),
            Err(err) => self.state.append_cli_output(format!("Import groups failed ({source}): {err}")),
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
                        egui::TextEdit::singleline(&mut self.import_groups_url).desired_width(420.0).hint_text("https://..."),
                    );
                    ui.horizontal(|ui| {
                        submit = ui.button("Import").clicked()
                            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
                        Err(err) => self.state.append_cli_output(format!("Import groups failed: {err}")),
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
                        Err(err) => self.state.append_cli_output(format!("Import groups failed: {err}")),
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
        if matches!(id, "ui_language" | "ui_left_player_dim_percent" | "ui_accent_headers") {
            "Appearance"
        } else if id.starts_with("ui_") || matches!(id, "vibecoded_notice_acknowledged" | "discord_setup_prompted") {
            "Interface"
        } else if id.starts_with("enforce_instances")
            || matches!(id, "vrchat_client_launch_on_instance_create" | "instance_follow_fast_poll_seconds")
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

    fn render_notification_matrix(ui: &mut egui::Ui, descriptors: &[std::sync::Arc<dyn SettingDescriptor>]) {
        const ROWS: [(&str, Option<&str>, Option<&str>); 9] = [
            ("Watched user", Some("tts_announce_watched_users"), Some("toast_notify_watched_users")),
            ("Watched group", Some("tts_announce_watched_groups"), Some("toast_notify_watched_groups")),
            ("Watched avatar", Some("tts_announce_watched_avatars"), Some("toast_notify_watched_avatars")),
            ("New player", Some("tts_announce_new_players"), Some("toast_notify_new_players")),
            ("Mixed-character name", Some("tts_announce_mixed_character_names"), Some("toast_notify_mixed_character_names")),
            ("Vote to kick", Some("tts_announce_votes_to_kick"), Some("toast_notify_votes_to_kick")),
            ("Suspicious pronouns", Some("tts_announce_suspicious_pronouns"), Some("toast_notify_suspicious_pronouns")),
            ("Moderation action", None, Some("toast_notify_moderation")),
            ("Staff activity", None, Some("toast_notify_staff")),
        ];
        let find = |id: &str| descriptors.iter().find(|descriptor| descriptor.id() == id);
        Self::card_frame().show(ui, |ui| {
            ui.label(egui::RichText::new("Event notifications").strong().size(crate::theme::TEXT_HEADING));
            ui.label(egui::RichText::new("Choose how each event should get your attention.").small().weak());
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

        egui::SidePanel::left("settings_categories")
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
                let count = visible.iter().filter(|d| d.category() == category).count();
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

        egui::CentralPanel::default()
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
                visible.iter().filter(|d| d.category() == category).copied().collect()
            } else {
                visible.clone()
            };
            let show_notification_matrix = query.is_empty() && category == SettingCategory::Notifications;
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

            if rows.is_empty() && !show_notification_matrix {
                ui.add_space(12.0);
                ui.weak(if query.is_empty() {
                    "No settings in this category.".to_string()
                } else {
                    format!("Nothing matches \"{}\".", self.settings_search.trim())
                });
                return;
            }

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                        ui.label(egui::RichText::new(group).strong().size(crate::theme::TEXT_HEADING));
                        ui.add_space(5.0);
                        for (offset, descriptor) in rows[start..end].iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width((ui.available_width() - CONTROL_WIDTH).max(120.0));
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
                                        scarlet_core::settings::help::describe(descriptor.id())
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
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    self.render_setting_control(ui, descriptor);
                                });
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
                }
                ui.add_space(4.0);
            });
        });
    }

    /// Java registers these as `FileValuedVoid` rows. They are UI actions,
    /// not stored values, so keep them beside the registry-backed controls.
    fn render_settings_actions(&mut self, ui: &mut egui::Ui, category: SettingCategory) {
        if !matches!(category, SettingCategory::General | SettingCategory::System | SettingCategory::Discord | SettingCategory::Integrations) {
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
                    });
                    ui.label(
                        egui::RichText::new(
                            "Settings, logins, staff lists and watched entities live here. \
                             Moving copies them to the new folder and remembers it for next \
                             launch; the old folder is left in place so you can check the \
                             move worked before deleting it.",
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
                }
                SettingCategory::Discord => {
                    if ui.button("Reset Discord connection settings").on_hover_text("Clears the stored bot token and guild ID; setup will ask again next launch.").clicked() {
                        if let Some(settings) = &self.settings {
                            use scarlet_core::settings::registry::registry_string_encrypted;
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
                if ui.button("Reset VRChat credentials and session").clicked() {
                    if let Some(settings) = &self.settings {
                        use scarlet_core::settings::registry::registry_string_encrypted;
                        registry_string_encrypted(settings.encrypted.clone(), scarlet_vrchat::client::USERNAME_KEY).clear();
                        registry_string_encrypted(settings.encrypted.clone(), scarlet_vrchat::client::PASSWORD_KEY).clear();
                        registry_string_encrypted(settings.encrypted.clone(), scarlet_vrchat::client::TOTP_SECRET_KEY).clear();
                        scarlet_vrchat::client::clear_session(settings);
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

    fn open_avatar_provider_manager(&mut self) {
        let Some(settings) = self.settings.as_ref() else { return };
        let builtins: Vec<String> = scarlet_discord::avatar_search::BUILTIN_PROVIDERS.iter().map(|url| (*url).to_string()).collect();
        let selected: std::collections::HashSet<String> = if settings.avatar_search_providers_enabled.get() {
            settings.avatar_search_providers.get().into_iter().collect()
        } else {
            builtins.iter().cloned().collect()
        };
        let mut known = builtins;
        for url in settings.avatar_search_providers.get() {
            if !known.contains(&url) && !url.to_ascii_lowercase().contains("worldbalancer") { known.push(url); }
        }
        self.avatar_provider_choices = known.into_iter()
            .filter(|url| !url.to_ascii_lowercase().contains("worldbalancer"))
            .map(|url| AvatarProviderChoice { enabled: selected.contains(&url), url })
            .collect();
        self.avatar_provider_input.clear();
        self.avatar_provider_error = None;
        self.avatar_providers_open = true;
    }

    fn save_avatar_provider_choices(&mut self) {
        let Some(settings) = self.settings.as_ref() else { return };
        let selected: Vec<String> = self.avatar_provider_choices.iter()
            .filter(|choice| choice.enabled).map(|choice| choice.url.clone()).collect();
        let defaults: std::collections::HashSet<&str> = scarlet_discord::avatar_search::BUILTIN_PROVIDERS.iter().copied().collect();
        let selected_set: std::collections::HashSet<&str> = selected.iter().map(String::as_str).collect();
        let uses_defaults = selected_set == defaults;
        settings.avatar_search_providers.set(if uses_defaults { None } else { Some(selected) }, "ui");
        settings.avatar_search_providers_enabled.set(Some(!uses_defaults), "ui");
        let search = self.avatar_search.clone();
        self.runtime_handle.spawn(async move { search.clear_provider_state().await });
        self.settings_notice = Some(if uses_defaults {
            "Avatar search providers restored to the built-in defaults.".to_string()
        } else {
            "Avatar search providers saved. New avatar lookups use this selection immediately.".to_string()
        });
    }

    fn settings_action_windows(&mut self, ctx: &egui::Context) {
        if self.ui_scale_open {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("UI scale").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label(format!("Scale: {:.0}%", self.ui_scale_percent));
                ui.add(egui::Slider::new(&mut self.ui_scale_percent, 50.0..=400.0).suffix("%"));
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    if ui.button("Cancel").clicked() { self.ui_scale_open = false; }
                });
            });
            if !open { self.ui_scale_open = false; }
            if apply {
                self.ui_scale = (self.ui_scale_percent / 100.0).clamp(0.5, 4.0);
                if let Some(settings) = &self.settings { settings.store.set_object("ui_scale", &self.ui_scale); }
                ctx.set_zoom_factor(self.ui_scale);
                self.ui_scale_open = false;
                self.settings_notice = Some(format!("UI scale set to {:.0}%.", self.ui_scale * 100.0));
            }
        }

        if self.accent_open {
            let mut open = true;
            let mut save = false;
            egui::Window::new("Accent colour").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.label("Choose a preset or a custom colour.");
                for (name, value) in [
                    ("Celeste blue", [38, 104, 170]), ("Crimson", [200, 55, 65]),
                    ("Cobalt", [59, 125, 216]), ("Forest", [46, 158, 91]),
                    ("Amber", [212, 130, 26]), ("Violet", [124, 77, 184]), ("Rose", [212, 68, 128]),
                ] {
                    if ui.selectable_label(self.accent_edit == value, name).clicked() { self.accent_edit = value; }
                }
                ui.add_space(6.0);
                ui.color_edit_button_srgb(&mut self.accent_edit);
                ui.horizontal(|ui| {
                    save = ui.button("Apply").clicked();
                    if ui.button("Cancel").clicked() { self.accent_open = false; }
                });
            });
            if !open { self.accent_open = false; }
            if save {
                self.accent = egui::Color32::from_rgb(self.accent_edit[0], self.accent_edit[1], self.accent_edit[2]);
                if let Some(settings) = &self.settings { settings.store.set_object("ui_accent_color", &self.accent_edit.to_vec()); }
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
            egui::Window::new("Avatar search providers").collapsible(false).resizable(true).default_width(620.0).open(&mut open).show(ctx, |ui| {
                ui.label("Choose the VRCX-compatible databases Celeste may query.");
                ui.label(egui::RichText::new("World Balancer entries are intentionally ignored.").small().weak());
                ui.add_space(6.0);
                // A database is a thing you recognise by name, not a URL
                // you proof-read. Named services show their name with the
                // host underneath; an endpoint the user added themselves has
                // no name we can honestly give it, so it shows its URL.
                for choice in &mut self.avatar_provider_choices {
                    let name = scarlet_discord::avatar_search::provider_name(&choice.url);
                    ui.horizontal(|ui| {
                        let label = name.unwrap_or("Custom endpoint");
                        ui.checkbox(&mut choice.enabled, label);
                        let detail = match name {
                            Some(_) => scarlet_discord::avatar_search::provider_host(&choice.url),
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
                    ui.add(egui::TextEdit::singleline(&mut self.avatar_provider_input).desired_width(430.0).hint_text("https://..."));
                    add = ui.button("Add").clicked();
                });
                if let Some(error) = &self.avatar_provider_error { ui.colored_label(crate::theme::DANGER, error); }
                ui.horizontal(|ui| {
                    restore = ui.button("Restore defaults").clicked();
                    save = ui.button("Save").clicked();
                    if ui.button("Cancel").clicked() { self.avatar_providers_open = false; }
                });
            });
            if add {
                let url = self.avatar_provider_input.trim().to_string();
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    self.avatar_provider_error = Some("Enter a full http(s) URL.".to_string());
                } else if url.to_ascii_lowercase().contains("worldbalancer") {
                    self.avatar_provider_error = Some("World Balancer is excluded from avatar searches.".to_string());
                } else {
                    if let Some(choice) = self.avatar_provider_choices.iter_mut().find(|choice| choice.url == url) { choice.enabled = true; }
                    else { self.avatar_provider_choices.push(AvatarProviderChoice { url, enabled: true }); }
                    self.avatar_provider_input.clear();
                    self.avatar_provider_error = None;
                }
            }
            if restore {
                let defaults: std::collections::HashSet<&str> = scarlet_discord::avatar_search::BUILTIN_PROVIDERS.iter().copied().collect();
                for choice in &mut self.avatar_provider_choices { choice.enabled = defaults.contains(choice.url.as_str()); }
            }
            if !open { self.avatar_providers_open = false; }
            if save { self.save_avatar_provider_choices(); self.avatar_providers_open = false; }
        }

        if self.settings_notice.is_some() {
            let message = self.settings_notice.clone().unwrap();
            let mut close = false;
            egui::Window::new("Celeste").collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(message);
                    ui.add_space(8.0);
                    close = ui.button("Close").clicked();
                });
            if close { self.settings_notice = None; }
        }
    }

    fn diagnostics_report(&self) -> String {
        let vrchat = self.vrchat_slot.try_read().map(|slot| if slot.is_some() { "connected" } else { "not connected" }).unwrap_or("busy");
        let discord = self.discord_http.try_read().map(|http| if http.is_some() { "connected" } else { "not connected / staff mode" }).unwrap_or("busy");
        let mut report = format!(
            "Celeste {}\nData folder: {}\n\nVRChat\n  Session: {}\n  Status: {}\n\nDiscord\n  Gateway/HTTP: {}\n\nText to speech\n  Local engine: {}\n",
            scarlet_core::identity::VERSION,
            self.data_dir.display(),
            vrchat,
            self.state.connection_status(),
            discord,
            self.tts.engine_name().unwrap_or("not installed"),
        );
        if let Some(settings) = &self.settings {
            let last_audit = settings.last_audit_query.get_or_null()
                .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "never".to_string());
            report.push_str(&format!("\nAudit log\n  Polling interval: {}s\n  Last query cursor: {}\n", settings.audit_polling_interval.get(), last_audit));
            let providers: Vec<String> = if settings.avatar_search_providers_enabled.get() {
                settings.avatar_search_providers.get()
            } else {
                scarlet_discord::avatar_search::BUILTIN_PROVIDERS.iter().map(|url| (*url).to_string()).collect()
            };
            let backoffs = self.avatar_search.provider_backoffs();
            report.push_str("\nAvatar search providers\n");
            for provider in providers.into_iter().filter(|url| !url.to_ascii_lowercase().contains("worldbalancer")) {
                let status = backoffs.as_ref().and_then(|states| states.get(&provider)).filter(|seconds| **seconds > 0)
                    .map(|seconds| format!("backoff {seconds}s")).unwrap_or_else(|| "ready".to_string());
                report.push_str(&format!("  [{status}] {provider}\n"));
            }
            if backoffs.is_none() { report.push_str("  (provider status is busy; no request was delayed)\n"); }
        }
        report
    }

    fn utility_windows(&mut self, ctx: &egui::Context) {
        if self.clear_instance_confirm {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Clear current instance?").collapsible(false).resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label("Clear every player row from the current Instance view?");
                    ui.label(egui::RichText::new("New VRChat log events will continue to populate the table.").small().weak());
                    ui.horizontal(|ui| {
                        confirm = ui.button("Clear instance").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if confirm {
                self.state.clear_instance();
                self.state.append_cli_output("Instance cleared.");
                self.clear_instance_confirm = false;
            } else if cancel { self.clear_instance_confirm = false; }
        }

        if self.diagnostics_open {
            let report = self.diagnostics_report();
            let mut open = true;
            egui::Window::new("Diagnostics").default_width(680.0).default_height(440.0).open(&mut open).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Copy report").clicked() { ui.ctx().copy_text(report.clone()); }
                    if ui.button("Open data folder").clicked() {
                        if let Err(error) = open::that(&self.data_dir) { self.state.append_cli_output(format!("Could not open data folder: {error}")); }
                    }
                });
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| { ui.monospace(&report); });
            });
            if !open { self.diagnostics_open = false; }
        }

        if self.credits_open {
            let credits: Vec<serde_json::Value> = serde_json::from_str(include_str!("../resources/credits.json")).unwrap_or_default();
            let mut open = true;
            egui::Window::new("Credits / About Celeste").default_width(650.0).default_height(420.0).open(&mut open).show(ctx, |ui| {
                ui.heading(format!("Celeste {}", scarlet_core::identity::VERSION));
                ui.label(scarlet_core::identity::FORK_NOTE);
                ui.label("A VRChat group moderation logger.");
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for credit in &credits {
                        let name = credit.get("name").and_then(serde_json::Value::as_str).unwrap_or("Unknown");
                        let role = credit.get("role").and_then(serde_json::Value::as_str).unwrap_or("");
                        let url = credit.get("url").and_then(serde_json::Value::as_str).unwrap_or("");
                        ui.horizontal_wrapped(|ui| {
                            if url.is_empty() { ui.strong(name); }
                            else if ui.link(name).clicked() { ui.ctx().open_url(egui::OpenUrl::new_tab(url)); }
                            ui.label(format!("— {role}"));
                        });
                    }
                });
            });
            if !open { self.credits_open = false; }
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
        let Some(target) = rfd::FileDialog::new()
            .set_title("Choose a folder for Celeste's data")
            .set_directory(&self.data_dir)
            .pick_folder()
        else {
            return;
        };
        if target == self.data_dir {
            self.settings_notice =
                Some("That is already the data folder; nothing was moved.".to_string());
            return;
        }
        match copy_data_dir(&self.data_dir, &target) {
            Ok(copied) => {
                let remembered = scarlet_core::paths::remember_data_dir(&target);
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

    fn prepare_cache_cleanup(&mut self) {
        let Some(settings) = self.settings.as_ref() else { return };
        let days = settings.cache_cleanup_days.get().clamp(1, 30);
        let cutoff = std::time::SystemTime::now()
            - std::time::Duration::from_secs(days as u64 * 24 * 60 * 60);
        let files = scarlet_core::cache_cleanup::collect_eligible(
            &self.data_dir,
            &scarlet_vrchat::log_tail::resolve_vrc_app_data_dir(),
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
                        let report = scarlet_core::cache_cleanup::delete_files(&pending.files);
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
        let Some(settings) = self.settings.clone() else { return };
        let mut rejected = Vec::new();
        for descriptor in settings.registry.all() {
            let Some(buffer) = self.settings_edit_buffers.get(descriptor.id()) else { continue };
            let stored = descriptor.get_as_string();
            if *buffer == stored {
                continue;
            }
            if !descriptor.set_from_string(buffer, "ui") {
                rejected.push(descriptor.id().to_string());
            }
        }
        for id in rejected {
            if let Some(descriptor) =
                settings.registry.all().into_iter().find(|d| d.id() == id)
            {
                self.settings_edit_buffers.insert(id.clone(), descriptor.get_as_string());
            }
            self.state
                .append_cli_output(format!("Setting {id}: value rejected (didn't validate)."));
        }
    }

    fn render_setting_control(&mut self, ui: &mut egui::Ui, descriptor: &std::sync::Arc<dyn SettingDescriptor>) {
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
                        if ui.small_button("Reset").on_hover_text("Reset to default").clicked() {
                            descriptor.set_from_string(&default.to_string(), "ui");
                        }
                    });
                }
            }
            SettingKind::IntegerRange { default, min, max } => {
                let mut value: i32 = descriptor.get_as_string().parse().unwrap_or(*default);
                if ui.add(egui::DragValue::new(&mut value).range(*min..=*max).speed(1.0)).changed() {
                    descriptor.set_from_string(&value.to_string(), "ui");
                }
                if value != *default {
                    ui.scope(|ui| {
                        if ui.small_button("Reset").on_hover_text("Reset to default").clicked() {
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
                            if ui.selectable_value(&mut current, variant.clone(), variant).changed() {
                                descriptor.set_from_string(variant, "ui");
                            }
                        }
                    });
            }
            SettingKind::StringChoice { valid_values } => {
                let mut current = descriptor.get_as_string();
                let values = valid_values();
                let selected = if current.is_empty() { "System default".to_string() } else { current.clone() };
                egui::ComboBox::from_id_salt(descriptor.id())
                    .selected_text(selected)
                    .width(CONTROL_WIDTH - 24.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut current, String::new(), "System default").changed() {
                            descriptor.set_from_string("", "ui");
                        }
                        if values.is_empty() {
                            ui.weak("(nothing installed)");
                        }
                        for value in values {
                            if ui.selectable_value(&mut current, value.clone(), &value).changed() {
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
                let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
                    egui::RichText::new("Run local Celeste maintenance commands and inspect their output.")
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
                || (response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter)));
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
        let (raw_op, rest) = command.split_once(char::is_whitespace).map_or((command.as_str(), ""), |(op, rest)| (op, rest.trim()));
        let op = canonicalize_cli_command(raw_op);
        match op.as_str() {
            "help" | "info" => self.state.append_cli_output(
                "Celeste CLI commands:\n  help/info                 show this list\n  logout                   log out of VRChat\n  exit/halt/quit/stop       stop Celeste\n  reboot/restart            restart Celeste\n  explore                  open the data folder\n  tts <text>               speak text\n  link <usr_id> <snowflake> link VRChat and Discord users\n  importgroups <file|URL>   import watched groups CSV\n  importgroupsjson <file|URL> import watched groups JSON\n  clear                    clear this output",
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
                    if let Some(settings) = settings { scarlet_vrchat::client::clear_session(&settings); }
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
                let data = self.scarlet_data.clone();
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
            self.state.append_cli_output(format!("Usage: importgroups{} <file|URL>", if extension == "json" { "json" } else { "" }));
            return;
        }
        if source.starts_with("http://") || source.starts_with("https://") {
            self.state.append_cli_output(format!("Importing watched groups {extension} from URL: {source}"));
            let url = source.to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            self.runtime_handle.spawn(async move {
                let _ = tx.send(scarlet_discord::url_guard::download_public_text(&url, 8 * 1024 * 1024).await);
            });
            self.import_groups_rx = Some((extension.to_string(), rx));
        } else {
            match std::fs::read(source) {
                Ok(contents) => self.apply_group_import(extension, &contents, source),
                Err(error) => self.state.append_cli_output(format!("[error] Exception importing watched groups from file {source}: {error}")),
            }
        }
    }
}

impl eframe::App for ScarletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));
        ctx.set_zoom_factor(self.ui_scale);
        self.install_visual_style(ctx);
        self.persist_window_bounds(ctx);

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
                        tracing::warn!("VRChat asked for an unsupported two-factor method: {methods:?}");
                    }
                }
            }
        }

        let (outcome, wizard_vrchat_client) = match &mut self.wizard {
            Some(wizard) => {
                let outcome = wizard.render(ctx);
                let vrchat_client =
                    if matches!(outcome, crate::setup::SetupOutcome::JustFinished { .. }) { wizard.vrchat_client() } else { None };
                (Some(outcome), vrchat_client)
            }
            None => (None, None),
        };
        // A code-only prompt finishing means a login happened and nothing
        // else: publish the client, but don't re-run setup's finishing work
        // (which would connect a second Discord client).
        let two_factor_only =
            self.wizard.as_ref().map(crate::setup::SetupWizard::is_two_factor_only).unwrap_or(false);
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
            Some(crate::setup::SetupOutcome::JustFinished { discord_token }) => {
                self.staff_mode = discord_token.as_deref().map(str::trim).unwrap_or("").is_empty();

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
                    None => {
                        if let Some(settings) = self.settings.clone() {
                            spawn_unattended_vrchat_login(
                                &self.runtime_handle,
                                self.vrchat_slot.clone(),
                                settings,
                                self.two_factor_tx.clone(),
                            );
                        }
                    }
                }

                if let Some(settings) = self.settings.clone() {
                    self.runtime_handle.spawn(scarlet_discord::client::connect_and_serve(
                        discord_token,
                        scarlet_discord::data::SharedState {
                            vrchat_client: self.vrchat_slot.clone(),
                            avatar_search: self.avatar_search.clone(),
                            http: self.discord_http.clone(),
                            discord_config: self.discord_config.clone(),
                            voice_manager: self.voice_manager.clone(),
                            process_controller: self.process_controller.clone(),
                            scarlet_data: self.scarlet_data.clone(),
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
                // Fall through to render the normal UI below on this same frame.
            }
            Some(crate::setup::SetupOutcome::AlreadyDone) => {
                self.wizard = None;
            }
            Some(crate::setup::SetupOutcome::InProgress) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            None => {}
        }

        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0)),
            )
            .show(ctx, |ui| {
            self.toolbar(ui);
        });

        egui::TopBottomPanel::bottom("status_bar")
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
            let (present, left, total) = self.state.status_counts();
            // Java's `updateStatusBar`, through the same message catalogue
            // so the wording (and its translations) stay in one place.
            let text = if total == 0 {
                scarlet_core::i18n::tr("ui.statusNoInstance", &[])
            } else {
                scarlet_core::i18n::tr(
                    "ui.statusBar",
                    &[&present.to_string(), &left.to_string(), &total.to_string()],
                )
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(text).small());
                // Which instance those players are in. Java never showed
                // this; without it there is no way to tell "no instance
                // open" from "instance open, nobody joined yet".
                match self.state.instance() {
                    Some(instance) => {
                        ui.separator();
                        let label = instance.world_name.clone().unwrap_or_else(|| instance.location.clone());
                        let label = if instance.is_group_instance { format!("{label}  (group)") } else { label };
                        ui.label(egui::RichText::new(label).small().strong()).on_hover_text(instance.location.clone());
                    }
                    None => {
                        ui.separator();
                        ui.label(egui::RichText::new(scarlet_core::i18n::tr("ui.statusNoInstance", &[]).trim()).small().weak());
                    }
                }
                // A dropped TTS announcement is otherwise silent in every
                // sense: nothing plays and nothing is shown.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(problem) = self.state.tts_status() {
                        ui.colored_label(crate::theme::WARN, "Voice warning")
                            .on_hover_text(&problem);
                    }
                });
            });
        });

        self.crash_report_window(ctx);
        self.exit_confirm_window(ctx);
        self.create_instance_window(ctx);
        self.import_groups_windows(ctx);
        self.avatar_stats_window(ctx);
        self.group_action_confirm_window(ctx);
        self.cache_cleanup_windows(ctx);
        self.settings_action_windows(ctx);
        self.utility_windows(ctx);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(crate::theme::INK)
                    .inner_margin(egui::Margin::same(crate::theme::PANEL_PAD)),
            )
            .show(ctx, |ui| match self.tab {
            Tab::Instance => self.instance_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Cli => self.cli_tab(ui),
        });

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
    if years != 0 { parts.push(format!("{years}y")); }
    if months != 0 { parts.push(format!("{months}m")); }
    if days != 0 { parts.push(format!("{days}d")); }
    if parts.is_empty() { "0D".to_string() } else { parts.join(" ") }
}

fn sort_players(players: &mut [scarlet_core::ui::ConnectedPlayer], sort: Option<TableSort>) {
    let Some(sort) = sort else {
        return;
    };
    fn text(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
        left.map(str::to_lowercase).cmp(&right.map(str::to_lowercase))
    }
    players.sort_by(|left, right| {
        let ordering = match sort.column {
            ColumnId::Name => text(Some(&left.name), Some(&right.name)),
            ColumnId::Id => text(Some(&left.id), Some(&right.id)),
            ColumnId::Pronouns => text(left.pronouns.as_deref(), right.pronouns.as_deref()),
            ColumnId::Avatar => text(left.avatar_name.as_deref(), right.avatar_name.as_deref()),
            ColumnId::Performance => text(
                left.avatar_info.as_ref().and_then(|info| info.performance_rating.as_deref()),
                right.avatar_info.as_ref().and_then(|info| info.performance_rating.as_deref()),
            ),
            ColumnId::AccAge => left.account_age.cmp(&right.account_age),
            ColumnId::Joined => left.joined.cmp(&right.joined),
            ColumnId::Left => left.left.cmp(&right.left),
            ColumnId::Advisory => text(left.advisory.as_deref(), right.advisory.as_deref()),
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
            | ColumnId::Invite => {
                std::cmp::Ordering::Equal
            }
        };
        if sort.ascending { ordering } else { ordering.reverse() }
    });
}

/// Recursively copies a data directory, skipping the regenerable parts.
/// Returns how many entries were copied.
///
/// `logs/` and `caches/` are deliberately not carried over: they are the
/// bulk of the folder, they rebuild themselves, and copying tens of
/// megabytes of old logs into a fresh location is not what "move my data"
/// means.
fn copy_data_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<usize> {
    const SKIP: [&str; 3] = ["logs", "caches", "backups"];
    std::fs::create_dir_all(to)?;
    let mut copied = 0usize;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if SKIP.iter().any(|skip| name == std::ffi::OsStr::new(skip)) {
            continue;
        }
        let source = entry.path();
        let destination = to.join(&name);
        if entry.file_type()?.is_dir() {
            copied += copy_data_dir(&source, &destination)?;
        } else {
            std::fs::copy(&source, &destination)?;
            copied += 1;
        }
    }
    Ok(copied)
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
            scarlet_discord::avatar_search::BUILTIN_PROVIDERS
                .iter()
                .map(|url| {
                    let label = scarlet_discord::avatar_search::provider_name(url)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            scarlet_discord::avatar_search::provider_host(url).to_string()
                        });
                    (label, (*url).to_string())
                })
                .collect(),
        ),
        _ => None,
    }
}

/// The window and taskbar icon, read from `icon.png` in the data folder or
/// beside the executable.
///
/// Loaded at runtime rather than embedded so replacing it does not mean
/// rebuilding, and so no placeholder artwork has to be invented to keep the
/// build compiling. Anything unreadable is ignored: a broken icon file is
/// not a reason to fail to start.
fn load_window_icon(data_dir: &std::path::Path) -> Option<egui::IconData> {
    let beside_binary = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("icon.png")));
    let candidates = [Some(data_dir.join("icon.png")), beside_binary];
    for path in candidates.into_iter().flatten() {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        match image::load_from_memory(&bytes) {
            Ok(image) => {
                let image = image.to_rgba8();
                let (width, height) = image.dimensions();
                tracing::info!("Using window icon {}", path.display());
                return Some(egui::IconData {
                    rgba: image.into_raw(),
                    width,
                    height,
                });
            }
            Err(error) => {
                tracing::warn!("Ignoring unreadable window icon {}: {error}", path.display());
            }
        }
    }
    None
}

fn settings_search_id() -> egui::Id {
    egui::Id::new("settings-search")
}

fn commit_text_setting(lost_focus: bool, enter_pressed: bool) -> bool {
    lost_focus || enter_pressed
}

fn row_background(player: &scarlet_core::ui::ConnectedPlayer) -> Option<egui::Color32> {
    (player.pronouns_flagged && player.left.is_none() && player.text_color.is_none())
        .then(|| crate::theme::PRONOUNS_FLAGGED_FILL)
}

/// Port of `getOverrideForegroundColor` in `ScarletUI.java:787-813`. The
/// "player has left" dimming Java does here is applied in the table body
/// instead, via `ui.set_opacity`, because egui composites the whole cell.
fn row_foreground(player: &scarlet_core::ui::ConnectedPlayer) -> Option<egui::Color32> {
    if let Some((r, g, b)) = player.text_color {
        return Some(egui::Color32::from_rgb(r, g, b));
    }
    player.pronouns_flagged.then(|| crate::theme::PRONOUNS_FLAGGED_TEXT)
}

pub fn run(
    state: UiState,
    shared: scarlet_discord::data::SharedState,
    data_dir: PathBuf,
    runtime_handle: tokio::runtime::Handle,
    tts: Arc<crate::tts::TtsService>,
    tts_route: Arc<crate::tts::TtsSinkAdapter>,
    pronoun_lists: Arc<scarlet_core::pronoun_lists::PronounLists>,
    two_factor_tx: std::sync::mpsc::Sender<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
    two_factor_rx: std::sync::mpsc::Receiver<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
) -> eframe::Result<()> {
    let bounds = shared.settings.ui_bounds.get_or_null().filter(|bounds| {
        bounds.width > 0 && bounds.height > 0
    });
    // Celeste has no artwork of its own yet -- the image that used to be
    // bundled was Scarlet's -- so rather than ship a placeholder, the icon
    // is read from disk at startup. Drop a PNG at `icon.png` in the data
    // folder (or beside the binary) and it is picked up on the next launch,
    // with no rebuild. Absent, the platform's neutral default is used.
    let mut viewport = egui::ViewportBuilder::default();
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
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        scarlet_core::identity::APP_NAME,
        options,
        Box::new(move |_cc| {
            Ok(Box::new(ScarletApp::new(
                state,
                shared,
                data_dir,
                runtime_handle,
                tts,
                tts_route,
                pronoun_lists,
                two_factor_tx,
                two_factor_rx,
            )))
        }),
    )
}

/// Spawns a background, unattended VRChat login using whatever credentials
/// are already stored, filling `slot` on success. Used whenever there's no
/// freshly-authenticated client from the setup wizard to reuse (setup
/// already completed in an earlier session, or the VRChat step was
/// skipped) — see `scarlet_vrchat::client::login_with_stored_credentials`
/// for what this can't do (no stored-TOTP-secret support, so an account
/// requiring 2FA needs the interactive wizard instead).
fn spawn_unattended_vrchat_login(
    runtime_handle: &tokio::runtime::Handle,
    slot: scarlet_discord::data::SharedVrChatClient,
    settings: Arc<Settings>,
    two_factor_tx: std::sync::mpsc::Sender<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>,
) {
    runtime_handle.spawn(async move {
        match scarlet_vrchat::client::login_with_stored_credentials_detailed(&settings).await {
            scarlet_vrchat::client::StoredLogin::LoggedIn(client) => {
                *slot.write().await = Some(Arc::new(client));
            }
            // The account has 2FA: hand the half-authenticated client to the
            // UI thread so it can ask for the code, the way Java prompts at
            // startup. Dropping it here (which is what used to happen) left
            // the app permanently signed out after first-run setup.
            scarlet_vrchat::client::StoredLogin::RequiresTwoFactor { client, methods } => {
                let _ = two_factor_tx.send((Arc::new(client), methods));
            }
            scarlet_vrchat::client::StoredLogin::Unavailable => {}
        }
    });
}

#[cfg(test)]
mod tests {
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
            scarlet_discord::avatar_search::BUILTIN_PROVIDERS.len()
        );
        for (label, value) in &catalogue {
            assert!(!label.is_empty(), "every choice needs a name to pick it by");
            assert!(!label.starts_with("http"), "a URL is not a name: {label}");
            assert!(value.starts_with("http"), "the stored value is still the URL");
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

        let copied = copy_data_dir(&from, &to).unwrap();

        assert!(to.join("settings.json").is_file(), "settings must come across");
        assert!(to.join("secure-prefs.json").is_file(), "the login must come across");
        assert!(to.join("usr").join("usr_1.json").is_file(), "per-user data must come across");
        assert!(!to.join("logs").exists(), "logs regenerate; don't carry them");
        assert!(!to.join("caches").exists(), "caches regenerate; don't carry them");
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
        let mut player = scarlet_core::ui::ConnectedPlayer::new("usr_1", "Someone", chrono::Utc::now().naive_utc());
        assert_eq!(row_foreground(&player), None);
        assert_eq!(row_background(&player), None);

        player.text_color = Some((200, 30, 30));
        assert_eq!(row_foreground(&player), Some(egui::Color32::from_rgb(200, 30, 30)));
    }

    #[test]
    fn flagged_pronouns_get_javas_amber_pair() {
        let mut player = scarlet_core::ui::ConnectedPlayer::new("usr_1", "Someone", chrono::Utc::now().naive_utc());
        player.pronouns_flagged = true;
        assert_eq!(row_foreground(&player), Some(egui::Color32::from_rgb(235, 190, 80)));
        assert_eq!(row_background(&player), Some(egui::Color32::from_rgb(80, 55, 10)));
    }

    /// Java's background condition is narrow on purpose: an advisory colour
    /// wins, and a departed player never gets the amber highlight.
    #[test]
    fn an_advisory_colour_or_a_departure_suppresses_the_amber_background() {
        let mut player = scarlet_core::ui::ConnectedPlayer::new("usr_1", "Someone", chrono::Utc::now().naive_utc());
        player.pronouns_flagged = true;
        player.text_color = Some((200, 30, 30));
        assert_eq!(row_background(&player), None);
        assert_eq!(row_foreground(&player), Some(egui::Color32::from_rgb(200, 30, 30)));

        let mut departed = scarlet_core::ui::ConnectedPlayer::new("usr_2", "Gone", chrono::Utc::now().naive_utc());
        departed.pronouns_flagged = true;
        departed.left = Some(chrono::Utc::now().naive_utc());
        assert_eq!(row_background(&departed), None);
    }

    use super::*;

    #[test]
    fn column_order_starts_as_all_columns_with_no_duplicates() {
        // Guards the drag-to-reorder column list against accidental
        // duplication/omission: exactly the 17 `propstable` columns,
        // matching ScarletUI.java's column set, each exactly once.
        let mut seen = std::collections::HashSet::new();
        for column in ColumnId::ALL {
            assert!(seen.insert(column), "duplicate column in ColumnId::ALL: {:?}", column);
        }
        assert_eq!(ColumnId::ALL.len(), 17);
        assert_eq!(ColumnId::DEFAULT_VISIBLE.len(), 15);
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
            scarlet_core::ui::ConnectedPlayer::new("usr_2", "Zulu", at),
            scarlet_core::ui::ConnectedPlayer::new("usr_1", "alpha", at),
        ];
        sort_players(
            &mut players,
            Some(TableSort { column: ColumnId::Name, ascending: true }),
        );
        assert_eq!(players[0].name, "alpha");
        sort_players(
            &mut players,
            Some(TableSort { column: ColumnId::Name, ascending: false }),
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
            ("Name".to_string(), UiPropsInfo { index: 1, width: 222 }),
            ("Id".to_string(), UiPropsInfo { index: 0, width: 333 }),
            ("Ban".to_string(), UiPropsInfo { index: -1, width: 70 }),
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
        assert_eq!(ScarletApp::setting_group("ui_language"), "Appearance");
        assert_eq!(ScarletApp::setting_group("ui_confirm_group_invite"), "Interface");
        assert_eq!(ScarletApp::setting_group("verified_role_snowflake"), "Verification");
        assert_eq!(ScarletApp::setting_group("discord_ping_outstanding_user_ban"), "Outstanding moderation");
        assert_eq!(ScarletApp::setting_group("custom_avatar_search_providers"), "Avatar search");
    }

    #[test]
    fn category_navigation_exits_global_search_mode() {
        let mut current = SettingCategory::General;
        let mut search = "voice channel".to_string();

        ScarletApp::activate_settings_category(
            &mut current,
            &mut search,
            SettingCategory::Discord,
        );

        assert_eq!(current, SettingCategory::Discord);
        assert!(search.is_empty());
    }

    #[test]
    fn notification_matrix_owns_shared_event_toggles() {
        assert!(ScarletApp::notification_matrix_id("tts_announce_watched_users"));
        assert!(ScarletApp::notification_matrix_id("toast_notify_staff"));
        assert!(!ScarletApp::notification_matrix_id("tts_voice_name"));
        assert!(!ScarletApp::notification_matrix_id("notify_desktop_enabled"));
    }

    #[test]
    fn world_id_resolver_accepts_an_id_or_vrchat_url() {
        let id = "wrld_01234567-89ab-cdef-0123-456789abcdef";
        assert_eq!(resolve_world_id(id).as_deref(), Some(id));
        assert_eq!(resolve_world_id(&format!("https://vrchat.com/home/world/{id}/info")).as_deref(), Some(id));
        assert_eq!(resolve_world_id("wrld_not-a-uuid"), None);
    }

    #[test]
    fn instance_name_validation_matches_java() {
        assert_eq!(validate_instance_display_name("  ").unwrap(), None);
        assert_eq!(validate_instance_display_name(" Movie night ").unwrap().as_deref(), Some("Movie night"));
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
        let request = create_instance_request(&form, "wrld_01234567-89ab-cdef-0123-456789abcdef", "grp_owner").unwrap();
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
        let request = create_instance_request(&form, "wrld_01234567-89ab-cdef-0123-456789abcdef", "usr_owner").unwrap();
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
}
