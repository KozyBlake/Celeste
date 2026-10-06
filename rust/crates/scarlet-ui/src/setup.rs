//! First-run setup wizard: port of the interactive parts of `Scarlet.run()`
//! (`showVibecodedNoticeIfNeeded`), `ScarletDiscordJDA`'s constructor (the
//! bot-token/guild-snowflake prompt), and `ScarletVRChat.login()` (the
//! username/password/2FA prompt) — all of which run as blocking Swing
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
//! ## VRChat group ID
//!
//! Port of `getStringOrRequireInput("vrchat_group_id", ...)` in
//! `ScarletVRChat`'s constructor — asked unconditionally (it's real
//! config, not optional the way a Discord bot token is), stored in the
//! plain (non-encrypted) settings store since a group id isn't a secret,
//! matching Java's use of `getString`/`setString` rather than
//! `RegistryStringEncrypted` for this key.

use std::sync::mpsc;
use std::sync::Arc;

use scarlet_core::settings::file_valued::FileValued;
use scarlet_core::settings::registry::{registry_string_encrypted, RegistryStringValued};
use scarlet_core::settings::store::SettingsStore;
use scarlet_core::settings::Settings;
use scarlet_vrchat::client::{
    LoginOutcome, VrChatClient, VrChatError, GROUP_ID_KEY as VRCHAT_GROUP_ID_KEY,
};

enum Stage {
    VibecodedNotice,
    LegacyImport {
        detected: Option<std::path::PathBuf>,
        error: Option<String>,
    },
    Discord {
        token: String,
        guild_id: String,
    },
    VrChatCredentials {
        username: String,
        password: String,
        error: Option<String>,
        in_flight: bool,
    },
    VrChatTwoFactor {
        method: TwoFactorMethod,
        code: String,
        authenticator_secret: String,
        error: Option<String>,
        in_flight: bool,
    },
    VrChatGroupId {
        group_id: String,
        error: Option<String>,
    },
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
    LegacyImport(std::path::PathBuf),
    LegacySkip,
    DiscordContinue {
        token: Option<String>,
        guild_id: Option<String>,
    },
    VrChatCredentialsSubmit {
        username: String,
        password: String,
    },
    VrChatSkip,
    TwoFactorSubmit {
        method: TwoFactorMethod,
        code: String,
        authenticator_secret: String,
    },
    TwoFactorCancel,
    GroupIdSubmit {
        group_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TwoFactorMethod {
    Totp,
    EmailOtp,
}

impl TwoFactorMethod {
    /// Java checks `totp` before `emailOtp`, so preserve that preference
    /// when VRChat offers both methods.
    fn preferred(methods: &[String]) -> Option<Self> {
        if methods.iter().any(|method| method == "totp") {
            Some(Self::Totp)
        } else if methods.iter().any(|method| method == "emailOtp") {
            Some(Self::EmailOtp)
        } else {
            None
        }
    }

    fn instructions(self) -> &'static str {
        match self {
            Self::Totp => "Enter the 6-digit code from your authenticator app.",
            Self::EmailOtp => "Enter the 6-digit code VRChat sent to your email address.",
        }
    }
}

/// What [`SetupWizard::render`] wants the caller ([`crate::app::ScarletApp`])
/// to do this frame.
pub enum SetupOutcome {
    /// Still running; render nothing else this frame.
    InProgress,
    /// Just finished (this frame). Act on `discord_token` exactly once —
    /// e.g. spawn the Discord client — then start rendering the normal UI.
    JustFinished { discord_token: Option<String> },
    /// A legacy import was queued and must run before any stores reopen.
    RestartForImport,
    /// Already finished (on a previous frame, or never needed to run).
    AlreadyDone,
}

pub struct SetupWizard {
    stage: Stage,
    done: bool,
    /// True when this wizard exists only to collect a two-factor code for
    /// an already-configured install (see
    /// [`SetupWizard::for_two_factor_only`]). The caller uses it to avoid
    /// re-running the "setup just finished" work — reconnecting Discord in
    /// particular — when all that happened was a login.
    two_factor_only: bool,
    runtime_handle: tokio::runtime::Handle,
    vibecoded_ack: Arc<FileValued<bool>>,
    discord_setup_prompted: Arc<FileValued<bool>>,
    discord_token_reg: RegistryStringValued<String>,
    discord_guild_reg: RegistryStringValued<String>,
    vrchat_username_reg: RegistryStringValued<String>,
    vrchat_password_reg: RegistryStringValued<String>,
    settings_store: Arc<SettingsStore>,
    /// Kept so a successful login or verified code can persist the cookie
    /// jar, the way Java's `ScarletVRChatCookieJar.save()` does — without
    /// it, the next launch asks for a two-factor code all over again.
    settings: Arc<Settings>,
    vrchat_client: Option<Arc<VrChatClient>>,
    login_rx: Option<mpsc::Receiver<Result<LoginOutcome, VrChatError>>>,
    two_factor_rx: Option<mpsc::Receiver<Result<bool, String>>>,
    /// A normalized authenticator setup key awaiting successful code
    /// verification. It is encrypted only after VRChat accepts the code.
    pending_totp_secret: Option<String>,
    restart_for_import: bool,
}

impl SetupWizard {
    pub fn new(settings: &Arc<Settings>, runtime_handle: tokio::runtime::Handle) -> Self {
        let discord_token_reg =
            registry_string_encrypted(settings.encrypted.clone(), "discord.token");
        let discord_guild_reg =
            registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id");
        // Matches ScarletVRChat's own encrypted-key domain ("global:username"
        // etc. -- see `new ScarletVRChat(this, "global", ...)` in Scarlet.java)
        // so a data directory carried over from a Java install round-trips.
        let vrchat_username_reg =
            registry_string_encrypted(settings.encrypted.clone(), "global:username");
        let vrchat_password_reg =
            registry_string_encrypted(settings.encrypted.clone(), "global:password");

        let mut wizard = SetupWizard {
            stage: Stage::Done, // placeholder; replaced below once every field exists
            done: false,
            two_factor_only: false,
            runtime_handle,
            vibecoded_ack: settings.vibecoded_notice_acknowledged.clone(),
            discord_setup_prompted: settings.discord_setup_prompted.clone(),
            discord_token_reg,
            discord_guild_reg,
            vrchat_username_reg,
            vrchat_password_reg,
            settings_store: settings.store.clone(),
            settings: settings.clone(),
            vrchat_client: None,
            login_rx: None,
            two_factor_rx: None,
            pending_totp_secret: None,
            restart_for_import: false,
        };
        wizard.stage = if !wizard.vibecoded_ack.get() {
            Stage::VibecodedNotice
        } else {
            wizard.compute_stage_after_vibecoded()
        };
        wizard.done = matches!(wizard.stage, Stage::Done);
        wizard
    }

    /// A wizard that shows nothing but the two-factor code prompt, for an
    /// install whose setup finished long ago but whose stored credentials
    /// need a fresh code this launch.
    ///
    /// Java prompts for the code on every launch that needs one (its
    /// `ScarletVRChat.login()` runs before the main window exists); this is
    /// the same prompt, shown when the background login asks for it instead
    /// of at a fixed point in startup.
    pub fn for_two_factor_only(
        settings: &Arc<Settings>,
        runtime_handle: tokio::runtime::Handle,
        client: Arc<VrChatClient>,
        methods: &[String],
    ) -> Option<Self> {
        let method = TwoFactorMethod::preferred(methods)?;
        let mut wizard = SetupWizard::new(settings, runtime_handle);
        wizard.two_factor_only = true;
        wizard.done = false;
        wizard.vrchat_client = Some(client);
        wizard.stage = Stage::VrChatTwoFactor {
            method,
            code: String::new(),
            authenticator_secret: String::new(),
            error: None,
            in_flight: false,
        };
        Some(wizard)
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// See [`SetupWizard::two_factor_only`].
    pub fn is_two_factor_only(&self) -> bool {
        self.two_factor_only
    }

    #[cfg(test)]
    fn stage_name(&self) -> &'static str {
        match self.stage {
            Stage::VibecodedNotice => "vibecoded_notice",
            Stage::LegacyImport { .. } => "legacy_import",
            Stage::Discord { .. } => "discord",
            Stage::VrChatCredentials { .. } => "vrchat_credentials",
            Stage::VrChatTwoFactor {
                method: TwoFactorMethod::Totp,
                ..
            } => "vrchat_totp",
            Stage::VrChatTwoFactor {
                method: TwoFactorMethod::EmailOtp,
                ..
            } => "vrchat_email_otp",
            Stage::VrChatGroupId { .. } => "vrchat_group_id",
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

    fn legacy_import_stage(&self) -> Stage {
        Stage::LegacyImport {
            detected: scarlet_core::paths::find_legacy_data_dirs(&self.settings.data_dir)
                .into_iter()
                .next(),
            error: None,
        }
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
        let vrchat_configured = self.vrchat_username_reg.get_or_null().is_some()
            && self.vrchat_password_reg.get_or_null().is_some();
        if vrchat_configured {
            self.compute_stage_after_vrchat()
        } else {
            Stage::VrChatCredentials {
                username: String::new(),
                password: String::new(),
                error: None,
                in_flight: false,
            }
        }
    }

    /// What comes after the VRChat login step resolves, whichever way it
    /// resolves (logged in, TOTP-verified, or skipped) -- the group id
    /// prompt if none is stored yet, else done.
    fn compute_stage_after_vrchat(&self) -> Stage {
        let group_id: Option<String> = self.settings_store.get_object(VRCHAT_GROUP_ID_KEY);
        match group_id {
            Some(id) if !id.trim().is_empty() => Stage::Done,
            _ => Stage::VrChatGroupId {
                group_id: String::new(),
                error: None,
            },
        }
    }

    fn vrchat_client_or_init(&mut self) -> Option<Arc<VrChatClient>> {
        if self.vrchat_client.is_none() {
            // Start from the saved cookie jar: VRChat's `twoFactorAuth`
            // cookie outlives a single session, so a fresh credential login
            // on a machine that has verified a code before doesn't need
            // another one.
            let saved = scarlet_core::settings::registry::registry_string_encrypted(
                self.settings.encrypted.clone(),
                scarlet_vrchat::client::COOKIE_STORE_KEY,
            )
            .get_or_null();
            match VrChatClient::with_cookies(saved.as_deref()) {
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
        let Some(client) = self.vrchat_client_or_init() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.login_rx = Some(rx);
        if let Stage::VrChatCredentials {
            in_flight, error, ..
        } = &mut self.stage
        {
            *in_flight = true;
            *error = None;
        }
        self.runtime_handle.spawn(async move {
            let result = client.login_with_credentials(&username, &password).await;
            let _ = tx.send(result);
        });
    }

    fn start_two_factor(
        &mut self,
        method: TwoFactorMethod,
        code: String,
        authenticator_secret: String,
    ) {
        self.pending_totp_secret = None;
        if method == TwoFactorMethod::Totp && !authenticator_secret.trim().is_empty() {
            match scarlet_vrchat::totp::normalize_secret(&authenticator_secret) {
                Ok(secret) => self.pending_totp_secret = Some(secret),
                Err(message) => {
                    if let Stage::VrChatTwoFactor { error, .. } = &mut self.stage {
                        *error = Some(message);
                    }
                    return;
                }
            }
        }
        let Some(client) = self.vrchat_client.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.two_factor_rx = Some(rx);
        if let Stage::VrChatTwoFactor {
            in_flight, error, ..
        } = &mut self.stage
        {
            *in_flight = true;
            *error = None;
        }
        self.runtime_handle.spawn(async move {
            let result = match method {
                TwoFactorMethod::Totp => client.verify_totp(&code).await,
                TwoFactorMethod::EmailOtp => client.verify_email_otp(&code).await,
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    /// Persists the current VRChat cookie jar (port of
    /// `ScarletVRChatCookieJar.save()`).
    fn save_vrchat_session(&self) {
        if let Some(client) = &self.vrchat_client {
            scarlet_vrchat::client::save_session(&self.settings, client);
        }
    }

    /// Drains any finished background VRChat call and applies its result to
    /// the current stage. Called at the start of every `render` so a result
    /// that landed between frames is reflected before this frame draws.
    fn poll_async_results(&mut self) {
        if let Some(rx) = &self.login_rx {
            match rx.try_recv() {
                Ok(Ok(LoginOutcome::LoggedIn(_user))) => {
                    self.login_rx = None;
                    self.save_vrchat_session();
                    self.stage = self.compute_stage_after_vrchat();
                }
                Ok(Ok(LoginOutcome::RequiresTwoFactor(methods))) => {
                    self.login_rx = None;
                    self.stage = match TwoFactorMethod::preferred(&methods) {
                        Some(method) => Stage::VrChatTwoFactor {
                            method,
                            code: String::new(),
                            authenticator_secret: String::new(),
                            error: None,
                            in_flight: false,
                        },
                        None => Stage::VrChatCredentials {
                            username: String::new(),
                            password: String::new(),
                            error: Some(format!(
                                "VRChat requested unsupported 2FA methods: {}",
                                methods.join(", ")
                            )),
                            in_flight: false,
                        },
                    };
                }
                Ok(Err(err)) => {
                    self.login_rx = None;
                    // Match `ScarletVRChat.login()`'s `catch (ApiException) {
                    // this.username.set(null); this.password.set(null); ...
                    // }` -- a *rejected* login (VRChat itself said the
                    // credentials are wrong, not a network hiccup) must not
                    // leave bad credentials sitting in the encrypted store,
                    // or the next launch's `compute_stage_after_discord`
                    // (which only checks "is something stored", not "did it
                    // actually work") will wrongly treat VRChat as
                    // configured and skip straight past this stage on
                    // restart. `.set(None)` is a deliberate no-op (see its
                    // docs) -- `.clear()` is the actual "remove this value"
                    // method. A transient `Request`/`UnexpectedShape` error
                    // isn't a credential problem, so those are left stored
                    // (matching Java, which only reaches this branch for an
                    // `ApiException` -- a different exception type entirely
                    // for a network failure).
                    if matches!(err, VrChatError::Api { .. }) {
                        self.vrchat_username_reg.clear();
                        self.vrchat_password_reg.clear();
                    }
                    if let Stage::VrChatCredentials {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some(err.to_string());
                        *in_flight = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.login_rx = None;
                    if let Stage::VrChatCredentials {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some("Login task ended unexpectedly — try again.".to_string());
                        *in_flight = false;
                    }
                }
            }
        }
        if let Some(rx) = &self.two_factor_rx {
            match rx.try_recv() {
                Ok(Ok(true)) => {
                    self.two_factor_rx = None;
                    if let Some(secret) = self.pending_totp_secret.take() {
                        if let Err(error) =
                            scarlet_vrchat::client::save_totp_secret(&self.settings, &secret)
                        {
                            tracing::error!("Could not save VRChat authenticator secret: {error}");
                        }
                    }
                    // VRChat issues the `twoFactorAuth` cookie on a verified
                    // code; saving the jar here is what stops the next
                    // launch asking again.
                    self.save_vrchat_session();
                    self.stage = self.compute_stage_after_vrchat();
                }
                Ok(Ok(false)) => {
                    self.two_factor_rx = None;
                    self.pending_totp_secret = None;
                    if let Stage::VrChatTwoFactor {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some("Incorrect code — try again.".to_string());
                        *in_flight = false;
                    }
                }
                Ok(Err(message)) => {
                    self.two_factor_rx = None;
                    self.pending_totp_secret = None;
                    if let Stage::VrChatTwoFactor {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some(message);
                        *in_flight = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.two_factor_rx = None;
                    self.pending_totp_secret = None;
                    if let Stage::VrChatTwoFactor {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error =
                            Some("Verification task ended unexpectedly — try again.".to_string());
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
                self.stage = self.legacy_import_stage();
            }
            Action::LegacyImport(source) => {
                match scarlet_core::legacy_import::queue(&self.settings.data_dir, &source) {
                    Ok(()) => {
                        self.vibecoded_ack.set(Some(true), "setup-wizard");
                        self.restart_for_import = true;
                    }
                    Err(import_error) => {
                        if let Stage::LegacyImport { error, .. } = &mut self.stage {
                            *error = Some(import_error.to_string());
                        }
                    }
                }
            }
            Action::LegacySkip => {
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
                self.stage = self.compute_stage_after_vrchat();
            }
            Action::TwoFactorSubmit {
                method,
                code,
                authenticator_secret,
            } => {
                if code.trim().is_empty() {
                    if let Stage::VrChatTwoFactor { error, .. } = &mut self.stage {
                        *error = Some("Enter the code.".to_string());
                    }
                } else {
                    self.start_two_factor(method, code, authenticator_secret);
                }
            }
            Action::TwoFactorCancel => {
                self.stage = Stage::VrChatCredentials {
                    username: String::new(),
                    password: String::new(),
                    error: None,
                    in_flight: false,
                };
            }
            Action::GroupIdSubmit { group_id } => {
                let trimmed = group_id.trim();
                if trimmed.starts_with("grp_") && trimmed.len() > "grp_".len() {
                    self.settings_store
                        .set_object(VRCHAT_GROUP_ID_KEY, &trimmed.to_string());
                    self.stage = Stage::Done;
                } else if let Stage::VrChatGroupId { error, .. } = &mut self.stage {
                    *error =
                        Some("Enter a valid VRChat group ID (starts with \"grp_\").".to_string());
                }
            }
        }
    }

    pub fn render(&mut self, ctx: &egui::Context) -> SetupOutcome {
        if self.restart_for_import {
            return SetupOutcome::RestartForImport;
        }
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
                    Stage::LegacyImport { detected, error } => {
                        render_legacy_import_stage(ui, detected, error, &mut action)
                    }
                    Stage::Discord { token, guild_id } => {
                        render_discord_stage(ui, token, guild_id, &mut action)
                    }
                    Stage::VrChatCredentials {
                        username,
                        password,
                        error,
                        in_flight,
                    } => render_vrchat_credentials_stage(
                        ui,
                        username,
                        password,
                        error,
                        *in_flight,
                        &mut action,
                    ),
                    Stage::VrChatTwoFactor {
                        method,
                        code,
                        authenticator_secret,
                        error,
                        in_flight,
                    } => render_vrchat_two_factor_stage(
                        ui,
                        *method,
                        code,
                        authenticator_secret,
                        error,
                        *in_flight,
                        &mut action,
                    ),
                    Stage::VrChatGroupId { group_id, error } => {
                        render_vrchat_group_id_stage(ui, group_id, error, &mut action)
                    }
                    Stage::Done => {}
                }
            });
        });

        self.apply_action(action);

        if self.restart_for_import {
            return SetupOutcome::RestartForImport;
        }

        if !self.done && matches!(self.stage, Stage::Done) {
            self.done = true;
            SetupOutcome::JustFinished {
                discord_token: self.discord_token_reg.get_or_null(),
            }
        } else if self.done {
            SetupOutcome::AlreadyDone
        } else {
            SetupOutcome::InProgress
        }
    }
}

fn render_vibecoded_notice(ui: &mut egui::Ui, action: &mut Action) {
    ui.heading("Celeste \u{2014} a note before you start");
    ui.add_space(12.0);
    ui.label(
        "This fork of Celeste is maintained by KozyBlake and developed largely with AI \
         assistance \u{2014} a \"vibe-coded\" project. Code is written and reasoned through with \
         an AI pair, then reviewed and built by KozyBlake.\n\n\
         That isn't for everyone, and that's completely fair. If you would rather not run a \
         vibe-coded project, you can close Celeste now \u{2014} no hard feelings.",
    );
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        if ui.button("Continue").clicked() {
            *action = Action::VibecodedContinue;
        }
        if ui.button("Close Celeste").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn render_legacy_import_stage(
    ui: &mut egui::Ui,
    detected: &mut Option<std::path::PathBuf>,
    error: &Option<String>,
    action: &mut Action,
) {
    ui.heading("Bring your Scarlet data into Celeste");
    ui.add_space(8.0);
    ui.label(
        "If you used Java Scarlet before, Celeste can add its staff lists, watched entities, \
         moderation tags, Discord permissions, account links and history. Existing Celeste \
         records win conflicts. Scarlet is left untouched, and settings and credentials are \
         intentionally entered fresh in the next setup steps.",
    );
    ui.add_space(12.0);
    if let Some(path) = detected.as_ref() {
        ui.label("Detected Scarlet folder:");
        ui.add(
            egui::Label::new(egui::RichText::new(path.display().to_string()).monospace())
                .truncate(),
        )
        .on_hover_text(path.display().to_string());
    } else {
        ui.label(
            egui::RichText::new("No Scarlet folder was detected automatically.")
                .color(egui::Color32::GRAY),
        );
    }
    if let Some(error) = error {
        ui.add_space(8.0);
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(16.0);
    ui.horizontal_wrapped(|ui| {
        if let Some(path) = detected.as_ref() {
            if ui.button("Import detected data").clicked() {
                *action = Action::LegacyImport(path.clone());
            }
        }
        if ui.button("Choose Scarlet folder...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose the Scarlet data folder")
                .pick_folder()
            {
                *action = Action::LegacyImport(path);
            }
        }
        if ui.button("Continue without importing").clicked() {
            *action = Action::LegacySkip;
        }
    });
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new(
            "Importing creates a backup and restarts Celeste before setup continues.",
        )
        .small()
        .weak(),
    );
}

fn render_discord_stage(
    ui: &mut egui::Ui,
    token: &mut String,
    guild_id: &mut String,
    action: &mut Action,
) {
    ui.heading("Discord setup");
    ui.add_space(8.0);
    ui.label(
        "Enter a Discord bot token to enable Discord moderation features, or leave this blank \
         to run in staff mode (VRChat-only, no Discord bot).",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Bot token:");
        ui.add(
            egui::TextEdit::singleline(token)
                .password(true)
                .desired_width(320.0),
        );
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
        ui.colored_label(
            egui::Color32::YELLOW,
            "A guild ID is required when a bot token is set.",
        );
    }
    if ui
        .add_enabled(can_continue, egui::Button::new("Continue"))
        .clicked()
    {
        let token = token.trim();
        let guild_id = guild_id.trim();
        *action = Action::DiscordContinue {
            token: if token.is_empty() {
                None
            } else {
                Some(token.to_string())
            },
            guild_id: if guild_id.is_empty() {
                None
            } else {
                Some(guild_id.to_string())
            },
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
    ui.label("Log in with the VRChat account Celeste should moderate as.");
    ui.add_space(8.0);
    ui.add_enabled_ui(!in_flight, |ui| {
        ui.horizontal(|ui| {
            ui.label("Username:");
            ui.add(egui::TextEdit::singleline(username).desired_width(320.0));
        });
        ui.horizontal(|ui| {
            ui.label("Password:");
            ui.add(
                egui::TextEdit::singleline(password)
                    .password(true)
                    .desired_width(320.0),
            );
        });
    });
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let login_label = if in_flight {
            "Logging in\u{2026}"
        } else {
            "Log in"
        };
        if ui
            .add_enabled(!in_flight, egui::Button::new(login_label))
            .clicked()
        {
            *action = Action::VrChatCredentialsSubmit {
                username: username.clone(),
                password: password.clone(),
            };
        }
        if ui
            .add_enabled(!in_flight, egui::Button::new("Skip for now"))
            .clicked()
        {
            *action = Action::VrChatSkip;
        }
    });
}

fn render_vrchat_two_factor_stage(
    ui: &mut egui::Ui,
    method: TwoFactorMethod,
    code: &mut String,
    authenticator_secret: &mut String,
    error: &Option<String>,
    in_flight: bool,
    action: &mut Action,
) {
    ui.heading("VRChat two-factor code");
    ui.add_space(8.0);
    ui.label(format!(
        "VRChat requires a two-factor code for this account. {}",
        method.instructions()
    ));
    ui.add_space(8.0);
    ui.add_enabled_ui(!in_flight, |ui| {
        ui.horizontal(|ui| {
            ui.label("Code:");
            ui.add(egui::TextEdit::singleline(code).desired_width(120.0));
        });
        if method == TwoFactorMethod::Totp {
            ui.add_space(8.0);
            ui.label(
                "Optional: enter the 32-character Base32 setup key from your authenticator. \
                 Celeste will encrypt it and generate future codes automatically. This is not \
                 the six-digit code above.",
            );
            ui.horizontal(|ui| {
                ui.label("Setup key:");
                ui.add(
                    egui::TextEdit::singleline(authenticator_secret)
                        .password(true)
                        .desired_width(320.0)
                        .hint_text("32-character Base32 key"),
                );
            });
        }
    });
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let verify_label = if in_flight {
            "Verifying\u{2026}"
        } else {
            "Verify"
        };
        if ui
            .add_enabled(!in_flight, egui::Button::new(verify_label))
            .clicked()
        {
            *action = Action::TwoFactorSubmit {
                method,
                code: code.clone(),
                authenticator_secret: authenticator_secret.clone(),
            };
        }
        if ui
            .add_enabled(!in_flight, egui::Button::new("Back"))
            .clicked()
        {
            *action = Action::TwoFactorCancel;
        }
    });
}

fn render_vrchat_group_id_stage(
    ui: &mut egui::Ui,
    group_id: &mut String,
    error: &Option<String>,
    action: &mut Action,
) {
    ui.heading("VRChat group");
    ui.add_space(8.0);
    ui.label("Which VRChat group should Celeste moderate? This is required.");
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Group ID:");
        ui.add(
            egui::TextEdit::singleline(group_id)
                .desired_width(320.0)
                .hint_text("grp_..."),
        );
    });
    if ui.link("Open vrchat.com/home/groups to find it").clicked() {
        ui.ctx().open_url(egui::OpenUrl::same_tab(
            "https://vrchat.com/home/groups".to_string(),
        ));
    }
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    ui.add_space(12.0);
    if ui.button("Continue").clicked() {
        *action = Action::GroupIdSubmit {
            group_id: group_id.clone(),
        };
    }
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
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        registry_string_encrypted(settings.encrypted.clone(), "global:username")
            .set(Some("someone".to_string()));
        registry_string_encrypted(settings.encrypted.clone(), "global:password")
            .set(Some("hunter2".to_string()));
        settings.store.set_object(
            VRCHAT_GROUP_ID_KEY,
            &"grp_11111111-1111-1111-1111-111111111111".to_string(),
        );

        let wizard = wizard(&settings);
        assert!(wizard.is_done());
        assert_eq!(wizard.stage_name(), "done");
    }

    #[tokio::test]
    async fn vibecoded_continue_opens_the_first_run_import_window() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VibecodedContinue);
        assert_eq!(wizard.stage_name(), "legacy_import");
        assert!(!settings.vibecoded_notice_acknowledged.get());
        wizard.apply_action(Action::LegacySkip);
        assert_eq!(wizard.stage_name(), "discord");
        assert!(settings.vibecoded_notice_acknowledged.get());
    }

    #[tokio::test]
    async fn first_run_import_is_queued_before_setup_continues() {
        let destination = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("settings.json"), r#"{"legacy":true}"#).unwrap();
        std::fs::write(source.path().join("staff_list.json"), r#"["usr_legacy"]"#).unwrap();
        let settings = Arc::new(Settings::open(destination.path()));
        let mut wizard = wizard(&settings);

        wizard.apply_action(Action::VibecodedContinue);
        wizard.apply_action(Action::LegacyImport(source.path().to_path_buf()));

        assert!(wizard.restart_for_import);
        assert!(settings.vibecoded_notice_acknowledged.get());
        let report = scarlet_core::legacy_import::run_pending(destination.path())
            .expect("an import was queued")
            .unwrap();
        assert_eq!(report.records_added, 1);
        assert_eq!(
            std::fs::read_to_string(destination.path().join("staff_list.json")).unwrap(),
            "[\n  \"usr_legacy\"\n]"
        );
    }

    #[tokio::test]
    async fn vibecoded_continue_skips_discord_when_already_prompted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VibecodedContinue);
        assert_eq!(wizard.stage_name(), "legacy_import");
        wizard.apply_action(Action::LegacySkip);
        // Vrchat isn't configured yet either, so it should land there, not "done".
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn discord_continue_persists_blank_token_as_staff_mode_and_marks_prompted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::DiscordContinue {
            token: None,
            guild_id: None,
        });
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
        assert_eq!(
            wizard.stored_discord_token().as_deref(),
            Some("my-bot-token")
        );
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), "discord.guild_id")
                .get_or_null()
                .as_deref(),
            Some("123456")
        );
    }

    #[tokio::test]
    async fn vrchat_skip_advances_to_group_id_without_storing_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        wizard.apply_action(Action::VrChatSkip);
        assert_eq!(wizard.stage_name(), "vrchat_group_id");
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), "global:username").get_or_null(),
            None,
            "skipping must not fabricate stored credentials"
        );
    }

    #[tokio::test]
    async fn group_id_stage_is_skipped_when_already_stored() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        registry_string_encrypted(settings.encrypted.clone(), "global:username")
            .set(Some("someone".to_string()));
        registry_string_encrypted(settings.encrypted.clone(), "global:password")
            .set(Some("hunter2".to_string()));
        settings.store.set_object(
            VRCHAT_GROUP_ID_KEY,
            &"grp_11111111-1111-1111-1111-111111111111".to_string(),
        );

        let wizard = wizard(&settings);
        assert!(wizard.is_done());
        assert_eq!(wizard.stage_name(), "done");
    }

    #[tokio::test]
    async fn invalid_group_id_shows_an_error_and_does_not_advance() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatGroupId {
            group_id: String::new(),
            error: None,
        };
        wizard.apply_action(Action::GroupIdSubmit {
            group_id: "not-a-group-id".to_string(),
        });
        assert_eq!(wizard.stage_name(), "vrchat_group_id");
        match &wizard.stage {
            Stage::VrChatGroupId { error, .. } => assert!(error.is_some()),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn valid_group_id_is_stored_and_finishes_setup() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatGroupId {
            group_id: String::new(),
            error: None,
        };
        wizard.apply_action(Action::GroupIdSubmit {
            group_id: "grp_11111111-1111-1111-1111-111111111111".to_string(),
        });
        assert_eq!(wizard.stage_name(), "done");
        let stored: Option<String> = settings.store.get_object(VRCHAT_GROUP_ID_KEY);
        assert_eq!(
            stored.as_deref(),
            Some("grp_11111111-1111-1111-1111-111111111111")
        );
    }

    #[tokio::test]
    async fn blank_vrchat_credentials_show_an_error_without_starting_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VrChatCredentialsSubmit {
            username: String::new(),
            password: String::new(),
        });
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        match &wizard.stage {
            Stage::VrChatCredentials {
                error, in_flight, ..
            } => {
                assert!(error.is_some());
                assert!(
                    !in_flight,
                    "must not have spawned a login for blank credentials"
                );
            }
            _ => unreachable!("just asserted stage_name() == vrchat_credentials"),
        }
    }

    #[tokio::test]
    async fn a_rejected_login_clears_stored_credentials_so_the_stage_reappears_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut first_wizard = wizard(&settings);
        assert_eq!(first_wizard.stage_name(), "vrchat_credentials");

        // Simulate what `Action::VrChatCredentialsSubmit` does before the
        // (real, network-hitting) login attempt resolves: persist the
        // entered credentials up front, same as `ScarletVRChat.login()`.
        first_wizard
            .vrchat_username_reg
            .set(Some("someone".to_string()));
        first_wizard
            .vrchat_password_reg
            .set(Some("wrong-password".to_string()));
        first_wizard.stage = Stage::VrChatCredentials {
            username: "someone".to_string(),
            password: "wrong-password".to_string(),
            error: None,
            in_flight: true,
        };

        // Simulate the login task resolving with a rejection -- a real
        // `VrChatError::Api` (VRChat itself rejecting the credentials, e.g.
        // a 401) rather than actually spawning a network call.
        let (tx, rx) = mpsc::channel();
        tx.send(Err(VrChatError::Api {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: "Invalid credentials".to_string(),
        }))
        .unwrap();
        first_wizard.login_rx = Some(rx);

        first_wizard.poll_async_results();

        assert_eq!(
            first_wizard.vrchat_username_reg.get_or_null(),
            None,
            "a rejected login must not leave stale credentials stored"
        );
        assert_eq!(
            first_wizard.vrchat_password_reg.get_or_null(),
            None,
            "a rejected login must not leave stale credentials stored"
        );
        match &first_wizard.stage {
            Stage::VrChatCredentials {
                error, in_flight, ..
            } => {
                assert!(error.is_some());
                assert!(!in_flight);
            }
            _ => panic!("expected to stay on VrChatCredentials"),
        }

        // The real bug: a fresh wizard (as if the app were relaunched) must
        // land back on the credentials stage, not skip straight to the
        // group-id prompt because leftover credentials made it look
        // "configured".
        let relaunched_wizard = wizard(&settings);
        assert_eq!(relaunched_wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn totp_cancel_returns_to_a_blank_credentials_form() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatTwoFactor {
            method: TwoFactorMethod::Totp,
            code: "123456".to_string(),
            authenticator_secret: String::new(),
            error: None,
            in_flight: false,
        };
        wizard.apply_action(Action::TwoFactorCancel);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
    }

    #[tokio::test]
    async fn blank_totp_code_shows_an_error_without_verifying() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatTwoFactor {
            method: TwoFactorMethod::Totp,
            code: String::new(),
            authenticator_secret: String::new(),
            error: None,
            in_flight: false,
        };
        wizard.apply_action(Action::TwoFactorSubmit {
            method: TwoFactorMethod::Totp,
            code: String::new(),
            authenticator_secret: String::new(),
        });
        match &wizard.stage {
            Stage::VrChatTwoFactor {
                error, in_flight, ..
            } => {
                assert!(error.is_some());
                assert!(!in_flight);
            }
            _ => panic!("expected to stay on VrChatTwoFactor"),
        }
    }

    #[test]
    fn two_factor_method_matches_java_preference_and_supports_email_only_accounts() {
        assert_eq!(
            TwoFactorMethod::preferred(&["emailOtp".to_string(), "totp".to_string()]),
            Some(TwoFactorMethod::Totp)
        );
        assert_eq!(
            TwoFactorMethod::preferred(&["emailOtp".to_string()]),
            Some(TwoFactorMethod::EmailOtp)
        );
        assert_eq!(TwoFactorMethod::preferred(&["unknown".to_string()]), None);
    }
}
