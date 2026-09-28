//! Port of `net.sybyline.scarlet.ScarletEventListener`.
//!
//! One VRChat log line at a time, this maintains:
//!
//! - **instance state** — `clientLocation`, whether it belongs to the
//!   configured VRChat group, and whether it is the same instance as last
//!   time (rejoin detection);
//! - **the roster** — who is present, in join order, with UI updates
//!   deferred during catch-up and flushed in log order
//!   ([`scarlet_core::instance_roster`]);
//! - **join checks** — watched users, watched groups, new accounts,
//!   suspicious pronouns, trust rank and mixed-character names, producing
//!   the row advisory/colour/priority and the spoken announcement;
//! - **audit emissions** — the group-instance-gated Discord events and the
//!   matching `ScarletData` custom events.
//!
//! ## What is ported here and what is not yet
//!
//! Ported: everything above, plus avatar-name tracking, vote-to-kick,
//! pedestal/sticker/prop spawns, video loads, and the `[API]`-logging
//! detection that tells the user their client was launched without the
//! debug flags.
//!
//! Not yet ported, each tracked in PORTING.md rather than faked:
//!
//! - **Avatar info hydration** (`maybeHydrateAvatarInfo`,
//!   `hydrateAvatarInfoFromApi`, `rescanAvatarInfo`) needs the avatar-search
//!   provider subsystem and four more VRChat endpoints. The hooks are here
//!   ([`ScarletEventListener::rescan_avatar_info`]) and do nothing yet, so
//!   the avatar *name* shows but its performance rating does not.
//! - **`ScarletMobile` notifications** are deliberately out of scope for
//!   this port (see PORTING.md's scope section); Java's `mobile.notify*`
//!   calls have no counterpart here.
//! - **Splash-screen feedback popups** (`scarlet.splash.queueFeedbackPopup`)
//!   have no equivalent in the egui UI; those messages go to the log and the
//!   CLI tab instead.
//!
//! ## One structural change, and why
//!
//! Java runs its join checks *inside* the roster's deferred closure, so
//! during catch-up they run at flush time and are skipped for anyone who
//! already left again. A deferred closure in Rust can't await, so instead:
//! catch-up rows are added immediately with no advisory, their ids are
//! remembered, and the checks run right after the flush for the players who
//! are still present. Same API calls, same skipping, same end state — just
//! sequenced with an await instead of a callback.
//!
//! Java also calls `checkPlayer` twice for a live join: once (in "preamble"
//! mode) for the advisory text and once for the TTS line, relying on its
//! VRChat client's user cache to make the second pass free. This runs one
//! pass that produces both, which is the same result against a client that
//! doesn't cache.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Datelike, NaiveDateTime, TimeZone, Utc};
use scarlet_core::data::ScarletData;
use scarlet_core::instance_roster::InstanceRoster;
use scarlet_core::pacer::Pacer;
use scarlet_core::pronoun_lists::PronounLists;
use scarlet_core::settings::Settings;
use scarlet_core::staff_list::StaffList;
use scarlet_core::trust_rank::TrustRank;
use scarlet_core::ui::{InstanceState, PlayerJoin, ScarletUi, NO_PRIORITY};
use scarlet_core::watched_entities::WatchedEntities;
use scarlet_core::watched_groups::WatchedGroups;
use scarlet_vrchat::client::VrChatClient;
use scarlet_vrchat::log_tail::{LogEvent, LogEventKind, TailerEvent};

use crate::emitter::{AuditEmitter, TtsSink};

/// `GroupAuditTypeEx` ids, verbatim from the Java enum.
mod audit_type {
    pub const STAFF_JOIN: &str = "groupex.instance.staff.join";
    pub const STAFF_LEAVE: &str = "groupex.instance.staff.leave";
    pub const USER_JOIN: &str = "groupex.instance.user.join";
    pub const USER_LEAVE: &str = "groupex.instance.user.leave";
    pub const USER_AVATAR: &str = "groupex.instance.user.avatar";
    pub const VTK_START: &str = "groupex.instance.vtk";
    pub const SPAWN_PEDESTAL: &str = "groupex.instance.user.pedestal";
    pub const SPAWN_STICKER: &str = "groupex.instance.user.sticker";
    pub const SPAWN_PROP: &str = "groupex.instance.user.prop";
    pub const USER_VIDEO: &str = "groupex.instance.user.video";
}

/// The VRChat client, shared with the Discord command handlers — `None`
/// until a login succeeds, so every use is "if we have a session".
pub type SharedVrChat = Arc<tokio::sync::RwLock<Option<Arc<VrChatClient>>>>;

/// Everything the listener needs from the rest of Scarlet, matching the
/// fields Java reaches through its `Scarlet` god-object.
pub struct ListenerDeps {
    pub ui: Arc<dyn ScarletUi>,
    pub vrchat: SharedVrChat,
    pub settings: Arc<Settings>,
    pub data: Arc<ScarletData>,
    pub staff_list: Arc<StaffList>,
    pub watched_users: Arc<WatchedEntities>,
    pub watched_avatars: Arc<WatchedEntities>,
    pub watched_groups: Arc<WatchedGroups>,
    pub pronoun_lists: Arc<PronounLists>,
    pub tts: Arc<dyn TtsSink>,
    pub emitter: Arc<dyn AuditEmitter>,
}

pub struct ScarletEventListener {
    deps: ListenerDeps,

    client_user_display_name: Option<String>,
    client_user_id: Option<String>,
    client_location: Option<String>,
    client_location_prev: Option<String>,
    /// From the `Entering Room:` line that precedes each join.
    pending_world_name: Option<String>,
    current_world_name: Option<String>,

    user_ids_join_order: Vec<String>,
    display_name_to_user_id: HashMap<String, String>,
    display_name_to_avatar_name: HashMap<String, String>,
    avatar_name_to_display_names: HashMap<String, HashSet<String>>,
    user_id_to_joined: HashMap<String, chrono::DateTime<Utc>>,
    /// Users who have left the current instance at least once — a later join
    /// by one of them is a rejoin.
    left_user_ids: HashSet<String>,
    prev_instance_user_ids: HashSet<String>,
    /// Joins seen during catch-up whose checks still have to run.
    deferred_checks: Vec<(String, String)>,

    roster: InstanceRoster,
    is_tailer_live: bool,
    has_tailer_caught_up: bool,
    is_in_group_instance: bool,
    is_same_as_previous_instance: bool,

    /// One-shot, so a full instance doesn't repeat the warning per player.
    warned_no_vrchat_session: bool,
    seen_api_log_lines: bool,
    live_avatar_switches_without_api_log: u32,
    warned_missing_api_logging: bool,

    check_player_limiter: Pacer,
}

/// Steam launch options that make VRChat write the `[API]` debug log lines
/// Scarlet needs for exact avatar statuses.
const VRCHAT_API_LOGGING_FLAGS: &str = "--enable-sdk-log-levels --enable-udon-debug-logging --enable-verbose-logging";
const VRCHAT_API_LOGGING_LEVELS: &str = "--log-debug-levels=API;All;Always;AssetBundleDownloadManager;ContentCreator;Errors;NetworkData;NetworkProcessing;NetworkTransport;Warnings";

/// What one run of the join checks produced.
#[derive(Debug)]
struct CheckOutcome {
    advisory: Option<String>,
    text_color: Option<(u8, u8, u8)>,
    priority: i32,
    pronouns: Option<String>,
    pronouns_flagged: bool,
    trust_rank: TrustRank,
    account_age: Option<(i32, i32, i32)>,
    age_verification_status: Option<String>,
}

impl CheckOutcome {
    fn new() -> Self {
        CheckOutcome {
            advisory: None,
            text_color: None,
            // Java starts every advisory pass at its "no priority"
            // sentinel, not zero.
            priority: NO_PRIORITY,
            pronouns: None,
            pronouns_flagged: false,
            trust_rank: TrustRank::Unknown,
            account_age: None,
            age_verification_status: None,
        }
    }
}

impl ScarletEventListener {
    pub fn new(deps: ListenerDeps) -> Self {
        ScarletEventListener {
            deps,
            client_user_display_name: None,
            client_user_id: None,
            client_location: None,
            client_location_prev: None,
            pending_world_name: None,
            current_world_name: None,
            user_ids_join_order: Vec::new(),
            display_name_to_user_id: HashMap::new(),
            display_name_to_avatar_name: HashMap::new(),
            avatar_name_to_display_names: HashMap::new(),
            user_id_to_joined: HashMap::new(),
            left_user_ids: HashSet::new(),
            prev_instance_user_ids: HashSet::new(),
            deferred_checks: Vec::new(),
            roster: InstanceRoster::new(),
            is_tailer_live: false,
            has_tailer_caught_up: false,
            is_in_group_instance: false,
            is_same_as_previous_instance: false,
            warned_no_vrchat_session: false,
            seen_api_log_lines: false,
            live_avatar_switches_without_api_log: 0,
            warned_missing_api_logging: false,
            // Java: `new Pacer(500L)`.
            check_player_limiter: Pacer::new(500),
        }
    }

    /// The configured VRChat group id, which gates every audit emission.
    fn group_id(&self) -> Option<String> {
        let id = self.deps.settings.store.get_string("vrchat_group_id")?;
        if id.trim().is_empty() {
            None
        } else {
            Some(id)
        }
    }

    /// True only while this player is actually present in the current
    /// instance. Departed players stay visible in the table but must not be
    /// tracked any further — no avatar lookups, no advisory refreshes.
    fn is_player_present(&self, user_id: &str) -> bool {
        self.client_location.is_some() && self.roster.is_present(user_id)
    }

    async fn vrchat(&self) -> Option<Arc<VrChatClient>> {
        self.deps.vrchat.read().await.clone()
    }

    /// Entry point: one [`TailerEvent`] from the log tailer.
    pub async fn handle(&mut self, event: TailerEvent) {
        match event {
            TailerEvent::Init { file } => self.log_init(&file),
            TailerEvent::CatchUp { file } => self.log_catch_up(file.as_deref()).await,
            TailerEvent::Entry(entry) => self.handle_entry(entry).await,
        }
    }

    /// Port of `log_init`.
    fn log_init(&mut self, file: &std::path::Path) {
        self.is_tailer_live = false;
        // New VRChat log file = new VRChat session: re-evaluate whether this
        // session was launched with API debug logging.
        self.seen_api_log_lines = false;
        self.live_avatar_switches_without_api_log = 0;
        tracing::debug!("Tailer init on {}", file.display());
    }

    /// Port of `log_catchUp`.
    async fn log_catch_up(&mut self, file: Option<&std::path::Path>) {
        if file.is_none() {
            // Java's `catchUp(null)`: there is no log to read. Say so once,
            // so a run on a machine with no VRChat client doesn't look like
            // a silently broken instance tab.
            if !self.has_tailer_caught_up {
                self.has_tailer_caught_up = true;
                self.deps.ui.append_cli_output(
                    "No VRChat client log found - the instance list stays empty until VRChat writes one.",
                );
            }
            return;
        }

        if self.is_tailer_live {
            // Per-log transition already happened; still flush, because each
            // VRChat session writes a new file and a restart while Scarlet
            // keeps running re-queues that session's existing players.
            self.roster.flush();
            self.run_deferred_checks().await;
            return;
        }

        self.is_tailer_live = true;
        self.roster.flush();
        self.run_deferred_checks().await;
        self.deps.ui.fire_sort();
        // Scarlet often starts mid-session: players already present had
        // their avatars recorded during catch-up, where the live switch
        // trigger never fires. Hydrate their statuses now.
        self.rescan_avatar_info().await;
        self.has_tailer_caught_up = true;
    }

    async fn handle_entry(&mut self, entry: LogEvent) {
        let LogEvent { preamble, timestamp, kind } = entry;
        match kind {
            LogEventKind::UserAuthenticated { display_name, user_id } => {
                self.client_user_display_name = Some(display_name);
                self.client_user_id = Some(user_id);
            }
            LogEventKind::UserQuit { lifetime_seconds } => self.log_user_quit(lifetime_seconds),
            LogEventKind::EnteringRoom { world_name } => {
                self.pending_world_name = Some(world_name);
            }
            LogEventKind::UserJoined { location } => self.log_user_joined(preamble, timestamp, location).await,
            LogEventKind::UserLeft => self.log_user_left(preamble).await,
            LogEventKind::PlayerJoined { display_name, user_id } => {
                self.log_player_joined(preamble, timestamp, display_name, user_id).await
            }
            LogEventKind::PlayerLeft { display_name, user_id } => {
                self.log_player_left(preamble, timestamp, display_name, user_id).await
            }
            LogEventKind::PlayerSwitchAvatar { display_name, avatar_display_name } => {
                self.log_player_switch_avatar(preamble, timestamp, display_name, avatar_display_name).await
            }
            LogEventKind::VtkInit { target_display_name, actor_display_name } => {
                self.log_vtk_init(preamble, timestamp, target_display_name, actor_display_name).await
            }
            LogEventKind::PlayerSpawnPedestal { display_name, user_id, content_type, content_id } => {
                self.log_spawn_pedestal(preamble, timestamp, display_name, user_id, content_type, content_id).await
            }
            LogEventKind::PlayerSpawnSticker { display_name, user_id, sticker_id } => {
                self.log_spawn_sticker(preamble, timestamp, display_name, user_id, sticker_id).await
            }
            LogEventKind::PlayerSpawnProp { user_id, prop_id } => {
                self.log_spawn_prop(preamble, timestamp, user_id, prop_id).await
            }
            LogEventKind::ApiRequest { index, method, url } => {
                self.log_api_request(preamble, index, &method, &url)
            }
            LogEventKind::VideoLoad { display_name, url, title } => {
                self.log_video_load(preamble, timestamp, display_name, url, title).await
            }
        }
    }

    /// Port of `log_userQuit`: the VRChat session ended, so drop all
    /// per-instance avatar state — nothing may look up avatars for people
    /// from a session that no longer exists.
    fn log_user_quit(&mut self, lifetime_seconds: f64) {
        self.display_name_to_avatar_name.clear();
        self.avatar_name_to_display_names.clear();
        self.deps.ui.append_cli_output(&format!("VRChat closed after {lifetime_seconds:.0}s this session"));
    }

    /// Port of `log_userJoined`.
    async fn log_user_joined(&mut self, preamble: bool, timestamp: NaiveDateTime, location: String) {
        let last_joined = self.deps.settings.last_instance_joined.get_or_null();
        if preamble {
            if last_joined.is_none_or(|previous| previous < timestamp) {
                self.deps.settings.last_instance_joined.set(Some(timestamp));
            }
        } else {
            self.deps.settings.last_instance_joined.set(Some(timestamp));
        }

        self.is_in_group_instance = match self.group_id() {
            Some(group_id) => location.contains(&format!("~group({group_id})")),
            None => false,
        };
        self.is_same_as_previous_instance = self.client_location_prev.as_deref() == Some(location.as_str());

        // Always clear the table when joining any world — this covers both
        // moving to a new instance and rejoining the same one. During
        // catch-up only clear when the instance actually changed, so data
        // still being read isn't wiped.
        if !preamble || !self.is_same_as_previous_instance {
            self.deps.ui.clear_instance();
        }

        self.current_world_name = self.pending_world_name.take();
        self.client_location = Some(location.clone());
        self.user_id_to_joined.clear();
        self.user_ids_join_order.clear();
        self.left_user_ids.clear();

        self.deps.ui.set_instance(Some(InstanceState {
            location: location.clone(),
            world_name: self.current_world_name.clone(),
            joined: timestamp,
            is_group_instance: self.is_in_group_instance,
        }));
        self.deps.ui.append_cli_output(&match &self.current_world_name {
            Some(world) => format!("Joined instance: {world} ({location})"),
            None => format!("Joined instance: {location}"),
        });
    }

    /// Port of `log_userLeft`.
    async fn log_user_left(&mut self, preamble: bool) {
        self.prev_instance_user_ids = self.roster.present_user_ids().into_iter().collect();
        self.client_location_prev = self.client_location.take();
        self.is_in_group_instance = false;
        self.roster.clear_pending();
        self.deferred_checks.clear();
        // The avatar state belongs to people who are no longer around. The
        // next instance's `Switching` lines rebuild it.
        self.display_name_to_avatar_name.clear();
        self.avatar_name_to_display_names.clear();
        // Clear as soon as the local client leaves — stale players from the
        // previous instance shouldn't linger while between worlds.
        if !preamble {
            self.deps.ui.clear_instance();
        }
        self.deps.ui.set_instance(None);
        self.deps.ui.append_cli_output("Left instance");
    }

    /// Port of `log_playerJoined`.
    async fn log_player_joined(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        user_id: String,
    ) {
        let avatar_name = self.display_name_to_avatar_name.get(&display_name).cloned();
        let joined_utc = Utc.from_utc_datetime(&timestamp);
        if !self.user_ids_join_order.contains(&user_id) {
            self.user_ids_join_order.push(user_id.clone());
        }
        self.user_id_to_joined.insert(user_id.clone(), joined_utc);
        let is_rejoin_from_prev = self.prev_instance_user_ids.remove(&user_id) && self.is_same_as_previous_instance;
        // Rejoin of the current instance: this user has left it at least
        // once already this session.
        let rejoined_instance = self.left_user_ids.contains(&user_id);
        self.display_name_to_user_id.insert(display_name.clone(), user_id.clone());

        if preamble {
            // Catch-up: add the row now (in log order, via the roster) and
            // run the checks after the flush, for whoever is still present.
            let ui = Arc::clone(&self.deps.ui);
            let mut join = PlayerJoin::new(user_id.clone(), display_name.clone(), timestamp);
            join.is_rejoin_from_prev = is_rejoin_from_prev;
            join.avatar_name = avatar_name.clone();
            self.roster.join(&user_id, &display_name, true, move || ui.player_join(true, join));
            self.deferred_checks.push((user_id.clone(), display_name.clone()));
        } else {
            let outcome = self.check_player(false, &display_name, &user_id, rejoined_instance).await;
            let ui = Arc::clone(&self.deps.ui);
            let mut join = PlayerJoin::new(user_id.clone(), display_name.clone(), timestamp);
            join.is_rejoin_from_prev = is_rejoin_from_prev;
            join.avatar_name = avatar_name.clone();
            join.advisory = outcome.advisory.clone();
            join.text_color = outcome.text_color;
            join.priority = outcome.priority;
            join.pronouns = outcome.pronouns.clone();
            join.pronouns_flagged = outcome.pronouns_flagged;
            join.trust_rank = outcome.trust_rank;
            join.account_age = outcome.account_age;
            join.age_verification_status = outcome.age_verification_status.clone();
            self.roster.join(&user_id, &display_name, false, move || {
                ui.player_join(false, join);
                // Java's `playerJoin` re-sorts the table for every live
                // update (`if (visible && !initialPreamble) { fireSort();
                // updateStatusBar(); }`). Without it the row lands wherever
                // it was inserted and stays there.
                ui.fire_sort();
            });
        }

        if Some(&user_id) == self.client_user_id.as_ref() {
            self.prev_instance_user_ids.clear();
        }

        if !preamble && self.is_in_group_instance {
            let location = self.client_location.clone().unwrap_or_default();
            self.deps.emitter.emit_user_join(timestamp, &location, &user_id, &display_name).await;
            self.deps.data.custom_event_new(
                audit_type::USER_JOIN,
                joined_utc,
                Some(user_id.clone()),
                Some(display_name.clone()),
                Some(location.clone()),
                None,
                None,
            );
            if self.deps.staff_list.is_id(&user_id) {
                self.deps.emitter.emit_staff_join(timestamp, &location, &user_id, &display_name).await;
                self.deps.data.custom_event_new(
                    audit_type::STAFF_JOIN,
                    joined_utc,
                    Some(user_id.clone()),
                    Some(display_name.clone()),
                    Some(location.clone()),
                    None,
                    None,
                );
            }
            if let Some(avatar) = avatar_name.clone() {
                self.switch_player_avatar(preamble, timestamp, &display_name, &user_id, &avatar).await;
            }
        }
    }

    /// Port of `log_playerLeft`.
    async fn log_player_left(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        user_id: String,
    ) {
        self.user_ids_join_order.retain(|id| id != &user_id);
        if !user_id.is_empty() {
            self.left_user_ids.insert(user_id.clone());
        }
        // Their last-known avatar name, stats and name->id mapping are kept
        // deliberately, so the row still shows what they were wearing.
        // Marking them absent is what matters: any avatar change after they
        // leave is ignored.
        let ui = Arc::clone(&self.deps.ui);
        let (id_for_ui, name_for_ui) = (user_id.clone(), display_name.clone());
        self.roster.leave(&user_id, preamble, move || {
            ui.player_leave(preamble, &id_for_ui, &name_for_ui, timestamp);
            if !preamble {
                // Same as the join path: this is what moves someone who has
                // just left down below everyone still present, which the
                // sort order has always specified but nothing triggered.
                ui.fire_sort();
            }
        });

        if !preamble && self.is_in_group_instance {
            let location = self.client_location.clone().unwrap_or_default();
            let left_utc = Utc.from_utc_datetime(&timestamp);
            self.deps.emitter.emit_user_leave(timestamp, &location, &user_id, &display_name).await;
            self.deps.data.custom_event_new(
                audit_type::USER_LEAVE,
                left_utc,
                Some(user_id.clone()),
                Some(display_name.clone()),
                Some(location.clone()),
                None,
                None,
            );
            if self.deps.staff_list.is_id(&user_id) {
                self.deps.emitter.emit_staff_leave(timestamp, &location, &user_id, &display_name).await;
                self.deps.data.custom_event_new(
                    audit_type::STAFF_LEAVE,
                    left_utc,
                    Some(user_id),
                    Some(display_name),
                    Some(location),
                    None,
                    None,
                );
            }
        }
    }

    /// Port of `log_playerSwitchAvatar`.
    async fn log_player_switch_avatar(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        avatar_display_name: String,
    ) {
        // Avatar statuses are built from `[API] ... analysis/file_...` log
        // lines, which VRChat only writes when launched with the debug
        // logging options. If avatars keep switching but no `[API]` lines
        // ever appear, the game was launched without them: statuses would
        // silently stay empty, so say so once.
        if !preamble && !self.seen_api_log_lines && !self.warned_missing_api_logging {
            self.live_avatar_switches_without_api_log += 1;
            if self.live_avatar_switches_without_api_log >= 3 {
                self.warned_missing_api_logging = true;
                tracing::warn!(
                    "VRChat is not logging API requests this session; avatar statuses are using the API fallback \
                     (avatar-database name lookup), which can miss private or ambiguous avatars. For exact \
                     statuses, launch VRChat through KozyBlake/Scarlet or add these Steam launch options: {} {}",
                    VRCHAT_API_LOGGING_FLAGS,
                    VRCHAT_API_LOGGING_LEVELS
                );
                self.deps.ui.append_cli_output(&scarlet_core::i18n::tr("adv.avatarFallbackMsg", &[]));
            }
        }

        let user_id = self.display_name_to_user_id.get(&display_name).cloned();
        // A player who has left keeps their last-known avatar, but avatar
        // *changes* after they leave must not be tracked.
        if let Some(id) = &user_id {
            if !self.is_player_present(id) {
                return;
            }
        }

        if let Some(id) = &user_id {
            let ui = Arc::clone(&self.deps.ui);
            let initial_preamble = !self.is_tailer_live;
            let new_avatar = avatar_display_name.clone();
            let id_for_ui = id.clone();
            self.roster.run_or_defer(preamble, move || {
                ui.player_update(initial_preamble, &id_for_ui, &mut |player| {
                    if player.avatar_name.as_deref() != Some(new_avatar.as_str()) {
                        // New avatar: drop the previous avatar's info until
                        // hydration fills it back in.
                        player.avatar_info = None;
                        player.avatar_info_note = None;
                    }
                    player.avatar_name = Some(new_avatar.clone());
                });
            });
        }

        self.set_player_avatar(&display_name, Some(&avatar_display_name));

        if !preamble {
            if let Some(id) = user_id.clone() {
                if self.is_in_group_instance {
                    self.switch_player_avatar(preamble, timestamp, &display_name, &id, &avatar_display_name).await;
                }
            }
        }
    }

    /// Port of `_setPlayerAvatar`.
    fn set_player_avatar(&mut self, display_name: &str, avatar_display_name: Option<&str>) {
        let Some(avatar_display_name) = avatar_display_name else {
            if let Some(old) = self.display_name_to_avatar_name.remove(display_name) {
                if let Some(names) = self.avatar_name_to_display_names.get_mut(&old) {
                    names.remove(display_name);
                }
            }
            return;
        };
        let previous = self
            .display_name_to_avatar_name
            .insert(display_name.to_string(), avatar_display_name.to_string());
        if previous.as_deref() == Some(avatar_display_name) {
            return;
        }
        if let Some(old) = previous {
            if let Some(names) = self.avatar_name_to_display_names.get_mut(&old) {
                names.remove(display_name);
            }
        }
        self.avatar_name_to_display_names
            .entry(avatar_display_name.to_string())
            .or_default()
            .insert(display_name.to_string());
    }

    /// Port of `switchPlayerAvatar`: the watched-avatar advisory, its TTS
    /// callout and the Discord emission.
    ///
    /// The avatar-id resolution Java does here (`AvatarSearch`, optionally
    /// by avatar image) is not ported yet, so `potential_ids` is always
    /// empty and no watched avatar can match — the same state Java is in
    /// with `Features.AVATAR_SEARCH_ENABLED` off. The surrounding gating,
    /// emission and event recording are ported in full so that wiring the
    /// search in later is a one-function change.
    async fn switch_player_avatar(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: &str,
        user_id: &str,
        avatar_display_name: &str,
    ) {
        let watched_avatars_enabled = scarlet_core::features::watched_avatars_enabled();
        let avatar_advisory_wanted = watched_avatars_enabled && self.deps.settings.advisory_show_watched_avatars.get();
        let tts_wanted =
            watched_avatars_enabled && !preamble && avatar_advisory_wanted && self.deps.settings.announce_watched_avatars.get();
        let discord_wanted = self.deps.emitter.is_emitting(audit_type::USER_AVATAR).await;
        if !avatar_advisory_wanted && !tts_wanted && !discord_wanted {
            return;
        }

        let potential_ids: Vec<String> = self.search_avatar(avatar_display_name);

        let mut watched: Vec<_> = potential_ids
            .iter()
            .filter_map(|id| self.deps.watched_avatars.get_watched_entity(id))
            .filter(|entity| !entity.silent)
            .collect();
        // Java sorts by the entity's natural order and takes the first.
        watched.sort_by(|a, b| {
            a.priority.cmp(&b.priority).then_with(|| a.critical.cmp(&b.critical)).then_with(|| a.silent.cmp(&b.silent))
        });
        let watched_avatar = watched.first().cloned();

        if watched_avatars_enabled {
            let (advisory, color, priority) = match (&watched_avatar, avatar_advisory_wanted) {
                (Some(entity), true) => (
                    Some(watched_avatar_advisory(entity.message.as_deref())),
                    entity.r#type.text_color(),
                    entity.priority,
                ),
                _ => (None, None, NO_PRIORITY),
            };
            let initial_preamble = !self.is_tailer_live;
            self.deps.ui.player_update(initial_preamble, user_id, &mut |player| {
                player.set_avatar_advisory(advisory.clone(), color, priority);
            });
        }

        if tts_wanted {
            if let Some(entity) = &watched_avatar {
                let mut line = scarlet_core::i18n::tr(
                    "adv.ttsWatchedAvatar",
                    &[&scarlet_core::names::sanitize_name_for_speech(display_name)],
                );
                if let Some(message) = &entity.message {
                    line.push(' ');
                    line.push_str(message);
                }
                self.deps.tts.speak(&marker("wa"), &line);
            }
        }

        if discord_wanted {
            let location = self.client_location.clone().unwrap_or_default();
            self.deps
                .emitter
                .emit_user_avatar(timestamp, &location, user_id, display_name, avatar_display_name, &potential_ids)
                .await;
            self.deps.data.custom_event_new(
                audit_type::USER_AVATAR,
                Utc.from_utc_datetime(&timestamp),
                Some(user_id.to_string()),
                Some(display_name.to_string()),
                potential_ids.first().filter(|_| potential_ids.len() == 1).cloned(),
                Some(avatar_display_name.to_string()),
                None,
            );
        }
    }

    /// Port of `searchAvatar`. The avatar-search provider subsystem isn't
    /// ported, so this is empty — deliberately, rather than substituting a
    /// plain VRChat avatar search, which would match different avatars than
    /// Java's provider cache and quietly change who gets flagged.
    fn search_avatar(&self, _avatar_display_name: &str) -> Vec<String> {
        Vec::new()
    }

    /// Port of `rescanAvatarInfo`. See the module docs: the hydration it
    /// drives isn't ported yet, so this only enforces the privacy guard that
    /// wraps it (never look anything up unless a session is actually live).
    async fn rescan_avatar_info(&mut self) {
        if self.client_location.is_none() {
            return;
        }
    }

    /// Runs the join checks queued during catch-up, for the players who are
    /// still present (see the module docs).
    async fn run_deferred_checks(&mut self) {
        let queued = std::mem::take(&mut self.deferred_checks);
        for (user_id, display_name) in queued {
            if !self.is_player_present(&user_id) {
                continue;
            }
            let rejoined = self.left_user_ids.contains(&user_id);
            let outcome = self.check_player(true, &display_name, &user_id, rejoined).await;
            let advisory = outcome.advisory.clone();
            let color = outcome.text_color;
            let priority = outcome.priority;
            let pronouns = outcome.pronouns.clone();
            let flagged = outcome.pronouns_flagged;
            let trust_rank = outcome.trust_rank;
            let account_age = outcome.account_age;
            let age_verification = outcome.age_verification_status.clone();
            self.deps.ui.player_update(true, &user_id, &mut |player| {
                player.set_base_advisory(advisory.clone(), color, priority);
                player.pronouns = pronouns.clone();
                player.pronouns_flagged = flagged;
                player.trust_rank = trust_rank;
                player.account_age = account_age;
                player.age_verification_status = age_verification.clone();
            });
        }
    }

    /// Port of `checkPlayer`: every join check, the advisory they build and
    /// the single combined TTS callout at the end.
    ///
    /// `preamble` here means Java's own sense of the parameter — during
    /// catch-up the checks are rate-limited and produce no speech.
    async fn check_player(
        &mut self,
        preamble: bool,
        display_name: &str,
        user_id: &str,
        rejoined_instance: bool,
    ) -> CheckOutcome {
        if preamble {
            self.check_player_limiter.wait().await;
        }

        let mut outcome = CheckOutcome::new();
        let mut advisories: Vec<String> = Vec::new();
        let mut tts_parts: Vec<String> = Vec::new();
        // When set, the combined callout is preceded by the bundled
        // nuisance alarm tone.
        let mut nuisance_chime = false;

        if self.client_user_id.as_deref() != Some(user_id)
            && scarlet_core::names::has_mixed_letter_scripts(display_name)
        {
            let wanted = self.deps.settings.advisory_show_mixed_character_names.get();
            if wanted {
                add_advisory(&mut advisories, Some(&scarlet_core::i18n::tr("adv.mixedCharName", &[])));
            }
            if !preamble && wanted && self.deps.settings.announce_mixed_character_names.get() {
                self.deps.tts.speak_mixed_character_name_alert(&marker("mix"));
            }
        }

        // check user
        if let Some(watched_user) = self.deps.watched_users.get_watched_entity(user_id) {
            if !watched_user.silent {
                let wanted = self.deps.settings.advisory_show_watched_users.get();
                if wanted {
                    add_advisory(&mut advisories, watched_user.message.as_deref());
                }
                if !preamble && wanted && self.deps.settings.announce_watched_users.get() {
                    if let Some(message) = watched_user.message.as_deref() {
                        if !message.trim().is_empty() {
                            tts_parts.push(end_dot(message));
                        }
                    }
                }
            }
        }

        let user = match self.vrchat().await {
            Some(client) => match client.get_user(user_id).await {
                Ok(user) => Some(user),
                Err(err) => {
                    tracing::debug!("Could not look up {user_id} for join checks: {err}");
                    None
                }
            },
            None => {
                // Everything profile-derived silently stays blank without a
                // session: account age, pronouns, trust rank, age
                // verification, and the watched-group check. Say so once,
                // rather than leaving empty columns to be interpreted as
                // "this player has no data".
                if !self.warned_no_vrchat_session {
                    self.warned_no_vrchat_session = true;
                    tracing::warn!(
                        "Not signed in to VRChat: account age, pronouns, trust rank, 18+ status and watched-group \
                         checks are unavailable until a VRChat login succeeds"
                    );
                    self.deps.ui.append_cli_output(
                        "Not signed in to VRChat - account age, pronouns, rank, 18+ and watched-group checks are \
                         unavailable until VRChat login succeeds.",
                    );
                }
                None
            }
        };

        // check groups
        let mut watched_groups = self.watched_groups_of(user_id, false).await;
        // Group membership is cached indefinitely, so a user who left a
        // watched group would keep getting flagged on every rejoin. Re-check
        // against fresh membership for an actual rejoin of an
        // already-flagged user.
        if !preamble && rejoined_instance && watched_groups.iter().any(|group| !group.silent) {
            watched_groups = self.watched_groups_of(user_id, true).await;
        }
        if !watched_groups.is_empty() {
            let show_groups = self.deps.settings.advisory_show_watched_groups.get();
            if let Some(first) = watched_groups.iter().find(|group| !group.silent) {
                if !preamble && show_groups && self.deps.settings.announce_watched_groups.get() {
                    let messages: Vec<String> = watched_groups
                        .iter()
                        .filter(|group| !group.silent)
                        .filter_map(|group| group.message.clone())
                        .filter(|message| !message.trim().is_empty())
                        .collect();
                    let clause = match messages.len() {
                        0 => scarlet_core::i18n::tr("adv.inWatchedGroup", &[]),
                        1 => scarlet_core::i18n::tr("adv.watchedGroupOne", &[&messages[0]]),
                        _ => scarlet_core::i18n::tr("adv.watchedGroupMany", &[&messages.join(", ")]),
                    };
                    tts_parts.push(end_dot(&clause));
                }
                outcome.priority = first.priority;
            }
            if show_groups {
                for group in &watched_groups {
                    add_advisory(&mut advisories, group.message.as_deref());
                }
            }
            outcome.text_color = watched_groups.iter().find_map(|group| group.r#type.text_color());
        }

        if let Some(user) = &user {
            outcome.age_verification_status = user.age_verification_status.clone();
            outcome.trust_rank = TrustRank::of(Some(&user.tags));
            outcome.pronouns = user.pronouns.clone();

            // check new user
            let show_new = self.deps.settings.advisory_show_new_players.get();
            let announce_new = show_new && self.deps.settings.announce_new_players.get();
            if let Some(date_joined) = user.date_joined {
                let today = Utc::now().date_naive();
                let account_age_days = today.num_days_from_ce() as i64 - date_joined.num_days_from_ce() as i64;
                outcome.account_age = Some(period_between(date_joined, today));
                if (show_new || (!preamble && announce_new))
                    && account_age_days <= self.deps.settings.announce_players_newer_than_days.get() as i64
                {
                    let days = account_age_days.to_string();
                    let unit = scarlet_core::i18n::tr(if account_age_days == 1 { "adv.day" } else { "adv.days" }, &[]);
                    if show_new {
                        add_advisory(
                            &mut advisories,
                            Some(&scarlet_core::i18n::tr("adv.newAccountDays", &[&days, &unit])),
                        );
                    }
                    if !preamble && announce_new {
                        tts_parts.push(scarlet_core::i18n::tr("adv.ttsNewDays", &[&days, &unit]));
                    }
                }
            }

            // check pronouns
            if scarlet_core::features::pronouns_enabled() && self.deps.settings.flag_suspicious_pronouns.get() {
                let flagged = scarlet_core::pronoun_validator::flag_reason(
                    user.pronouns.as_deref(),
                    Some(&self.deps.pronoun_lists),
                );
                if flagged.is_some() {
                    let wanted = self.deps.settings.advisory_show_suspicious_pronouns.get();
                    outcome.pronouns_flagged = wanted;
                    if wanted {
                        add_advisory(
                            &mut advisories,
                            Some(&scarlet_core::i18n::tr("adv.suspiciousPronounsWarn", &[])),
                        );
                    }
                    if !preamble && wanted && self.deps.settings.announce_suspicious_pronouns.get() {
                        let label = scarlet_core::i18n::tr("adv.suspiciousPronounsLabel", &[]);
                        tts_parts.push(format!("{label}: {}", end_dot(user.pronouns.as_deref().unwrap_or(""))));
                    }
                }
            }

            // check trust rank — Nuisance warrants a flag plus an alarm
            // chime; Visitor is opt-in.
            match outcome.trust_rank {
                TrustRank::Nuisance => {
                    if self.deps.settings.advisory_flag_nuisance_rank.get() {
                        add_advisory(&mut advisories, Some(&scarlet_core::i18n::tr("adv.nuisanceRank", &[])));
                        outcome.priority = outcome.priority.max(1000);
                    }
                    if !preamble && self.deps.settings.announce_nuisance_rank.get() {
                        tts_parts.push(scarlet_core::i18n::tr("adv.ttsNuisanceRank", &[]));
                        nuisance_chime = true;
                    }
                }
                TrustRank::Visitor => {
                    if self.deps.settings.advisory_flag_visitor_rank.get() {
                        add_advisory(&mut advisories, Some(&scarlet_core::i18n::tr("adv.visitorRank", &[])));
                    }
                    if !preamble && self.deps.settings.announce_visitor_rank.get() {
                        tts_parts.push(scarlet_core::i18n::tr("adv.ttsVisitorRank", &[]));
                    }
                }
                _ => {}
            }
        }

        // One combined callout, e.g. "Vex joined the lobby. Watched groups:
        // Raiders, Trolls. New, 5 days."
        if !preamble && !tts_parts.is_empty() {
            let mut line = scarlet_core::i18n::tr(
                "adv.joinedLobbyFmt",
                &[&scarlet_core::names::sanitize_name_for_speech(display_name)],
            );
            for part in &tts_parts {
                line.push(' ');
                line.push_str(part);
            }
            let marker = marker("join");
            if nuisance_chime {
                self.deps.tts.speak_after_nuisance_alert(&marker, &line);
            } else {
                self.deps.tts.speak(&marker, &line);
            }
        }

        outcome.advisory = format_advisories(&advisories);
        outcome
    }

    /// Port of `watchedGroupsOf`.
    ///
    /// Java additionally repeats the lookup under each configured alternate
    /// VRChat account, so a membership hidden from the main account can
    /// still be seen. Alt accounts aren't ported (see PORTING.md), so this
    /// is the primary account's view only.
    async fn watched_groups_of(
        &self,
        user_id: &str,
        force_fresh: bool,
    ) -> Vec<scarlet_core::watched_groups::WatchedGroup> {
        let _ = force_fresh; // no membership cache here yet; every call is fresh
        let Some(client) = self.vrchat().await else { return Vec::new() };
        let groups = match client.get_user_groups(user_id).await {
            Ok(groups) => groups,
            Err(err) => {
                tracing::debug!("Could not read groups for {user_id}: {err}");
                return Vec::new();
            }
        };
        let mut watched: Vec<_> = groups
            .into_iter()
            .filter_map(|group| group.group_id)
            .filter_map(|group_id| self.deps.watched_groups.get_watched_group(&group_id))
            .collect();
        watched.sort_by(|a, b| {
            a.priority.cmp(&b.priority).then_with(|| a.critical.cmp(&b.critical)).then_with(|| a.silent.cmp(&b.silent))
        });
        watched
    }

    /// Port of `log_vtkInit`.
    async fn log_vtk_init(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        target_display_name: String,
        actor_display_name: Option<String>,
    ) {
        if preamble {
            return;
        }
        let user_id = self.display_name_to_user_id.get(&target_display_name).cloned();
        let actor_id = actor_display_name.as_ref().and_then(|name| self.display_name_to_user_id.get(name).cloned());

        match (&actor_id, &actor_display_name) {
            (Some(actor_id), Some(actor_name)) => tracing::info!(
                "Vote-to-Kick initiated: {target_display_name} ({user_id:?}), started by {actor_name} ({actor_id})"
            ),
            _ => tracing::info!("Vote-to-Kick initiated: {target_display_name} ({user_id:?})"),
        }

        if !self.is_in_group_instance {
            return;
        }
        let location = self.client_location.clone().unwrap_or_default();
        self.deps
            .emitter
            .emit_vtk_initiated(
                timestamp,
                &location,
                user_id.as_deref(),
                &target_display_name,
                actor_id.as_deref(),
                actor_display_name.as_deref(),
            )
            .await;
        if self.deps.settings.advisory_show_votes_to_kick.get() && self.deps.settings.announce_votes_to_kick.get() {
            let target = scarlet_core::names::sanitize_name_for_speech(&target_display_name);
            let line = match &actor_display_name {
                Some(actor) if actor_id.is_some() => scarlet_core::i18n::tr(
                    "adv.vtkAgainstBy",
                    &[&target, &scarlet_core::names::sanitize_name_for_speech(actor)],
                ),
                _ => scarlet_core::i18n::tr("adv.vtkAgainst", &[&target]),
            };
            self.deps.tts.speak(&marker("vtk"), &line);
        }
        self.deps.data.custom_event_new(
            audit_type::VTK_START,
            Utc.from_utc_datetime(&timestamp),
            actor_id,
            actor_display_name,
            user_id,
            Some(target_display_name),
            None,
        );
    }

    /// Port of `log_playerSpawnPedestal`.
    async fn log_spawn_pedestal(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        user_id: String,
        content_type: String,
        content_id: String,
    ) {
        if preamble || !self.is_in_group_instance {
            return;
        }
        let location = self.client_location.clone().unwrap_or_default();
        self.deps
            .emitter
            .emit_user_spawn_pedestal(timestamp, &location, &user_id, &display_name, &content_type, &content_id)
            .await;
        self.deps.data.custom_event_new(
            audit_type::SPAWN_PEDESTAL,
            Utc.from_utc_datetime(&timestamp),
            Some(user_id),
            Some(display_name),
            Some(content_id),
            None,
            None,
        );
    }

    /// Port of `log_playerSpawnSticker`.
    async fn log_spawn_sticker(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        user_id: String,
        sticker_id: String,
    ) {
        if preamble || !self.is_in_group_instance {
            return;
        }
        let location = self.client_location.clone().unwrap_or_default();
        self.deps.emitter.emit_user_spawn_sticker(timestamp, &location, &user_id, &display_name, &sticker_id).await;
        self.deps.data.custom_event_new(
            audit_type::SPAWN_STICKER,
            Utc.from_utc_datetime(&timestamp),
            Some(user_id),
            Some(display_name),
            Some(sticker_id),
            None,
            None,
        );
    }

    /// Port of `log_playerSpawnProp`.
    ///
    /// Java also fetches the prop itself (`vrc.getProp`) to decorate the
    /// Discord embed; that endpoint isn't ported, so the emission carries
    /// the id alone.
    async fn log_spawn_prop(&mut self, preamble: bool, timestamp: NaiveDateTime, user_id: String, prop_id: String) {
        if preamble || !self.is_in_group_instance {
            return;
        }
        let display_name = self.roster.display_name_or(&user_id, &user_id);
        let location = self.client_location.clone().unwrap_or_default();
        self.deps.emitter.emit_user_spawn_prop(timestamp, &location, &user_id, &display_name, &prop_id).await;
        self.deps.data.custom_event_new(
            audit_type::SPAWN_PROP,
            Utc.from_utc_datetime(&timestamp),
            Some(user_id),
            Some(display_name),
            Some(prop_id),
            None,
            None,
        );
    }

    /// Port of `log_apiRequest`.
    ///
    /// The print/emoji spawn detection and the avatar-bundle analysis path
    /// both need VRChat endpoints that aren't ported yet; what is ported
    /// here is the part everything else depends on — noting that this
    /// session *does* write `[API]` lines, which is what suppresses the
    /// "launched without debug logging" warning.
    fn log_api_request(&mut self, preamble: bool, index: i32, method: &str, url: &str) {
        self.seen_api_log_lines = true;
        if !preamble {
            tracing::trace!("[API #{index}] {method} {url}");
        }
    }

    /// Port of `log_videoLoad`.
    async fn log_video_load(
        &mut self,
        preamble: bool,
        timestamp: NaiveDateTime,
        display_name: String,
        url: String,
        title: String,
    ) {
        self.deps.ui.append_cli_output(&format!("{display_name} loaded video: {url}"));
        if preamble || !self.is_in_group_instance {
            return;
        }
        let mut user_id = self.display_name_to_user_id.get(&display_name).cloned();
        if user_id.is_none() {
            if let Some(client) = self.vrchat().await {
                if let Ok(results) = client.search_users(&display_name, 1, 0).await {
                    user_id = results
                        .into_iter()
                        .find(|result| result.display_name.as_deref() == Some(display_name.as_str()))
                        .map(|result| result.id);
                }
            }
            if let Some(found) = &user_id {
                self.display_name_to_user_id.insert(display_name.clone(), found.clone());
            }
        }
        let user_id = user_id.unwrap_or_default();
        let location = self.client_location.clone().unwrap_or_default();
        self.deps.emitter.emit_user_video(timestamp, &location, &user_id, &display_name, &url, &title).await;
        self.deps.data.custom_event_new(
            audit_type::USER_VIDEO,
            Utc.from_utc_datetime(&timestamp),
            Some(user_id),
            Some(display_name),
            Some(url),
            Some(title),
            None,
        );
    }
}

/// Drains the tailer's events into a listener until the tailer stops.
pub async fn run(
    mut listener: ScarletEventListener,
    mut events: tokio::sync::mpsc::UnboundedReceiver<TailerEvent>,
) {
    while let Some(event) = events.recv().await {
        listener.handle(event).await;
    }
}

/// Java builds these as `"join-" + Long.toUnsignedString(System.nanoTime())`.
fn marker(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}")
}

/// Port of `endDot`: trims a fragment and ensures it ends with sentence
/// punctuation, for clean concatenation.
fn end_dot(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match trimmed.chars().next_back() {
        Some('.') | Some('!') | Some('?') => trimmed.to_string(),
        _ => format!("{trimmed}."),
    }
}

/// Port of `addAdvisory`.
fn add_advisory(advisories: &mut Vec<String>, advisory: Option<&str>) {
    let Some(advisory) = advisory else { return };
    let trimmed = advisory.trim();
    if !trimmed.is_empty() {
        advisories.push(trimmed.to_string());
    }
}

/// Port of `formatAdvisories`: distinct, trimmed, joined with " / ".
fn format_advisories(advisories: &[String]) -> Option<String> {
    let mut seen: Vec<&str> = Vec::new();
    for advisory in advisories {
        let trimmed = advisory.trim();
        if !trimmed.is_empty() && !seen.contains(&trimmed) {
            seen.push(trimmed);
        }
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.join(" / "))
    }
}

/// Port of `watchedAvatarAdvisory`.
fn watched_avatar_advisory(message: Option<&str>) -> String {
    match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(message) => scarlet_core::i18n::tr("adv.watchedAvatarMsg", &[message]),
        None => scarlet_core::i18n::tr("adv.watchedAvatar", &[]),
    }
}

/// Java's `Period.between(dateJoined, today)` as (years, months, days).
///
/// This is `LocalDate.until(LocalDate)`'s actual algorithm, not an
/// approximation: total whole months first, then the day remainder measured
/// from the start date advanced by those months — with month-end clamping
/// (2020-01-31 plus 73 months is 2026-02-28, not an invalid 2026-02-31).
/// Subtracting calendar fields directly gets this wrong by a day or two
/// around month ends, which a first attempt here did.
fn period_between(from: chrono::NaiveDate, to: chrono::NaiveDate) -> (i32, i32, i32) {
    if to < from {
        // Java returns a negative period; no caller here can produce one (a
        // VRChat account can't be created in the future), and a negative
        // "account age" in the table would be nonsense, so clamp.
        return (0, 0, 0);
    }
    let mut total_months =
        (to.year() as i64 * 12 + to.month() as i64) - (from.year() as i64 * 12 + from.month() as i64);
    let mut days = to.day() as i64 - from.day() as i64;
    if total_months > 0 && days < 0 {
        total_months -= 1;
        let advanced = from
            .checked_add_months(chrono::Months::new(total_months as u32))
            .unwrap_or(from);
        days = (to - advanced).num_days();
    }
    ((total_months / 12) as i32, (total_months % 12) as i32, days as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{RecordingEmitter, RecordingTts, RecordingUi};
    use scarlet_vrchat::log_tail::{LogEvent, LogEventKind, TailerEvent};

    const GROUP_ID: &str = "grp_11111111-1111-1111-1111-111111111111";
    const GROUP_LOCATION: &str =
        "wrld_aaaa:12345~group(grp_11111111-1111-1111-1111-111111111111)~groupAccessType(members)~region(us)";
    const PRIVATE_LOCATION: &str = "wrld_bbbb:54321~private(usr_owner)~region(us)";

    struct Harness {
        listener: ScarletEventListener,
        ui: RecordingUi,
        emitter: RecordingEmitter,
        tts: RecordingTts,
        _dir: tempfile::TempDir,
    }

    fn harness(with_group_id: bool) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(dir.path()));
        if with_group_id {
            settings.store.set_string("vrchat_group_id", GROUP_ID);
        }
        let ui = RecordingUi::default();
        let emitter = RecordingEmitter::default();
        let tts = RecordingTts::default();
        let listener = ScarletEventListener::new(ListenerDeps {
            ui: Arc::new(ui.clone()),
            // No VRChat session in tests: every profile lookup is skipped,
            // which is also what a real run looks like before login
            // completes — the listener must still track the instance.
            vrchat: Arc::new(tokio::sync::RwLock::new(None)),
            settings: settings.clone(),
            data: Arc::new(ScarletData::open(dir.path())),
            staff_list: Arc::new(StaffList::open(dir.path(), "staff_list.json")),
            watched_users: Arc::new(WatchedEntities::open(
                dir.path(),
                "watched_users.json",
                scarlet_core::watched_entities::is_valid_user_id,
            )),
            watched_avatars: Arc::new(WatchedEntities::open(
                dir.path(),
                "watched_avatars.json",
                scarlet_core::watched_entities::is_valid_avatar_id,
            )),
            watched_groups: Arc::new(WatchedGroups::open(dir.path(), "watched_groups.json")),
            pronoun_lists: Arc::new(PronounLists::open(dir.path())),
            tts: Arc::new(tts.clone()),
            emitter: Arc::new(emitter.clone()),
        });
        Harness { listener, ui, emitter, tts, _dir: dir }
    }

    fn at(secs: i64) -> NaiveDateTime {
        chrono::DateTime::from_timestamp(secs, 0).unwrap().naive_utc()
    }

    fn entry(preamble: bool, secs: i64, kind: LogEventKind) -> TailerEvent {
        TailerEvent::Entry(LogEvent { preamble, timestamp: at(secs), kind })
    }

    fn joined(location: &str) -> LogEventKind {
        LogEventKind::UserJoined { location: location.to_string() }
    }

    fn player_joined(name: &str, id: &str) -> LogEventKind {
        LogEventKind::PlayerJoined { display_name: name.to_string(), user_id: id.to_string() }
    }

    fn player_left(name: &str, id: &str) -> LogEventKind {
        LogEventKind::PlayerLeft { display_name: name.to_string(), user_id: id.to_string() }
    }

    /// The reported bug: the app didn't think an instance was open. Joining
    /// one must populate instance state *and* the player table.
    #[tokio::test]
    async fn joining_an_instance_opens_it_and_players_appear() {
        let mut h = harness(true);
        assert!(h.ui.calls().instance.is_none(), "not in an instance before any log line");

        h.listener.handle(entry(false, 100, LogEventKind::EnteringRoom { world_name: "Popcorn Palace".into() })).await;
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;

        let instance = h.ui.calls().instance.clone().expect("an instance must be open after Joining");
        assert_eq!(instance.location, GROUP_LOCATION);
        assert_eq!(instance.world_name.as_deref(), Some("Popcorn Palace"), "the world name comes from Entering Room");
        assert!(instance.is_group_instance, "this location carries the configured group id");

        h.listener.handle(entry(false, 101, player_joined("Church", "usr_church"))).await;
        assert_eq!(h.ui.calls().joins.len(), 1);
        assert_eq!(h.ui.player("usr_church").unwrap().name, "Church");
    }

    #[tokio::test]
    async fn leaving_closes_the_instance_and_clears_the_table() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Church", "usr_church"))).await;

        h.listener.handle(entry(false, 200, LogEventKind::UserLeft)).await;
        assert!(h.ui.calls().instance.is_none(), "no instance is open after OnLeftRoom");
        assert!(h.ui.calls().players.is_empty(), "the table is cleared when the client leaves");
    }

    /// A location without the configured group id is not a group instance,
    /// which is the gate on every Discord emission and custom event.
    #[tokio::test]
    async fn only_group_instances_emit_audit_events() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(PRIVATE_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Someone", "usr_someone"))).await;
        assert!(h.emitter.events().is_empty(), "a private instance emits nothing");
        // ...but the player still shows in the table.
        assert_eq!(h.ui.calls().joins.len(), 1);

        h.listener.handle(entry(false, 200, LogEventKind::UserLeft)).await;
        h.listener.handle(entry(false, 201, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 202, player_joined("Someone", "usr_someone"))).await;
        assert_eq!(h.emitter.events(), vec!["user_join:usr_someone".to_string()]);

        h.listener.handle(entry(false, 203, player_left("Someone", "usr_someone"))).await;
        assert_eq!(
            h.emitter.events(),
            vec!["user_join:usr_someone".to_string(), "user_leave:usr_someone".to_string()]
        );
    }

    /// With no group id configured nothing is ever a group instance — the
    /// stale/blank `vrchat_group_id` failure mode that silently stops all
    /// logging in the Java build.
    #[tokio::test]
    async fn without_a_configured_group_id_nothing_is_a_group_instance() {
        let mut h = harness(false);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Someone", "usr_someone"))).await;
        assert!(h.emitter.events().is_empty());
        assert!(
            !h.ui.calls().instance.as_ref().unwrap().is_group_instance,
            "no configured group means the instance can't be ours"
        );
    }

    /// Catch-up rows are queued and flushed in log order at `CatchUp`, so
    /// people already in the instance when Scarlet starts still appear.
    #[tokio::test]
    async fn catch_up_rows_appear_when_catch_up_completes() {
        let mut h = harness(true);
        h.listener.handle(TailerEvent::Init { file: std::path::PathBuf::from("output_log_x.txt") }).await;
        h.listener.handle(entry(true, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(true, 101, player_joined("Early", "usr_early"))).await;
        h.listener.handle(entry(true, 102, player_joined("AlsoEarly", "usr_also"))).await;
        assert!(h.ui.calls().joins.is_empty(), "catch-up rows are deferred, not shown as they are read");

        h.listener.handle(TailerEvent::CatchUp { file: Some(std::path::PathBuf::from("output_log_x.txt")) }).await;
        let names: Vec<String> = h.ui.calls().joins.iter().map(|j| j.name.clone()).collect();
        assert_eq!(names, vec!["Early".to_string(), "AlsoEarly".to_string()], "flushed in log order");
        // Catch-up is silent: no announcements for people who were already
        // there before Scarlet started.
        assert!(h.tts.spoken().is_empty());
        // And no Discord emission for historical joins either.
        assert!(h.emitter.events().is_empty());
    }

    /// A join and a leave for the same player during catch-up must flush in
    /// order — the inversion that used to leave departed players marked
    /// present and inflate the count.
    #[tokio::test]
    async fn a_catch_up_join_then_leave_ends_as_departed() {
        let mut h = harness(true);
        h.listener.handle(entry(true, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(true, 101, player_joined("Passer", "usr_passer"))).await;
        h.listener.handle(entry(true, 102, player_left("Passer", "usr_passer"))).await;
        h.listener.handle(TailerEvent::CatchUp { file: Some(std::path::PathBuf::from("x")) }).await;

        let player = h.ui.player("usr_passer").expect("the row still exists");
        assert!(player.left.is_some(), "a player who left during catch-up must not read as present");
    }

    /// Rejoining the *same* instance marks the row as a carried-over rejoin
    /// so its original join time is kept.
    #[tokio::test]
    async fn rejoining_the_same_instance_is_flagged_as_a_rejoin() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Stayer", "usr_stayer"))).await;
        h.listener.handle(entry(false, 150, LogEventKind::UserLeft)).await;
        h.listener.handle(entry(false, 160, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 161, player_joined("Stayer", "usr_stayer"))).await;

        let last = h.ui.calls().joins.last().cloned().unwrap();
        assert!(last.is_rejoin_from_prev, "same location + present before = carried-over rejoin");

        // A different instance is not a rejoin.
        h.listener.handle(entry(false, 200, LogEventKind::UserLeft)).await;
        h.listener.handle(entry(false, 201, joined(PRIVATE_LOCATION))).await;
        h.listener.handle(entry(false, 202, player_joined("Stayer", "usr_stayer"))).await;
        assert!(!h.ui.calls().joins.last().unwrap().is_rejoin_from_prev);
    }

    /// The mixed-character-name check runs with no VRChat session at all,
    /// because it reads only the display name — so it works during the
    /// window before login completes.
    #[tokio::test]
    async fn a_mixed_script_name_is_flagged_and_announced() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        // Latin name with Cyrillic lookalikes, from a real log.
        h.listener.handle(entry(false, 101, player_joined("Вlakе", "usr_spoof"))).await;

        let advisory = h.ui.player("usr_spoof").unwrap().advisory.unwrap();
        assert_eq!(advisory, scarlet_core::i18n::tr("adv.mixedCharName", &[]));
        assert_eq!(h.tts.spoken(), vec!["[mixed-name alert]".to_string()]);
    }

    /// The local user is exempt: Scarlet must not flag its own account's
    /// name on every single join.
    #[tokio::test]
    async fn the_local_users_own_name_is_not_flagged() {
        let mut h = harness(true);
        h.listener
            .handle(entry(
                false,
                90,
                LogEventKind::UserAuthenticated { display_name: "Вlakе".into(), user_id: "usr_self".into() },
            ))
            .await;
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Вlakе", "usr_self"))).await;

        assert!(h.ui.player("usr_self").unwrap().advisory.is_none());
        assert!(h.tts.spoken().is_empty());
    }

    /// A watched user's message becomes the row advisory and is spoken.
    #[tokio::test]
    async fn a_watched_user_produces_an_advisory_and_a_callout() {
        let h_dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::open(h_dir.path()));
        settings.store.set_string("vrchat_group_id", GROUP_ID);
        let watched_users = Arc::new(WatchedEntities::open(
            h_dir.path(),
            "watched_users.json",
            scarlet_core::watched_entities::is_valid_user_id,
        ));
        let mut entity = scarlet_core::watched_entities::WatchedEntity::new("usr_11111111-1111-1111-1111-111111111111");
        entity.message = Some("Known ban evader".to_string());
        watched_users.add_watched_entity(entity);

        let ui = RecordingUi::default();
        let tts = RecordingTts::default();
        let mut listener = ScarletEventListener::new(ListenerDeps {
            ui: Arc::new(ui.clone()),
            vrchat: Arc::new(tokio::sync::RwLock::new(None)),
            settings,
            data: Arc::new(ScarletData::open(h_dir.path())),
            staff_list: Arc::new(StaffList::open(h_dir.path(), "staff_list.json")),
            watched_users,
            watched_avatars: Arc::new(WatchedEntities::open(
                h_dir.path(),
                "watched_avatars.json",
                scarlet_core::watched_entities::is_valid_avatar_id,
            )),
            watched_groups: Arc::new(WatchedGroups::open(h_dir.path(), "watched_groups.json")),
            pronoun_lists: Arc::new(PronounLists::open(h_dir.path())),
            tts: Arc::new(tts.clone()),
            emitter: Arc::new(crate::NoopAuditEmitter),
        });

        listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        listener
            .handle(entry(false, 101, player_joined("Evader", "usr_11111111-1111-1111-1111-111111111111")))
            .await;

        let player = ui.player("usr_11111111-1111-1111-1111-111111111111").unwrap();
        assert_eq!(player.advisory.as_deref(), Some("Known ban evader"));
        assert_eq!(tts.spoken().len(), 1);
        assert!(tts.spoken()[0].contains("Evader"), "the callout names the player: {:?}", tts.spoken());
        assert!(tts.spoken()[0].contains("Known ban evader"));
    }

    /// Avatar switches update the row, and a switch by someone who has
    /// already left is ignored.
    #[tokio::test]
    async fn avatar_switches_track_present_players_only() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        h.listener.handle(entry(false, 101, player_joined("Wearer", "usr_wearer"))).await;
        h.listener
            .handle(entry(
                false,
                102,
                LogEventKind::PlayerSwitchAvatar {
                    display_name: "Wearer".into(),
                    avatar_display_name: "Thicc Catocat".into(),
                },
            ))
            .await;
        assert_eq!(h.ui.player("usr_wearer").unwrap().avatar_name.as_deref(), Some("Thicc Catocat"));

        h.listener.handle(entry(false, 103, player_left("Wearer", "usr_wearer"))).await;
        h.listener
            .handle(entry(
                false,
                104,
                LogEventKind::PlayerSwitchAvatar {
                    display_name: "Wearer".into(),
                    avatar_display_name: "Something Else".into(),
                },
            ))
            .await;
        assert_eq!(
            h.ui.player("usr_wearer").unwrap().avatar_name.as_deref(),
            Some("Thicc Catocat"),
            "a departed player's avatar is frozen, not updated"
        );
    }

    /// With no VRChat client at all, no log line may panic or stall — this
    /// is the state during startup and on a machine that never logs in.
    #[tokio::test]
    async fn every_event_kind_is_safe_without_a_vrchat_session() {
        let mut h = harness(true);
        h.listener.handle(entry(false, 100, joined(GROUP_LOCATION))).await;
        let events = vec![
            player_joined("A", "usr_a"),
            LogEventKind::PlayerSwitchAvatar { display_name: "A".into(), avatar_display_name: "Av".into() },
            LogEventKind::VtkInit { target_display_name: "A".into(), actor_display_name: Some("B".into()) },
            LogEventKind::PlayerSpawnPedestal {
                display_name: "A".into(),
                user_id: "usr_a".into(),
                content_type: "avatar".into(),
                content_id: "avtr_1".into(),
            },
            LogEventKind::PlayerSpawnSticker {
                display_name: "A".into(),
                user_id: "usr_a".into(),
                sticker_id: "sticker_1".into(),
            },
            LogEventKind::PlayerSpawnProp { user_id: "usr_a".into(), prop_id: "prop_1".into() },
            LogEventKind::ApiRequest { index: 1, method: "GET".into(), url: "https://api.vrchat.cloud/api/1/x".into() },
            LogEventKind::VideoLoad { display_name: "A".into(), url: "https://x/y".into(), title: String::new() },
            player_left("A", "usr_a"),
            LogEventKind::UserQuit { lifetime_seconds: 12.0 },
            LogEventKind::UserLeft,
        ];
        for (index, kind) in events.into_iter().enumerate() {
            h.listener.handle(entry(false, 200 + index as i64, kind)).await;
        }
        // Reaching here without a panic is the assertion; the emitter having
        // seen the group-instance events confirms it did real work.
        assert!(h.emitter.events().iter().any(|event| event.starts_with("user_join:")));
    }

    #[test]
    fn advisory_helpers_match_java() {
        assert_eq!(end_dot(" hello "), "hello.");
        assert_eq!(end_dot("done!"), "done!");
        assert_eq!(end_dot("   "), "");

        let mut advisories = Vec::new();
        add_advisory(&mut advisories, Some(" first "));
        add_advisory(&mut advisories, Some("   "));
        add_advisory(&mut advisories, None);
        add_advisory(&mut advisories, Some("first"));
        add_advisory(&mut advisories, Some("second"));
        // Duplicates collapse in the formatter, matching `distinct()`.
        assert_eq!(format_advisories(&advisories).as_deref(), Some("first / second"));
        assert_eq!(format_advisories(&[]), None);
    }

    #[test]
    fn account_age_period_matches_javas_period_between() {
        use chrono::NaiveDate;
        let from = NaiveDate::from_ymd_opt(2020, 1, 31).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        // Java's Period.between(2020-01-31, 2026-03-01) is 6y 1m 1d.
        assert_eq!(period_between(from, to), (6, 1, 1));
        assert_eq!(
            period_between(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), NaiveDate::from_ymd_opt(2026, 9, 27).unwrap()),
            (0, 0, 26)
        );
        // Month-end clamping: 2020-01-31 + 73 months lands on 2026-02-28.
        assert_eq!(
            period_between(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap(), NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
            (0, 1, 1)
        );
        // A future join date can't produce a negative age.
        assert_eq!(
            period_between(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(), NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            (0, 0, 0)
        );
    }
}
