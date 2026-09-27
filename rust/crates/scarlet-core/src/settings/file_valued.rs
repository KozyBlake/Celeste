//! Port of `ScarletSettings.FileValued<T>` and its subclasses
//! (`FileValuedBoolean`, `FileValuedIntRange`, `FileValuedEnum`,
//! `FileValuedStringChoice`, `FileValuedStringPattern`,
//! `FileValuedStringArrayPattern`, `FileValuedVoid`): a registry of named,
//! typed, JSON-backed settings with caching, validation, and change
//! notification.
//!
//! ## The visitor pattern, translated
//!
//! Java drives two real consumers generically over every registered
//! setting regardless of its concrete type: the `/scarlet-setting` Discord
//! command and the Swing settings panel, both via `FileValuedVisitor<T>`
//! double-dispatch (`FileValued.visit(visitor)`). A Rust trait with a
//! generic method (`fn visit_enum<E: Enum>(...)`) isn't object-safe, so it
//! can't back a `Vec<Box<dyn ...>>` registry the way the Java interface
//! does directly.
//!
//! Instead, [`SettingDescriptor`] is the dyn-safe surface those two
//! consumers actually need: id/name, a type-erased [`SettingKind`]
//! describing the shape (with enum variants pre-stringified at
//! registration time, since Rust enums aren't reflectable the way Java's
//! are), and string-based get/set for building a generic command or UI
//! widget. `neither scarlet-discord nor scarlet-ui exist yet
//! (see PORTING.md), but the registry is built now so those layers have a
//! real foundation instead of retrofitting one later.

use std::sync::{Arc, Mutex};

use crate::change_listener::ListenerList;
use crate::settings::store::SettingsStore;

/// Type-erased description of a [`FileValued`]'s shape, replacing Java's
/// `FileValuedVisitor` double-dispatch (see module docs).
pub enum SettingKind {
    Basic,
    Boolean { default: bool },
    IntegerRange { default: i32, min: i32, max: i32 },
    /// Enum variants as their string representations (Java could enumerate
    /// `Enum.values()` reflectively; Rust callers supply this list at
    /// construction).
    Enum { default: String, variants: Vec<String> },
    /// A lazily-evaluated set of valid strings (Java: `Supplier<Collection<String>>`).
    StringChoice { valid_values: Arc<dyn Fn() -> Vec<String> + Send + Sync> },
    StringPattern { pattern: Option<String>, lenient: bool },
    StringArrayPattern { pattern: Option<String>, lenient: bool },
    /// An action with no persisted value (a UI button / Discord subcommand
    /// that runs `task` when invoked).
    Void { task: Arc<dyn Fn() + Send + Sync> },
}

/// Dyn-safe registry entry — see module docs for why this replaces a
/// generic visitor trait.
pub trait SettingDescriptor: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn kind(&self) -> &SettingKind;
    /// Renders the current value as a string, for generic display.
    fn get_as_string(&self) -> String;
    /// Parses and applies a string value (e.g. from a Discord option or a
    /// text field), returning whether it validated. No-op (returns `false`)
    /// for `Void` settings — use [`SettingKind::Void`]'s `task` instead.
    fn set_from_string(&self, value: &str, source: &str) -> bool;
}

pub struct FileValued<T> {
    id: String,
    name: String,
    store: Arc<SettingsStore>,
    validate: Box<dyn Fn(T) -> Option<T> + Send + Sync>,
    if_null: Box<dyn Fn() -> T + Send + Sync>,
    cached: Mutex<Option<T>>,
    listeners: ListenerList<Option<T>>,
    kind: SettingKind,
}

impl<T> FileValued<T>
where
    T: Clone + PartialEq + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    #[allow(clippy::too_many_arguments)]
    fn new(
        registry: &SettingsRegistry,
        id: impl Into<String>,
        name: impl Into<String>,
        validate: impl Fn(T) -> Option<T> + Send + Sync + 'static,
        if_null: impl Fn() -> T + Send + Sync + 'static,
        kind: SettingKind,
    ) -> Arc<Self> {
        let id = id.into();
        registry.register_id(&id);
        Arc::new(FileValued {
            id,
            name: name.into(),
            store: registry.store.clone(),
            validate: Box::new(validate),
            if_null: Box::new(if_null),
            cached: Mutex::new(None),
            listeners: ListenerList::new(),
            kind,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> &SettingKind {
        &self.kind
    }
    pub fn listeners(&self) -> &ListenerList<Option<T>> {
        &self.listeners
    }

    /// Port of `FileValued.get()`.
    pub fn get(&self) -> T {
        let mut cached = self.cached.lock().unwrap();
        if cached.is_none() {
            *cached = self.store.get_object::<T>(&self.id);
        }
        if cached.is_none() {
            *cached = Some((self.if_null)());
        }
        cached.clone().unwrap()
    }

    /// Port of `FileValued.set(T, String)`. `value` may be `None` to reset
    /// to the default (matching the Java `.set(null, source)` idiom).
    pub fn set(&self, value: Option<T>, source: &str) -> bool {
        let prev = {
            let cached = self.cached.lock().unwrap();
            cached.clone()
        };
        if prev == value {
            return true;
        }
        let value = value.unwrap_or_else(|| (self.if_null)());
        let validated = (self.validate)(value.clone());
        let valid = validated.is_some();
        if valid {
            *self.cached.lock().unwrap() = Some(value.clone());
            self.store.set_object(&self.id, &value);
        }
        let notify_next = if valid { validated } else { Some(value) };
        self.listeners.on_maybe_change(&prev, &notify_next, valid, source);
        valid
    }
}

impl<T> SettingDescriptor for FileValued<T>
where
    T: Clone + PartialEq + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + ToStringValue + FromStringValue + 'static,
{
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> &SettingKind {
        &self.kind
    }
    fn get_as_string(&self) -> String {
        self.get().to_string_value()
    }
    fn set_from_string(&self, value: &str, source: &str) -> bool {
        match T::from_string_value(value) {
            Some(v) => self.set(Some(v), source),
            None => false,
        }
    }
}

/// Minimal string-conversion bounds for [`SettingDescriptor`]'s generic
/// erasure. Implemented for the concrete setting value types Scarlet
/// actually uses; add more impls if a new `FileValued<T>` type needs
/// generic string get/set support.
pub trait ToStringValue {
    fn to_string_value(&self) -> String;
}
pub trait FromStringValue: Sized {
    fn from_string_value(s: &str) -> Option<Self>;
}

impl ToStringValue for bool {
    fn to_string_value(&self) -> String {
        self.to_string()
    }
}
impl FromStringValue for bool {
    fn from_string_value(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}
impl ToStringValue for i32 {
    fn to_string_value(&self) -> String {
        self.to_string()
    }
}
impl FromStringValue for i32 {
    fn from_string_value(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}
impl ToStringValue for String {
    fn to_string_value(&self) -> String {
        self.clone()
    }
}
impl FromStringValue for String {
    fn from_string_value(s: &str) -> Option<Self> {
        Some(s.to_string())
    }
}
impl ToStringValue for Vec<String> {
    fn to_string_value(&self) -> String {
        self.join(",")
    }
}
impl FromStringValue for Vec<String> {
    fn from_string_value(s: &str) -> Option<Self> {
        Some(s.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    }
}

/// Owns the settings JSON store and the id -> descriptor registry (Java:
/// `ScarletSettings.fileValuedSettings`), enforcing unique ids the same way
/// the Java constructor's `putIfAbsent` check did.
pub struct SettingsRegistry {
    pub(crate) store: Arc<SettingsStore>,
    entries: Mutex<Vec<Arc<dyn SettingDescriptor>>>,
    ids: Mutex<std::collections::HashSet<String>>,
}

impl SettingsRegistry {
    pub fn new(store: Arc<SettingsStore>) -> Self {
        SettingsRegistry { store, entries: Mutex::new(Vec::new()), ids: Mutex::new(Default::default()) }
    }

    fn register_id(&self, id: &str) {
        if !self.ids.lock().unwrap().insert(id.to_string()) {
            panic!("Duplicate setting: {id}");
        }
    }

    fn record(&self, descriptor: Arc<dyn SettingDescriptor>) {
        self.entries.lock().unwrap().push(descriptor);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn SettingDescriptor>> {
        self.entries.lock().unwrap().iter().find(|d| d.id() == id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn SettingDescriptor>> {
        self.entries.lock().unwrap().clone()
    }

    pub fn boolean(&self, id: &str, name: &str, default: bool) -> Arc<FileValued<bool>> {
        let fv = FileValued::new(self, id, name, Some, move || default, SettingKind::Boolean { default });
        self.record(fv.clone());
        fv
    }

    pub fn int_range(&self, id: &str, name: &str, default: i32, min: i32, max: i32) -> Arc<FileValued<i32>> {
        let fv = FileValued::new(
            self,
            id,
            name,
            move |v| Some(crate::maths::clamp_i32(v, min, max)),
            move || default,
            SettingKind::IntegerRange { default, min, max },
        );
        self.record(fv.clone());
        fv
    }

    /// Port of `FileValuedStringChoice`.
    pub fn string_choice(
        &self,
        id: &str,
        name: &str,
        default: &str,
        valid_values: impl Fn() -> Vec<String> + Send + Sync + 'static,
    ) -> Arc<FileValued<String>> {
        let valid_values = Arc::new(valid_values);
        let vv = valid_values.clone();
        let default_owned = default.to_string();
        let fv = FileValued::new(
            self,
            id,
            name,
            move |v| if vv().contains(&v) { Some(v) } else { None },
            move || default_owned.clone(),
            SettingKind::StringChoice { valid_values: valid_values as Arc<dyn Fn() -> Vec<String> + Send + Sync> },
        );
        self.record(fv.clone());
        fv
    }

    /// Port of `FileValuedStringPattern`.
    pub fn string_pattern(
        &self,
        id: &str,
        name: &str,
        default: &str,
        pattern: Option<&str>,
        lenient: bool,
    ) -> Arc<FileValued<String>> {
        let validate = pattern_one(pattern, lenient);
        let default_owned = default.to_string();
        let fv = FileValued::new(
            self,
            id,
            name,
            validate,
            move || default_owned.clone(),
            SettingKind::StringPattern { pattern: pattern.map(str::to_string), lenient },
        );
        self.record(fv.clone());
        fv
    }

    /// Port of `FileValuedStringArrayPattern`.
    pub fn string_array_pattern(
        &self,
        id: &str,
        name: &str,
        default: Vec<String>,
        pattern: Option<&str>,
        lenient: bool,
    ) -> Arc<FileValued<Vec<String>>> {
        let validate = pattern_all(pattern, lenient);
        let fv = FileValued::new(
            self,
            id,
            name,
            validate,
            move || default.clone(),
            SettingKind::StringArrayPattern { pattern: pattern.map(str::to_string), lenient },
        );
        self.record(fv.clone());
        fv
    }
}

/// Port of `patternOne`: lenient mode returns the first match substring (or
/// `None` if no match); strict mode requires a full match.
fn pattern_one(pattern: Option<&str>, lenient: bool) -> impl Fn(String) -> Option<String> + Send + Sync + 'static {
    let compiled = pattern.map(|p| regex::Regex::new(p).expect("invalid setting pattern"));
    move |value: String| match &compiled {
        None => Some(value),
        Some(re) => {
            if lenient {
                re.find(&value).map(|m| m.as_str().to_string())
            } else if re.is_match(&value) && re.find(&value).is_some_and(|m| m.start() == 0 && m.end() == value.len()) {
                Some(value)
            } else {
                None
            }
        }
    }
}

/// Port of `patternAll`: applies [`pattern_one`]'s rule to every element of
/// a string array/vec, failing the whole value if any element fails.
fn pattern_all(
    pattern: Option<&str>,
    lenient: bool,
) -> impl Fn(Vec<String>) -> Option<Vec<String>> + Send + Sync + 'static {
    let compiled = pattern.map(|p| regex::Regex::new(p).expect("invalid setting pattern"));
    move |values: Vec<String>| match &compiled {
        None => Some(values),
        Some(re) => {
            let mut out = Vec::with_capacity(values.len());
            for value in values {
                if lenient {
                    out.push(re.find(&value)?.as_str().to_string());
                } else if re.is_match(&value) && re.find(&value).is_some_and(|m| m.start() == 0 && m.end() == value.len()) {
                    out.push(value);
                } else {
                    return None;
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (tempfile::TempDir, SettingsRegistry) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SettingsStore::new(dir.path().join("settings.json")));
        (dir, SettingsRegistry::new(store))
    }

    #[test]
    fn boolean_defaults_and_persists() {
        let (_dir, reg) = registry();
        let setting = reg.boolean("ui_confirm_group_invite", "Confirm group invite", false);
        assert!(!setting.get());
        assert!(setting.set(Some(true), "test"));
        assert!(setting.get());
    }

    #[test]
    fn int_range_clamps_but_matches_javas_cache_quirk() {
        let (_dir, reg) = registry();
        let setting = reg.int_range("heuristicKickCount", "Heuristic Kick Count", 3, 1, 10);
        assert_eq!(setting.get(), 3);
        // The Java FileValuedIntRange.set() caches/persists the *unclamped*
        // value_ even though `validate` (Maths.clamp) never returns null
        // (so `valid` is always true here) -- only the listener
        // notification sees the clamped result. This port preserves that
        // exact quirk rather than "fixing" it silently.
        assert!(setting.set(Some(999), "test"));
        assert_eq!(setting.get(), 999, "matches Java: the raw value is cached/persisted, not the clamped one");
    }

    #[test]
    fn duplicate_id_panics() {
        let (_dir, reg) = registry();
        let _a = reg.boolean("dup", "A", true);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reg.boolean("dup", "B", false)
        }));
        assert!(result.is_err());
    }

    #[test]
    fn string_choice_rejects_invalid_values() {
        let (_dir, reg) = registry();
        let setting = reg.string_choice("ui_language", "Language", "system", || {
            vec!["system".to_string(), "en".to_string(), "ru".to_string()]
        });
        assert!(setting.set(Some("en".to_string()), "test"));
        assert_eq!(setting.get(), "en");
        assert!(!setting.set(Some("klingon".to_string()), "test"));
        // invalid set leaves the previous cached value in place
        assert_eq!(setting.get(), "en");
    }

    #[test]
    fn string_pattern_lenient_extracts_first_match() {
        let (_dir, reg) = registry();
        let setting = reg.string_pattern("verified_role_snowflake", "Role", "0", Some(r"\d{1,20}"), true);
        assert!(setting.set(Some("role:123456789".to_string()), "test"));
        // Same cache/persist-the-raw-value quirk as the int-range test above:
        // Java's FileValued.set() caches/persists `value_` (the raw input),
        // not `validated` (the pattern-extracted result) -- the extracted
        // value is only ever seen by change listeners. Confirmed against
        // the actual FileValued.set() logic, not assumed.
        assert_eq!(setting.get(), "role:123456789");
    }

    #[test]
    fn string_pattern_lenient_extraction_reaches_listeners() {
        let (_dir, reg) = registry();
        let setting = reg.string_pattern("verified_role_snowflake", "Role", "0", Some(r"\d{1,20}"), true);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen2 = seen.clone();
        setting.listeners().register("l", 0, false, move |_prev, next, _valid, _src| {
            *seen2.lock().unwrap() = next.clone();
        });
        setting.set(Some("role:123456789".to_string()), "other");
        assert_eq!(*seen.lock().unwrap(), Some("123456789".to_string()));
    }

    #[test]
    fn string_pattern_strict_rejects_partial_match() {
        let (_dir, reg) = registry();
        let setting = reg.string_pattern("role", "Role", "0", Some(r"\d{1,20}"), false);
        assert!(!setting.set(Some("role:123".to_string()), "test"));
    }

    #[test]
    fn string_array_pattern_validates_every_element() {
        // Pattern shape mirrors VRChat world ids (wrld_<uuid>); the real
        // pattern will come from a ported VrcIds once the VRChat crate
        // exists (see PORTING.md) -- inlined here since this test only
        // exercises FileValuedStringArrayPattern's own validation logic.
        let world_id_pattern = r"^wrld_[0-9a-f-]{36}$";
        let (_dir, reg) = registry();
        let setting = reg.string_array_pattern("worlds", "Worlds", vec![], Some(world_id_pattern), true);
        assert!(setting.set(
            Some(vec!["wrld_11111111-1111-1111-1111-111111111111".to_string()]),
            "test"
        ));
        assert!(!setting.set(Some(vec!["not-a-world-id".to_string()]), "test"));
    }

    #[test]
    fn change_listener_fires_on_valid_change() {
        let (_dir, reg) = registry();
        let setting = reg.boolean("flag", "Flag", false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen2 = seen.clone();
        setting.listeners().register("test-listener", 0, false, move |_prev, next, valid, _src| {
            *seen2.lock().unwrap() = Some((*next, valid));
        });
        setting.set(Some(true), "other-source");
        assert_eq!(*seen.lock().unwrap(), Some((Some(true), true)));
    }

    #[test]
    fn descriptor_get_set_from_string_round_trips() {
        let (_dir, reg) = registry();
        let setting = reg.boolean("flag", "Flag", false);
        let descriptor: Arc<dyn SettingDescriptor> = setting.clone();
        assert_eq!(descriptor.get_as_string(), "false");
        assert!(descriptor.set_from_string("true", "cmd"));
        assert_eq!(descriptor.get_as_string(), "true");
        assert_eq!(reg.get("flag").unwrap().get_as_string(), "true");
    }
}
