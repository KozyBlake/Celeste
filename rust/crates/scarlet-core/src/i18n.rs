//! Port of `net.sybyline.scarlet.util.I18n`.
//!
//! Java looks the strings up in `messages.properties` (English, the base and
//! fallback) plus `messages_<lang>.properties` per language, bundled in the
//! jar, with an optional external `lang/` folder overlaid on top so a
//! translator can fix or add a language without a rebuild. Lookups fall back
//! language → English → the key itself, so a missing translation degrades to
//! English rather than blowing up.
//!
//! ## Where the strings come from
//!
//! The bundles are embedded from this crate's Rust-owned resources. They
//! were copied byte-for-byte from the Java reference bundles so removing
//! the Java tree later cannot break or change the Rust build.
//!
//! ## Deliberate simplifications, tracked not hidden
//!
//! - Java formats with `java.text.MessageFormat`. [`format`] implements the
//!   subset the message catalogue actually uses: `{0}`-style argument
//!   substitution and MessageFormat's quoting rules (`''` is a literal
//!   apostrophe, `'...'` quotes a literal section). No format *types*
//!   (`{0,number,percent}` and friends) are supported, because the
//!   catalogue contains none — verified by grepping every
//!   `messages*.properties` for `{n,` patterns. If one is ever added, it
//!   renders literally instead of silently formatting wrong.
//! - Java's `Locale` handling is reduced to a language string. Celeste's
//!   catalogue is language-only (`de`, `ja`, ...), never region-specific.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, RwLock};

/// Languages Celeste ships translations for (base English is always present).
pub const SHIPPED_LANGUAGES: [&str; 6] = ["en", "de", "es", "id", "ja", "ru"];

const BUNDLE_EN: &str = include_str!("../resources/i18n/messages.properties");
const BUNDLE_DE: &str = include_str!("../resources/i18n/messages_de.properties");
const BUNDLE_ES: &str = include_str!("../resources/i18n/messages_es.properties");
const BUNDLE_ID: &str = include_str!("../resources/i18n/messages_id.properties");
const BUNDLE_JA: &str = include_str!("../resources/i18n/messages_ja.properties");
const BUNDLE_RU: &str = include_str!("../resources/i18n/messages_ru.properties");

fn bundled_source(lang: &str) -> Option<&'static str> {
    match lang {
        "" | "en" => Some(BUNDLE_EN),
        "de" => Some(BUNDLE_DE),
        "es" => Some(BUNDLE_ES),
        "id" => Some(BUNDLE_ID),
        "ja" => Some(BUNDLE_JA),
        "ru" => Some(BUNDLE_RU),
        _ => None,
    }
}

fn cache() -> &'static Mutex<HashMap<String, &'static HashMap<String, String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static HashMap<String, String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_locale() -> &'static RwLock<String> {
    static LOCALE: OnceLock<RwLock<String>> = OnceLock::new();
    LOCALE.get_or_init(|| RwLock::new(default_locale()))
}

fn external_dir() -> &'static RwLock<Option<PathBuf>> {
    static DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    DIR.get_or_init(|| RwLock::new(None))
}

/// Port of `Locale.getDefault()` reduced to a language code: the first two
/// letters of `LC_ALL`/`LC_MESSAGES`/`LANG`, or English when unset (which is
/// also what a Windows or minimal container environment gives).
fn default_locale() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            let lang: String = value.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if !lang.is_empty() {
                return normalize_lang(&lang.to_ascii_lowercase());
            }
        }
    }
    "en".to_string()
}

/// Port of `I18n.normalizeLang`: Java's `Locale.getLanguage()` still returns
/// a few deprecated ISO-639 codes. The bundle files use the modern ones, so
/// map back or (for example) Indonesian users silently get English.
pub fn normalize_lang(lang: &str) -> String {
    match lang {
        "in" => "id".to_string(),
        "iw" => "he".to_string(),
        "ji" => "yi".to_string(),
        other => other.to_string(),
    }
}

/// Pins the desktop language (port of `I18n.setLocale`). Pass `"en"` or an
/// empty string for the base bundle.
pub fn set_locale(lang: &str) {
    *current_locale().write().unwrap() = normalize_lang(&lang.to_ascii_lowercase());
}

/// The language currently used by [`tr`].
pub fn locale() -> String {
    current_locale().read().unwrap().clone()
}

/// Port of `I18n.setExternalDir`: a folder of `messages_<lang>.properties`
/// overlaid over the bundled ones, so a translator can fix or add a language
/// without rebuilding. External keys win over bundled ones.
pub fn set_external_dir(dir: Option<PathBuf>) {
    *external_dir().write().unwrap() = dir;
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// Port of `I18n.loadLang`: bundled bundle first, external overlay second.
fn load_lang(lang: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(source) = bundled_source(lang) {
        parse_properties_into(&mut out, source);
    }
    if let Some(dir) = external_dir().read().unwrap().clone() {
        let file_name =
            if lang.is_empty() || lang == "en" { "messages.properties".to_string() } else { format!("messages_{lang}.properties") };
        let path = dir.join(file_name);
        if path.is_file() {
            match std::fs::read_to_string(&path) {
                Ok(source) => parse_properties_into(&mut out, &source),
                Err(err) => tracing::warn!("Failed loading external i18n {}: {}", path.display(), err),
            }
        }
    }
    out
}

fn bundle(lang: &str) -> &'static HashMap<String, String> {
    let key = normalize_lang(lang);
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = cache.get(&key) {
        return existing;
    }
    // Bundles live for the process, and there are at most a handful of them
    // (one per language actually used), so leaking each one is cheaper and
    // simpler than handing out `Arc`s for something that is never dropped.
    let leaked: &'static HashMap<String, String> = Box::leak(Box::new(load_lang(&key)));
    cache.insert(key, leaked);
    leaked
}

/// Minimal `java.util.Properties` reader for the `key=value` subset this
/// catalogue uses: `#`/`!` comments, blank lines, and `=` or `:` separators.
/// The real format's line continuations and unicode escapes are not used by
/// any `messages*.properties` in this repository (checked), so they are not
/// implemented rather than half-implemented.
fn parse_properties_into(out: &mut HashMap<String, String>, source: &str) {
    for line in source.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some(sep) = line.find(['=', ':']) else { continue };
        let key = line[..sep].trim().to_string();
        let value = line[sep + 1..].trim().to_string();
        if !key.is_empty() {
            out.insert(key, value);
        }
    }
}

/// Port of `I18n.tr(String, Object...)`: translate in the current desktop
/// language.
pub fn tr(key: &str, args: &[&str]) -> String {
    tr_in(&locale(), key, args)
}

/// Port of `I18n.tr(Locale, String, Object...)`: translate in a specific
/// language (a Discord user's, say), falling back to English and then to the
/// key itself.
pub fn tr_in(lang: &str, key: &str, args: &[&str]) -> String {
    let value = bundle(lang)
        .get(key)
        .or_else(|| bundle("en").get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string());
    if args.is_empty() {
        // Java only runs MessageFormat when args are present, so an
        // apostrophe in an argument-less string stays as typed.
        return value;
    }
    format(&value, args)
}

/// True when a translation for `key` exists in `lang` itself, not counting
/// the English fallback (port of `I18n.has`).
pub fn has(lang: &str, key: &str) -> bool {
    bundle(lang).contains_key(key)
}

/// The `MessageFormat` subset described in the module docs.
pub fn format(pattern: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                } else {
                    // Quoted literal section: everything up to the next
                    // single quote is copied verbatim.
                    for q in chars.by_ref() {
                        if q == '\'' {
                            break;
                        }
                        out.push(q);
                    }
                }
            }
            '{' => {
                let mut index = String::new();
                let mut closed = false;
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    index.push(next);
                }
                match (closed, index.parse::<usize>()) {
                    (true, Ok(i)) => out.push_str(args.get(i).copied().unwrap_or("")),
                    // Not an argument reference (or an unsupported format
                    // type): render it literally rather than guessing.
                    _ => {
                        out.push('{');
                        out.push_str(&index);
                        if closed {
                            out.push('}');
                        }
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_bundle_has_the_advisory_strings() {
        assert_eq!(bundle("en").get("adv.mixedCharName").map(String::as_str), Some("Mixed-character name"));
        assert!(bundle("en").len() > 300, "the full catalogue should load, got {}", bundle("en").len());
    }

    #[test]
    fn falls_back_language_then_english_then_key() {
        // Every shipped language resolves a key it does translate...
        assert!(!tr_in("de", "adv.mixedCharName", &[]).is_empty());
        // ...an untranslated language falls back to the English text...
        assert_eq!(tr_in("zz", "adv.mixedCharName", &[]), "Mixed-character name");
        // ...and an unknown key falls back to itself, never to an empty string.
        assert_eq!(tr_in("en", "no.such.key.exists", &[]), "no.such.key.exists");
    }

    #[test]
    fn formats_arguments_like_message_format() {
        assert_eq!(format("New account: {0} {1}", &["5", "days"]), "New account: 5 days");
        assert_eq!(format("{0} joined the lobby.", &["Vex"]), "Vex joined the lobby.");
        // MessageFormat quoting: '' is a literal apostrophe.
        assert_eq!(format("it''s {0}", &["fine"]), "it's fine");
        // A quoted section is literal, braces and all.
        assert_eq!(format("'{0}' stays", &["x"]), "{0} stays");
        // A missing argument renders empty rather than panicking.
        assert_eq!(format("{0}/{1}", &["only"]), "only/");
    }

    #[test]
    fn real_catalogue_strings_render() {
        assert_eq!(tr_in("en", "adv.newAccountDays", &["5", "days"]), "New account: 5 days");
        assert_eq!(tr_in("en", "adv.watchedGroupOne", &["Raiders"]), "Watched group: Raiders");
    }

    #[test]
    fn normalizes_deprecated_language_codes() {
        assert_eq!(normalize_lang("in"), "id");
        assert_eq!(normalize_lang("iw"), "he");
        assert_eq!(normalize_lang("de"), "de");
    }
}
