//! Port of `net.sybyline.scarlet.util.Names`.
//!
//! Display-name analysis for moderation and TTS: does a name mix writing
//! systems (the classic impersonation trick — `Вlake` with a Cyrillic В
//! reads as `Blake` on a nameplate but is a different account), can it be
//! rendered readably in ASCII, and which scripts is it actually written in.
//!
//! ## Crate equivalents for the Java facilities
//!
//! | Java | Rust |
//! |---|---|
//! | `Character.UnicodeScript.of` | `unicode_script::Script::from` |
//! | `Character.getType` | `unicode_general_category::get_general_category` |
//! | `Normalizer.normalize(.., NFKD)` | `unicode_normalization::UnicodeNormalization::nfkd` |
//! | `Junidecode.unidecode` | `deunicode::deunicode` |
//!
//! `deunicode` emits `[?]` for codepoints it has no transliteration for,
//! exactly as junidecode does, so the same `\[\?\]`-stripping step applies.
//!
//! The two hand-written tables (stroke-Latin and visual confusables) are
//! transcribed verbatim from the Java source — they are the part a
//! "reasonable equivalent" would silently get wrong, so they are diffed
//! against it rather than rewritten from knowledge of Unicode.

use unicode_general_category::GeneralCategory;
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script, UnicodeScript};

/// Port of `Names.toAscii`: a readable ASCII rendering of `name`, or `None`
/// when nothing readable is left.
pub fn to_ascii(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let n = strip_control_or_format(name);
    let n = strip_decorative_noise(&n);
    let n = map_stroke_latin(&n);
    let n: String = n.nfkd().collect();
    let mut n = strip_combining_marks(&n);
    if n.chars().any(|c| !c.is_ascii()) {
        n = deunicode::deunicode(&n);
        n = n.replace("[?]", "");
        n = strip_combining_marks(&n);
        n = n.chars().filter(|c| c.is_ascii()).collect();
    }
    let n = collapse_whitespace(&n);
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// Port of `Names.toAscii(String, String)`.
pub fn to_ascii_or(name: &str, fallback: &str) -> String {
    to_ascii(name).unwrap_or_else(|| fallback.to_string())
}

/// Port of `Names.toVisualAscii`: like [`to_ascii`], but only for names that
/// already *look* Latin — confusable codepoints are replaced by the Latin
/// letter they impersonate, so `Аpple` (Cyrillic А) reads as `Apple`, which
/// is what the moderator sees on the nameplate. `None` when the name isn't
/// visually Latin.
pub fn to_visual_ascii(name: &str) -> Option<String> {
    if name.is_empty() || !has_latin_letter(name) {
        return None;
    }
    let n = strip_control_or_format(name);
    let n = strip_decorative_noise(&n);
    let n = map_stroke_latin(&n);
    let n = map_visual_latin_confusables(&n);
    let n: String = n.nfkd().collect();
    let n = strip_combining_marks(&n);
    if n.chars().any(|c| !c.is_ascii()) {
        return None;
    }
    let n = collapse_whitespace(&n);
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// Port of `Names.hasLatinLetter`.
pub fn has_latin_letter(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let n = strip_control_or_format(name);
    let n = map_stroke_latin(&n);
    let n: String = n.nfkd().collect();
    n.chars().filter(|c| is_letter_like(*c)).any(|c| c.script() == Script::Latin)
}

/// Port of `Names.hasMixedLetterScripts`: true when the name's letters come
/// from more than one writing system. This is the impersonation signal.
pub fn has_mixed_letter_scripts(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // `Script` is only `PartialEq`, so this is a small `Vec` rather than a
    // set — it never holds more than two entries before returning anyway.
    let mut seen: Vec<Script> = Vec::new();
    for c in name.chars() {
        if !is_letter_like(c) {
            continue;
        }
        let script = c.script();
        if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            continue;
        }
        if !seen.contains(&script) {
            seen.push(script);
        }
        if seen.len() > 1 {
            return true;
        }
    }
    false
}

/// Port of `Names.isEntirelyInScripts`: every letter belongs to one of
/// `scripts` (ignoring neutral characters), and there is at least one
/// letter. Lets TTS leave a name already written in the user's own script
/// native instead of romanising it.
pub fn is_entirely_in_scripts(name: &str, scripts: &[Script]) -> bool {
    if name.is_empty() || scripts.is_empty() {
        return false;
    }
    let mut saw_letter = false;
    for c in name.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        let script = c.script();
        if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            continue;
        }
        if !scripts.contains(&script) {
            return false;
        }
        saw_letter = true;
    }
    saw_letter
}

/// Port of `TtsService.sanitizeName`: the form of a display name that a
/// speech engine can actually pronounce.
///
/// A name written entirely in the active UI language's own script is left
/// as-is, so a Russian voice reads Cyrillic natively. Latin-language voices
/// take the romanise-everything path so a foreign handle stays
/// intelligible, preferring the *visual* reading (what the moderator sees
/// on the nameplate) over a transliteration of the impersonating script.
pub fn sanitize_name_for_speech(display_name: &str) -> String {
    let native = native_scripts_for_language(&crate::i18n::locale());
    if !native.contains(&Script::Latin) && is_entirely_in_scripts(display_name, &native) {
        return display_name.trim().to_string();
    }
    if let Some(visual) = to_visual_ascii(display_name) {
        return visual;
    }
    let Some(ascii) = to_ascii(display_name) else {
        return crate::i18n::tr("adv.unnamedUser", &[]);
    };
    if has_mixed_letter_scripts(display_name) {
        return ascii;
    }
    let scripts = describe_scripts(display_name);
    if scripts.is_empty() {
        ascii
    } else {
        format!("{scripts} {ascii}")
    }
}

/// Port of `Names.describeScripts`: a spoken label for the non-Latin
/// scripts in a name ("Cyrillic", "Cyrillic and Greek", "mixed scripts"),
/// translated through the same message catalogue Java uses.
pub fn describe_scripts(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let mut scripts: Vec<String> = Vec::new();
    for c in name.chars() {
        let category = unicode_general_category::get_general_category(c);
        if matches!(category, GeneralCategory::Format | GeneralCategory::Control | GeneralCategory::Surrogate) {
            continue;
        }
        let script = c.script();
        if matches!(script, Script::Latin | Script::Common | Script::Inherited | Script::Unknown) {
            continue;
        }
        let friendly = friendly_script_name(script);
        if !scripts.contains(&friendly) {
            scripts.push(friendly);
        }
    }
    match scripts.len() {
        0 => String::new(),
        1 => scripts.remove(0),
        2 => format!("{} {} {}", scripts[0], crate::i18n::tr("script.andWord", &[]), scripts[1]),
        _ => crate::i18n::tr("script.mixed", &[]),
    }
}

/// Translated name of a writing system, English fallback — port of
/// `Names.scriptLabel`.
fn script_label(english: &str) -> String {
    let key = format!("script.{english}");
    let locale = crate::i18n::locale();
    if crate::i18n::has(&locale, &key) {
        crate::i18n::tr(&key, &[])
    } else {
        english.to_string()
    }
}

fn friendly_script_name(script: Script) -> String {
    let english = match script {
        Script::Cyrillic => "Cyrillic",
        Script::Arabic => "Arabic",
        Script::Han => "CJK",
        Script::Hiragana | Script::Katakana => "Japanese",
        Script::Hangul => "Korean",
        Script::Thai => "Thai",
        Script::Hebrew => "Hebrew",
        Script::Devanagari => "Devanagari",
        Script::Greek => "Greek",
        Script::Georgian => "Georgian",
        Script::Armenian => "Armenian",
        Script::Myanmar => "Myanmar",
        Script::Khmer => "Khmer",
        Script::Tibetan => "Tibetan",
        Script::Ethiopic => "Ethiopic",
        Script::Sinhala => "Sinhala",
        Script::Tamil => "Tamil",
        Script::Telugu => "Telugu",
        Script::Kannada => "Kannada",
        Script::Malayalam => "Malayalam",
        Script::Bengali => "Bengali",
        Script::Gujarati => "Gujarati",
        Script::Gurmukhi => "Gurmukhi",
        Script::Lao => "Lao",
        Script::Mongolian => "Mongolian",
        Script::Cherokee => "Cherokee",
        _ => "non-Latin",
    };
    script_label(english)
}

/// The writing systems a locale's own speakers read natively — port of
/// `I18n.nativeScriptsForLocale`, kept here because it only exists to serve
/// [`crate::names`]-driven TTS decisions.
pub fn native_scripts_for_language(lang: &str) -> Vec<Script> {
    match crate::i18n::normalize_lang(lang).as_str() {
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "kk" | "ky" | "tg" | "mn" => vec![Script::Cyrillic],
        "el" => vec![Script::Greek],
        "he" | "yi" => vec![Script::Hebrew],
        "ar" | "fa" | "ur" => vec![Script::Arabic],
        "th" => vec![Script::Thai],
        "hi" | "mr" | "ne" => vec![Script::Devanagari],
        "zh" => vec![Script::Han],
        "ko" => vec![Script::Hangul, Script::Han],
        "ja" => vec![Script::Han, Script::Hiragana, Script::Katakana],
        _ => vec![Script::Latin],
    }
}

/// Java's `isLetterLike`: the letter categories plus `LETTER_NUMBER`.
fn is_letter_like(c: char) -> bool {
    matches!(
        unicode_general_category::get_general_category(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::LetterNumber
    )
}

/// `[\p{Cc}\p{Cf}]`
fn strip_control_or_format(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(unicode_general_category::get_general_category(*c), GeneralCategory::Control | GeneralCategory::Format))
        .collect()
}

/// `[\p{So}\p{Sk}\p{Cs}\p{Co}\p{Cn}]|[\p{Po}&&[^\x00-\x7F]]`
fn strip_decorative_noise(s: &str) -> String {
    s.chars()
        .filter(|c| {
            let category = unicode_general_category::get_general_category(*c);
            let decorative = matches!(
                category,
                GeneralCategory::OtherSymbol
                    | GeneralCategory::ModifierSymbol
                    | GeneralCategory::Surrogate
                    | GeneralCategory::PrivateUse
                    | GeneralCategory::Unassigned
            );
            let non_ascii_punctuation = matches!(category, GeneralCategory::OtherPunctuation) && !c.is_ascii();
            !(decorative || non_ascii_punctuation)
        })
        .collect()
}

/// `\p{M}+`
fn strip_combining_marks(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                unicode_general_category::get_general_category(*c),
                GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark | GeneralCategory::EnclosingMark
            )
        })
        .collect()
}

/// `\s+` → a single space, then trim.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Port of `Names.mapStrokeLatin`.
pub fn map_stroke_latin(s: &str) -> String {
    s.chars().map(|c| stroke_latin_for(c).unwrap_or(&*char_as_str(c)).to_string()).collect()
}

/// Port of `Names.mapVisualLatinConfusables`.
pub fn map_visual_latin_confusables(s: &str) -> String {
    s.chars().map(|c| visual_latin_for(c).unwrap_or(&*char_as_str(c)).to_string()).collect()
}

fn char_as_str(c: char) -> Box<str> {
    c.to_string().into_boxed_str()
}

/// Latin-Extended "letter with stroke/bar" glyphs that NFKD does not
/// decompose (the stroke is part of the character's identity, not a
/// combining mark) and that the transliterator has gaps for.
fn stroke_latin_for(c: char) -> Option<&'static str> {
    Some(match c {
        'Ø' => "O", 'ø' => "o",
        'Đ' => "D", 'đ' => "d",
        'Ħ' => "H", 'ħ' => "h",
        'Ł' => "L", 'ł' => "l",
        'Ŧ' => "T", 'ŧ' => "t",
        'ƀ' => "b",
        'Ƃ' => "B", 'ƃ' => "b",
        'Ⱥ' => "A",
        'Ȼ' => "C", 'ȼ' => "c",
        'Ƚ' => "L",
        'Ⱦ' => "T",
        'Ɇ' => "E", 'ɇ' => "e",
        'Ɉ' => "J", 'ɉ' => "j",
        'Ɋ' => "Q", 'ɋ' => "q",
        'Ɍ' => "R", 'ɍ' => "r",
        'Ɏ' => "Y", 'ɏ' => "y",
        'ɨ' => "i",
        'ʈ' => "t",
        'ᵻ' => "I", 'ᵾ' => "U",
        _ => return None,
    })
}

/// Non-Latin codepoints that visually impersonate ASCII Latin letters.
fn visual_latin_for(c: char) -> Option<&'static str> {
    Some(match c {
        // Cyrillic uppercase that visually impersonate Latin caps
        'А' => "A", 'В' => "B", 'Е' => "E",
        'З' => "3", 'І' => "I", 'Ј' => "J",
        'К' => "K", 'М' => "M",
        'Н' => "H", // looks like H, not N (famous gotcha)
        'О' => "O",
        'Р' => "P", // looks like P, not R (famous gotcha)
        'С' => "C", 'Т' => "T", 'У' => "Y",
        'Х' => "X", 'Ѕ' => "S", 'Ү' => "Y",
        'Ғ' => "F", 'Ԛ' => "Q", 'Ԝ' => "W",

        // Cyrillic lowercase
        'а' => "a", 'е' => "e", 'о' => "o",
        'р' => "p", 'с' => "c", 'у' => "y",
        'х' => "x", 'і' => "i", 'ј' => "j",
        'ѕ' => "s", 'ԛ' => "q", 'ԝ' => "w",
        'ӏ' => "l",

        // Greek uppercase
        'Α' => "A", 'Β' => "B", 'Ε' => "E",
        'Ζ' => "Z", 'Η' => "H", 'Ι' => "I",
        'Κ' => "K", 'Μ' => "M", 'Ν' => "N",
        'Ο' => "O", 'Ρ' => "P", 'Τ' => "T",
        'Υ' => "Y", 'Χ' => "X",

        // Greek lowercase that read clearly as Latin
        'ο' => "o", 'ν' => "v", 'α' => "a",
        'κ' => "k", 'ρ' => "p", 'ι' => "i",
        'τ' => "t",

        // Armenian
        'Օ' => "O", 'օ' => "o", 'հ' => "h",

        // Cherokee letters that look like Latin caps
        'Ꭰ' => "D", 'Ꭱ' => "R", 'Ꭲ' => "T",
        'Ꭺ' => "A", 'Ꭻ' => "J", 'Ꭼ' => "E",
        'Ꮃ' => "W", 'Ꮄ' => "L", 'Ꮋ' => "H",
        'Ꮍ' => "Y", 'Ꮤ' => "W",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The display name out of the repo owner's own VRChat log: Latin text
    /// with Cyrillic lookalikes substituted in. Exactly the case this module
    /// exists to catch, so it's pinned as a test rather than a synthetic
    /// example.
    const SPOOFED: &str = "ВlаkеВеllаdоnnа";

    #[test]
    fn detects_a_real_spoofed_name_from_a_live_log() {
        assert!(has_mixed_letter_scripts(SPOOFED), "Cyrillic-in-Latin name must be flagged");
        // ...and reads aloud as what the nameplate shows, not as a
        // transliteration of the Cyrillic.
        assert_eq!(to_visual_ascii(SPOOFED).as_deref(), Some("BlakeBelladonna"));
    }

    #[test]
    fn a_single_script_name_is_not_mixed() {
        assert!(!has_mixed_letter_scripts("BlakeBelladonna"));
        assert!(!has_mixed_letter_scripts("Блэйк"));
        assert!(!has_mixed_letter_scripts("さくら"), "one script plus no Latin is not mixed");
        assert!(!has_mixed_letter_scripts(""));
        // Digits, spaces and punctuation are neutral.
        assert!(!has_mixed_letter_scripts("Player 123 !!"));
    }

    #[test]
    fn to_ascii_romanises_and_strips_decoration() {
        assert_eq!(to_ascii("Блэйк").as_deref(), Some("Bleik"));
        assert_eq!(to_ascii("Łukasz").as_deref(), Some("Lukasz"));
        assert_eq!(to_ascii("café").as_deref(), Some("cafe"));
        // Nothing readable left.
        assert_eq!(to_ascii(""), None);
        assert_eq!(to_ascii_or("", "an unnamed user"), "an unnamed user");
    }

    #[test]
    fn visual_ascii_only_applies_to_names_that_look_latin() {
        // Wholly non-Latin: no visual reading, caller falls back to to_ascii.
        assert_eq!(to_visual_ascii("Блэйк"), None);
        assert_eq!(to_visual_ascii("さくら"), None);
        // Plain Latin passes through unchanged.
        assert_eq!(to_visual_ascii("Blake").as_deref(), Some("Blake"));
    }

    #[test]
    fn describe_scripts_names_the_non_latin_systems() {
        assert_eq!(describe_scripts("Blake"), "");
        assert_eq!(describe_scripts("Блэйк"), "Cyrillic");
        assert!(describe_scripts("Блэйкさくら").contains("Cyrillic"));
        // Three or more collapses to the "mixed" label.
        assert_eq!(describe_scripts("Блэйкさくらالعربية"), crate::i18n::tr("script.mixed", &[]));
    }

    #[test]
    fn entirely_in_scripts_needs_at_least_one_letter() {
        assert!(is_entirely_in_scripts("Блэйк", &[Script::Cyrillic]));
        assert!(!is_entirely_in_scripts("Блэйк", &[Script::Greek]));
        assert!(!is_entirely_in_scripts("123 !!", &[Script::Cyrillic]), "no letters at all");
        assert!(is_entirely_in_scripts("Блэйк 123", &[Script::Cyrillic]), "digits are neutral");
    }

    #[test]
    fn native_scripts_follow_the_language() {
        assert_eq!(native_scripts_for_language("ru"), vec![Script::Cyrillic]);
        assert_eq!(native_scripts_for_language("ja"), vec![Script::Han, Script::Hiragana, Script::Katakana]);
        assert_eq!(native_scripts_for_language("en"), vec![Script::Latin]);
        // Java's deprecated-code mapping applies here too.
        assert_eq!(native_scripts_for_language("in"), vec![Script::Latin]);
    }

    #[test]
    fn speech_sanitisation_prefers_the_visual_reading() {
        // The spoofed name is spoken as the nameplate reads, not as a
        // transliteration of its Cyrillic letters.
        assert_eq!(sanitize_name_for_speech(SPOOFED), "BlakeBelladonna");
        // A wholly non-Latin name is romanised with its script named, so a
        // moderator hears "Cyrillic Bleik" rather than a mangled attempt.
        assert_eq!(sanitize_name_for_speech("Блэйк"), "Cyrillic Bleik");
        // Nothing pronounceable at all falls back to the catalogue string.
        assert_eq!(sanitize_name_for_speech("\u{1F600}"), crate::i18n::tr("adv.unnamedUser", &[]));
    }

    #[test]
    fn stroke_latin_maps_characters_nfkd_leaves_alone() {
        assert_eq!(map_stroke_latin("Øystein"), "Oystein");
        assert_eq!(map_stroke_latin("Đorđe"), "Dorde");
        assert_eq!(map_stroke_latin("plain"), "plain");
    }
}
