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
    discord_config: Arc<scarlet_discord::data::DiscordConfigStore>,
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
        let discord_config = shared.discord_config.clone();
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
                        http: discord_http.clone(),
                        discord_config: discord_config.clone(),
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
            discord_http,
            discord_config,
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
            if ui.button("Import Groups (URL)").clicked() {
                self.state.append_cli_output("Import Groups (URL): not wired up yet.");
            }
            if ui.button("Import Groups (File)").clicked() {
                self.state.append_cli_output("Import Groups (File): not wired up yet.");
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
                    self.state.append_cli_output(format!("Avatar stats for {}: not wired up yet.", player.name));
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
                    self.state
                        .append_cli_output(format!("Ban {}: not wired up yet (needs the VRChat group-ban call).", player.name));
                }
            }
        }
    }

    fn instance_tab(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let players = self.state.ordered_players();
        let column_order = self.column_order.clone();

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
                            for column in &column_order {
                                row.col(|ui| {
                                    // One line per row: anything too long is
                                    // clipped with an ellipsis and readable
                                    // in full on hover, instead of wrapping
                                    // and pushing the row's height out.
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
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

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let Some(settings) = &self.settings else {
            ui.label("Settings unavailable (no settings store attached).");
            return;
        };
        let all = settings.registry.all();

        egui::SidePanel::left("settings_categories").resizable(false).show_inside(ui, |ui| {
            ui.add_space(4.0);
            for category in SettingCategory::ALL {
                let count = all.iter().filter(|d| d.category() == category).count();
                let label = format!("{}  ({count})", category.label());
                ui.selectable_value(&mut self.settings_category, category, label);
            }
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let category = self.settings_category;
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading(category.label());
                ui.separator();
                for descriptor in all.iter().filter(|d| d.category() == category) {
                    ui.horizontal(|ui| {
                        ui.label(descriptor.name());
                        match descriptor.kind() {
                            SettingKind::Boolean { .. } => {
                                let mut value = descriptor.get_as_string() == "true";
                                if ui.checkbox(&mut value, "").changed() {
                                    descriptor.set_from_string(&value.to_string(), "ui");
                                }
                            }
                            SettingKind::IntegerRange { min, max, .. } => {
                                let mut value: i32 = descriptor.get_as_string().parse().unwrap_or(*min);
                                if ui.add(egui::Slider::new(&mut value, *min..=*max)).changed() {
                                    descriptor.set_from_string(&value.to_string(), "ui");
                                }
                            }
                            SettingKind::Enum { variants, .. } => {
                                let mut current = descriptor.get_as_string();
                                egui::ComboBox::from_id_salt(descriptor.id()).selected_text(current.clone()).show_ui(ui, |ui| {
                                    for variant in variants {
                                        if ui.selectable_value(&mut current, variant.clone(), variant).changed() {
                                            descriptor.set_from_string(variant, "ui");
                                        }
                                    }
                                });
                            }
                            SettingKind::StringChoice { valid_values } => {
                                let mut current = descriptor.get_as_string();
                                let selected = if current.is_empty() { "System default".to_string() } else { current.clone() };
                                egui::ComboBox::from_id_salt(descriptor.id()).selected_text(selected).show_ui(ui, |ui| {
                                    if ui.selectable_value(&mut current, String::new(), "System default").changed() {
                                        descriptor.set_from_string("", "ui");
                                    }
                                    for value in valid_values() {
                                        if ui.selectable_value(&mut current, value.clone(), &value).changed() {
                                            descriptor.set_from_string(&value, "ui");
                                        }
                                    }
                                });
                            }
                            _ => {
                                let buf = self
                                    .settings_edit_buffers
                                    .entry(descriptor.id().to_string())
                                    .or_insert_with(|| descriptor.get_as_string());
                                ui.text_edit_singleline(buf);
                                if ui.button("Set").clicked() {
                                    descriptor.set_from_string(buf, "ui");
                                }
                            }
                        }
                    });
                }
                if !all.iter().any(|d| d.category() == category) {
                    ui.label("(No settings in this category.)");
                }
            });
        });
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

    /// Incremental port of Scarlet's command interpreter.
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
                            http: self.discord_http.clone(),
                            discord_config: self.discord_config.clone(),
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
                if let Some(announcement) = self.state.announcement() {
                    ui.separator();
                    ui.colored_label(egui::Color32::YELLOW, announcement.title.as_deref().unwrap_or("Scarlet announcement"));
                }
            });
        });

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
pub fn run(
    state: UiState,
    shared: scarlet_discord::data::SharedState,
    data_dir: PathBuf,
    runtime_handle: tokio::runtime::Handle,
    tts: Arc<crate::tts::TtsService>,
) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
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
