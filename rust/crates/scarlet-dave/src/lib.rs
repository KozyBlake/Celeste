//! Rust port of the vendored `libdave-maven` (`moe.kyokobot.libdave`), the
//! DAVE (Discord Audio/Video End-to-end Encryption, MLS-based) library.
//!
//! **Status: data types only.** The MLS crypto engine
//! (`libdave-jvm-impl`/`libdave-jvm-adapter-jda`, backed by libdave's native
//! C++ implementation via JNI) and the native Opus/video codec bindings are
//! *not* ported — that's a substantial cryptography project of its own
//! (a full MLS 1.0 implementation plus the Discord-specific ratchet/roster
//! protocol) and shouldn't be rushed. This crate currently only carries the
//! plain data model from `libdave-maven/api/src/main/java/moe/kyokobot/libdave`
//! so callers (scarlet-discord's voice layer, eventually) have real types to
//! build against once the engine exists. See ../../PORTING.md.

use std::collections::HashMap;

/// Port of `moe.kyokobot.libdave.MediaType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaType {
    Audio,
    Video,
}

impl MediaType {
    pub const fn value(self) -> i32 {
        match self {
            MediaType::Audio => 0,
            MediaType::Video => 1,
        }
    }

    pub fn from_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(MediaType::Audio),
            1 => Some(MediaType::Video),
            _ => None,
        }
    }
}

/// Port of `moe.kyokobot.libdave.Codec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    Unknown,
    Opus,
    Vp8,
    Vp9,
    H264,
    H265,
    Av1,
}

impl Codec {
    pub const fn value(self) -> i32 {
        match self {
            Codec::Unknown => 0,
            Codec::Opus => 1,
            Codec::Vp8 => 2,
            Codec::Vp9 => 3,
            Codec::H264 => 4,
            Codec::H265 => 5,
            Codec::Av1 => 6,
        }
    }

    pub fn from_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Codec::Unknown),
            1 => Some(Codec::Opus),
            2 => Some(Codec::Vp8),
            3 => Some(Codec::Vp9),
            4 => Some(Codec::H264),
            5 => Some(Codec::H265),
            6 => Some(Codec::Av1),
            _ => None,
        }
    }
}

/// Port of `moe.kyokobot.libdave.DecryptorResultCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum DecryptorResultCode {
    #[error("decryption succeeded")]
    Success,
    #[error("decryption failed")]
    DecryptionFailure,
    #[error("missing key ratchet for sender")]
    MissingKeyRatchet,
    #[error("invalid nonce")]
    InvalidNonce,
    #[error("missing or invalid cryptor")]
    MissingCryptor,
}

/// Port of `moe.kyokobot.libdave.EncryptorResultCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum EncryptorResultCode {
    #[error("encryption succeeded")]
    Success,
    #[error("encryption failed")]
    EncryptionFailure,
    #[error("missing key ratchet for stream")]
    MissingKeyRatchet,
    #[error("missing or invalid cryptor")]
    MissingCryptor,
    #[error("too many attempts to encrypt the frame")]
    TooManyAttempts,
}

/// Port of `moe.kyokobot.libdave.RosterMap`: user id -> key material.
/// The Java version is an immutable `Map` view backed by parallel native
/// arrays; a plain owned `HashMap` is the direct equivalent once there's no
/// native array to view into.
pub type RosterMap = HashMap<u64, Vec<u8>>;

/// Port of `moe.kyokobot.libdave.CommitResult`, the result of processing an
/// MLS commit via `Session::processCommit`.
#[derive(Debug, Clone)]
pub enum CommitResult {
    Failed,
    Ignored,
    Success(RosterMap),
}

impl CommitResult {
    pub fn is_failed(&self) -> bool {
        matches!(self, CommitResult::Failed)
    }

    pub fn is_ignored(&self) -> bool {
        matches!(self, CommitResult::Ignored)
    }

    /// Returns the roster map, or `None` for `Failed`/`Ignored` (the Java
    /// version throws `IllegalStateException` instead; a Rust caller should
    /// prefer matching on the enum, but this mirrors the accessor for
    /// straightforward translation from call sites that assumed success).
    pub fn roster_map(&self) -> Option<&RosterMap> {
        match self {
            CommitResult::Success(map) => Some(map),
            _ => None,
        }
    }
}

/// Port of `moe.kyokobot.libdave.KeyRatchet`. In Java this wraps a native
/// handle behind `AutoCloseable`; here `Drop` plays that role once a real
/// implementation exists (this trait has no implementors yet).
pub trait KeyRatchet {
    fn encryption_key(&self, key_generation: u32) -> Option<Vec<u8>>;
    fn delete_key(&mut self, key_generation: u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_round_trips() {
        for c in [
            Codec::Unknown,
            Codec::Opus,
            Codec::Vp8,
            Codec::Vp9,
            Codec::H264,
            Codec::H265,
            Codec::Av1,
        ] {
            assert_eq!(Codec::from_value(c.value()), Some(c));
        }
    }

    #[test]
    fn commit_result_accessors() {
        assert!(CommitResult::Failed.is_failed());
        assert!(CommitResult::Ignored.is_ignored());
        let mut map = RosterMap::new();
        map.insert(1, vec![1, 2, 3]);
        let success = CommitResult::Success(map.clone());
        assert!(!success.is_failed());
        assert_eq!(success.roster_map(), Some(&map));
    }
}
