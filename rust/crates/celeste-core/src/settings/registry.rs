//! Port of `ScarletSettings.RegistryStringValued<T>` and its subclasses
//! (`RegistryString`, `RegistryOffsetDateTime`, `RegistryLocalDateTime`,
//! `RegistryRectangle`, and the encrypted variants).
//!
//! ## Scope decision: no `java.util.prefs.Preferences`
//!
//! The Java version backs these with `Preferences.userNodeForPackage(...)`
//! (the Windows registry, or a `prefs.xml`/native store elsewhere), with a
//! `preferences` (namespaced) node falling back to a `globalPreferences`
//! node. In the actual Celeste codebase both are always constructed as the
//! same object (`createSecurePrefs(prefs, "")` is called with an empty
//! namespace everywhere; `globalPreferences`/`preferences` and
//! `globalEncrypted`/`encrypted` are literally the same references) — so
//! that fallback chain is dead weight in practice, not a real feature to
//! preserve. This port goes straight to a single flat store: a small JSON
//! file (`registry.json` in the data directory) for plain values, and
//! [`crate::secure_store::EncryptedPrefs`] for the encrypted ones — no
//! registry/Preferences equivalent needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::secure_store::EncryptedPrefs;

/// Flat, plain (non-encrypted) key/value store backing the non-secret
/// `Registry*` values (last-run timestamps, UI window bounds, ...).
pub struct RegistryStore {
    path: PathBuf,
    map: Mutex<HashMap<String, String>>,
}

impl RegistryStore {
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("registry.json");
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        RegistryStore { path, map: Mutex::new(map) }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }

    pub fn put(&self, key: &str, value: &str) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), value.to_string());
        self.flush();
    }

    pub fn remove(&self, key: &str) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(key);
        self.flush();
    }

    fn flush(&self) {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(json) = serde_json::to_string_pretty(&*map) {
            if let Err(err) = crate::file_backups::write(&self.path, &json) {
                tracing::warn!("Failed to flush registry store: {}", err);
            }
        }
    }
}

/// Where a `RegistryStringValued<T>` reads/writes its raw string form.
enum Backing {
    Plain(Arc<RegistryStore>),
    Encrypted(Arc<EncryptedPrefs>),
}

type IfNullFn<T> = Box<dyn Fn() -> T + Send + Sync>;
type ParseFn<T> = Box<dyn Fn(&str) -> Option<T> + Send + Sync>;
type StringifyFn<T> = Box<dyn Fn(&T) -> String + Send + Sync>;

/// Generic port of `RegistryStringValued<T>` (and, when constructed with an
/// encrypted backing, `RegistryStringValuedEncrypted<T>`).
pub struct RegistryStringValued<T> {
    name: String,
    backing: Backing,
    if_null: Option<IfNullFn<T>>,
    parse: ParseFn<T>,
    stringify: StringifyFn<T>,
    cached: Mutex<Option<T>>,
}

impl<T: Clone> RegistryStringValued<T> {
    pub fn plain(
        store: Arc<RegistryStore>,
        name: impl Into<String>,
        if_null: Option<Box<dyn Fn() -> T + Send + Sync>>,
        parse: impl Fn(&str) -> Option<T> + Send + Sync + 'static,
        stringify: impl Fn(&T) -> String + Send + Sync + 'static,
    ) -> Self {
        RegistryStringValued {
            name: name.into(),
            backing: Backing::Plain(store),
            if_null,
            parse: Box::new(parse),
            stringify: Box::new(stringify),
            cached: Mutex::new(None),
        }
    }

    pub fn encrypted(
        store: Arc<EncryptedPrefs>,
        name: impl Into<String>,
        if_null: Option<Box<dyn Fn() -> T + Send + Sync>>,
        parse: impl Fn(&str) -> Option<T> + Send + Sync + 'static,
        stringify: impl Fn(&T) -> String + Send + Sync + 'static,
    ) -> Self {
        RegistryStringValued {
            name: name.into(),
            backing: Backing::Encrypted(store),
            if_null,
            parse: Box::new(parse),
            stringify: Box::new(stringify),
            cached: Mutex::new(None),
        }
    }

    fn read(&self) -> Option<String> {
        match &self.backing {
            Backing::Plain(store) => store.get(&self.name),
            Backing::Encrypted(store) => store.get(&self.name),
        }
    }

    fn write(&self, value: &str) {
        match &self.backing {
            Backing::Plain(store) => store.put(&self.name, value),
            Backing::Encrypted(store) => store.put(&self.name, Some(value)),
        }
    }

    /// Port of `getOrNull`: returns the cached/stored value without ever
    /// supplying `ifNull`.
    pub fn get_or_null(&self) -> Option<T> {
        self.get(false)
    }

    /// Port of `getOrSupply`: falls back to (and persists) `ifNull` if
    /// nothing is stored yet.
    pub fn get_or_supply(&self) -> Option<T> {
        self.get(true)
    }

    fn get(&self, or_now: bool) -> Option<T> {
        {
            let cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
            if cached.is_some() {
                return cached.clone();
            }
        }
        let string = self.read();
        if let Some(string) = &string {
            if let Some(parsed) = (self.parse)(string) {
                *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = Some(parsed.clone());
                self.write(string);
                return Some(parsed);
            }
        }
        if or_now {
            if let Some(if_null) = &self.if_null {
                let value = if_null();
                *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = Some(value.clone());
                self.write(&(self.stringify)(&value));
                return Some(value);
            }
        }
        None
    }

    pub fn set(&self, value: Option<T>) {
        let Some(value) = value else { return };
        *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = Some(value.clone());
        self.write(&(self.stringify)(&value));
    }

    /// Removes this value from storage and clears the in-memory cache.
    pub fn clear(&self) {
        *self.cached.lock().unwrap_or_else(|e| e.into_inner()) = None;
        match &self.backing {
            Backing::Plain(store) => store.remove(&self.name),
            Backing::Encrypted(store) => store.remove(&self.name),
        }
    }
}

/// Port of `RegistryString`.
pub fn registry_string(store: Arc<RegistryStore>, name: impl Into<String>) -> RegistryStringValued<String> {
    RegistryStringValued::plain(store, name, None, |s| Some(s.to_string()), |s: &String| s.clone())
}

/// Port of `RegistryStringEncrypted`.
pub fn registry_string_encrypted(
    store: Arc<EncryptedPrefs>,
    name: impl Into<String>,
) -> RegistryStringValued<String> {
    RegistryStringValued::encrypted(store, name, None, |s| Some(s.to_string()), |s: &String| s.clone())
}

/// Port of `RegistryOffsetDateTime` (Java: `OffsetDateTime` formatted with
/// `DateTimeFormatter.ISO_OFFSET_DATE_TIME`; Rust: `chrono::DateTime<Utc>`
/// via RFC 3339, which is the same ISO-8601-with-offset format).
pub fn registry_offset_date_time(
    store: Arc<RegistryStore>,
    name: impl Into<String>,
) -> RegistryStringValued<chrono::DateTime<chrono::Utc>> {
    RegistryStringValued::plain(
        store,
        name,
        Some(Box::new(chrono::Utc::now)),
        |s| chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&chrono::Utc)),
        |dt: &chrono::DateTime<chrono::Utc>| dt.to_rfc3339(),
    )
}

/// Port of `RegistryLocalDateTime` (Java: `LocalDateTime`, no offset; Rust:
/// `chrono::NaiveDateTime`).
pub fn registry_local_date_time(
    store: Arc<RegistryStore>,
    name: impl Into<String>,
) -> RegistryStringValued<chrono::NaiveDateTime> {
    RegistryStringValued::plain(
        store,
        name,
        Some(Box::new(|| chrono::Local::now().naive_local())),
        |s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok(),
        |dt: &chrono::NaiveDateTime| dt.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
    )
}

/// Port of `RegistryJsonEncrypted<T>`: an encrypted value serialized as
/// JSON (used for e.g. the VRChat cookie-jar's alternate-account cookie
/// map).
pub fn registry_json_encrypted<T>(
    store: Arc<EncryptedPrefs>,
    name: impl Into<String>,
) -> RegistryStringValued<T>
where
    T: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    RegistryStringValued::encrypted(
        store,
        name,
        None,
        |s| serde_json::from_str(s).ok(),
        |v: &T| serde_json::to_string(v).unwrap_or_default(),
    )
}

/// Simple rectangle (x, y, width, height), matching `java.awt.Rectangle`'s
/// four `int` fields, ported without the AWT dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Port of `RegistryRectangle`.
pub fn registry_rectangle(store: Arc<RegistryStore>, name: impl Into<String>) -> RegistryStringValued<Rectangle> {
    RegistryStringValued::plain(
        store,
        name,
        None,
        |s| {
            let parts: Vec<&str> = s.split(',').collect();
            if parts.len() != 4 {
                return None;
            }
            Some(Rectangle {
                x: parts[0].parse().ok()?,
                y: parts[1].parse().ok()?,
                width: parts[2].parse().ok()?,
                height: parts[3].parse().ok()?,
            })
        },
        |r: &Rectangle| format!("{},{},{},{}", r.x, r.y, r.width, r.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_string_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RegistryStore::open(dir.path()));
        let reg = registry_string(store, "lastRunVersion");
        assert_eq!(reg.get_or_null(), None);
        reg.set(Some("0.4.20".to_string()));
        assert_eq!(reg.get_or_null(), Some("0.4.20".to_string()));
    }

    #[test]
    fn offset_date_time_round_trips_and_supplies_now() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RegistryStore::open(dir.path()));
        let reg = registry_offset_date_time(store, "lastRunTime");
        assert!(reg.get_or_null().is_none());
        let supplied = reg.get_or_supply().unwrap();
        assert!(supplied <= chrono::Utc::now());
        // Second call returns the cached value, not a new "now".
        assert_eq!(reg.get_or_supply(), Some(supplied));
    }

    #[test]
    fn rectangle_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(RegistryStore::open(dir.path()));
        let reg = registry_rectangle(store, "uiBounds");
        reg.set(Some(Rectangle { x: 10, y: 20, width: 800, height: 600 }));
        assert_eq!(reg.get_or_null(), Some(Rectangle { x: 10, y: 20, width: 800, height: 600 }));
    }

    #[test]
    fn survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Arc::new(RegistryStore::open(dir.path()));
            let reg = registry_string(store, "lastRunVersion");
            reg.set(Some("0.4.20".to_string()));
        }
        let store2 = Arc::new(RegistryStore::open(dir.path()));
        let reg2 = registry_string(store2, "lastRunVersion");
        assert_eq!(reg2.get_or_null(), Some("0.4.20".to_string()));
    }

    #[test]
    fn encrypted_backing_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let enc = Arc::new(EncryptedPrefs::open(dir.path(), "test-pw"));
        let reg = registry_string_encrypted(enc, "discord.token");
        reg.set(Some("secret-token".to_string()));
        assert_eq!(reg.get_or_null(), Some("secret-token".to_string()));
    }
}
