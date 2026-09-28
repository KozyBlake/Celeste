//! Port target: `ScarletUI.java` (5,716 lines — the full Swing main window)
//! and `ScarletDiscordUI.java` (2,345 lines — Discord bot config panel).
//!
//! This covers the *shape* real-to-real: the toolbar row, the Instance/
//! Settings/CLI tab structure, the live player table with the same columns
//! as `propstable` in the Java source, and the status bar text format — all
//! driven by real state ([`crate::state::UiState`] for players/status,
//! [`scarlet_core::settings`] for the Settings tab), not fixtures. What
//! isn't ported: several toolbar buttons' actual behavior (Create Instance,
//! Import Groups, Sync Discord are logged as not-yet-wired
//! rather than faked), moderation actions (Ban/Unban/Invite are stubs),
//! and most of the CLI command interpreter. TTS pause/resume, skip, and the
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
}

impl ColumnId {
    const ALL: [ColumnId; 15] = [
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
        }
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
            ColumnId::Ban => 45.0,
        }
    }

    /// How far a column can be dragged in before its header stops being
    /// readable.
    fn min_width(&self) -> f32 {
        match self {
            ColumnId::Age18 | ColumnId::Ban | ColumnId::Profile | ColumnId::CopyId | ColumnId::AvatarStats => 40.0,
            _ => 55.0,
        }
    }
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
    two_factor_rx: Option<std::sync::mpsc::Receiver<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)>>,
    scarlet_data: Arc<scarlet_core::data::ScarletData>,
    staff_list: Arc<scarlet_core::staff_list::StaffList>,
    secret_staff_list: Arc<scarlet_core::staff_list::StaffList>,
    moderation_tags: Arc<scarlet_core::moderation_tags::ModerationTags>,
    watched_groups: Arc<scarlet_core::watched_groups::WatchedGroups>,
    watched_users: Arc<scarlet_core::watched_entities::WatchedEntities>,
    watched_avatars: Arc<scarlet_core::watched_entities::WatchedEntities>,
    tts: Arc<crate::tts::TtsService>,
    tab: Tab,
    cli_input: String,
    settings_edit_buffers: std::collections::HashMap<String, String>,
    /// Current left-to-right order of the Instance tab's columns. Starts as
    /// `ColumnId::ALL` (matching `propstable`'s declared order) and can be
    /// changed by dragging a header, matching the original Swing
    /// `JTable`'s column-reordering behavior (`JTableHeader` lets you drag
    /// columns; there's no persisted-order file in the Java version to
    /// match either, so this resets on restart there too).
    column_order: Vec<ColumnId>,
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
    logo: Option<egui::TextureHandle>,
    ban_confirm: Option<(String, String)>,
    import_groups_url_open: bool,
    import_groups_url: String,
    import_groups_rx: Option<(String, std::sync::mpsc::Receiver<Result<String, String>>)>,
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

        let mut pending_two_factor = None;
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
                pending_two_factor = Some(spawn_unattended_vrchat_login(&runtime_handle, vrchat_slot.clone(), settings.clone()));

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
            two_factor_rx: pending_two_factor.take(),
            scarlet_data,
            staff_list,
            secret_staff_list,
            moderation_tags,
            watched_groups,
            watched_users,
            watched_avatars,
            tts,
            tab: Tab::Instance,
            cli_input: String::new(),
            settings_edit_buffers: std::collections::HashMap::new(),
            column_order: ColumnId::ALL.to_vec(),
            settings_category: SettingCategory::General,
            settings_search: String::new(),
            avatar_stats_open: None,
            logo: None,
            ban_confirm: None,
                    import_groups_url_open: false,
            import_groups_url: String::new(),
            import_groups_rx: None,
        }
    }

    fn window_title(&self) -> String {
        if self.staff_mode {
            format!("{} (staff mode)", scarlet_core::identity::NAME)
        } else {
            format!(
                "{} {} \u{2014} {}",
                scarlet_core::identity::APP_NAME,
                scarlet_core::identity::VERSION,
                scarlet_core::identity::FORK_NOTE
            )
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Data Folder").on_hover_text("Open the KozyBlake/Scarlet data directory").clicked() {
                if let Err(err) = open::that(&self.data_dir) {
                    self.state.append_cli_output(format!("Could not open data folder: {err}"));
                }
            }
            if ui.button("Create Instance").clicked() {
                self.state.append_cli_output("Create Instance: not wired up yet (needs the VRChat instance-creation flow).");
            }
            if ui.button("Import Groups (URL)").on_hover_text("Import watched groups from a CSV or JSON URL").clicked() {
                self.import_groups_url_open = true;
                self.import_groups_url.clear();
            }
            if ui.button("Import Groups (File)").on_hover_text("Import watched groups from a CSV or JSON file").clicked() {
                self.import_groups_from_file();
            }
            if ui.button("Sync Discord").clicked() {
                self.state.append_cli_output("Sync Discord: not wired up yet.");
            }
            if ui.button("Clear Instance").clicked() {
                self.state.clear_instance();
                self.state.append_cli_output("Instance cleared.");
            }
            let tts_on = self.state.tts_enabled();
            if ui.button(if tts_on { "TTS: On" } else { "TTS: Off" }).clicked() {
                self.state.set_tts_enabled(!tts_on);
                self.tts.set_paused(tts_on);
            }
            if ui.button("Skip TTS").clicked() {
                self.state.append_cli_output(if self.tts.skip() { "Skipped the current TTS announcement." } else { "No TTS announcement is currently playing." });
            }
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Instance, "  Instance  ");
            ui.selectable_value(&mut self.tab, Tab::Settings, "  Settings  ");
            ui.selectable_value(&mut self.tab, Tab::Cli, "  CLI  ");
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
                if ui.button("View").clicked() {
                    self.avatar_stats_open = Some(AvatarStatsView {
                        player_name: player.name.clone(),
                        avatar_name: player.avatar_name.clone(),
                        info: player.avatar_info.clone(),
                        note: player.avatar_info_note.clone(),
                    });
                }
            }
            ColumnId::AccAge => {
                let age = player.account_age.map(|(y, m, d)| format!("{y}y {m}m {d}d")).unwrap_or_default();
                Self::text_cell(ui, &age);
            }
            ColumnId::Joined => {
                Self::text_cell(ui, &player.joined.format("%Y-%m-%d %H:%M").to_string());
            }
            ColumnId::Left => {
                let left = player.left.map(|l| l.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
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
                if ui.button("Open").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::same_tab(format!("https://vrchat.com/home/user/{}", player.id)));
                }
            }
            ColumnId::CopyId => {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(player.id.clone());
                }
            }
            ColumnId::Ban => {
                if ui.button("Ban").clicked() {
                    // Java confirms before banning from the table; so does
                    // this, because a stray click in a row list is easy.
                    self.ban_confirm = Some((player.id.clone(), player.name.clone()));
                }
            }
        }
    }

    fn instance_tab(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let players = self.state.ordered_players();
        if players.is_empty() {
            return self.instance_empty_state(ui);
        }
        let column_order = self.column_order.clone();
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

        egui::ScrollArea::horizontal().show(ui, |ui| {
            // `Column::auto()` sizes to content, which is what let a long
            // VRChat id wrap over five lines. A fixed initial width that
            // clips (and can still be dragged wider) is the Swing
            // `JTable`'s behaviour.
            let mut table = TableBuilder::new(ui).striped(true).resizable(true);
            for column in &column_order {
                table = table.column(
                    Column::initial(column.initial_width()).at_least(column.min_width()).clip(true).resizable(true),
                );
            }

            table
                .header(20.0, |mut header| {
                    for (index, column) in column_order.iter().enumerate() {
                        header.col(|ui| {
                            let (_, dropped) = ui.dnd_drop_zone::<usize, ()>(egui::Frame::none(), |ui| {
                                ui.dnd_drag_source(egui::Id::new("instance_column_header").with(index), index, |ui| {
                                    ui.strong(column.label());
                                });
                            });
                            if let Some(dragged_from) = dropped {
                                swap = Some((*dragged_from, index));
                            }
                        });
                    }
                })
                .body(|mut body| {
                    for player in &players {
                        body.row(20.0, |mut row| {
                            let departed = player.left.is_some();
                            for column in &column_order {
                                row.col(|ui| {
                                    // One line per row: anything too long is
                                    // clipped with an ellipsis and readable
                                    // in full on hover, instead of wrapping
                                    // and pushing the row's height out.
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
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

        if let Some((from, to)) = swap {
            if from != to && from < self.column_order.len() && to < self.column_order.len() {
                self.column_order.swap(from, to);
            }
        }
    }

    /// What the Instance tab shows before anyone has been logged: the logo,
    /// and which of the two reasons applies — no instance open, or an
    /// instance open that nobody has joined yet. Java shows a plain "No
    /// instance loaded" card here.
    fn instance_empty_state(&mut self, ui: &mut egui::Ui) {
        let logo = self.logo_texture(ui.ctx());
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            if let Some(logo) = logo {
                ui.add(egui::Image::new(&logo).max_width(96.0).tint(egui::Color32::from_white_alpha(70)));
                ui.add_space(12.0);
            }
            match self.state.instance() {
                Some(instance) => {
                    let where_ = instance.world_name.clone().unwrap_or_else(|| instance.location.clone());
                    ui.label(egui::RichText::new(format!("In {where_}")).size(15.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Nobody has joined yet \u{2014} players appear here as they arrive.").weak());
                }
                None => {
                    ui.label(egui::RichText::new(scarlet_core::i18n::tr("ui.statusNoInstance", &[]).trim()).size(15.0));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(scarlet_core::i18n::tr("ui.playerDataWillAppear", &[])).weak());
                }
            }
        });
    }

    /// The logo, uploaded to the renderer once and reused.
    fn logo_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if let Some(handle) = &self.logo {
            return Some(handle.clone());
        }
        const LOGO_PNG: &[u8] = include_bytes!("../../../assets/celeste.png");
        let icon = image_from_png(LOGO_PNG)?;
        let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
        let handle = ctx.load_texture("celeste-logo", image, egui::TextureOptions::LINEAR);
        self.logo = Some(handle.clone());
        Some(handle)
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
                        }
                        None => {
                            ui.label("Performance");
                            ui.weak("not resolved");
                            ui.end_row();
                        }
                    }
                });

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

    /// The table's Ban action. Same call the `/vrchat-user-ban` command
    /// makes, run on the shared VRChat session.
    fn ban_confirm_window(&mut self, ctx: &egui::Context) {
        let Some((user_id, display_name)) = self.ban_confirm.clone() else { return };
        let mut open = true;
        let mut decided = false;
        egui::Window::new("Ban from the VRChat group")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Ban {display_name} from the configured VRChat group?"));
                ui.add(egui::Label::new(egui::RichText::new(&user_id).small().monospace().weak()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Ban").clicked() {
                        decided = true;
                        self.spawn_group_ban(user_id.clone(), display_name.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        decided = true;
                    }
                });
            });
        if decided || !open {
            self.ban_confirm = None;
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

    /// Port of `ScarletUI.importWG(true)`: pick a `.csv` or `.json` file
    /// and import it over the current watched groups.
    fn import_groups_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import watched groups")
            .add_filter("CSV or JSON", &["csv", "json"])
            .pick_file()
        else {
            self.state.append_cli_output("Import groups: canceled.");
            return;
        };
        let contents = match std::fs::read(&path) {
            Ok(contents) => contents,
            Err(err) => {
                self.state.append_cli_output(format!("Import groups: could not read {}: {err}", path.display()));
                return;
            }
        };
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        self.apply_group_import(&extension, &contents, &path.display().to_string());
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

        if let Some((extension, rx)) = &self.import_groups_rx {
            if let Ok(result) = rx.try_recv() {
                let extension = extension.clone();
                self.import_groups_rx = None;
                match result {
                    Ok(text) => self.apply_group_import(&extension, text.as_bytes(), "the URL"),
                    Err(err) => self.state.append_cli_output(format!("Import groups failed: {err}")),
                }
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

    /// A readable group heading from a setting's id, so a category of forty
    /// entries reads as a handful of short lists instead of one wall.
    /// Java's Swing UI had only the category sidebar; this is the "1:1 plus
    /// fixes" part.
    fn setting_group(id: &str) -> &'static str {
        const GROUPS: [(&str, &str); 14] = [
            ("tts_", "Text to speech"),
            ("advisory_", "Advisories"),
            ("toast_", "Desktop notifications"),
            ("notify_", "Desktop notifications"),
            ("discord_ping_", "Discord pings"),
            ("discord_", "Discord"),
            ("enforce_instances", "Instance enforcement"),
            ("auto_invite", "Auto-invite"),
            ("ui_", "Appearance"),
            ("cache_", "Cache"),
            ("audit_", "Audit polling"),
            ("moderation_", "Moderation"),
            ("instance_", "Instances"),
            ("vrchat_", "VRChat"),
        ];
        GROUPS
            .iter()
            .find(|(prefix, _)| id.starts_with(prefix))
            .map(|(_, label)| *label)
            .unwrap_or("Other")
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

        egui::SidePanel::left("settings_categories").resizable(false).min_width(150.0).show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("CATEGORIES").small().weak());
            ui.add_space(2.0);
            for category in SettingCategory::ALL {
                let count = visible.iter().filter(|d| d.category() == category).count();
                let label = if count == 0 {
                    egui::RichText::new(format!("{}  ({count})", category.label())).weak()
                } else {
                    egui::RichText::new(format!("{}  ({count})", category.label()))
                };
                ui.selectable_value(&mut self.settings_category, category, label);
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Settings for features this build doesn't have are hidden.").small().weak());
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let category = self.settings_category;

            ui.horizontal(|ui| {
                ui.heading(category.label());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.settings_search.is_empty() && ui.button("\u{2715}").on_hover_text("Clear search").clicked() {
                        self.settings_search.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_search)
                            .desired_width(220.0)
                            .hint_text("Search all settings"),
                    );
                });
            });
            ui.separator();

            // A search looks across every category; otherwise this is the
            // selected one.
            let rows: Vec<_> = if query.is_empty() {
                visible.iter().filter(|d| d.category() == category).copied().collect()
            } else {
                visible.clone()
            };

            if rows.is_empty() {
                ui.add_space(12.0);
                ui.weak(if query.is_empty() {
                    "No settings in this category.".to_string()
                } else {
                    format!("Nothing matches \"{}\".", self.settings_search.trim())
                });
                return;
            }

            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut current_group = "";
                for descriptor in rows {
                    let group = Self::setting_group(descriptor.id());
                    if group != current_group {
                        current_group = group;
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new(group.to_uppercase()).small().weak());
                        ui.add_space(2.0);
                    }

                    // One row: name and id on the left, the control on the
                    // right, at a fixed width so controls line up down the
                    // column instead of wandering with the label length.
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - CONTROL_WIDTH);
                            ui.label(descriptor.name());
                            ui.label(egui::RichText::new(descriptor.id()).small().weak().monospace())
                                .on_hover_text("Use this id with /settings edit in Discord");
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_setting_control(ui, descriptor);
                        });
                    });
                    ui.add_space(2.0);
                    ui.separator();
                }
                ui.add_space(12.0);
            });
        });
    }

    /// The control for one setting, sized so the column lines up.
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
                        if ui.small_button("\u{21BA}").on_hover_text("Reset to default").clicked() {
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
                        if ui.small_button("\u{21BA}").on_hover_text("Reset to default").clicked() {
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
            _ => {
                // Text-ish settings commit on Enter or focus loss, so a
                // half-typed value is never written, and the field shows
                // whether the last commit validated.
                let id = descriptor.id().to_string();
                let buf = self.settings_edit_buffers.entry(id.clone()).or_insert_with(|| descriptor.get_as_string());
                let response = ui.add(egui::TextEdit::singleline(buf).desired_width(CONTROL_WIDTH - 12.0));
                let commit =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || response.lost_focus();
                if commit {
                    let accepted = descriptor.set_from_string(buf, "ui");
                    if !accepted {
                        // Put the stored value back rather than leaving a
                        // rejected string sitting in the box looking saved.
                        *buf = descriptor.get_as_string();
                        self.state.append_cli_output(format!("Setting {id}: value rejected (didn't validate)."));
                    }
                }
            }
        }
    }

    fn cli_tab(&mut self, ui: &mut egui::Ui) {
        let available_height = ui.available_height() - 30.0;
        egui::ScrollArea::vertical().max_height(available_height.max(0.0)).stick_to_bottom(true).show(ui, |ui| {
            for line in self.state.cli_log() {
                ui.monospace(&line.0);
            }
        });
        ui.horizontal(|ui| {
            let response = ui.text_edit_singleline(&mut self.cli_input);
            let run = ui.button("Run").clicked() || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if run && !self.cli_input.trim().is_empty() {
                self.run_cli_command(self.cli_input.trim().to_string());
                self.cli_input.clear();
            }
        });
    }

    /// Incremental port of Celeste's command interpreter.
    fn run_cli_command(&mut self, command: String) {
        self.state.append_cli_output(format!("> {command}"));
        if let Some(text) = command.strip_prefix("tts ").map(str::trim).filter(|text| !text.is_empty()) {
            let voice = self.settings.as_ref().map(|settings| settings.tts_voice_name.get()).unwrap_or_default();
            match self.tts.submit(text, voice) {
                Ok(()) => self.state.append_cli_output(format!("Submitting TTS: `{text}`")),
                Err(err) => self.state.append_cli_output(format!("[warn] TTS service not available: {err}")),
            }
            return;
        }
        match command.as_str() {
            "help" => self.state.append_cli_output("Commands: help, clear, tts <text>. More Java CLI commands are still being ported."),
            "clear" => self.state.clear_cli_log(),
            other => self.state.append_cli_output(format!("Unknown command: {other} (type 'help')")),
        }
    }
}

impl eframe::App for ScarletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));

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
            if let Some(rx) = &self.two_factor_rx {
                if let Ok((client, methods)) = rx.try_recv() {
                    self.two_factor_rx = None;
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
                            self.two_factor_rx =
                                Some(spawn_unattended_vrchat_login(&self.runtime_handle, self.vrchat_slot.clone(), settings));
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

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
            ui.separator();
            self.tab_bar(ui);
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
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
                ui.label(text);
                // Which instance those players are in. Java never showed
                // this; without it there is no way to tell "no instance
                // open" from "instance open, nobody joined yet".
                match self.state.instance() {
                    Some(instance) => {
                        ui.separator();
                        let label = instance.world_name.clone().unwrap_or_else(|| instance.location.clone());
                        let label = if instance.is_group_instance { format!("{label}  (group)") } else { label };
                        ui.label(label).on_hover_text(instance.location.clone());
                    }
                    None => {
                        ui.separator();
                        ui.weak(scarlet_core::i18n::tr("ui.statusNoInstance", &[]).trim().to_string());
                    }
                }
                // The VRChat session, which every profile-derived column
                // depends on.
                let vrchat_status = self.state.connection_status();
                if !vrchat_status.is_empty() {
                    ui.separator();
                    let signed_in = vrchat_status.contains("signed in as");
                    let colour = if signed_in { egui::Color32::from_rgb(95, 200, 120) } else { egui::Color32::from_rgb(235, 165, 65) };
                    ui.colored_label(colour, vrchat_status);
                }
                // A dropped TTS announcement is otherwise silent in every
                // sense: nothing plays and nothing is shown.
                if let Some(problem) = self.state.tts_status() {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(235, 165, 65), "TTS")
                        .on_hover_text(&problem);
                }
                if let Some(announcement) = self.state.announcement() {
                    ui.separator();
                    ui.colored_label(egui::Color32::YELLOW, announcement.title.as_deref().unwrap_or("Celeste announcement"));
                }
            });
        });

        self.import_groups_windows(ctx);
        self.avatar_stats_window(ctx);
        self.ban_confirm_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
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
/// The window/taskbar icon.
///
/// Embedded rather than loaded from disk so a build that has been copied
/// somewhere else still has it. A boot splash was considered and
/// deliberately not added: this starts in well under a second, so a splash
/// would be a delay rather than a progress indicator — the reason Java
/// needed one (JVM start plus dependency loading) doesn't exist here.
fn app_icon() -> std::sync::Arc<egui::IconData> {
    const ICON_PNG: &[u8] = include_bytes!("../../../assets/celeste.png");
    // A failure here must never stop the window opening, so fall back to an
    // empty icon rather than unwrapping.
    match image_from_png(ICON_PNG) {
        Some(icon) => std::sync::Arc::new(icon),
        None => std::sync::Arc::new(egui::IconData { rgba: vec![0; 4], width: 1, height: 1 }),
    }
}

fn image_from_png(bytes: &[u8]) -> Option<egui::IconData> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    Some(egui::IconData { rgba, width: info.width, height: info.height })
}

pub fn run(
    state: UiState,
    shared: scarlet_discord::data::SharedState,
    data_dir: PathBuf,
    runtime_handle: tokio::runtime::Handle,
    tts: Arc<crate::tts::TtsService>,
) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        scarlet_core::identity::APP_NAME,
        options,
        Box::new(move |_cc| Ok(Box::new(ScarletApp::new(state, shared, data_dir, runtime_handle, tts)))),
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
) -> std::sync::mpsc::Receiver<(Arc<scarlet_vrchat::client::VrChatClient>, Vec<String>)> {
    let (tx, rx) = std::sync::mpsc::channel();
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
                let _ = tx.send((Arc::new(client), methods));
            }
            scarlet_vrchat::client::StoredLogin::Unavailable => {}
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_order_starts_as_all_columns_with_no_duplicates() {
        // Guards the drag-to-reorder column list against accidental
        // duplication/omission: exactly the 15 `propstable` columns,
        // matching ScarletUI.java's column set, each exactly once.
        let mut seen = std::collections::HashSet::new();
        for column in ColumnId::ALL {
            assert!(seen.insert(column), "duplicate column in ColumnId::ALL: {:?}", column);
        }
        assert_eq!(ColumnId::ALL.len(), 15);
    }

    #[test]
    fn every_column_has_a_non_empty_label_and_positive_width() {
        for column in ColumnId::ALL {
            assert!(!column.label().is_empty());
            assert!(column.min_width() > 0.0);
        }
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
}
