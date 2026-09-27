//! First-run setup wizard: port of the interactive parts of `Scarlet.run()`
//! (`showVibecodedNoticeIfNeeded`), `ScarletDiscordJDA`'s constructor (the
//! bot-token/guild-snowflake prompt), and `ScarletVRChat.login()` (the
//! username/password/TOTP prompt) — all of which run as blocking Swing
//! dialogs in Java, before the main window is usable. `main.rs` previously
//! skipped straight to the main window with whatever was (or wasn't)
//! already in `settings.json`; this is what was missing.
//!
//! Swing's dialogs block the calling thread until answered. egui is
//! immediate-mode with no blocking-dialog equivalent, so this is instead a
//! small state machine (`Stage`) that [`app::ScarletApp`] renders in place
//! of the normal toolbar/tabs/table until it reports finished, exactly like
//! a modal would from the user's point of view (nothing else is shown or
//! interactive while it's running).
//!
//! ## Deliberate deviations from the exact Java gating
//!
//! Java re-prompts for the Discord bot token on *every* launch if it was
//! ever left blank — its only gate is "is a token already stored", and
//! leaving the prompt blank never stores anything (`normalizeDiscordBotToken`
//! turns an empty string back into `null`, and `RegistryStringValued::set`
//! is a no-op for `None`). That reads as a Java edge case rather than an
//! intentional feature, and copying it faithfully would mean staff-mode
//! users get nagged on every single launch. Instead, this gates the whole
//! Discord step on a dedicated `discord_setup_prompted` flag
//! (`scarlet_core::settings::Settings`) that's set once the step is
//! completed, however it's answered.
//!
//! VRChat's stage stays closer to Java's real behavior: it's gated on
//! whether credentials are already stored, matching `ScarletVRChat.login`,
//! since VRChat login can't have a permanent "skip forever" — an
//! unconfigured VRChat account means the bot has nothing to moderate.
//! "Skip for now" just leaves credentials unset, so the prompt reappears
//! next launch.
//!
//! ## Known gap: email-OTP two-factor
//!
//! VRChat's `requiresTwoFactorAuth` can list `totp` and/or `emailOtp`.
//! `scarlet_vrchat::client::VrChatClient` only has `verify_totp` so far (no
//! `verify2FAEmailCode` equivalent — see that crate's PORTING.md entry).
//! This wizard always submits to the TOTP endpoint regardless of which
//! methods VRChat reported, which works for the common authenticator-app
//! case but will reject an email code with a generic error until that
//! endpoint is ported.

use std::sync::mpsc;
use std::sync::Arc;

use scarlet_core::settings::file_valued::FileValued;
use scarlet_core::settings::registry::{registry_string_encrypted, RegistryStringValued};
use scarlet_core::settings::Settings;
use scarlet_vrchat::client::{LoginOutcome, VrChatClient};

enum Stage {
    VibecodedNotice,
    Discord { token: String, guild_id: String },
    VrChatCredentials { username: String, password: String, error: Option<String>, in_flight: bool },
    VrChatTotp { code: String, error: Option<String>, in_flight: bool },
    Done,
}

/// What the wizard did this frame, decided during rendering and applied
/// afterwards. Kept as a plain value (rather than mutating `self` directly
/// from inside the render match) so the render closure only ever needs
/// `&mut` access to the current stage's own local buffers, not all of
/// `self` — see the module docs' note on why this is a two-phase design.
enum Action {
    None,
    VibecodedContinue,
    DiscordContinue { token: Option<String>, guild_id: Option<String> },
    VrChatCredentialsSubmit { username: String, password: String },
    VrChatSkip,
    TotpSubmit { code: String },
    TotpCancel,
}

/// What [`SetupWizard::render`] wants the caller ([`crate::app::ScarletApp`])
/// to do this frame.
pub enum SetupOutcome {
    /// Still running; render nothing else this frame.
    InProgress,
    /// Just finished (this frame). Act on `discord_token` exactly once —
    /// e.g. spawn the Discord client — then start rendering the normal UI.
    JustFinished { discord_token: Option<String> },
    /// Already finished (on a previous frame, or never needed to run).
    AlreadyDone,
}

pub struct SetupWizard {
    stage: Stage,
    done: bool,
    runtime_handle: tokio::runtime::Handle,
    vibecoded_ack: Arc<FileValued<bool>>,
    discord_setup_prompted: Arc<FileValued<bool>>,
    discord_token_reg: RegistryStringValued<String>,
    discord_guild_reg: RegistryStringValued<String>,
    vrchat_username_reg: RegistryStringValued<String>,
    vrchat_password_reg: RegistryStringValued<String>,
    vrchat_client: Option<Arc<VrChatClient>>,
    login_rx: Option<mpsc::Receiver<Result<LoginOutcome, String>>>,
    totp_rx: Option<mpsc::Receiver<Result<bool, String>>>,
}

impl SetupWizard {
    pub fn new(settings: &Arc<Settings>, runtime_handle: tokio::runtime::Handle) -> Self {
        let discord_token_reg = registry_string_encrypted(settings.encrypted.clone(), "discord.token");
        let discord_guild_reg = registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id");
        // Matches ScarletVRChat's own encrypted-key domain ("global:username"
        // etc. -- see `new ScarletVRChat(this, "global", ...)` in Scarlet.java)
        // so a data directory carried over from a Java install round-trips.
        let vrchat_username_reg = registry_string_encrypted(settings.encrypted.clone(), "global:username");
        let vrchat_password_reg = registry_string_encrypted(settings.encrypted.clone(), "global:password");

        let mut wizard = SetupWizard {
            stage: Stage::Done, // placeholder; replaced below once every field exists
            done: false,
            runtime_handle,
            vibecoded_ack: settings.vibecoded_notice_acknowledged.clone(),
            discord_setup_prompted: settings.discord_setup_prompted.clone(),
            discord_token_reg,
            discord_guild_reg,
            vrchat_username_reg,
            vrchat_password_reg,
            vrchat_client: None,
            login_rx: None,
            totp_rx: None,
        };
        wizard.stage =
            if !wizard.vibecoded_ack.get() { Stage::VibecodedNotice } else { wizard.compute_stage_after_vibecoded() };
        wizard.done = matches!(wizard.stage, Stage::Done);
        wizard
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    #[cfg(test)]
    fn stage_name(&self) -> &'static str {
        match self.stage {
            Stage::VibecodedNotice => "vibecoded_notice",
            Stage::Discord { .. } => "discord",
            Stage::VrChatCredentials { .. } => "vrchat_credentials",
            Stage::VrChatTotp { .. } => "vrchat_totp",
            Stage::Done => "done",
        }
    }

    /// The Discord bot token currently on disk, regardless of wizard state.
    /// Used by the caller to connect Discord immediately when the wizard is
    /// already done at construction time (setup ran in an earlier session,
    /// so there's nothing left to show, but Discord still needs to start).
    pub fn stored_discord_token(&self) -> Option<String> {
        self.discord_token_reg.get_or_null()
    }

    /// The already-authenticated VRChat client from a successful login
    /// during this wizard run, if the VRChat stage completed with one
    /// (rather than being skipped, or never needed because setup already
    /// ran in an earlier session). The caller should reuse this instead of
    /// logging in again from scratch, since re-logging in would needlessly
    /// ask VRChat's API a second time and could re-trigger 2FA.
    pub fn vrchat_client(&self) -> Option<Arc<VrChatClient>> {
        self.vrchat_client.clone()
    }

    fn compute_stage_after_vibecoded(&self) -> Stage {
        if !self.discord_setup_prompted.get() {
            Stage::Discord {
                token: self.discord_token_reg.get_or_null().unwrap_or_default(),
                guild_id: self.discord_guild_reg.get_or_null().unwrap_or_default(),
            }
        } else {
            self.compute_stage_after_discord()
        }
    }

    fn compute_stage_after_discord(&self) -> Stage {
        let vrchat_configured =
            self.vrchat_username_reg.get_or_null().is_some() && self.vrchat_password_reg.get_or_null().is_some();
        if vrchat_configured {
            Stage::Done
        } else {
            Stage::VrChatCredentials { username: String::new(), password: String::new(), error: None, in_flight: false }
        }
    }

    fn vrchat_client_or_init(&mut self) -> Option<Arc<VrChatClient>> {
        if self.vrchat_client.is_none() {
            match VrChatClient::new() {
                Ok(client) => self.vrchat_client = Some(Arc::new(client)),
                Err(err) => {
                    if let Stage::VrChatCredentials { error, .. } = &mut self.stage {
                        *error = Some(format!("Could not start the VRChat client: {err}"));
                    }
                    return None;
                }
            }
        }
        self.vrchat_client.clone()
    }

    fn start_login(&mut self, username: String, password: String) {
        let Some(client) = self.vrchat_client_or_init() else { return };
        let (tx, rx) = mpsc::channel();
        self.login_rx = Some(rx);
        if let Stage::VrChatCredentials { in_flight, error, .. } = &mut self.stage {
            *in_flight = true;
            *error = None;
        }
        self.runtime_handle.spawn(async move {
            let result = client.login_with_credentials(&username, &password).await.map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    fn start_totp(&mut self, code: String) {
        let Some(client) = self.vrchat_client.clone() else { return };
        let (tx, rx) = mpsc::channel();
        self.totp_rx = Some(rx);
        if let Stage::VrChatTotp { in_flight, error, .. } = &mut self.stage {
            *in_flight = true;
            *error = None;
        }
        self.runtime_handle.spawn(async move {
            let result = client.verify_totp(&code).await.map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    /// Drains any finished background VRChat call and applies its result to
    /// the current stage. Called at the start of every `render` so a result
    /// that landed between frames is reflected before this frame draws.
    fn poll_async_results(&mut self) {
        if let Some(rx) = &self.login_rx {
            match rx.try_recv() {
                Ok(Ok(LoginOutcome::LoggedIn(_user))) => {
                    self.login_rx = None;
                    self.stage = Stage::Done;
                }
                Ok(Ok(LoginOutcome::RequiresTwoFactor(_methods))) => {
                    self.login_rx = None;
                    self.stage = Stage::VrChatTotp { code: String::new(), error: None, in_flight: false };
                }
                Ok(Err(message)) => {
                    self.login_rx = None;
                    if let Stage::VrChatCredentials { error, in_flight, .. } = &mut self.stage {
                        *error = Some(message);
                        *in_flight = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.login_rx = None;
                    if let Stage::VrChatCredentials { error, in_flight, .. } = &mut self.stage {
                        *error = Some("Login task ended unexpectedly — try again.".to_string());
                        *in_flight = false;
                    }
                }
            }
        }
        if let Some(rx) = &self.totp_rx {
            match rx.try_recv() {
                Ok(Ok(true)) => {
                    self.totp_rx = None;
                    self.stage = Stage::Done;
                }
                Ok(Ok(false)) => {
                    self.totp_rx = None;
                    if let Stage::VrChatTotp { error, in_flight, .. } = &mut self.stage {
                        *error = Some("Incorrect code — try again.".to_string());
                        *in_flight = false;
                    }
                }
                Ok(Err(message)) => {
                    self.totp_rx = None;
                    if let Stage::VrChatTotp { error, in_flight, .. } = &mut self.stage {
                        *error = Some(message);
                        *in_flight = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.totp_rx = None;
                    if let Stage::VrChatTotp { error, in_flight, .. } = &mut self.stage {
                        *error = Some("Verification task ended unexpectedly — try again.".to_string());
                        *in_flight = false;
                    }
                }
            }
        }
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::VibecodedContinue => {
                self.vibecoded_ack.set(Some(true), "setup-wizard");
                self.stage = self.compute_stage_after_vibecoded();
            }
            Action::DiscordContinue { token, guild_id } => {
                if let Some(token) = token {
                    self.discord_token_reg.set(Some(token));
                }
                if let Some(guild_id) = guild_id {
                    self.discord_guild_reg.set(Some(guild_id));
                }
                self.discord_setup_prompted.set(Some(true), "setup-wizard");
                self.stage = self.compute_stage_after_discord();
            }
            Action::VrChatCredentialsSubmit { username, password } => {
                if username.trim().is_empty() || password.is_empty() {
                    if let Stage::VrChatCredentials { error, .. } = &mut self.stage {
                        *error = Some("Enter both a username and a password.".to_string());
                    }
                } else {
                    self.vrchat_username_reg.set(Some(username.clone()));
                    self.vrchat_password_reg.set(Some(password.clone()));
                    self.start_login(username, password);
                }
            }
            Action::VrChatSkip => {
                self.stage = Stage::Done;
            }
            Action::TotpSubmit { code } => {
                if code.trim().is_empty() {
                    if let Stage::VrChatTotp { error, .. } = &mut self.stage {
                        *error = Some("Enter the code.".to_string());
                    }
                } else {
                    self.start_totp(code);
                }
            }
            Action::TotpCancel => {
                self.stage =
                    Stage::VrChatCredentials { username: String::new(), password: String::new(), error: None, in_flight: false };
            }
        }
    }

    pub fn render(&mut self, ctx: &egui::Context) -> SetupOutcome {
        if self.done {
            return SetupOutcome::AlreadyDone;
        }

        self.poll_async_results();

        let mut action = Action::None;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.set_max_width(480.0);
                match &mut self.stage {
                    Stage::VibecodedNotice => render_vibecoded_notice(ui, &mut action),
                    Stage::Discord { token, guild_id } => render_discord_stage(ui, token, guild_id, &mut action),
                    Stage::VrChatCredentials { username, password, error, in_flight } => {
                        render_vrchat_credentials_stage(ui, username, password, error, *in_flight, &mut action)
                    }
                    Stage::VrChatTotp { code, error, in_flight } => {
                        render_vrchat_totp_stage(ui, code, error, *in_flight, &mut action)
                    }
                    Stage::Done => {}
                }
            });
        });

        self.apply_action(action);

        if !self.done && matches!(self.stage, Stage::Done) {
            self.done = true;
            SetupOutcome::JustFinished { discord_token: self.discord_token_reg.get_or_null() }
        } else if self.done {
            SetupOutcome::AlreadyDone
        } else {
            SetupOutcome::InProgress
        }
    }
}

fn render_vibecoded_notice(ui: &mut egui::Ui, action: &mut Action) {
    ui.heading("Scarlet \u{2014} a note before you start");
    ui.add_space(12.0);
    ui.label(
        "This fork of Scarlet is maintained by KozyBlake and developed largely with AI \
         assistance \u{2014} a \"vibe-coded\" project. Code is written and reasoned through with \
         an AI pair, then reviewed and built by KozyBlake.\n\n\
         That isn't for everyone, and that's completely fair. If you would rather not run a \
         vibe-coded project, you can close Scarlet now \u{2014} no hard feelings.",
    );
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        if ui.button("Continue").clicked() {
            *action = Action::VibecodedContinue;
        }
        if ui.button("Close Scarlet").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn render_discord_stage(ui: &mut egui::Ui, token: &mut String, guild_id: &mut String, action: &mut Action) {
    ui.heading("Discord setup");
    ui.add_space(8.0);
    ui.label(
        "Enter a Discord bot token to enable Discord moderation features, or leave this blank \
         to run in staff mode (VRChat-only, no Discord bot).",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Bot token:");
        ui.add(egui::TextEdit::singleline(token).password(true).desired_width(320.0));
    });
    let token_present = !token.trim().is_empty();
    if token_present {
        ui.horizontal(|ui| {
            ui.label("Guild (server) ID:");
            ui.add(egui::TextEdit::singleline(guild_id).desired_width(320.0));
        });
    }
    ui.add_space(12.0);
    let can_continue = !token_present || !guild_id.trim().is_empty();
    if token_present && !can_continue {
        ui.colored_label(egui::Color32::YELLOW, "A guild ID is required when a bot token is set.");
    }
    if ui.add_enabled(can_continue, egui::Button::new("Continue")).clicked() {
        let token = token.trim();
        let guild_id = guild_id.trim();
        *action = Action::DiscordContinue {
            token: if token.is_empty() { None } else { Some(token.to_string()) },
            guild_id: if guild_id.is_empty() { None } else { Some(guild_id.to_string()) },
        };
    }
}

fn render_vrchat_credentials_stage(
    ui: &mut egui::Ui,
    username: &mut String,
    password: &mut String,
    error: &Option<String>,
    in_flight: bool,
    action: &mut Action,
) {
    ui.heading("VRChat login");
    ui.add_space(8.0);
    ui.label("Log in with the VRChat account Scarlet should moderate as.");
    ui.add_space(8.0);
    ui.add_enabled_ui(!in_flight, |ui| {
        ui.horizontal(|ui| {
            ui.label("Username:");
            ui.add(egui::TextEdit::singleline(username).desired_width(320.0));
        });
        ui.horizontal(|ui| {
            ui.label("Password:");
            ui.add(egui::TextEdit::singleline(password).password(true).desired_width(320.0));
        });
    });
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let login_label = if in_flight { "Logging in\u{2026}" } else { "Log in" };
        if ui.add_enabled(!in_flight, egui::Button::new(login_label)).clicked() {
            *action = Action::VrChatCredentialsSubmit { username: username.clone(), password: password.clone() };
        }
        if ui.add_enabled(!in_flight, egui::Button::new("Skip for now")).clicked() {
            *action = Action::VrChatSkip;
        }
    });
}

fn render_vrchat_totp_stage(
    ui: &mut egui::Ui,
    code: &mut String,
    error: &Option<String>,
    in_flight: bool,
    action: &mut Action,
) {
    ui.heading("VRChat two-factor code");
    ui.add_space(8.0);
    ui.label(
        "VRChat requires a two-factor code for this account. Enter the 6-digit code from your \
         authenticator app.",
    );
    ui.add_space(8.0);
    ui.add_enabled_ui(!in_flight, |ui| {
        ui.horizontal(|ui| {
            ui.label("Code:");
            ui.add(egui::TextEdit::singleline(code).desired_width(120.0));
        });
    });
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let verify_label = if in_flight { "Verifying\u{2026}" } else { "Verify" };
        if ui.add_enabled(!in_flight, egui::Button::new(verify_label)).clicked() {
            *action = Action::TotpSubmit { code: code.clone() };
        }
        if ui.add_enabled(!in_flight, egui::Button::new("Back")).clicked() {
            *action = Action::TotpCancel;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wizard(settings: &Arc<Settings>) -> SetupWizard {
        SetupWizard::new(settings, tokio::runtime::Handle::current())
    }

    #[tokio::test]
    async fn fresh_settings_start_at_vibecoded_notice() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let wizard = wizard(&settings);
        assert!(!wizard.is_done());
        assert_eq!(wizard.stage_name(), "vibecoded_notice");
    }

    #[tokio::test]
    async fn fully_configured_settings_skip_straight_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings.vibecoded_notice_acknowledged.set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        registry_string_encrypted(settings.encrypted.clone(), "global:username").set(Some("someone".to_string()));
        registry_string_encrypted(settings.encrypted.clone(), "global:password").set(Some("hunter2".to_string()));

        let wizard = wizard(&settings);
        assert!(wizard.is_done());
        assert_eq!(wizard.stage_name(), "done");
    }

    #[tokio::test]
    async fn vibecoded_continue_advances_to_discord_when_not_yet_prompted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VibecodedContinue);
        assert_eq!(wizard.stage_name(), "discord");
        assert!(settings.vibecoded_notice_acknowledged.get());
    }

    #[tokio::test]
    async fn vibecoded_continue_skips_discord_when_already_prompted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VibecodedContinue);
        // Vrchat isn't configured yet either, so it should land there, not "done".
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn discord_continue_persists_blank_token_as_staff_mode_and_marks_prompted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::DiscordContinue { token: None, guild_id: None });
        assert!(settings.discord_setup_prompted.get());
        assert_eq!(wizard.stored_discord_token(), None);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn discord_continue_persists_a_real_token() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::DiscordContinue {
            token: Some("my-bot-token".to_string()),
            guild_id: Some("123456".to_string()),
        });
        assert_eq!(wizard.stored_discord_token().as_deref(), Some("my-bot-token"));
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id").get_or_null().as_deref(),
            Some("123456")
        );
    }

    #[tokio::test]
    async fn vrchat_skip_finishes_setup_without_storing_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings.vibecoded_notice_acknowledged.set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        wizard.apply_action(Action::VrChatSkip);
        assert_eq!(wizard.stage_name(), "done");
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), "global:username").get_or_null(),
            None,
            "skipping must not fabricate stored credentials"
        );
    }

    #[tokio::test]
    async fn blank_vrchat_credentials_show_an_error_without_starting_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings.vibecoded_notice_acknowledged.set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VrChatCredentialsSubmit { username: String::new(), password: String::new() });
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        match &wizard.stage {
            Stage::VrChatCredentials { error, in_flight, .. } => {
                assert!(error.is_some());
                assert!(!in_flight, "must not have spawned a login for blank credentials");
            }
            _ => unreachable!("just asserted stage_name() == vrchat_credentials"),
        }
    }

    #[tokio::test]
    async fn totp_cancel_returns_to_a_blank_credentials_form() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatTotp { code: "123456".to_string(), error: None, in_flight: false };
        wizard.apply_action(Action::TotpCancel);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn blank_totp_code_shows_an_error_without_verifying() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatTotp { code: String::new(), error: None, in_flight: false };
        wizard.apply_action(Action::TotpSubmit { code: String::new() });
        match &wizard.stage {
            Stage::VrChatTotp { error, in_flight, .. } => {
                assert!(error.is_some());
                assert!(!in_flight);
            }
            _ => panic!("expected to stay on VrChatTotp"),
        }
    }
}
