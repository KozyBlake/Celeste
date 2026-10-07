//! Port of `java.lang.String.hashCode()`'s algorithm
//! (`s[0]*31^(n-1) + s[1]*31^(n-2) + ... + s[n-1]`), needed where Celeste's
//! data format embeds it (`CelesteData.customEvent_new`/`customEvent_filter`
//! encode a `GroupAuditTypeEx` id's Java hash code into a UUID's low bits to
//! filter custom events by type without a separate index).
//!
//! Kept as an exact algorithmic port (not just "a deterministic hash") so a
//! data directory carried over from the Java build stays filterable by type
//! without re-writing every `ex/*.json` file's UUID.
//!
//! Operates on UTF-16 code units like Java does; for the ASCII-only ids
//! Celeste actually hashes with this (`GroupAuditTypeEx`'s `"groupex.*"`
//! ids), UTF-16 code units and Unicode scalar values coincide, so iterating
//! `char`s (each a valid ASCII code point, one UTF-16 unit) matches exactly.
//! A string with characters outside the Basic Multilingual Plane would need
//! actual UTF-16 surrogate-pair iteration to match bit-for-bit; not needed
//! here and not implemented.
pub fn java_string_hash_code(s: &str) -> i32 {
    let mut hash: i32 = 0;
    for c in s.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(c as i32);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_java_hash_codes() {
        // Values below are the actual output of "x".hashCode() in Java.
        assert_eq!(java_string_hash_code(""), 0);
        assert_eq!(java_string_hash_code("a"), 97);
        assert_eq!(java_string_hash_code("hello"), 99162322);
    }
}
