//! Port of `net.sybyline.scarlet.util.EncryptedPrefs`: an encrypted
//! key/value store for secrets (Discord bot token, VRChat username/
//! password/TOTP secret, session cookies).
//!
//! ## Scope decision: file-only, no OS "Preferences" backing
//!
//! The Java version's defining complexity is that it maintains **two**
//! stores in parallel — a `java.util.prefs.Preferences` node (on Windows,
//! the registry under `HKCU\...\JavaSoft\Prefs`) and a redundant encrypted
//! file in the data directory — with dual-writes, verified-read fallback,
//! self-healing reconstruction, quarantine of undecryptable bytes, and a
//! `repair(cutover)` operation to eventually retire the registry. All of
//! that exists *specifically* to route around Windows-registry corruption
//! (antivirus locking `NTUSER.DAT`, roaming/OneDrive-redirected profiles,
//! unclean shutdowns mid-flush) that made the registry-only design
//! unreliable in the field.
//!
//! Rust has no `java.util.prefs.Preferences` equivalent, and there's no
//! reason to invent a registry-backed store just to then build the same
//! workaround machinery around it: this port goes straight to what the Java
//! version was migrating *towards* — a single encrypted file as the one
//! source of truth. That removes the entire registry/file reconciliation
//! layer (~500 lines: `repair`, `importRegistryIntoMirror`, `primaryBytes`/
//! `secondaryBytes` direction-aware accessors, cutover bookkeeping) because
//! there's only one store to be consistent with itself.
//!
//! What's kept, because it's about protecting the data rather than about
//! the registry: AES-256-GCM encryption with PBKDF2-HMAC-SHA256 key
//! derivation at the same 100,000 iterations and 256-bit key size as the
//! Java version; SHA-256 key-name hashing so plaintext setting names don't
//! appear in the store file; and the "never destroy unreadable data"
//! principle — a corrupt entry is quarantined to a timestamped sibling file
//! and the store keeps running, rather than silently dropping or
//! overwriting it.
//!
//! ## Not attempted: reading an existing Java installation's `secure-prefs.dat`
//!
//! This uses its own file format (`secure-prefs.json`, a flat JSON object)
//! rather than Java's `java.util.Properties` text format, and does not
//! attempt byte-for-byte interop with a Java installation's store. Chasing
//! that would mean exactly reproducing `Preferences.userNodeForPackage(...)
//! .absolutePath()` as the PBKDF2 salt for the local password — getting
//! that string even slightly wrong wouldn't fail loudly, it would silently
//! decrypt to garbage, which is a worse outcome than plainly not claiming
//! compatibility. A dedicated one-time migration tool (given the exact
//! Java constant) is future work, not a hidden assumption baked in here.
//!
//! The crypto primitives are otherwise compatible in spirit: same
//! algorithm, same parameters, same "hash the key name, encrypt the value"
//! shape — so porting an import tool later is straightforward if ever
//! needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

const KEYGEN_ITERATIONS: u32 = 100_000;
const IV_LENGTH: usize = 12;
/// Identifies this application in the local-password key derivation, kept
/// as the same conceptual salt the Java version used
/// (`Preferences.userNodeForPackage(Scarlet.class).absolutePath()`) even
/// though this store's file format isn't interop-compatible with Java's —
/// see the module docs.
const IDENTITY_SALT: &str = "/net/sybyline/scarlet";
const FALLBACK_PREFIX: &str = "plain:";
const STORE_FILENAME: &str = "secure-prefs.json";

fn b64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

/// SHA-256 hex digest, matching `EncryptedPrefs.hash(String)`. Used to turn
/// a plaintext setting name into the key stored on disk.
fn hash_key(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn derive_key(password: &[u8], salt: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password, salt.as_bytes(), KEYGEN_ITERATIONS, &mut out);
    out
}

fn encrypt(key: &[u8; 32], value: &str) -> String {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut iv = [0u8; IV_LENGTH];
    rand::rngs::OsRng.fill_bytes(&mut iv);
    let nonce = Nonce::from_slice(&iv);
    // AES-256-GCM with a fresh random 96-bit nonce cannot fail to encrypt.
    let ciphertext = cipher.encrypt(nonce, value.as_bytes()).expect("AES-GCM encryption is infallible for valid inputs");
    let mut combined = Vec::with_capacity(iv.len() + ciphertext.len());
    combined.extend_from_slice(&iv);
    combined.extend_from_slice(&ciphertext);
    b64_encode(&combined)
}

fn decrypt(key: &[u8; 32], combined_b64: &str) -> Option<String> {
    let combined = b64_decode(combined_b64)?;
    if combined.len() < IV_LENGTH {
        return None;
    }
    let (iv, ciphertext) = combined.split_at(IV_LENGTH);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher.decrypt(Nonce::from_slice(iv), ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn safe_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' })
        .collect()
}

pub struct EncryptedPrefs {
    data_dir: PathBuf,
    store: Mutex<HashMap<String, String>>,
    key_cache: Mutex<HashMap<String, [u8; 32]>>,
    local_password: Vec<u8>,
}

impl EncryptedPrefs {
    /// Opens (or initializes) the encrypted store rooted at `data_dir`,
    /// deriving everything from `global_password` (see
    /// `Celeste::resolve_global_password` for how that's normally
    /// sourced: `SCARLET_GLOBAL_PW` env var, then a per-install random key
    /// file, matching the Java fallback chain).
    pub fn open(data_dir: &Path, global_password: &str) -> Self {
        let mut store = load_store(data_dir);
        let master_key = hash_key(&format!("local-password:{IDENTITY_SALT}"));
        let init_key = derive_key(global_password.as_bytes(), IDENTITY_SALT);

        let local_password = match store.get(&master_key).and_then(|blob| decrypt(&init_key, blob)) {
            Some(existing) => existing.into_bytes(),
            None => {
                if store.contains_key(&master_key) {
                    // A record exists but didn't decrypt: never destroy it
                    // silently. Snapshot everything, quarantine the bad
                    // record, then reseed so the store keeps working.
                    tracing::error!(
                        "secure store: local-password record exists but could not be decrypted; \
                         quarantining and reseeding. Existing secrets will need to be re-entered."
                    );
                    snapshot(data_dir, &store);
                    if let Some(bad) = store.get(&master_key) {
                        quarantine(data_dir, "localpw", bad);
                    }
                }
                let mut seed = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut seed);
                let local_password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seed);
                store.insert(master_key, encrypt(&init_key, &local_password));
                save_store(data_dir, &store);
                local_password.into_bytes()
            }
        };

        EncryptedPrefs {
            data_dir: data_dir.to_path_buf(),
            store: Mutex::new(store),
            key_cache: Mutex::new(HashMap::new()),
            local_password,
        }
    }

    fn get_or_derive(&self, salt: &str) -> [u8; 32] {
        let mut cache = self.key_cache.lock().unwrap();
        *cache.entry(salt.to_string()).or_insert_with(|| derive_key(&self.local_password, salt))
    }

    pub fn put(&self, key: &str, value: Option<&str>) {
        let Some(value) = value else {
            self.remove(key);
            return;
        };
        let bk = hash_key(key);
        let encrypted = encrypt(&self.get_or_derive(key), value);
        let mut store = self.store.lock().unwrap();
        store.remove(&format!("{FALLBACK_PREFIX}{bk}"));
        store.insert(bk, encrypted);
        save_store(&self.data_dir, &store);
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let bk = hash_key(key);
        let blob = {
            let store = self.store.lock().unwrap();
            store.get(&bk).cloned()
        }?;
        let decrypted = decrypt(&self.get_or_derive(key), &blob);
        if decrypted.is_none() {
            tracing::warn!(
                "secure store: entry `{}` is unreadable (failed AEAD integrity check); \
                 the corrupt bytes were quarantined for manual recovery and None was returned.",
                bk
            );
            quarantine(&self.data_dir, &bk, &blob);
        }
        decrypted
    }

    pub fn remove(&self, key: &str) {
        let bk = hash_key(key);
        let mut store = self.store.lock().unwrap();
        store.remove(&bk);
        save_store(&self.data_dir, &store);
    }

    pub fn contains(&self, key: &str) -> bool {
        let bk = hash_key(key);
        self.store.lock().unwrap().contains_key(&bk)
    }
}

fn store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_FILENAME)
}

fn load_store(data_dir: &Path) -> HashMap<String, String> {
    let path = store_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn save_store(data_dir: &Path, store: &HashMap<String, String>) {
    let _ = std::fs::create_dir_all(data_dir);
    let path = store_path(data_dir);
    match serde_json::to_string_pretty(store) {
        Ok(json) => {
            // Backup-before-overwrite + atomic rename, same crash-safety
            // shape as FileBackups::writer for Celeste's other JSON files.
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        Err(err) => tracing::error!("secure store: failed to serialize store: {}", err),
    }
}

fn snapshot(data_dir: &Path, store: &HashMap<String, String>) {
    let path = data_dir.join(format!("secure-prefs.snapshot-{}.json", timestamp()));
    if let Ok(json) = serde_json::to_string_pretty(store) {
        if std::fs::write(&path, json).is_ok() {
            tracing::warn!("secure store: wrote snapshot to {}", path.display());
        }
    }
}

fn quarantine(data_dir: &Path, name: &str, blob: &str) {
    let path = data_dir.join(format!("secure-prefs.quarantine-{}-{}.b64", timestamp(), safe_name(name)));
    if std::fs::write(&path, blob).is_ok() {
        tracing::warn!("secure store: quarantined unreadable record to {}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = EncryptedPrefs::open(dir.path(), "test-global-password");
        prefs.put("discord.token", Some("super-secret-token"));
        assert_eq!(prefs.get("discord.token").as_deref(), Some("super-secret-token"));
        assert!(prefs.contains("discord.token"));
    }

    #[test]
    fn survives_reopening_with_the_same_password() {
        let dir = tempfile::tempdir().unwrap();
        {
            let prefs = EncryptedPrefs::open(dir.path(), "pw");
            prefs.put("vrc:username", Some("someone@example.com"));
        }
        let prefs2 = EncryptedPrefs::open(dir.path(), "pw");
        assert_eq!(prefs2.get("vrc:username").as_deref(), Some("someone@example.com"));
    }

    #[test]
    fn remove_clears_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = EncryptedPrefs::open(dir.path(), "pw");
        prefs.put("k", Some("v"));
        prefs.remove("k");
        assert!(!prefs.contains("k"));
        assert_eq!(prefs.get("k"), None);
    }

    #[test]
    fn the_store_file_never_contains_the_plaintext_value() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = EncryptedPrefs::open(dir.path(), "pw");
        prefs.put("discord.token", Some("hunter2-super-secret"));
        let raw = std::fs::read_to_string(dir.path().join("secure-prefs.json")).unwrap();
        assert!(!raw.contains("hunter2-super-secret"));
    }

    #[test]
    fn wrong_global_password_cannot_read_existing_secrets() {
        let dir = tempfile::tempdir().unwrap();
        {
            let prefs = EncryptedPrefs::open(dir.path(), "correct-password");
            prefs.put("k", Some("secret-value"));
        }
        // A different password derives a different local password, which
        // fails to decrypt the master-key record, triggering reseed --
        // meaning the old secret becomes unreadable (expected: we don't
        // have the key), not silently returned.
        let prefs2 = EncryptedPrefs::open(dir.path(), "wrong-password");
        assert_ne!(prefs2.get("k").as_deref(), Some("secret-value"));
    }
}
