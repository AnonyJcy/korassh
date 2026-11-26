//! Local secret encryption vault (AES-256-GCM + Argon2id).
//!
//! Eliminates plaintext passwords from `hosts.toml` and `sync.toml` across all
//! platforms (Windows, macOS, Linux, Termux) without native C dependencies:
//!
//! - **Auto mode (default)**: a random 256-bit data encryption key (DEK) is stored
//!   in `<config_dir>/vault.key` (`0600` on Unix). Secrets are transparently
//!   encrypted before hitting disk (`enc:v1:...`) and decrypted in memory on load.
//!   Legacy plaintext passwords are automatically migrated on load/save.
//! - **Master Password mode (optional)**: the DEK is wrapped with a key derived from
//!   the user's master password via Argon2id and stored in `<config_dir>/vault.json`,
//!   while `vault.key` is deleted. On launch the vault starts locked until unlocked
//!   with the master password.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand::Rng;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::utils::platform;

/// Prefix marking an encrypted secret stored on disk.
pub const ENC_PREFIX: &str = "enc:v1:";

const VAULT_KEY_FILE: &str = "vault.key";
const VAULT_META_FILE: &str = "vault.json";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;

/// How the local vault protects its data encryption key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultMode {
    /// Automatic local key file — zero friction on launch, no plaintext in configs.
    Auto,
    /// Master password wraps the key via Argon2id — requires unlocking after launch.
    MasterPassword,
}

/// Snapshot of the vault state exposed to frontends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub mode: VaultMode,
    pub unlocked: bool,
}

/// Serialized wrapper for Master Password mode (`vault.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MasterVaultFile {
    version: u8,
    kdf: String,
    salt: String,
    nonce: String,
    wrapped_key: String,
}

/// Portable passphrase-encrypted envelope shared with backup & sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub version: u8,
    pub kdf: String,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Default)]
struct VaultRuntime {
    /// Active 32-byte DEK in memory (zeroed on drop / lock).
    dek: Option<Zeroizing<[u8; KEY_LEN]>>,
    /// Config dir the cached DEK belongs to (guards tests switching dirs).
    dir: Option<PathBuf>,
}

static RUNTIME: OnceLock<RwLock<VaultRuntime>> = OnceLock::new();

fn runtime() -> &'static RwLock<VaultRuntime> {
    RUNTIME.get_or_init(|| RwLock::new(VaultRuntime::default()))
}

/// Returns `true` if `value` carries the vault's encrypted prefix.
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENC_PREFIX)
}

fn argon2_instance() -> Argon2<'static> {
    // OWASP-aligned interactive Argon2id parameters: 19 MiB, 2 iterations, 1 lane.
    let params = Params::new(19 * 1024, 2, 1, Some(KEY_LEN)).expect("valid Argon2id params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn derive_kek(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let mut kek = Zeroizing::new([0u8; KEY_LEN]);
    argon2_instance()
        .hash_password_into(passphrase.as_bytes(), salt, kek.as_mut())
        .map_err(|e| anyhow!("Argon2id key derivation failed: {e}"))?;
    Ok(kek)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::rng().fill_bytes(&mut out);
    out
}

fn encrypt_bytes_with_key(
    key: &[u8; KEY_LEN],
    plaintext: &[u8],
) -> Result<([u8; NONCE_LEN], Vec<u8>)> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let nonce_bytes = random_bytes::<NONCE_LEN>();
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow!("AES-256-GCM encryption failed: {e}"))?;
    Ok((nonce_bytes, ciphertext))
}

fn decrypt_bytes_with_key(
    key: &[u8; KEY_LEN],
    nonce_bytes: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let nonce_arr: [u8; NONCE_LEN] = nonce_bytes
        .try_into()
        .map_err(|_| anyhow!("invalid AES-GCM nonce length"))?;
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key));
    let nonce = Nonce::from(nonce_arr);
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| anyhow!("decryption failed (wrong password or corrupted data)"))
}

/// Encrypts arbitrary bytes into a portable passphrase-protected envelope
/// (Argon2id + AES-256-GCM). Used by encrypted backup export and WebDAV/Git sync.
pub fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &str) -> Result<EncryptedEnvelope> {
    if passphrase.is_empty() {
        bail!("encryption passphrase cannot be empty");
    }
    let salt = random_bytes::<SALT_LEN>();
    let kek = derive_kek(passphrase, &salt)?;
    let (nonce, ciphertext) = encrypt_bytes_with_key(&kek, plaintext)?;
    Ok(EncryptedEnvelope {
        version: 1,
        kdf: "argon2id".to_string(),
        salt: B64.encode(salt),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    })
}

/// Decrypts a portable passphrase-protected envelope back to plaintext bytes.
pub fn decrypt_with_passphrase(envelope: &EncryptedEnvelope, passphrase: &str) -> Result<Vec<u8>> {
    if envelope.version != 1 {
        bail!("unsupported encrypted bundle version: {}", envelope.version);
    }
    if envelope.kdf != "argon2id" {
        bail!("unsupported KDF: {}", envelope.kdf);
    }
    let salt = B64
        .decode(&envelope.salt)
        .context("decode encrypted bundle salt")?;
    let nonce = B64
        .decode(&envelope.nonce)
        .context("decode encrypted bundle nonce")?;
    let ciphertext = B64
        .decode(&envelope.ciphertext)
        .context("decode encrypted bundle ciphertext")?;
    let kek = derive_kek(passphrase, &salt)?;
    decrypt_bytes_with_key(&kek, &nonce, &ciphertext)
}

fn write_restricted_file(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} to {}", tmp.display(), path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Directory-scoped vault handle (testable in isolated temp directories).
#[derive(Debug, Clone)]
pub struct VaultDir {
    dir: PathBuf,
}

impl VaultDir {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the vault rooted at the application's config directory.
    pub fn default_dir() -> Result<Self> {
        let dir = platform::app_config_dir().context("cannot determine app config directory")?;
        Ok(Self { dir })
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join(VAULT_KEY_FILE)
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join(VAULT_META_FILE)
    }

    /// Current mode determined by files on disk.
    pub fn mode(&self) -> VaultMode {
        if self.meta_path().exists() {
            VaultMode::MasterPassword
        } else {
            VaultMode::Auto
        }
    }

    /// Current mode + lock state.
    pub fn status(&self) -> VaultStatus {
        let mode = self.mode();
        let unlocked = match mode {
            VaultMode::Auto => true,
            VaultMode::MasterPassword => self.cached_dek().is_some(),
        };
        VaultStatus { mode, unlocked }
    }

    fn cached_dek(&self) -> Option<Zeroizing<[u8; KEY_LEN]>> {
        let guard = runtime().read().expect("vault lock poisoned");
        if guard.dir.as_ref() == Some(&self.dir) {
            guard.dek.clone()
        } else {
            None
        }
    }

    fn store_cached_dek(&self, dek: Zeroizing<[u8; KEY_LEN]>) {
        let mut guard = runtime().write().expect("vault lock poisoned");
        guard.dir = Some(self.dir.clone());
        guard.dek = Some(dek);
    }

    /// Clears the in-memory DEK (locks a master-password vault).
    pub fn lock(&self) {
        let mut guard = runtime().write().expect("vault lock poisoned");
        if guard.dir.as_ref() == Some(&self.dir) {
            guard.dek = None;
        }
    }

    /// Returns the active DEK if unlocked; in `Auto` mode lazily reads or creates `vault.key`.
    pub fn active_dek(&self) -> Result<Option<Zeroizing<[u8; KEY_LEN]>>> {
        if let Some(dek) = self.cached_dek() {
            return Ok(Some(dek));
        }
        if self.mode() == VaultMode::MasterPassword {
            return Ok(None);
        }
        let key_path = self.key_path();
        if key_path.exists() {
            let raw = fs::read_to_string(&key_path)
                .with_context(|| format!("read {}", key_path.display()))?;
            let decoded = B64.decode(raw.trim()).context("decode vault.key base64")?;
            if decoded.len() != KEY_LEN {
                bail!("vault.key has invalid length ({})", decoded.len());
            }
            let mut key = Zeroizing::new([0u8; KEY_LEN]);
            key.copy_from_slice(&decoded);
            self.store_cached_dek(key.clone());
            return Ok(Some(key));
        }
        // First time: generate a fresh 256-bit DEK and persist with 0600 perms.
        let key = Zeroizing::new(random_bytes::<KEY_LEN>());
        let encoded = B64.encode(*key);
        write_restricted_file(&key_path, encoded.as_bytes())?;
        self.store_cached_dek(key.clone());
        Ok(Some(key))
    }

    /// Encrypts a plaintext string into `enc:v1:...`. Returns an already-encrypted
    /// string verbatim (idempotent, preserves secrets even while locked).
    pub fn encrypt_secret(&self, value: &str) -> Result<String> {
        if value.is_empty() || is_encrypted(value) {
            return Ok(value.to_string());
        }
        let dek = self
            .active_dek()?
            .ok_or_else(|| anyhow!("vault is locked — unlock with your master password first"))?;
        let (nonce, ciphertext) = encrypt_bytes_with_key(&dek, value.as_bytes())?;
        let mut combined = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        combined.extend_from_slice(&nonce);
        combined.extend_from_slice(&ciphertext);
        Ok(format!("{ENC_PREFIX}{}", B64.encode(combined)))
    }

    /// Decrypts `enc:v1:...` into plaintext, or passes legacy plaintext through.
    /// Returns `Ok(None)` when the secret is encrypted and the vault is currently locked.
    pub fn try_decrypt_secret(&self, value: &str) -> Result<Option<String>> {
        if !is_encrypted(value) {
            return Ok(Some(value.to_string()));
        }
        let Some(dek) = self.active_dek()? else {
            return Ok(None);
        };
        let payload = B64
            .decode(value.trim_start_matches(ENC_PREFIX))
            .context("decode encrypted secret")?;
        if payload.len() <= NONCE_LEN {
            bail!("corrupted encrypted secret payload");
        }
        let (nonce, ciphertext) = payload.split_at(NONCE_LEN);
        let plain = decrypt_bytes_with_key(&dek, nonce, ciphertext)?;
        let text = String::from_utf8(plain).context("decrypted secret is not valid UTF-8")?;
        Ok(Some(text))
    }

    /// Unlocks a `MasterPassword` vault into memory.
    pub fn unlock(&self, master_password: &str) -> Result<()> {
        if self.mode() == VaultMode::Auto {
            let _ = self.active_dek()?;
            return Ok(());
        }
        let raw = fs::read_to_string(self.meta_path()).context("read vault.json")?;
        let meta: MasterVaultFile = serde_json::from_str(&raw).context("parse vault.json")?;
        if meta.version != 1 || meta.kdf != "argon2id" {
            bail!("unsupported vault.json format");
        }
        let salt = B64.decode(&meta.salt).context("decode vault salt")?;
        let nonce = B64.decode(&meta.nonce).context("decode vault nonce")?;
        let wrapped = B64
            .decode(&meta.wrapped_key)
            .context("decode wrapped key")?;
        let kek = derive_kek(master_password, &salt)?;
        let raw_dek = decrypt_bytes_with_key(&kek, &nonce, &wrapped)
            .map_err(|_| anyhow!("incorrect master password"))?;
        if raw_dek.len() != KEY_LEN {
            bail!("unwrapped vault key has invalid length");
        }
        let mut dek = Zeroizing::new([0u8; KEY_LEN]);
        dek.copy_from_slice(&raw_dek);
        self.store_cached_dek(dek);
        Ok(())
    }

    /// Enables or changes the master password.
    ///
    /// - From `Auto` mode: `current_password` is ignored; existing DEK is wrapped and `vault.key` removed.
    /// - From `MasterPassword` mode: `current_password` is verified first before re-wrapping the DEK.
    pub fn set_master_password(
        &self,
        current_password: Option<&str>,
        new_password: &str,
    ) -> Result<()> {
        if new_password.len() < 4 {
            bail!("master password must be at least 4 characters");
        }
        let dek = match self.mode() {
            VaultMode::Auto => self
                .active_dek()?
                .ok_or_else(|| anyhow!("failed to initialize local vault key"))?,
            VaultMode::MasterPassword => {
                let current = current_password
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| anyhow!("current master password is required"))?;
                self.unlock(current)?;
                self.cached_dek()
                    .ok_or_else(|| anyhow!("vault key missing after unlock"))?
            }
        };

        let salt = random_bytes::<SALT_LEN>();
        let kek = derive_kek(new_password, &salt)?;
        let (nonce, wrapped) = encrypt_bytes_with_key(&kek, &dek[..])?;
        let meta = MasterVaultFile {
            version: 1,
            kdf: "argon2id".to_string(),
            salt: B64.encode(salt),
            nonce: B64.encode(nonce),
            wrapped_key: B64.encode(wrapped),
        };
        let json = serde_json::to_string_pretty(&meta).context("serialize vault.json")?;
        write_restricted_file(&self.meta_path(), json.as_bytes())?;
        // Remove the unprotected auto key file now that the DEK is wrapped.
        if self.key_path().exists() {
            let _ = fs::remove_file(self.key_path());
        }
        self.store_cached_dek(dek);
        Ok(())
    }

    /// Removes the master password after verifying it, returning to `Auto` mode.
    pub fn remove_master_password(&self, current_password: &str) -> Result<()> {
        if self.mode() == VaultMode::Auto {
            return Ok(());
        }
        self.unlock(current_password)?;
        let dek = self
            .cached_dek()
            .ok_or_else(|| anyhow!("vault key missing after unlock"))?;
        let encoded = B64.encode(*dek);
        write_restricted_file(&self.key_path(), encoded.as_bytes())?;
        if self.meta_path().exists() {
            fs::remove_file(self.meta_path()).context("remove vault.json")?;
        }
        self.store_cached_dek(dek);
        Ok(())
    }
}

/// Top-level convenience wrappers targeting the app's default config directory.
pub fn status() -> VaultStatus {
    VaultDir::default_dir()
        .map(|v| v.status())
        .unwrap_or(VaultStatus {
            mode: VaultMode::Auto,
            unlocked: true,
        })
}

pub fn unlock(master_password: &str) -> Result<()> {
    VaultDir::default_dir()?.unlock(master_password)
}

pub fn lock() {
    if let Ok(v) = VaultDir::default_dir() {
        v.lock();
    }
}

pub fn set_master_password(current_password: Option<&str>, new_password: &str) -> Result<()> {
    VaultDir::default_dir()?.set_master_password(current_password, new_password)
}

pub fn remove_master_password(current_password: &str) -> Result<()> {
    VaultDir::default_dir()?.remove_master_password(current_password)
}

pub fn encrypt_secret(value: &str) -> Result<String> {
    VaultDir::default_dir()?.encrypt_secret(value)
}

pub fn try_decrypt_secret(value: &str) -> Result<Option<String>> {
    VaultDir::default_dir()?.try_decrypt_secret(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn auto_mode_encrypts_and_roundtrips_without_plaintext() {
        let dir = TempDir::new().expect("tempdir");
        let vault = VaultDir::new(dir.path().to_path_buf());
        assert_eq!(vault.mode(), VaultMode::Auto);
        assert!(vault.status().unlocked);

        let enc = vault.encrypt_secret("s3cr3t-pass!").expect("encrypt");
        assert!(is_encrypted(&enc));
        assert!(!enc.contains("s3cr3t-pass!"));
        // Idempotent on already-encrypted input.
        assert_eq!(vault.encrypt_secret(&enc).expect("idempotent"), enc);

        // Reloading from disk (cleared cache) still decrypts seamlessly.
        vault.lock();
        assert_eq!(
            vault.try_decrypt_secret(&enc).expect("decrypt"),
            Some("s3cr3t-pass!".to_string())
        );
    }

    #[test]
    fn master_password_locks_unlocks_changes_and_removes_cleanly() {
        let dir = TempDir::new().expect("tempdir");
        let vault = VaultDir::new(dir.path().to_path_buf());

        let enc = vault.encrypt_secret("db-password").expect("encrypt");
        vault
            .set_master_password(None, "master-1234")
            .expect("enable master password");
        assert_eq!(vault.mode(), VaultMode::MasterPassword);
        assert!(!vault.key_path().exists(), "vault.key must be removed");
        assert!(vault.meta_path().exists());

        // Lock clears memory; decryption returns None rather than plaintext.
        vault.lock();
        assert!(!vault.status().unlocked);
        assert_eq!(vault.try_decrypt_secret(&enc).expect("locked"), None);

        // Wrong password fails cleanly.
        assert!(vault.unlock("wrong-pass").is_err());
        assert!(!vault.status().unlocked);

        // Right password unlocks and decrypts original secret.
        vault.unlock("master-1234").expect("unlock");
        assert!(vault.status().unlocked);
        assert_eq!(
            vault.try_decrypt_secret(&enc).expect("decrypt"),
            Some("db-password".to_string())
        );

        // Change master password.
        vault
            .set_master_password(Some("master-1234"), "new-master-5678")
            .expect("change master");
        vault.lock();
        assert!(vault.unlock("master-1234").is_err());
        vault.unlock("new-master-5678").expect("unlock new");
        assert_eq!(
            vault
                .try_decrypt_secret(&enc)
                .expect("decrypt after change"),
            Some("db-password".to_string())
        );

        // Remove master password returns to Auto mode and keeps existing ciphertexts valid.
        vault
            .remove_master_password("new-master-5678")
            .expect("remove master");
        assert_eq!(vault.mode(), VaultMode::Auto);
        vault.lock();
        assert_eq!(
            vault.try_decrypt_secret(&enc).expect("auto decrypt"),
            Some("db-password".to_string())
        );
    }

    #[test]
    fn passphrase_envelope_roundtrips_and_rejects_tampering() {
        let env = encrypt_with_passphrase(b"hello korassh", "sync-pass").expect("encrypt");
        let plain = decrypt_with_passphrase(&env, "sync-pass").expect("decrypt");
        assert_eq!(plain, b"hello korassh");
        assert!(decrypt_with_passphrase(&env, "wrong-pass").is_err());
    }
}
