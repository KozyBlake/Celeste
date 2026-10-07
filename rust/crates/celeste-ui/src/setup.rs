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
//! small state machine (`Stage`) that [`app::CelesteApp`] renders in place
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
//! (`celeste_core::settings::Settings`) that's set once the step is
//! completed, however it's answered.
//!
//! VRChat's stage is offered once during initial setup. Choosing to skip
//! leaves credentials unset and is remembered, so Celeste does not nag on
//! every launch; the integration remains available from Settings later.
//!
//! ## VRChat group ID
//!
//! Port of `getStringOrRequireInput("vrchat_group_id", ...)` in
//! `ScarletVRChat`'s constructor. Celeste asks for it after a successful
//! VRChat sign-in, but not when the user chooses to skip VRChat entirely:
//! requiring moderation configuration for an integration the user declined
//! would make the skip option misleading. The id is stored in the plain
//! (non-encrypted) settings store since it isn't a secret, matching Java's
//! use of `getString`/`setString` rather than `RegistryStringEncrypted` for
//! this key.

use std::sync::mpsc;
use std::sync::Arc;

use celeste_core::settings::file_valued::FileValued;
use celeste_core::settings::registry::{registry_string_encrypted, RegistryStringValued};
use celeste_core::settings::store::SettingsStore;
use celeste_core::settings::Settings;
use celeste_vrchat::client::{
    LoginOutcome, VrChatClient, VrChatError, GROUP_ID_KEY as VRCHAT_GROUP_ID_KEY,
};

enum Stage {
    VibecodedNotice,
    LegacyImport {
        detected: Option<std::path::PathBuf>,
        archive: Option<std::path::PathBuf>,
        overlaps: Vec<String>,
        mode: celeste_core::legacy_import::ImportMode,
        error: Option<String>,
    },
    SetupProfile {
        selected: SetupProfile,
    },
    Discord {
        token: String,
        guild_id: String,
        use_default_audio_device: bool,
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
        warning: Option<String>,
        in_flight: bool,
        can_continue_anyway: bool,
    },
    Finish,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupProfile {
    Full,
    VrChatOnly,
    DiscordOnly,
    LocalLogger,
}

impl Default for SetupProfile {
    fn default() -> Self {
        Self::Full
    }
}

/// What the wizard did this frame, decided during rendering and applied
/// afterwards. Kept as a plain value (rather than mutating `self` directly
/// from inside the render match) so the render closure only ever needs
/// `&mut` access to the current stage's own local buffers, not all of
/// `self` — see the module docs' note on why this is a two-phase design.
enum Action {
    None,
    VibecodedContinue,
    LegacyImport(
        std::path::PathBuf,
        Option<std::path::PathBuf>,
        celeste_core::legacy_import::ImportMode,
    ),
    LegacySkip,
    SetupProfileContinue(SetupProfile),
    Back,
    DiscordContinue {
        token: Option<String>,
        guild_id: Option<String>,
        use_default_audio_device: bool,
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
    GroupIdContinueAnyway,
    GroupIdTryAnother,
    GroupIdSkip,
    FinishStart,
    FinishReviewSettings,
    FinishOpenDataFolder,
    FinishTestTts,
    OpenSystemSoundSettings,
    #[cfg(target_os = "windows")]
    OpenClassicSoundPanel,
}

struct GroupValidation {
    group_id: String,
    group_name: String,
    missing_permissions: Vec<String>,
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

/// What [`SetupWizard::render`] wants the caller ([`crate::app::CelesteApp`])
/// to do this frame.
pub enum SetupOutcome {
    /// Still running; render nothing else this frame.
    InProgress,
    /// Just finished (this frame). Act on `discord_token` exactly once —
    /// e.g. spawn the Discord client — then start rendering the normal UI.
    JustFinished {
        discord_token: Option<String>,
        open_settings: bool,
    },
    /// Keep setup open while the app routes a sample through its real TTS
    /// service, using the output choice just configured.
    TestTts,
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
    setup_profile_prompted: Arc<FileValued<bool>>,
    discord_setup_prompted: Arc<FileValued<bool>>,
    vrchat_setup_prompted: Arc<FileValued<bool>>,
    setup_profile: SetupProfile,
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
    group_validation_rx: Option<mpsc::Receiver<Result<GroupValidation, String>>>,
    /// A normalized authenticator setup key awaiting successful code
    /// verification. It is encrypted only after VRChat accepts the code.
    pending_totp_secret: Option<String>,
    restart_for_import: bool,
    open_settings_after_finish: bool,
    test_tts_requested: bool,
    audio_settings_notice: Option<String>,
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
            setup_profile_prompted: settings.setup_profile_prompted.clone(),
            discord_setup_prompted: settings.discord_setup_prompted.clone(),
            vrchat_setup_prompted: settings.vrchat_setup_prompted.clone(),
            setup_profile: SetupProfile::Full,
            discord_token_reg,
            discord_guild_reg,
            vrchat_username_reg,
            vrchat_password_reg,
            settings_store: settings.store.clone(),
            settings: settings.clone(),
            vrchat_client: None,
            login_rx: None,
            two_factor_rx: None,
            group_validation_rx: None,
            pending_totp_secret: None,
            restart_for_import: false,
            open_settings_after_finish: false,
            test_tts_requested: false,
            audio_settings_notice: None,
        };
        // This flag was added after Celeste already had users. Existing
        // installations must not be mistaken for fresh installs and forced
        // back through the new profile picker after an upgrade.
        if !wizard.setup_profile_prompted.get() {
            let has_existing_setup = wizard.discord_setup_prompted.get()
                || wizard.vrchat_username_reg.get_or_null().is_some()
                || wizard.vrchat_password_reg.get_or_null().is_some()
                || wizard
                    .settings_store
                    .get_object::<String>(VRCHAT_GROUP_ID_KEY)
                    .is_some_and(|id| !id.trim().is_empty());
            if has_existing_setup {
                wizard
                    .setup_profile_prompted
                    .set(Some(true), "setup-profile-upgrade");
            }
        }
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
            Stage::SetupProfile { .. } => "setup_profile",
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
            Stage::Finish => "finish",
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
        let detected = celeste_core::paths::find_legacy_data_dirs(&self.settings.data_dir)
            .into_iter()
            .next();
        let overlaps = detected
            .as_deref()
            .and_then(|source| celeste_core::legacy_import::inspect(source, &self.settings.data_dir).ok())
            .map(|plan| plan.overlapping_files)
            .unwrap_or_default();
        Stage::LegacyImport {
            detected,
            archive: None,
            overlaps,
            mode: celeste_core::legacy_import::ImportMode::Merge,
            error: None,
        }
    }

    fn compute_stage_after_vibecoded(&self) -> Stage {
        if !self.setup_profile_prompted.get() {
            Stage::SetupProfile {
                selected: SetupProfile::Full,
            }
        } else if !self.discord_setup_prompted.get() {
            Stage::Discord {
                token: self.discord_token_reg.get_or_null().unwrap_or_default(),
                guild_id: self.discord_guild_reg.get_or_null().unwrap_or_default(),
                use_default_audio_device: self.settings.tts_use_default_audio_device.get(),
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
        } else if self.vrchat_setup_prompted.get() {
            Stage::Done
        } else {
            Stage::VrChatCredentials {
                username: String::new(),
                password: String::new(),
                error: None,
                in_flight: false,
            }
        }
    }

    fn discord_stage(&self) -> Stage {
        Stage::Discord {
            token: self.discord_token_reg.get_or_null().unwrap_or_default(),
            guild_id: self.discord_guild_reg.get_or_null().unwrap_or_default(),
            use_default_audio_device: self.settings.tts_use_default_audio_device.get(),
        }
    }

    fn vrchat_credentials_stage(&self) -> Stage {
        Stage::VrChatCredentials {
            username: String::new(),
            password: String::new(),
            error: None,
            in_flight: false,
        }
    }

    fn begin_setup_profile(&mut self, profile: SetupProfile) {
        self.setup_profile = profile;
        self.setup_profile_prompted.set(Some(true), "setup-wizard");
        self.stage = match profile {
            SetupProfile::Full => self.discord_stage(),
            SetupProfile::VrChatOnly => {
                self.discord_setup_prompted.set(Some(true), "setup-wizard");
                self.vrchat_credentials_stage()
            }
            SetupProfile::DiscordOnly => {
                self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                self.discord_stage()
            }
            SetupProfile::LocalLogger => {
                self.discord_setup_prompted.set(Some(true), "setup-wizard");
                self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                self.settings
                    .tts_use_default_audio_device
                    .set(Some(true), "setup-wizard");
                Stage::Finish
            }
        };
    }

    fn go_back(&mut self) {
        let current = std::mem::replace(&mut self.stage, Stage::Done);
        self.stage = match current {
            Stage::LegacyImport { .. } => Stage::VibecodedNotice,
            Stage::SetupProfile { .. } => self.legacy_import_stage(),
            Stage::Discord { .. } => Stage::SetupProfile {
                selected: self.setup_profile,
            },
            Stage::VrChatCredentials { .. } => match self.setup_profile {
                SetupProfile::Full => self.discord_stage(),
                _ => Stage::SetupProfile {
                    selected: self.setup_profile,
                },
            },
            Stage::VrChatGroupId { .. } => self.vrchat_credentials_stage(),
            stage => stage,
        };
    }

    /// What comes after the VRChat login step resolves, whichever way it
    /// resolves successfully (logged in or TOTP-verified) -- the group id
    /// prompt if none is stored yet, else done. Skipping VRChat bypasses
    /// this path entirely.
    fn compute_stage_after_vrchat(&self) -> Stage {
        let group_id: Option<String> = self.settings_store.get_object(VRCHAT_GROUP_ID_KEY);
        match group_id {
            Some(id) if !id.trim().is_empty() => Stage::Done,
            _ => Stage::VrChatGroupId {
                group_id: String::new(),
                error: None,
                warning: None,
                in_flight: false,
                can_continue_anyway: false,
            },
        }
    }

    fn vrchat_client_or_init(&mut self) -> Option<Arc<VrChatClient>> {
        if self.vrchat_client.is_none() {
            // Start from the saved cookie jar: VRChat's `twoFactorAuth`
            // cookie outlives a single session, so a fresh credential login
            // on a machine that has verified a code before doesn't need
            // another one.
            let saved = celeste_core::settings::registry::registry_string_encrypted(
                self.settings.encrypted.clone(),
                celeste_vrchat::client::COOKIE_STORE_KEY,
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
            match celeste_vrchat::totp::normalize_secret(&authenticator_secret) {
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

    fn start_group_validation(&mut self, group_id: String) {
        let Some(client) = self.vrchat_client.clone() else {
            if let Stage::VrChatGroupId { error, .. } = &mut self.stage {
                *error = Some(
                    "The VRChat session is unavailable. Go back and sign in again, or skip this step for now."
                        .to_string(),
                );
            }
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.group_validation_rx = Some(rx);
        if let Stage::VrChatGroupId {
            error,
            warning,
            in_flight,
            can_continue_anyway,
            ..
        } = &mut self.stage
        {
            *error = None;
            *warning = None;
            *in_flight = true;
            *can_continue_anyway = false;
        }
        self.runtime_handle.spawn(async move {
            let result = async {
                let current = client.get_current_user().await.map_err(|error| error.to_string())?;
                let group = client
                    .get_group(&group_id, true)
                    .await
                    .map_err(|error| format!("Celeste could not find or read that group: {error}"))?;

                let required = [
                    (celeste_vrchat::client::PERMISSION_GROUP_AUDIT_VIEW, "view audit logs"),
                    (celeste_vrchat::client::PERMISSION_GROUP_MEMBERS_VIEWALL, "view all members"),
                    (celeste_vrchat::client::PERMISSION_GROUP_BANS_MANAGE, "manage bans"),
                    (celeste_vrchat::client::PERMISSION_GROUP_INVITES_MANAGE, "manage invites"),
                    (celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_MANAGE, "manage instances"),
                    (celeste_vrchat::client::PERMISSION_GROUP_INSTANCE_MODERATE, "moderate instances"),
                ];
                let is_owner = group.owner_id.as_deref() == Some(current.id.as_str());
                let granted = if is_owner {
                    vec![celeste_vrchat::client::PERMISSION_ALL.to_string()]
                } else {
                    let member = client
                        .get_group_member(&group_id, &current.id)
                        .await
                        .map_err(|error| format!("Celeste could not check group membership: {error}"))?;
                    match member {
                        Some(member) => group
                            .roles
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|role| member.role_ids.contains(&role.id))
                            .flat_map(|role| role.permissions)
                            .collect::<Vec<_>>(),
                        None => Vec::new(),
                    }
                };
                let has_all = granted
                    .iter()
                    .any(|permission| permission == celeste_vrchat::client::PERMISSION_ALL);
                let missing_permissions = required
                    .into_iter()
                    .filter(|(permission, _)| {
                        !has_all && !granted.iter().any(|granted| granted == permission)
                    })
                    .map(|(permission, label)| format!("{label} (`{permission}`)"))
                    .collect();
                Ok(GroupValidation {
                    group_id,
                    group_name: group.name,
                    missing_permissions,
                })
            }
            .await;
            let _ = tx.send(result);
        });
    }

    /// Persists the current VRChat cookie jar (port of
    /// `ScarletVRChatCookieJar.save()`).
    fn save_vrchat_session(&self) {
        if let Some(client) = &self.vrchat_client {
            celeste_vrchat::client::save_session(&self.settings, client);
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
                            celeste_vrchat::client::save_totp_secret(&self.settings, &secret)
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
        if let Some(rx) = &self.group_validation_rx {
            match rx.try_recv() {
                Ok(Ok(validation)) => {
                    self.group_validation_rx = None;
                    if validation.missing_permissions.is_empty() {
                        self.settings_store
                            .set_object(VRCHAT_GROUP_ID_KEY, &validation.group_id);
                        self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                        self.stage = Stage::Finish;
                    } else if let Stage::VrChatGroupId {
                        warning,
                        in_flight,
                        can_continue_anyway,
                        ..
                    } = &mut self.stage
                    {
                        *warning = Some(format!(
                            "Celeste found {} but the signed-in account is missing: {}. Continuing can cause audit logging, member lookup, moderation commands, or instance management to fail. Only continue if you do not intend to use the affected features.",
                            validation.group_name,
                            validation.missing_permissions.join(", ")
                        ));
                        *in_flight = false;
                        *can_continue_anyway = true;
                    }
                }
                Ok(Err(message)) => {
                    self.group_validation_rx = None;
                    if let Stage::VrChatGroupId {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some(format!(
                            "{message} Check the group ID and account permissions, or skip this step and configure the group later in Settings."
                        ));
                        *in_flight = false;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.group_validation_rx = None;
                    if let Stage::VrChatGroupId {
                        error, in_flight, ..
                    } = &mut self.stage
                    {
                        *error = Some(
                            "The group check ended unexpectedly. Try again or skip this step for now."
                                .to_string(),
                        );
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
            Action::LegacyImport(source, archive, mode) => {
                let queued = if let Some(archive) = archive.as_deref() {
                    celeste_core::legacy_import::queue_prepared_archive(
                        &self.settings.data_dir, &source, archive, mode,
                    )
                } else {
                    celeste_core::legacy_import::queue(&self.settings.data_dir, &source, mode)
                };
                match queued {
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
                if let Stage::LegacyImport { detected: Some(source), archive: Some(_), .. } = &self.stage {
                    celeste_core::legacy_archive::cleanup_staging(source, &self.settings.data_dir);
                }
                self.vibecoded_ack.set(Some(true), "setup-wizard");
                self.stage = self.compute_stage_after_vibecoded();
            }
            Action::SetupProfileContinue(profile) => self.begin_setup_profile(profile),
            Action::Back => self.go_back(),
            Action::DiscordContinue { token, guild_id, use_default_audio_device } => {
                if let Some(token) = token {
                    self.discord_token_reg.set(Some(token));
                }
                if let Some(guild_id) = guild_id {
                    self.discord_guild_reg.set(Some(guild_id));
                }
                self.settings
                    .tts_use_default_audio_device
                    .set(Some(use_default_audio_device), "setup-wizard");
                self.discord_setup_prompted.set(Some(true), "setup-wizard");
                self.stage = if self.setup_profile == SetupProfile::DiscordOnly {
                    Stage::Finish
                } else {
                    self.compute_stage_after_discord()
                };
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
                // A group id is only useful to Celeste's authenticated
                // VRChat integration. If the user declines that integration,
                // finish setup instead of requiring its group configuration.
                // Remember that the choice was offered so first-run setup
                // does not nag on every launch. The complete VRChat and
                // group settings remain available later in Settings.
                self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                self.stage = Stage::Finish;
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
                    self.start_group_validation(trimmed.to_string());
                } else if let Stage::VrChatGroupId { error, .. } = &mut self.stage {
                    *error =
                        Some("Enter a valid VRChat group ID (starts with \"grp_\").".to_string());
                }
            }
            Action::GroupIdContinueAnyway => {
                let group_id = match &self.stage {
                    Stage::VrChatGroupId {
                        group_id,
                        can_continue_anyway: true,
                        ..
                    } => Some(group_id.trim().to_string()),
                    _ => None,
                };
                if let Some(group_id) = group_id {
                    self.settings_store
                        .set_object(VRCHAT_GROUP_ID_KEY, &group_id);
                    self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                    self.stage = Stage::Finish;
                }
            }
            Action::GroupIdTryAnother => {
                if let Stage::VrChatGroupId {
                    warning,
                    can_continue_anyway,
                    ..
                } = &mut self.stage
                {
                    *warning = None;
                    *can_continue_anyway = false;
                }
            }
            Action::GroupIdSkip => {
                self.vrchat_setup_prompted.set(Some(true), "setup-wizard");
                self.stage = Stage::Finish;
            }
            Action::FinishStart => self.stage = Stage::Done,
            Action::FinishReviewSettings => {
                self.open_settings_after_finish = true;
                self.stage = Stage::Done;
            }
            Action::FinishOpenDataFolder => {
                if let Err(error) = open::that(&self.settings.data_dir) {
                    tracing::warn!(%error, "Could not open the Celeste data folder from setup");
                }
            }
            Action::FinishTestTts => self.test_tts_requested = true,
            Action::OpenSystemSoundSettings => {
                self.audio_settings_notice = Some(match crate::platform::open_audio_settings(
                    crate::platform::AudioSettingsPanel::System,
                ) {
                    Ok(panel) => format!("Opened {panel}. Select the output device you want Celeste to use."),
                    Err(error) => error,
                });
            }
            #[cfg(target_os = "windows")]
            Action::OpenClassicSoundPanel => {
                self.audio_settings_notice = Some(match crate::platform::open_audio_settings(
                    crate::platform::AudioSettingsPanel::Classic,
                ) {
                    Ok(panel) => format!("Opened {panel}. Select the output device you want Celeste to use."),
                    Err(error) => error,
                });
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
                    Stage::LegacyImport { detected, archive, overlaps, mode, error } => {
                        render_legacy_import_stage(
                            ui,
                            &self.settings.data_dir,
                            detected,
                            archive,
                            overlaps,
                            mode,
                            error,
                            &mut action,
                        )
                    }
                    Stage::SetupProfile { selected } => {
                        render_setup_profile_stage(ui, selected, &mut action)
                    }
                    Stage::Discord { token, guild_id, use_default_audio_device } => {
                        render_discord_stage(
                            ui,
                            token,
                            guild_id,
                            use_default_audio_device,
                            self.audio_settings_notice.as_deref(),
                            &mut action,
                        )
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
                    Stage::VrChatGroupId {
                        group_id,
                        error,
                        warning,
                        in_flight,
                        can_continue_anyway,
                    } => render_vrchat_group_id_stage(
                        ui,
                        group_id,
                        error,
                        warning,
                        *in_flight,
                        *can_continue_anyway,
                        &mut action,
                    ),
                    Stage::Finish => render_finish_stage(
                        ui,
                        self.settings.tts_use_default_audio_device.get(),
                        self.audio_settings_notice.as_deref(),
                        &mut action,
                    ),
                    Stage::Done => {}
                }
            });
        });

        self.apply_action(action);

        if self.restart_for_import {
            return SetupOutcome::RestartForImport;
        }

        if std::mem::take(&mut self.test_tts_requested) {
            return SetupOutcome::TestTts;
        }

        if !self.done && matches!(self.stage, Stage::Done) {
            self.done = true;
            SetupOutcome::JustFinished {
                discord_token: self.discord_token_reg.get_or_null(),
                open_settings: self.open_settings_after_finish,
            }
        } else if self.done {
            SetupOutcome::AlreadyDone
        } else {
            SetupOutcome::InProgress
        }
    }
}

fn render_finish_stage(
    ui: &mut egui::Ui,
    use_default_audio_device: bool,
    audio_settings_notice: Option<&str>,
    action: &mut Action,
) {
    ui.heading("Celeste is ready");
    ui.add_space(8.0);
    ui.label(
        "Initial setup is complete. Every integration and advanced option remains available in Settings.",
    );
    ui.add_space(18.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Start Celeste").clicked() {
            *action = Action::FinishStart;
        }
        if ui.button("Review settings").clicked() {
            *action = Action::FinishReviewSettings;
        }
        if ui.button("Open data folder").clicked() {
            *action = Action::FinishOpenDataFolder;
        }
        if ui
            .add_enabled(
                use_default_audio_device,
                egui::Button::new("Test TTS"),
            )
            .clicked()
        {
            *action = Action::FinishTestTts;
        }
    });
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new(
            if use_default_audio_device {
                "The TTS test uses this computer's default audio device."
            } else {
                "Start Celeste before testing Discord TTS so the bot has time to connect to Discord voice."
            },
        )
        .small()
        .weak(),
    );
    if use_default_audio_device {
        render_audio_settings_buttons(ui, action);
    }
    if let Some(notice) = audio_settings_notice {
        ui.label(egui::RichText::new(notice).small().color(egui::Color32::YELLOW));
    }
}

fn render_vibecoded_notice(ui: &mut egui::Ui, action: &mut Action) {
    ui.heading("Celeste \u{2014} a note before you start");
    ui.add_space(12.0);
    ui.label(
        "Celeste is maintained by KozyBlake and is heavily based on Scarlet. It is developed \
         largely with AI assistance \u{2014} a \"vibe-coded\" project. Code is written and \
         reasoned through with an AI pair, then reviewed and built by KozyBlake.\n\n\
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
    destination: &std::path::Path,
    detected: &mut Option<std::path::PathBuf>,
    archive: &mut Option<std::path::PathBuf>,
    overlaps: &mut Vec<String>,
    mode: &mut celeste_core::legacy_import::ImportMode,
    error: &mut Option<String>,
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
    ui.add_space(8.0);
    ui.colored_label(
        egui::Color32::YELLOW,
        "Security reminder: VRChat and Discord credentials are never transferred. Make sure you have them available to enter again, and double-check the imported data after Celeste restarts.",
    );
    ui.add_space(12.0);
    if let Some(path) = archive.as_ref().or(detected.as_ref()) {
        ui.label(if archive.is_some() { "Selected Scarlet archive:" } else { "Detected Scarlet folder:" });
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
    if !overlaps.is_empty() {
        ui.add_space(10.0);
        ui.strong(format!("{} overlapping data location(s) found", overlaps.len()));
        egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
            for file in overlaps.iter() {
                ui.label(format!("• {file}"));
            }
        });
        ui.radio_value(
            mode,
            celeste_core::legacy_import::ImportMode::Merge,
            "Add missing Scarlet records; keep existing Celeste values (recommended)",
        );
        ui.radio_value(
            mode,
            celeste_core::legacy_import::ImportMode::Replace,
            "Replace overlapping recognized data with Scarlet's copy",
        );
        ui.label(
            egui::RichText::new("A full Celeste backup is created before either option. Settings and credentials are always excluded.")
                .small()
                .weak(),
        );
    }
    ui.add_space(16.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Back").clicked() {
            *action = Action::Back;
        }
        if let Some(path) = detected.as_ref() {
            if ui.button("Import detected data").clicked() {
                *action = Action::LegacyImport(path.clone(), archive.clone(), *mode);
            }
        }
        if ui.button("Choose Scarlet folder...").clicked() {
            match crate::dialogs::pick_folder("Choose the Scarlet data folder", None) {
                Ok(Some(path)) => {
                    if archive.take().is_some() {
                        if let Some(staged) = detected.as_deref() {
                            celeste_core::legacy_archive::cleanup_staging(staged, destination);
                        }
                    }
                    *overlaps = celeste_core::legacy_import::inspect(&path, destination)
                        .map(|plan| plan.overlapping_files)
                        .unwrap_or_default();
                    *detected = Some(path);
                    *archive = None;
                    *error = None;
                }
                Ok(None) => {}
                Err(dialog_error) => {
                    *error = Some(format!("Could not open the folder picker: {dialog_error}"));
                }
            }
        }
        if ui.button("Choose archive...").clicked() {
            match crate::dialogs::pick_archive("Choose a Scarlet data archive", None) {
                Ok(Some(path)) => match celeste_core::legacy_archive::prepare(&path, destination) {
                    Ok(staged) => {
                        if archive.take().is_some() {
                            if let Some(previous) = detected.as_deref() {
                                celeste_core::legacy_archive::cleanup_staging(previous, destination);
                            }
                        }
                        *overlaps = celeste_core::legacy_import::inspect(&staged, destination)
                            .map(|plan| plan.overlapping_files)
                            .unwrap_or_default();
                        *detected = Some(staged);
                        *archive = Some(path);
                        *error = None;
                    }
                    Err(import_error) => *error = Some(format!("Could not read the archive: {import_error}")),
                },
                Ok(None) => {}
                Err(dialog_error) => *error = Some(format!("Could not open the archive picker: {dialog_error}")),
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

fn render_setup_profile_stage(
    ui: &mut egui::Ui,
    selected: &mut SetupProfile,
    action: &mut Action,
) {
    ui.heading("How will you use Celeste?");
    ui.add_space(8.0);
    ui.label(
        "Choose the closest starting point. This only simplifies initial setup; every setting and integration remains available later.",
    );
    ui.add_space(12.0);

    ui.radio_value(selected, SetupProfile::Full, "Full setup (recommended)");
    ui.label(
        egui::RichText::new(
            "Configure Discord, VRChat sign-in, group moderation, and voice callouts.",
        )
        .small()
        .weak(),
    );
    ui.add_space(6.0);
    ui.radio_value(selected, SetupProfile::VrChatOnly, "VRChat only");
    ui.label(
        egui::RichText::new(
            "Use VRChat API and group moderation features locally without a Discord bot.",
        )
        .small()
        .weak(),
    );
    ui.add_space(6.0);
    ui.radio_value(selected, SetupProfile::DiscordOnly, "Discord only");
    ui.label(
        egui::RichText::new(
            "Run Discord features without signing Celeste into VRChat. VRChat-dependent commands will explain how to enable the integration.",
        )
        .small()
        .weak(),
    );
    ui.add_space(6.0);
    ui.radio_value(selected, SetupProfile::LocalLogger, "Local moderation logger");
    ui.label(
        egui::RichText::new(
            "Watch the running VRChat client's logs and show instance activity locally—no VRChat account or Discord bot required.",
        )
        .small()
        .weak(),
    );

    ui.add_space(18.0);
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            *action = Action::Back;
        }
        if ui.button("Continue").clicked() {
            *action = Action::SetupProfileContinue(*selected);
        }
    });
}

fn render_discord_stage(
    ui: &mut egui::Ui,
    token: &mut String,
    guild_id: &mut String,
    use_default_audio_device: &mut bool,
    audio_settings_notice: Option<&str>,
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
    ui.add_space(10.0);
    ui.strong("Where should voice callouts play?");
    ui.radio_value(
        use_default_audio_device,
        true,
        "This computer's default audio device (recommended)",
    );
    ui.add_enabled_ui(token_present, |ui| {
        ui.radio_value(
            use_default_audio_device,
            false,
            "The configured Discord voice channel",
        );
    });
    if !token_present {
        *use_default_audio_device = true;
        ui.label(
            egui::RichText::new(
                "Discord voice requires a bot token. You can change this later in Settings → Notifications.",
            )
            .small()
            .weak(),
        );
    } else {
        ui.label(
            egui::RichText::new(
                "This can be changed later in Settings → Notifications. A Discord voice channel must also be assigned. If the bot or channel is unavailable, Celeste warns you and falls back to this computer's default audio device.",
            )
            .small()
            .weak(),
        );
    }
    if *use_default_audio_device {
        ui.add_space(5.0);
        render_audio_settings_buttons(ui, action);
        if let Some(notice) = audio_settings_notice {
            ui.label(egui::RichText::new(notice).small().color(egui::Color32::YELLOW));
        }
    }
    ui.add_space(12.0);
    let can_continue = !token_present || !guild_id.trim().is_empty();
    if token_present && !can_continue {
        ui.colored_label(
            egui::Color32::YELLOW,
            "A guild ID is required when a bot token is set.",
        );
    }
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            *action = Action::Back;
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
                use_default_audio_device: *use_default_audio_device,
            };
        }
    });
}

fn render_audio_settings_buttons(ui: &mut egui::Ui, action: &mut Action) {
    ui.horizontal_wrapped(|ui| {
        #[cfg(target_os = "windows")]
        {
            if ui.button("Open Windows Sound Settings").clicked() {
                *action = Action::OpenSystemSoundSettings;
            }
            if ui.button("Open classic Sound panel").clicked() {
                *action = Action::OpenClassicSoundPanel;
            }
        }
        #[cfg(target_os = "linux")]
        if ui.button("Open system sound controls").clicked() {
            *action = Action::OpenSystemSoundSettings;
        }
    });
    #[cfg(target_os = "linux")]
    ui.label(
        egui::RichText::new(
            "Celeste tries your desktop's sound settings, then pavucontrol or pwvucontrol.",
        )
        .small()
        .weak(),
    );
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
        if ui
            .add_enabled(!in_flight, egui::Button::new("Back"))
            .clicked()
        {
            *action = Action::Back;
        }
    });
    ui.label(
        egui::RichText::new(
            "Skipping disables VRChat API details, group audit logs, group moderation, and VRChat-dependent Discord commands. Local client-log moderation still works. Enable VRChat later from Settings if you change your mind.",
        )
        .small()
        .weak(),
    );
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
    warning: &Option<String>,
    in_flight: bool,
    can_continue_anyway: bool,
    action: &mut Action,
) {
    ui.heading("VRChat group");
    ui.add_space(8.0);
    ui.label(
        "Which VRChat group should Celeste moderate? Celeste will verify the group and the signed-in account's permissions before saving it.",
    );
    ui.add_space(8.0);
    ui.add_enabled_ui(!in_flight && !can_continue_anyway, |ui| {
        ui.horizontal(|ui| {
            ui.label("Group ID:");
            ui.add(
                egui::TextEdit::singleline(group_id)
                    .desired_width(320.0)
                    .hint_text("grp_..."),
            );
        });
    });
    if ui.link("Open vrchat.com/home/groups to find it").clicked() {
        ui.ctx().open_url(egui::OpenUrl::same_tab(
            "https://vrchat.com/home/groups".to_string(),
        ));
    }
    if let Some(error) = error {
        ui.colored_label(egui::Color32::RED, error);
    }
    if let Some(warning) = warning {
        ui.colored_label(egui::Color32::YELLOW, warning);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!in_flight, egui::Button::new("Back")).clicked() {
            *action = Action::Back;
        }
        let check_label = if in_flight { "Checking group…" } else { "Check and continue" };
        if ui
            .add_enabled(!in_flight && !can_continue_anyway, egui::Button::new(check_label))
            .clicked()
        {
            *action = Action::GroupIdSubmit {
                group_id: group_id.clone(),
            };
        }
        if can_continue_anyway && ui.button("Use this group anyway").clicked() {
            *action = Action::GroupIdContinueAnyway;
        }
        if can_continue_anyway && ui.button("Try another group").clicked() {
            *action = Action::GroupIdTryAnother;
        }
        if ui
            .add_enabled(!in_flight, egui::Button::new("Skip for now"))
            .clicked()
        {
            *action = Action::GroupIdSkip;
        }
    });
    ui.label(
        egui::RichText::new(
            "Skipping leaves group-specific moderation disabled. You can configure the group later in Settings.",
        )
        .small()
        .weak(),
    );
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
        assert_eq!(wizard.stage_name(), "setup_profile");
        assert!(settings.vibecoded_notice_acknowledged.get());
    }

    #[tokio::test]
    async fn full_setup_is_the_default_profile_and_leads_to_discord() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::VibecodedContinue);
        wizard.apply_action(Action::LegacySkip);
        assert_eq!(wizard.stage_name(), "setup_profile");
        wizard.apply_action(Action::SetupProfileContinue(SetupProfile::Full));
        assert_eq!(wizard.stage_name(), "discord");
        assert!(settings.setup_profile_prompted.get());
    }

    #[tokio::test]
    async fn local_logger_profile_needs_neither_discord_nor_vrchat() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::SetupProfileContinue(SetupProfile::LocalLogger));
        assert_eq!(wizard.stage_name(), "finish");
        assert!(settings.discord_setup_prompted.get());
        assert!(settings.vrchat_setup_prompted.get());
        assert!(settings.tts_use_default_audio_device.get());
    }

    #[tokio::test]
    async fn setup_back_returns_to_the_profile_picker_without_losing_the_choice() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.apply_action(Action::SetupProfileContinue(SetupProfile::VrChatOnly));
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        wizard.apply_action(Action::Back);
        assert_eq!(wizard.stage_name(), "setup_profile");
        match wizard.stage {
            Stage::SetupProfile { selected } => assert_eq!(selected, SetupProfile::VrChatOnly),
            _ => unreachable!(),
        }
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
        wizard.apply_action(Action::LegacyImport(
            source.path().to_path_buf(),
            None,
            celeste_core::legacy_import::ImportMode::Merge,
        ));

        assert!(wizard.restart_for_import);
        assert!(settings.vibecoded_notice_acknowledged.get());
        let report = celeste_core::legacy_import::run_pending(destination.path())
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
            use_default_audio_device: true,
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
            use_default_audio_device: false,
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
        assert!(!settings.tts_use_default_audio_device.get());
    }

    #[tokio::test]
    async fn vrchat_skip_finishes_without_requiring_a_group_id_or_storing_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        settings
            .vibecoded_notice_acknowledged
            .set(Some(true), "test");
        settings.discord_setup_prompted.set(Some(true), "test");
        let mut wizard = wizard(&settings);
        assert_eq!(wizard.stage_name(), "vrchat_credentials");
        wizard.apply_action(Action::VrChatSkip);
        assert_eq!(wizard.stage_name(), "finish");
        assert!(settings.vrchat_setup_prompted.get());
        assert_eq!(
            settings.store.get_object::<String>(VRCHAT_GROUP_ID_KEY),
            None,
            "skipping VRChat must not fabricate a group configuration"
        );
        assert_eq!(
            registry_string_encrypted(settings.encrypted.clone(), "global:username").get_or_null(),
            None,
            "skipping must not fabricate stored credentials"
        );

        wizard.apply_action(Action::FinishStart);
        let next_launch = SetupWizard::new(&settings, tokio::runtime::Handle::current());
        assert!(next_launch.is_done());
        assert_eq!(next_launch.stage_name(), "done");
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
            warning: None,
            in_flight: false,
            can_continue_anyway: false,
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
    async fn accepted_group_warning_can_be_stored_and_finishes_setup() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        let mut wizard = wizard(&settings);
        wizard.stage = Stage::VrChatGroupId {
            group_id: "grp_11111111-1111-1111-1111-111111111111".to_string(),
            error: None,
            warning: Some("Missing permissions".to_string()),
            in_flight: false,
            can_continue_anyway: true,
        };
        wizard.apply_action(Action::GroupIdContinueAnyway);
        assert_eq!(wizard.stage_name(), "finish");
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
