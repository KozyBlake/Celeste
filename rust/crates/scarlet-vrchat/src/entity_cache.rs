//! Port of the caching half of `ScarletVRChat`'s entity getters.
//!
//! Java gives every entity lookup the same three-step shape (see
//! `ScarletVRChat.getUser(String, long)`, and the identical bodies for
//! groups, worlds, avatars, prints, props, model files, user groups and
//! public profiles):
//!
//! ```java
//! T value = this.cachedXs.get(id, minEpoch);   // 1. a cache entry that's new enough
//! if (value != null) return value;
//! if (this.cachedXs.is404(id)) return null;    // 2. a known-404 never re-fetches
//! try { value = api.getX(id); this.cachedXs.put(id, value); return value; }
//! catch (ApiException e) {                     // 3. fetch, then record
//!     if (is404(e)) this.cachedXs.add404(id);
//!     return null;
//! }
//! ```
//!
//! The port had `ScarletJsonCache` written but nothing wired to it, so every
//! lookup was an unconditional HTTP call. That is not merely slower than
//! Java: watched-entity autocomplete calls `_getEntity(id,
//! ALWAYS_PREFER_CACHED)` once per watched entity **per keystroke**, which
//! Java answers entirely from disk after the first time and the port was
//! answering with up to 25 live API calls. Sustained, that is the kind of
//! request rate VRChat suspends accounts over.
//!
//! The caches are opt-in: a [`VrChatClient`](crate::client::VrChatClient)
//! built without a cache root behaves exactly as before (always fetch, never
//! record), which is what the wiremock tests want. The application sets the
//! root to `<data dir>/caches`, matching Java's `new File(Scarlet.dir,
//! "caches")` so the on-disk layout stays drop-in compatible.

use std::path::Path;

use scarlet_core::json_cache::ScarletJsonCache;

pub use scarlet_core::json_cache::{ALWAYS_FETCH, ALWAYS_PREFER_CACHED};

/// Java's `long within1day = System.currentTimeMillis() - 86400_000L`, the
/// max-age most read-only call sites pass: a cached entry from within the
/// last day is good enough, anything older is re-fetched.
pub fn within_1_day() -> i64 {
    now_millis() - 86_400_000
}

/// Java's `System.currentTimeMillis()`; cache entry ages are file mtimes in
/// epoch millis, so the two are directly comparable.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

use crate::models::{Avatar, Group, LimitedUserGroup, PublicProfile, User, World};

/// The per-kind caches `ScarletVRChat` constructs in its constructor. The
/// `kind` strings are Java's and decide the on-disk directory name, so they
/// are part of the data-compatibility surface — do not rename them.
pub struct EntityCaches {
    pub users: ScarletJsonCache<User>,
    pub public_profiles: ScarletJsonCache<PublicProfile>,
    pub worlds: ScarletJsonCache<World>,
    pub groups: ScarletJsonCache<Group>,
    pub user_groups: ScarletJsonCache<Vec<LimitedUserGroup>>,
    pub avatars: ScarletJsonCache<Avatar>,
}

impl EntityCaches {
    /// `caches_root` is Java's `<Scarlet.dir>/caches`.
    pub fn new(caches_root: &Path) -> Self {
        Self {
            users: ScarletJsonCache::new(caches_root, "usr"),
            public_profiles: ScarletJsonCache::new(caches_root, "pubprof"),
            worlds: ScarletJsonCache::new(caches_root, "wrld"),
            groups: ScarletJsonCache::new(caches_root, "grp"),
            user_groups: ScarletJsonCache::new(caches_root, "gmem"),
            avatars: ScarletJsonCache::new(caches_root, "avtr"),
        }
    }
}

/// Java holds its caches in a `static final File caches = new
/// File(Scarlet.dir, "caches")`, so every `ScarletVRChat` shares one set
/// keyed off the process-wide data directory. This is that static: the
/// application calls [`install`] once the data directory is resolved, and
/// every [`VrChatClient`](crate::client::VrChatClient) built afterwards
/// picks the caches up. A process that never installs them (tests, the
/// wiremock suite) keeps the pre-cache behaviour of always fetching.
static SHARED: std::sync::OnceLock<std::sync::Arc<EntityCaches>> = std::sync::OnceLock::new();

/// Installs the process-wide caches under `<data dir>/caches`. Only the
/// first call takes effect, and it must happen before any client is built
/// (`main` does it right after resolving the data directory). Returns
/// whether this call was the one that installed them.
pub fn install(data_dir: &Path) -> bool {
    let root = data_dir.join("caches");
    SHARED.set(std::sync::Arc::new(EntityCaches::new(&root))).is_ok()
}

/// The installed caches, if any.
pub fn shared() -> Option<std::sync::Arc<EntityCaches>> {
    SHARED.get().cloned()
}

/// What a cache lookup decided before any network call happens.
pub(crate) enum CacheDecision<T> {
    /// A cached value new enough for this call site's `min_epoch`.
    Hit(T),
    /// The id is in the known-404 set: Java returns null without fetching,
    /// and does so even when `min_epoch` is `ALWAYS_FETCH`.
    Known404,
    /// Nothing usable cached; the caller should fetch.
    Miss,
}

/// The read half of Java's pattern, factored out so every getter shares it.
pub(crate) fn decide<T>(
    cache: Option<&ScarletJsonCache<T>>,
    id: &str,
    min_epoch: i64,
) -> CacheDecision<T>
where
    T: Clone + Send + serde::Serialize + serde::de::DeserializeOwned,
{
    let Some(cache) = cache else {
        return CacheDecision::Miss;
    };
    if let Some(value) = cache.get_min_epoch(id, min_epoch) {
        return CacheDecision::Hit(value);
    }
    if cache.is_404(Some(id)) {
        return CacheDecision::Known404;
    }
    CacheDecision::Miss
}

/// The write half: record a successful fetch, or a 404, exactly as Java's
/// catch block does. Any other error is left unrecorded so it is retried.
pub(crate) fn record<T>(
    cache: Option<&ScarletJsonCache<T>>,
    id: &str,
    result: &Result<T, crate::client::VrChatError>,
) where
    T: Clone + Send + serde::Serialize + serde::de::DeserializeOwned,
{
    let Some(cache) = cache else { return };
    match result {
        Ok(value) => {
            cache.put(id, value.clone());
            // A successful fetch clears a stale 404, like `getUserFresh`.
            cache.remove_404(id);
        }
        Err(error) if is_not_found(error) => {
            cache.add_404(id);
        }
        Err(_) => {}
    }
}

/// Java matches on `apiex.getMessage().contains("HTTP response code: 404")`.
pub(crate) fn is_not_found(error: &crate::client::VrChatError) -> bool {
    matches!(error, crate::client::VrChatError::Api { status, .. } if status.as_u16() == 404)
}
