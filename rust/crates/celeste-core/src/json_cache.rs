//! Port of `net.sybyline.scarlet.ScarletJsonCache`: a disk-backed, LRU-capped
//! JSON cache keyed by string id, with a per-"kind" known-404 set to avoid
//! repeatedly re-fetching ids that are known not to exist.
//!
//! Layout on disk (unchanged from Java): `<caches_root>/<kind>/<id>.json`
//! for each cached value, plus `<caches_root>/<kind>/known_404s.json` for
//! the 404 set. The Java version resolved `caches_root` from the
//! `Scarlet.dir` static; that data-dir resolution logic isn't ported yet
//! (see PORTING.md), so this takes `caches_root` as an explicit constructor
//! argument instead of reaching for a global — the caller (eventually
//! `celeste_core::paths`) decides where that lives.
//!
//! ## Locking, ported faithfully
//!
//! The Java `entries` map is `LRUMap.ofSynchronized()` —
//! `Collections.synchronizedMap` wraps *every* method, including
//! `compute()`, in `synchronized (mutex)` on the whole map, not just the
//! affected bucket (that finer-grained locking is a `ConcurrentHashMap`
//! thing; a synchronized `LinkedHashMap` doesn't get it). That means the
//! *real* Java behavior is: a slow `populate` callback (e.g. a network
//! call) blocks every other `get`/`put` on the same cache instance, not
//! just concurrent calls for the same id. This port reproduces that
//! exactly with a single `std::sync::Mutex` held for the duration of
//! [`CelesteJsonCache::get_entry`]'s populate call — it looks like an odd
//! contention bottleneck until you know it's intentised parity, not
//! accidental coarseness.
//!
//! `populate` is a synchronous closure (`Callable<T>` in Java is blocking
//! too). If a future async call site needs this, bridge at the call site
//! (e.g. `tokio::task::spawn_blocking`, or fetch first and call
//! [`CelesteJsonCache::put`]) rather than holding a sync mutex across an
//! `.await` — don't make this cache `async fn` speculatively before there's
//! a real async caller to design it against.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Matches `CelesteJsonCache.ALWAYS_FETCH`: pass as `min_epoch` to force a
/// cache miss (no cached entry is ever "new enough"), always invoking
/// `populate`.
pub const ALWAYS_FETCH: i64 = i64::MAX;
/// Matches `CelesteJsonCache.ALWAYS_PREFER_CACHED`: pass as `min_epoch` to
/// accept any cached entry regardless of age.
pub const ALWAYS_PREFER_CACHED: i64 = i64::MIN;

const DEFAULT_MAX_ENTRIES: usize = 1024;

#[derive(Clone)]
pub struct Entry<T> {
    id: String,
    value: T,
    /// Last-modified time of the backing file, in epoch millis (0 for an
    /// entry that hasn't been saved to disk yet).
    modified: i64,
}

impl<T> Entry<T> {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn modified(&self) -> i64 {
        self.modified
    }
}

pub struct CelesteJsonCache<T> {
    kind: String,
    dir: PathBuf,
    entries: Mutex<lru::LruCache<String, Entry<T>>>,
    known_404s: Mutex<HashSet<String>>,
}

impl<T> CelesteJsonCache<T>
where
    T: Serialize + DeserializeOwned + Clone + Send,
{
    pub fn new(caches_root: &Path, kind: &str) -> Self {
        let cache = CelesteJsonCache {
            kind: kind.to_string(),
            dir: caches_root.join(kind),
            entries: Mutex::new(lru::LruCache::new(
                NonZeroUsize::new(DEFAULT_MAX_ENTRIES).unwrap(),
            )),
            known_404s: Mutex::new(HashSet::new()),
        };
        cache.load_404s();
        cache
    }

    pub fn is_404(&self, id: Option<&str>) -> bool {
        match id {
            None => true,
            Some(id) => self.known_404s.lock().unwrap_or_else(|e| e.into_inner()).contains(id),
        }
    }

    pub fn add_404(&self, id: &str) -> bool {
        let inserted = self.known_404s.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        if inserted {
            self.save_404s();
        }
        inserted
    }

    pub fn remove_404(&self, id: &str) -> bool {
        let removed = self.known_404s.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
        if removed {
            self.save_404s();
        }
        removed
    }

    fn known_404s_path(&self) -> PathBuf {
        self.dir.join("known_404s.json")
    }

    fn load_404s(&self) {
        let path = self.known_404s_path();
        if !path.is_file() {
            return;
        }
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                tracing::error!("Exception reading known 404s JSON of kind `{}`", self.kind);
                return;
            }
        };
        match serde_json::from_str::<Vec<String>>(&contents) {
            Ok(list) => {
                self.known_404s.lock().unwrap_or_else(|e| e.into_inner()).extend(list);
            }
            Err(_) => {
                tracing::error!("Exception reading known 404s JSON of kind `{}`", self.kind);
            }
        }
    }

    fn save_404s(&self) {
        let _ = std::fs::create_dir_all(&self.dir);
        let path = self.known_404s_path();
        let set = self.known_404s.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<&String> = set.iter().collect();
        list.sort();
        match serde_json::to_string_pretty(&list) {
            Ok(json) => {
                if std::fs::write(&path, json).is_err() {
                    tracing::error!("Exception writing known 404s JSON of kind `{}`", self.kind);
                }
            }
            Err(_) => {
                tracing::error!("Exception writing known 404s JSON of kind `{}`", self.kind);
            }
        }
    }

    /// Port of `get(String id)`: any cached value, regardless of age.
    pub fn get(&self, id: &str) -> Option<T> {
        self.get_entry(id).map(|e| e.value)
    }

    /// Port of `get(String id, long minEpoch)`.
    pub fn get_min_epoch(&self, id: &str, min_epoch: i64) -> Option<T> {
        self.get_entry_min_epoch(id, min_epoch).map(|e| e.value)
    }

    /// Port of `getFrom`/`get(id, minEpoch, populate)`: populates via the
    /// given closure on a cache miss (absent, or older than `min_epoch`).
    pub fn get_from(
        &self,
        id: &str,
        min_epoch: i64,
        populate: impl FnOnce(&str) -> anyhow::Result<Option<T>>,
    ) -> Option<T> {
        self.get_entry_from(id, min_epoch, populate).map(|e| e.value)
    }

    pub fn get_entry(&self, id: &str) -> Option<Entry<T>> {
        self.get_entry_min_epoch(id, ALWAYS_PREFER_CACHED)
    }

    pub fn get_entry_min_epoch(&self, id: &str, min_epoch: i64) -> Option<Entry<T>> {
        self.get_entry_impl(id, min_epoch, None::<fn(&str) -> anyhow::Result<Option<T>>>)
    }

    pub fn get_entry_from(
        &self,
        id: &str,
        min_epoch: i64,
        populate: impl FnOnce(&str) -> anyhow::Result<Option<T>>,
    ) -> Option<Entry<T>> {
        self.get_entry_impl(id, min_epoch, Some(populate))
    }

    /// Port of `getEntry(String id, long minEpoch, Populator<T> populate)`;
    /// see the module docs for the locking behavior this preserves.
    fn get_entry_impl(
        &self,
        id: &str,
        min_epoch: i64,
        populate: Option<impl FnOnce(&str) -> anyhow::Result<Option<T>>>,
    ) -> Option<Entry<T>> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());

        let mut entry = entries.pop(id);
        match &entry {
            None => entry = self.load_entry(id, min_epoch),
            Some(e) if e.modified >= min_epoch => {
                let fresh = e.clone();
                entries.put(id.to_string(), fresh.clone());
                return Some(fresh);
            }
            // DEVIATION (deliberate). Java falls through here and, with no
            // populator, returns the too-old in-memory entry anyway --
            // `loadEntry` enforces `minEpoch` when reading from disk, but
            // the `entries` LRU hit does not. The visible effect is that
            // `getUser(id)` (which is `getUser(id, ALWAYS_FETCH)`) stops
            // fetching after the first call for as long as the entry stays
            // in the LRU, which is presumably why `getUserFresh` had to
            // exist at all.
            //
            // Honouring `min_epoch` on this path too is what the constant
            // is named for, and it is the safe direction: the alternative
            // silently serves a stale display name or ban state to every
            // call site that asked for current data. The caching benefit is
            // unaffected, because it comes from the call sites that pass a
            // real max-age (`within1day`, `ALWAYS_PREFER_CACHED`), and
            // those still hit above.
            Some(_) => entry = None,
        }

        if let Some(populate) = populate {
            match populate(id) {
                Ok(Some(value)) => {
                    match &mut entry {
                        None => entry = Some(Entry { id: id.to_string(), value, modified: 0 }),
                        Some(e) => e.value = value,
                    }
                    if let Some(e) = &mut entry {
                        self.save_entry(e);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::error!(
                        "Exception populating JSON cache of kind `{}`, id `{}`: {:?}",
                        self.kind,
                        id,
                        err
                    );
                }
            }
        }

        if let Some(e) = &entry {
            entries.put(id.to_string(), e.clone());
        }
        entry
    }

    /// Port of `put(String id, T value)`.
    pub fn put(&self, id: &str, value: T) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut entry = entries
            .pop(id)
            .unwrap_or_else(|| Entry { id: id.to_string(), value: value.clone(), modified: 0 });
        entry.value = value;
        self.save_entry(&mut entry);
        entries.put(id.to_string(), entry);
    }

    fn entry_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn load_entry(&self, id: &str, min_epoch: i64) -> Option<Entry<T>> {
        let file = self.entry_path(id);
        if !file.is_file() {
            return None;
        }
        let modified = match Self::last_modified_millis(&file) {
            Ok(m) => m,
            Err(_) => {
                tracing::error!("Exception reading cached JSON of kind `{}`, id `{}`", self.kind, id);
                return None;
            }
        };
        if modified < min_epoch {
            return None;
        }
        let contents = match std::fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => {
                tracing::error!("Exception reading cached JSON of kind `{}`, id `{}`", self.kind, id);
                return None;
            }
        };
        match serde_json::from_str(&contents) {
            Ok(value) => Some(Entry { id: id.to_string(), value, modified }),
            Err(_) => {
                tracing::error!("Exception reading cached JSON of kind `{}`, id `{}`", self.kind, id);
                None
            }
        }
    }

    fn save_entry(&self, entry: &mut Entry<T>) {
        let _ = std::fs::create_dir_all(&self.dir);
        let file = self.entry_path(&entry.id);
        let json = match serde_json::to_string_pretty(&entry.value) {
            Ok(j) => j,
            Err(_) => {
                tracing::error!("Exception writing cached JSON of kind `{}`, id `{}`", self.kind, entry.id);
                return;
            }
        };
        if std::fs::write(&file, json).is_err() {
            tracing::error!("Exception writing cached JSON of kind `{}`, id `{}`", self.kind, entry.id);
            return;
        }
        if let Ok(m) = Self::last_modified_millis(&file) {
            entry.modified = m;
        }
    }

    fn last_modified_millis(file: &Path) -> std::io::Result<i64> {
        let meta = std::fs::metadata(file)?;
        let modified = meta.modified()?;
        Ok(modified.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
    }
}

#[cfg(test)]
mod tests {
    /// Guards the deliberate deviation documented in `get_entry_impl`: an
    /// in-memory entry that is older than `min_epoch` must not satisfy the
    /// read, so `ALWAYS_FETCH` really does always miss.
    #[test]
    fn a_too_old_in_memory_entry_does_not_satisfy_the_read() {
        let root = std::env::temp_dir().join(format!(
            "celeste-json-cache-minepoch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let cache: CelesteJsonCache<String> = CelesteJsonCache::new(&root, "test");
        cache.put("a", "value".to_string());

        // Warm (any age accepted) hits; ALWAYS_FETCH must not.
        assert_eq!(cache.get_min_epoch("a", ALWAYS_PREFER_CACHED).as_deref(), Some("value"));
        assert_eq!(cache.get_min_epoch("a", ALWAYS_FETCH), None);
        // ... and asking again doesn't resurrect it from the LRU either.
        assert_eq!(cache.get_min_epoch("a", ALWAYS_FETCH), None);
        assert_eq!(cache.get_min_epoch("a", ALWAYS_PREFER_CACHED).as_deref(), Some("value"));

        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Widget {
        name: String,
        count: u32,
    }

    #[test]
    fn put_then_get_round_trips_through_disk() {
        let dir = tempdir();
        let cache: CelesteJsonCache<Widget> = CelesteJsonCache::new(dir.path(), "widgets");
        cache.put("w1", Widget { name: "foo".into(), count: 3 });
        assert_eq!(cache.get("w1"), Some(Widget { name: "foo".into(), count: 3 }));
        assert!(dir.path().join("widgets/w1.json").is_file());
    }

    #[test]
    fn get_from_populates_on_miss_and_caches_the_result() {
        let dir = tempdir();
        let cache: CelesteJsonCache<Widget> = CelesteJsonCache::new(dir.path(), "widgets");
        let calls = std::cell::Cell::new(0);
        let value = cache.get_from("w1", ALWAYS_PREFER_CACHED, |_id| {
            calls.set(calls.get() + 1);
            Ok(Some(Widget { name: "populated".into(), count: 1 }))
        });
        assert_eq!(value, Some(Widget { name: "populated".into(), count: 1 }));
        assert_eq!(calls.get(), 1);

        // Second call should hit the in-memory cache and not populate again.
        let value2 = cache.get_from("w1", ALWAYS_PREFER_CACHED, |_id| {
            calls.set(calls.get() + 1);
            Ok(Some(Widget { name: "should-not-be-used".into(), count: 99 }))
        });
        assert_eq!(value2, Some(Widget { name: "populated".into(), count: 1 }));
        assert_eq!(calls.get(), 1, "populate should not run again on a fresh cache hit");
    }

    #[test]
    fn always_fetch_forces_repopulation() {
        let dir = tempdir();
        let cache: CelesteJsonCache<Widget> = CelesteJsonCache::new(dir.path(), "widgets");
        cache.put("w1", Widget { name: "old".into(), count: 1 });
        let value = cache.get_from(ALWAYS_FETCH_ID, ALWAYS_FETCH, |_id| {
            Ok(Some(Widget { name: "new".into(), count: 2 }))
        });
        // Different id path exercised separately below; this call is against
        // a fresh id so it must populate.
        assert_eq!(value, Some(Widget { name: "new".into(), count: 2 }));

        let value2 = cache.get_from("w1", ALWAYS_FETCH, |_id| Ok(Some(Widget { name: "new2".into(), count: 3 })));
        assert_eq!(value2, Some(Widget { name: "new2".into(), count: 3 }));
    }
    const ALWAYS_FETCH_ID: &str = "fresh-id";

    #[test]
    fn known_404s_persist_across_instances() {
        let dir = tempdir();
        {
            let cache: CelesteJsonCache<Widget> = CelesteJsonCache::new(dir.path(), "widgets");
            assert!(cache.add_404("missing1"));
            assert!(!cache.add_404("missing1"), "adding the same id twice returns false");
        }
        let cache2: CelesteJsonCache<Widget> = CelesteJsonCache::new(dir.path(), "widgets");
        assert!(cache2.is_404(Some("missing1")));
        assert!(!cache2.is_404(Some("missing2")));
        assert!(cache2.is_404(None));
        assert!(cache2.remove_404("missing1"));
        assert!(!cache2.is_404(Some("missing1")));
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }
}
