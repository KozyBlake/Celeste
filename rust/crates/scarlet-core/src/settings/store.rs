//! Port of the JSON-file-backed half of `ScarletSettings`: `getJson`/
//! `saveJson`/`getString`/`setString`/`getObject`/`setObject`/`getElement`/
//! `setElementNoSave`.
//!
//! Corrupt-or-empty-file recovery mirrors the Java version exactly: after a
//! hard crash / power loss, `settings.json` can come back truncated or
//! literal-empty. Rather than silently starting from a blank config, the
//! newest parseable dated backup (written by [`crate::file_backups`]) is
//! recovered instead.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

pub struct SettingsStore {
    path: PathBuf,
    json: Mutex<Option<Map<String, Value>>>,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        SettingsStore { path, json: Mutex::new(None) }
    }

    /// Returns a clone of the whole settings object, loading (and
    /// recovering from backup if needed) on first access. Exposed for
    /// callers that need bulk access (e.g. migration tooling); prefer
    /// `get_string`/`get_object` for normal use.
    pub fn snapshot(&self) -> Map<String, Value> {
        self.with_json(|json| json.clone())
    }

    fn with_json<R>(&self, f: impl FnOnce(&mut Map<String, Value>) -> R) -> R {
        let mut guard = self.json.lock().unwrap();
        if guard.is_none() {
            *guard = Some(self.load());
        }
        f(guard.as_mut().unwrap())
    }

    fn load(&self) -> Map<String, Value> {
        if self.path.exists() {
            match std::fs::read_to_string(&self.path) {
                Ok(contents) => match serde_json::from_str::<Value>(&contents) {
                    Ok(Value::Object(map)) if !map.is_empty() => return map,
                    Ok(_) => {} // empty/null/non-object: fall through to backup recovery
                    Err(err) => {
                        tracing::error!("Exception loading settings: {}", err);
                    }
                },
                Err(err) => {
                    tracing::error!("Exception loading settings: {}", err);
                }
            }
            if let Some(recovered) = self.load_newest_good_backup() {
                return recovered;
            }
        }
        Map::new()
    }

    fn load_newest_good_backup(&self) -> Option<Map<String, Value>> {
        for backup in crate::file_backups::backups_newest_first(&self.path) {
            match std::fs::read_to_string(&backup) {
                Ok(contents) => match serde_json::from_str::<Value>(&contents) {
                    Ok(Value::Object(map)) if !map.is_empty() => {
                        tracing::warn!(
                            "settings.json was empty or corrupt (likely an unclean shutdown); \
                             recovered {} setting(s) from backup {}",
                            map.len(),
                            backup.display()
                        );
                        return Some(map);
                    }
                    _ => continue,
                },
                Err(_) => continue,
            }
        }
        None
    }

    pub fn save(&self) {
        let mut guard = self.json.lock().unwrap();
        let Some(json) = guard.as_mut() else { return };
        let text = match serde_json::to_string_pretty(json) {
            Ok(t) => t,
            Err(err) => {
                tracing::error!("Exception saving settings: {}", err);
                return;
            }
        };
        if let Err(err) = crate::file_backups::write(&self.path, &text) {
            tracing::error!("Exception saving settings: {}", err);
        }
    }

    /// Port of `getString`: falls back to the `SCARLET_SETTING_<KEY>`
    /// environment variable when the key is absent from the JSON store.
    /// The Java version falls back to the JVM system property
    /// `net.sybyline.scarlet.setting.<key>` — an environment variable is
    /// the closest Rust equivalent of "override a setting from the launch
    /// environment" (Rust has no per-process settable system-property
    /// store distinct from env vars).
    pub fn get_string(&self, key: &str) -> Option<String> {
        let from_json = self.with_json(|json| match json.get(key) {
            Some(Value::Null) | None => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => Some(other.to_string()),
        });
        from_json.or_else(|| std::env::var(env_override_name(key)).ok())
    }

    pub fn set_string(&self, key: &str, value: &str) {
        self.with_json(|json| {
            json.insert(key.to_string(), Value::String(value.to_string()));
        });
        self.save();
    }

    pub fn get_object<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let element = self.with_json(|json| json.get(key).cloned());
        element.and_then(|element| match serde_json::from_value(element) {
            Ok(value) => Some(value),
            Err(err) => {
                tracing::error!("Exception deserializing setting `{}`: {}", key, err);
                None
            }
        })
    }

    pub fn set_object<T: Serialize>(&self, key: &str, value: &T) {
        let element = match serde_json::to_value(value) {
            Ok(e) => e,
            Err(err) => {
                tracing::error!("Exception serializing setting `{}`: {}", key, err);
                return;
            }
        };
        self.with_json(|json| {
            json.insert(key.to_string(), element);
        });
        self.save();
    }

    pub fn set_element_no_save(&self, key: &str, element: Value) {
        self.with_json(|json| {
            json.insert(key.to_string(), element);
        });
    }

    pub fn get_element(&self, key: &str) -> Option<Value> {
        self.with_json(|json| json.get(key).cloned())
    }

    pub fn clear_cached(&self) {
        *self.json.lock().unwrap() = None;
    }
}

fn env_override_name(key: &str) -> String {
    format!("SCARLET_SETTING_{}", key.to_uppercase().replace(['.', '-'], "_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_string() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        store.set_string("greeting", "hello");
        assert_eq!(store.get_string("greeting"), Some("hello".to_string()));
    }

    #[test]
    fn round_trips_a_typed_object() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Point {
            x: i32,
            y: i32,
        }
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        store.set_object("origin", &Point { x: 1, y: 2 });
        assert_eq!(store.get_object::<Point>("origin"), Some(Point { x: 1, y: 2 }));
    }

    #[test]
    fn recovers_from_backup_when_the_primary_file_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        {
            let store = SettingsStore::new(path.clone());
            // The first save has nothing to back up yet (no prior file);
            // the second save is what actually produces a backup of "a=1".
            store.set_string("a", "1");
            store.set_string("b", "2");
        }
        // Simulate an unclean shutdown: truncate the primary file.
        std::fs::write(&path, "").unwrap();
        let store2 = SettingsStore::new(path);
        assert_eq!(store2.get_string("a"), Some("1".to_string()));
    }

    #[test]
    fn missing_key_falls_back_to_env_var() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        std::env::set_var("SCARLET_SETTING_SOME_KEY", "from-env");
        assert_eq!(store.get_string("some.key"), Some("from-env".to_string()));
        std::env::remove_var("SCARLET_SETTING_SOME_KEY");
    }
}
