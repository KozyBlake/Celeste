//! Port of `net.sybyline.scarlet.PronounValidator`.
//!
//! Decides whether a VRChat pronouns field looks like a genuine pronoun set
//! or has been repurposed for abuse, a link, or troll content. Two ideas, as
//! in Java:
//!
//! 1. **Recognise real pronouns.** If the field *contains* any token from a
//!    broad list (binary, singular-they, and the common neopronoun sets) it
//!    is legitimate even with extra words alongside — `she/her puppy`
//!    passes.
//! 2. **Always flag genuine abuse.** Self-harm/violence directives,
//!    deny-listed strings and links are checked *first*, so they trip even
//!    when a real pronoun is present (`she/kys`).
//!
//! Checking order (first match wins) is Java's exactly: empty → allow-list →
//! built-in abuse → deny-list → link → recognised pronoun → structural
//! fallback → not flagged.
//!
//! The only structural difference: Java reaches a global
//! `Scarlet.pronounLists` static, returning null before startup finishes.
//! Here the lists are passed in ([`flag_reason`] takes an `Option`), which is
//! the same behaviour without the global — `None` means "not loaded yet" and
//! skips both list checks, as Java's null does.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::pronoun_lists::PronounLists;

/// Max total length before a no-pronoun field is treated as a phrase/dump.
const MAX_TOTAL_LEN: usize = 80;
/// Max slash/space/comma-separated segments for a no-pronoun field.
const MAX_SEGMENTS: usize = 6;

/// Recognised pronoun tokens: a field containing any of these is accepted
/// unless it also contains abuse, a deny-listed string, or a link.
const PRONOUN_TOKENS: &[&str] = &[
    // she / he / they / it / one
    "she", "her", "hers", "herself",
    "he", "him", "his", "himself",
    "they", "them", "their", "theirs", "themself", "themselves",
    "it", "its", "itself",
    "one", "ones", "oneself",
    // ae
    "ae", "aer", "aers", "aerself",
    // ey (Elverson) + e (Spivak)
    "ey", "em", "eir", "eirs", "emself", "eyself",
    "e", "es", "em's",
    // fae
    "fae", "faer", "faers", "faeself",
    // xe
    "xe", "xem", "xyr", "xyrs", "xemself", "xir", "xirs", "xis", "xeself",
    // ze / zie + hir / zir
    "ze", "zie", "zir", "zirs", "zirself", "hir", "hirs", "hirself",
    "zem", "zes", "zeself", "zhe", "zher", "zhers",
    // co
    "co", "cos", "coself",
    // hu (humanist)
    "hu", "hum", "hus", "humself",
    // ne
    "ne", "nem", "nir", "nirs", "nemself", "nis",
    // per (person)
    "per", "pers", "perself",
    // thon
    "thon", "thons", "thonself",
    // ve / vi
    "ve", "ver", "vers", "verself", "vis",
    "vi", "vir", "virs", "vim", "vims",
    // ki (kinship)
    "ki", "kin", "kins",
    // meta / catch-all answers people actually write
    "any", "all", "ask", "none", "name", "names", "null", "nil",
    "mirror", "pronoun", "pronouns", "neopronouns",
];

/// Abuse substrings, matched against a leet-folded letters-only collapse of
/// the whole field so separators and basic character swaps don't hide them.
const ABUSE_SUBSTRINGS: &[&str] = & [
    "killyourself", "killyourselves", "killurself", "killyaself",
    "gokillyourself", "kysnow", "neckyourself", "neckurself",
    "hangyourself", "hangurself", "endyourself", "endurlife",
    "slityourwrist", "slittyourwrist", "drinkbleach", "godie", "godrown",
    "ropeyourself",
];

const ABUSE_TOKENS: &[&str] = &["kys", "kms", "kysw"];

fn pronoun_token_set() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| PRONOUN_TOKENS.iter().copied().collect())
}

/// True if the pronouns field should be flagged.
pub fn is_flagged(pronouns: Option<&str>, lists: Option<&PronounLists>) -> bool {
    flag_reason(pronouns, lists).is_some()
}

/// A short human-readable reason if the pronouns are flagged, or `None` if
/// they look legitimate. The raw value is deliberately not echoed back —
/// callers that want it (TTS, Discord) already have it.
pub fn flag_reason(pronouns: Option<&str>, lists: Option<&PronounLists>) -> Option<&'static str> {
    let pronouns = pronouns?;
    let trimmed = pronouns.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();

    // 1. Whole-field allow-list wins outright.
    if lists.is_some_and(|l| l.is_known_good(&lower)) {
        return None;
    }

    // 2. Abuse always flags — even if a real pronoun is also present.
    let folded = leet_fold(trimmed);
    if ABUSE_SUBSTRINGS.iter().any(|bad| folded.contains(bad)) {
        return Some("abusive or self-harm content");
    }
    let tokens = tokens(&lower);
    if tokens.iter().any(|t| ABUSE_TOKENS.contains(&t.as_str())) {
        return Some("abusive or self-harm content");
    }

    // 3. User deny-list (substring, so embedded entries are caught).
    if lists.is_some_and(|l| l.contains_known_bad(&lower)) {
        return Some("matches deny-list");
    }

    // 4. Links / invites don't belong in a pronouns field.
    if contains_link(trimmed) {
        return Some("contains a link or invite");
    }

    // 5. Contains a recognised pronoun → legitimate, extra words allowed.
    if tokens.iter().any(|t| pronoun_token_set().contains(t.as_str())) {
        return None;
    }

    // 6. No recognised pronoun: only flag clearly abnormal shapes.
    if trimmed.chars().count() > MAX_TOTAL_LEN {
        return Some("unusually long for a pronouns field");
    }
    let segments = trimmed.split(['/', '\\', ',', '&', '|']).filter(|s| !s.is_empty()).count();
    if segments > MAX_SEGMENTS {
        return Some("too many segments to be a pronoun set");
    }

    // 7. Otherwise leave it alone.
    None
}

/// Splits a field into lowercase letter-only word tokens (Java's
/// `TOKEN_SPLIT = [^\p{L}]+`).
fn tokens(lower: &str) -> Vec<String> {
    lower.split(|c: char| !c.is_alphabetic()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Port of Java's `LINK` regex, hand-written rather than pulled in as a
/// `regex` dependency for one pattern: scheme/`www.`/known invite hosts, or
/// a bare `something.tld` in the common abused TLDs.
fn contains_link(value: &str) -> bool {
    let lower = value.to_lowercase();
    const MARKERS: [&str; 6] = ["http://", "https://", "www.", "discord.gg", "discord.com/invite", "t.me/"];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    const TLDS: [&str; 7] = [".com", ".net", ".org", ".gg", ".io", ".xyz", ".me"];
    for tld in TLDS {
        let mut rest = lower.as_str();
        while let Some(at) = rest.find(tld) {
            let (before, after) = (&rest[..at], &rest[at + tld.len()..]);
            // Java's `\b[\w-]+\.(com|...)\b`: at least one word character
            // immediately before the dot, and a word boundary after.
            let has_label = before.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
            let boundary_after = after.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            if has_label && boundary_after {
                return true;
            }
            rest = &rest[at + tld.len()..];
        }
    }
    false
}

/// Collapses to lowercase ASCII letters only, mapping common leetspeak so
/// simple obfuscation (`k1ll y0urs3lf`) still matches the abuse substrings.
fn leet_fold(s: &str) -> String {
    s.chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| match c {
            '0' => 'o',
            '1' | '!' | '|' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            other => other,
        })
        .filter(|c| c.is_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_missing_fields_are_never_flagged() {
        assert_eq!(flag_reason(None, None), None);
        assert_eq!(flag_reason(Some("   "), None), None);
    }

    #[test]
    fn ordinary_pronoun_sets_pass_even_with_extra_words() {
        for ok in ["she/her", "they/them", "she/her puppy", "any pronouns", "ze/hir", "ask me"] {
            assert_eq!(flag_reason(Some(ok), None), None, "{ok} should not be flagged");
        }
    }

    #[test]
    fn abuse_flags_even_alongside_a_real_pronoun() {
        assert_eq!(flag_reason(Some("she/kys"), None), Some("abusive or self-harm content"));
        assert_eq!(flag_reason(Some("kill/yourself"), None), Some("abusive or self-harm content"));
        // Leet obfuscation still matches.
        assert_eq!(flag_reason(Some("k1ll y0urs3lf"), None), Some("abusive or self-harm content"));
    }

    #[test]
    fn links_and_invites_are_flagged() {
        assert_eq!(flag_reason(Some("discord.gg/abcdef"), None), Some("contains a link or invite"));
        assert_eq!(flag_reason(Some("https://example.org"), None), Some("contains a link or invite"));
        assert_eq!(flag_reason(Some("myshop.xyz"), None), Some("contains a link or invite"));
        // A pronoun set with a slash is not a domain.
        assert_eq!(flag_reason(Some("he/him"), None), None);
    }

    #[test]
    fn structural_fallback_only_applies_without_a_recognised_pronoun() {
        let long = "a".repeat(120);
        assert_eq!(flag_reason(Some(&long), None), Some("unusually long for a pronouns field"));
        assert_eq!(flag_reason(Some("aa/bb/cc/dd/ee/ff/gg"), None), Some("too many segments to be a pronoun set"));
        // The same shape with a real pronoun in it is accepted.
        assert_eq!(flag_reason(Some("she/her/hers/aa/bb/cc/dd"), None), None);
    }

    #[test]
    fn deny_and_allow_lists_are_consulted_in_javas_order() {
        let dir = tempfile::tempdir().unwrap();
        let lists = PronounLists::open(dir.path());
        assert_eq!(flag_reason(Some("who/cares"), Some(&lists)), Some("matches deny-list"));
        // Allow-list wins over everything except built-in abuse.
        assert_eq!(flag_reason(Some("she/her"), Some(&lists)), None);
    }
}
