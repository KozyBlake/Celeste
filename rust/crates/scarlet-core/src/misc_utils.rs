//! Port of the string helpers in `net.sybyline.scarlet.util.MiscUtils`.

/// Port of `MiscUtils.maybeEllipsis(int, String)`:
/// ```java
/// return string == null || string.length() <= maxLength
///     ? string
///     : (string.substring(0, maxLength - 1) + '…');
/// ```
///
/// Java measures and slices in UTF-16 code units; Rust cannot slice mid-`char`,
/// so this counts `char`s and truncates on a `char` boundary. For the ASCII
/// reasons and display names these limits guard (embed fields, button custom
/// ids) the two agree; for astral-plane text Rust keeps slightly more content
/// than Java would, which is the safe direction for a length *limit* only
/// because every call site's real budget is counted the same way by Discord.
pub fn maybe_ellipsis(max_length: usize, value: &str) -> String {
    if max_length == 0 {
        return String::new();
    }
    if value.chars().count() <= max_length {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_length - 1).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_than_the_limit_is_returned_unchanged() {
        assert_eq!(maybe_ellipsis(10, "abc"), "abc");
        assert_eq!(maybe_ellipsis(3, "abc"), "abc");
    }

    #[test]
    fn longer_than_the_limit_keeps_max_minus_one_chars_plus_the_ellipsis() {
        assert_eq!(maybe_ellipsis(3, "abcd"), "ab\u{2026}");
        assert_eq!(maybe_ellipsis(1, "abcd"), "\u{2026}");
        // The result is exactly `max_length` characters long, like Java's.
        assert_eq!(maybe_ellipsis(5, "abcdefgh").chars().count(), 5);
    }

    #[test]
    fn truncates_on_a_char_boundary() {
        assert_eq!(maybe_ellipsis(3, "héllo"), "hé\u{2026}");
    }
}
