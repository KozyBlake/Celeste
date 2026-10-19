//! The two outbound interfaces the event listener needs, as traits.
//!
//! Java calls `scarlet.discord.emitExtended*` and `scarlet.getTtsService()`
//! directly. Both are defined here as traits so this crate doesn't depend on
//! `celeste-discord` (which would be a cycle once Discord commands need
//! listener state) or on `celeste-ui` (which would drag egui into headless
//! runs). The real implementations live in those crates; the no-op ones
//! below are what a headless run without Discord, or a test, uses.

use chrono::NaiveDateTime;

/// What was spawned, for the two spawn events that carry a file: a print or
/// an emoji. The who/where/when half stays as arguments, because every
/// emission shares it; this is the half that differs per event.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedFile<'a> {
    /// The print or emoji id.
    pub id: &'a str,
    pub file_id: Option<&'a str>,
    pub image_url: Option<&'a str>,
}

/// What was spawned, for a prop, which carries its author rather than a
/// file id.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedProp<'a> {
    pub id: &'a str,
    pub author_id: Option<&'a str>,
    pub author_name: Option<&'a str>,
    pub image_url: Option<&'a str>,
}

/// Port of the `ScarletDiscordJDA.emitExtended*` surface the event listener
/// calls, plus `isEmitting`.
///
/// Every method is "fire and forget" from the listener's point of view:
/// Java's implementations post to Discord and return, and a failure there
/// must never stall log processing.
#[async_trait::async_trait]
pub trait AuditEmitter: Send + Sync {
    /// Port of `isEmitting(GroupAuditTypeEx)`: whether any Discord channel
    /// or webhook is configured for this event type. The listener checks it
    /// before doing the work an emission would need.
    async fn is_emitting(&self, event_type: &str) -> bool;

    async fn emit_user_join(&self, timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str);
    async fn emit_user_leave(&self, timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str);
    async fn emit_staff_join(&self, timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str);
    async fn emit_staff_leave(&self, timestamp: NaiveDateTime, location: &str, user_id: &str, display_name: &str);
    async fn emit_user_avatar(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        avatar_display_name: &str,
        potential_avatar_ids: &[String],
    );
    async fn emit_vtk_initiated(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        target_id: Option<&str>,
        target_display_name: &str,
        actor_id: Option<&str>,
        actor_display_name: Option<&str>,
    );
    async fn emit_user_spawn_pedestal(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        content_type: &str,
        content_id: &str,
    );
    async fn emit_user_spawn_sticker(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        sticker_id: &str,
        file_id: Option<&str>,
    );
    async fn emit_user_spawn_print(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        print: SpawnedFile<'_>,
    );
    async fn emit_user_spawn_emoji(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        emoji: SpawnedFile<'_>,
    );
    async fn emit_user_spawn_prop(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        prop: SpawnedProp<'_>,
    );
    async fn emit_user_video(
        &self,
        timestamp: NaiveDateTime,
        location: &str,
        user_id: &str,
        display_name: &str,
        url: &str,
        title: &str,
    );
}

/// What a run with no Discord connection uses. Every emission is dropped and
/// `is_emitting` is false, which is exactly how Java behaves with no audit
/// channels configured.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAuditEmitter;

#[async_trait::async_trait]
impl AuditEmitter for NoopAuditEmitter {
    async fn is_emitting(&self, _event_type: &str) -> bool {
        false
    }
    async fn emit_user_join(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str) {}
    async fn emit_user_leave(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str) {}
    async fn emit_staff_join(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str) {}
    async fn emit_staff_leave(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str) {}
    async fn emit_user_avatar(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _a: &str, _p: &[String]) {}
    async fn emit_vtk_initiated(
        &self,
        _t: NaiveDateTime,
        _l: &str,
        _ti: Option<&str>,
        _td: &str,
        _ai: Option<&str>,
        _ad: Option<&str>,
    ) {
    }
    async fn emit_user_spawn_pedestal(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _c: &str, _i: &str) {}
    async fn emit_user_spawn_sticker(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _s: &str, _f: Option<&str>) {}
    async fn emit_user_spawn_print(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _p: SpawnedFile<'_>) {}
    async fn emit_user_spawn_emoji(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _e: SpawnedFile<'_>) {}
    async fn emit_user_spawn_prop(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _p: SpawnedProp<'_>) {}
    async fn emit_user_video(&self, _t: NaiveDateTime, _l: &str, _u: &str, _d: &str, _url: &str, _title: &str) {}
}

/// The slice of `TtsService` the event listener uses.
///
/// `marker` is Java's per-utterance id (`join-<nanos>`, `vtk-<nanos>`, ...),
/// used to correlate log lines with what was spoken.
pub trait TtsSink: Send + Sync {
    /// Speak `text`.
    fn speak(&self, marker: &str, text: &str);
    /// Play the bundled nuisance-rank alarm tone, then speak `text`
    /// (`submitNuisanceJoinAlert`).
    fn speak_after_nuisance_alert(&self, marker: &str, text: &str);
    /// Play the bundled mixed-character-name alert, then speak the
    /// catalogue's standard warning (`submitMixedCharacterNameJoinAlert`).
    fn speak_mixed_character_name_alert(&self, marker: &str);
}

/// Used when TTS is unavailable (headless, no speech engine installed).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTts;

impl TtsSink for NoopTts {
    fn speak(&self, _marker: &str, _text: &str) {}
    fn speak_after_nuisance_alert(&self, _marker: &str, _text: &str) {}
    fn speak_mixed_character_name_alert(&self, _marker: &str) {}
}


/// One avatar as an avatar-search provider knows it (port of the fields
/// `AvatarSearch.VrcxAvatar` exposes that the status path reads).
#[derive(Debug, Clone, Default)]
pub struct SearchedAvatar {
    pub id: String,
    pub name: Option<String>,
    /// Performance rating a provider has recorded, if any — the fallback
    /// used when VRChat's own API won't hand over a private avatar's data.
    pub performance_rating: Option<String>,
}

/// The slice of `AvatarSearch` the event listener needs.
///
/// Java reaches the provider list through `ScarletDiscordJDA`; defining it
/// as a trait keeps that dependency out of this crate, the same way
/// [`AuditEmitter`] does for Discord posting.
#[async_trait::async_trait]
pub trait AvatarSearch: Send + Sync {
    /// Avatars whose name matches `avatar_display_name` exactly, across the
    /// configured providers.
    async fn search_by_name(&self, avatar_display_name: &str) -> Vec<SearchedAvatar>;
}

/// Used when avatar search is disabled or unavailable: finds nothing, which
/// is the same state Java is in with `Features.AVATAR_SEARCH_ENABLED` off.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAvatarSearch;

#[async_trait::async_trait]
impl AvatarSearch for NoAvatarSearch {
    async fn search_by_name(&self, _avatar_display_name: &str) -> Vec<SearchedAvatar> {
        Vec::new()
    }
}
