//! Account metadata on disk; Microsoft credentials only in the OS keychain,
//! or in a local encrypted file when the keychain is unavailable and the user
//! explicitly approves that fallback.
use crate::error::{AppError, AppResult};
use crate::util;
use hmac::{Hmac, Mac};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

const ACCOUNTS_FILE: &str = "launcher_accounts.json";
/// Ciphertext for the opt-in local secret store (hex encoded).
const SECRETS_FILE: &str = "launcher_secrets.enc";
/// Random key protecting [`SECRETS_FILE`]; its presence records user consent.
const SECRETS_KEY_FILE: &str = "launcher_secrets.key";
/// `1`/`0` overrides the interactive fallback prompt (see README).
const ALLOW_FILE_STORE_ENV: &str = "MC_LAUNCHER_ALLOW_FILE_STORE";

/// Local key file layout: `[0..ENC_LEN]` encryption key, `[ENC_LEN..]` MAC key.
const KEY_LEN: usize = 64;
const ENC_LEN: usize = 32;
const MAC_LEN: usize = 32;
const NONCE_LEN: usize = 16;

type HmacSha256 = Hmac<Sha256>;

trait SecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

struct Keychain;
impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new("mc-launcher", key).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new("mc-launcher", key)
            .map_err(|e| e.to_string())?
            .set_password(value)
            .map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new("mc-launcher", key).map_err(|e| e.to_string())?;
        match entry.delete_password() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Whether the encrypted local fallback may be used.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Consent {
    /// Ask interactively the first time the OS keychain fails.
    Prompt,
    /// Approved via `MC_LAUNCHER_ALLOW_FILE_STORE`.
    Always,
    /// Refused via `MC_LAUNCHER_ALLOW_FILE_STORE`.
    Never,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Os,
    File,
}

/// OS keychain first; on failure it can switch to an encrypted local file once
/// the user consents. Secrets are never written in cleartext.
struct FallbackStore {
    os: Box<dyn SecretStore>,
    file: EncryptedFileStore,
    consent: Consent,
    mode: Cell<Option<Mode>>,
    declined: Cell<bool>,
}

impl FallbackStore {
    fn new(game_dir: &Path) -> Self {
        let consent = match std::env::var(ALLOW_FILE_STORE_ENV) {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "y" | "on" => Consent::Always,
                _ => Consent::Never,
            },
            Err(_) => Consent::Prompt,
        };
        Self::with_parts(
            Box::new(Keychain),
            EncryptedFileStore::new(game_dir),
            consent,
        )
    }

    fn with_parts(os: Box<dyn SecretStore>, file: EncryptedFileStore, consent: Consent) -> Self {
        Self {
            os,
            file,
            consent,
            mode: Cell::new(None),
            declined: Cell::new(false),
        }
    }

    fn resolved_mode(&self) -> Mode {
        if let Some(mode) = self.mode.get() {
            return mode;
        }
        // An already-approved local store wins, so credentials stay in one place.
        let mode = if self.file.approved() {
            Mode::File
        } else {
            Mode::Os
        };
        self.mode.set(Some(mode));
        mode
    }

    fn promote_to_file(&self) -> Result<(), String> {
        self.file.ensure_key()?;
        self.mode.set(Some(Mode::File));
        crate::warn_msg!(
            "Storing Microsoft credentials in the encrypted local file store ({SECRETS_FILE})."
        );
        Ok(())
    }

    fn allow_file(&self, platform_error: &str, prompt: bool) -> Result<(), String> {
        if self.consent == Consent::Never {
            return Err(format!(
                "the encrypted fallback store is disabled by {ALLOW_FILE_STORE_ENV}"
            ));
        }
        if self.file.approved() {
            return self.promote_to_file();
        }
        match self.consent {
            Consent::Always => self.promote_to_file(),
            Consent::Never => unreachable!(),
            Consent::Prompt => {
                if self.declined.get() || !prompt || !ask_consent(platform_error) {
                    self.declined.set(true);
                    return Err(format!(
                        "no OS keychain is available and the encrypted fallback store was not approved \
                         (run in a terminal to approve it, or set {ALLOW_FILE_STORE_ENV}=1)"
                    ));
                }
                self.promote_to_file()
            }
        }
    }

    fn run<T>(
        &self,
        prompt: bool,
        op: impl Fn(&dyn SecretStore) -> Result<T, String>,
    ) -> Result<T, String> {
        match self.resolved_mode() {
            Mode::File => op(&self.file),
            Mode::Os => match op(self.os.as_ref()) {
                Ok(value) => Ok(value),
                Err(platform_error) => match self.allow_file(&platform_error, prompt) {
                    Ok(()) => op(&self.file),
                    Err(reason) => Err(format!("{reason}; OS keychain error: {platform_error}")),
                },
            },
        }
    }
}

impl SecretStore for FallbackStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.run(true, |store| store.get(key))
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.run(true, |store| store.set(key, value))
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        // Deleting must never block on an interactive prompt.
        self.run(false, |store| store.delete(key))
    }
}

fn ask_consent(platform_error: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprintln!();
    eprintln!("  [WARN] OS keychain is unavailable: {platform_error}");
    eprintln!(
        "         Microsoft tokens can instead be kept in an encrypted file in the game directory."
    );
    eprintln!(
        "         It is protected by a random local key ({SECRETS_KEY_FILE}, mode 0600), which is"
    );
    eprintln!("         weaker than an OS keychain because the key sits next to the data.");
    eprint!("         Use the encrypted local store? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(_) => matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

/// Encrypt-then-MAC secret map persisted as a hex file next to the accounts
/// file. A random per-directory key is stored in a separate `0600` key file.
struct EncryptedFileStore {
    data_path: PathBuf,
    key_path: PathBuf,
    key: RefCell<Option<[u8; KEY_LEN]>>,
}

impl EncryptedFileStore {
    fn new(game_dir: &Path) -> Self {
        Self {
            data_path: game_dir.join(SECRETS_FILE),
            key_path: game_dir.join(SECRETS_KEY_FILE),
            key: RefCell::new(None),
        }
    }

    /// Consent is recorded by the existence of the key file.
    fn approved(&self) -> bool {
        self.key_path.exists()
    }

    fn ensure_key(&self) -> Result<[u8; KEY_LEN], String> {
        self.load_key()
    }

    fn load_key(&self) -> Result<[u8; KEY_LEN], String> {
        if let Some(key) = *self.key.borrow() {
            return Ok(key);
        }
        let key = if self.key_path.exists() {
            let text = fs::read_to_string(&self.key_path)
                .map_err(|e| format!("Cannot read {}: {e}", self.key_path.display()))?;
            let bytes = hex::decode(text.trim())
                .map_err(|e| format!("Corrupt key file {}: {e}", self.key_path.display()))?;
            <[u8; KEY_LEN]>::try_from(bytes.as_slice()).map_err(|_| {
                format!(
                    "Corrupt key file {}: expected {KEY_LEN} bytes",
                    self.key_path.display()
                )
            })?
        } else {
            let mut bytes = [0u8; KEY_LEN];
            getrandom::getrandom(&mut bytes)
                .map_err(|e| format!("Cannot generate a local encryption key: {e}"))?;
            util::atomic_write(&self.key_path, hex::encode(bytes).as_bytes())
                .map_err(|e| format!("Cannot write {}: {e}", self.key_path.display()))?;
            bytes
        };
        *self.key.borrow_mut() = Some(key);
        Ok(key)
    }

    fn read_map(&self) -> Result<BTreeMap<String, String>, String> {
        if !self.data_path.exists() {
            return Ok(BTreeMap::new());
        }
        let key = self.load_key()?;
        let text = fs::read_to_string(&self.data_path)
            .map_err(|e| format!("Cannot read {}: {e}", self.data_path.display()))?;
        let blob = hex::decode(text.trim())
            .map_err(|e| format!("Corrupt secrets file {}: {e}", self.data_path.display()))?;
        if blob.len() < NONCE_LEN + MAC_LEN {
            return Err(format!(
                "Corrupt secrets file {}: too short",
                self.data_path.display()
            ));
        }
        let (nonce, rest) = blob.split_at(NONCE_LEN);
        let (tag, ciphertext) = rest.split_at(MAC_LEN);
        if verify_tag(&key[MAC_LEN..], nonce, ciphertext, tag).is_err() {
            return Err(format!(
                "Cannot decrypt {}: wrong key or corrupted file",
                self.data_path.display()
            ));
        }
        let plaintext = xor_keystream(&key[..ENC_LEN], nonce, ciphertext);
        serde_json::from_slice(&plaintext)
            .map_err(|e| format!("Corrupt secrets file {}: {e}", self.data_path.display()))
    }

    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        let key = self.load_key()?;
        let plaintext = serde_json::to_vec(map).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|e| format!("Cannot generate a nonce: {e}"))?;
        let ciphertext = xor_keystream(&key[..ENC_LEN], &nonce, &plaintext);
        let tag = compute_tag(&key[MAC_LEN..], &nonce, &ciphertext);
        let mut blob = Vec::with_capacity(NONCE_LEN + MAC_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&tag);
        blob.extend_from_slice(&ciphertext);
        util::atomic_write(&self.data_path, hex::encode(blob).as_bytes())
            .map_err(|e| format!("Cannot write {}: {e}", self.data_path.display()))
    }
}

impl SecretStore for EncryptedFileStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.read_map()?.remove(key))
    }
    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        let mut map = self.read_map()?;
        map.insert(key.to_string(), value.to_string());
        self.write_map(&map)
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let mut map = self.read_map()?;
        map.remove(key);
        self.write_map(&map)
    }
}

/// HMAC-SHA256 in counter mode: XOR the plaintext with a PRF keystream.
fn xor_keystream(key: &[u8], nonce: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut block = [0u8; ENC_LEN];
    let mut offset = block.len();
    let mut counter: u64 = 0;
    for &byte in data {
        if offset == block.len() {
            let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
            mac.update(nonce);
            mac.update(&counter.to_be_bytes());
            block.copy_from_slice(&mac.finalize().into_bytes());
            counter += 1;
            offset = 0;
        }
        out.push(byte ^ block[offset]);
        offset += 1;
    }
    out
}

fn compute_tag(key: &[u8], nonce: &[u8], ciphertext: &[u8]) -> [u8; MAC_LEN] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(nonce);
    mac.update(ciphertext);
    let mut tag = [0u8; MAC_LEN];
    tag.copy_from_slice(&mac.finalize().into_bytes());
    tag
}

fn verify_tag(key: &[u8], nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Result<(), ()> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(nonce);
    mac.update(ciphertext);
    mac.verify_slice(tag).map_err(|_| ())
}

pub struct AccountManager {
    path: PathBuf,
    data: serde_json::Value,
    secret_key: String,
    secrets: Box<dyn SecretStore>,
}

impl AccountManager {
    pub fn new(game_dir: &Path) -> AppResult<Self> {
        let game_dir = game_dir
            .canonicalize()
            .or_else(|_| util::absolute_path(game_dir))?;
        Self::with_store(&game_dir, Box::new(FallbackStore::new(&game_dir)))
    }

    fn resolve_dir(game_dir: &Path) -> AppResult<PathBuf> {
        Ok(game_dir
            .canonicalize()
            .or_else(|_| util::absolute_path(game_dir))?)
    }

    fn accounts_path(game_dir: &Path) -> AppResult<PathBuf> {
        Ok(Self::resolve_dir(game_dir)?.join(ACCOUNTS_FILE))
    }

    fn secret_key_for(game_dir: &Path) -> AppResult<String> {
        let dir = Self::resolve_dir(game_dir)?;
        Ok(format!(
            "msa:{:x}",
            Sha1::digest(dir.to_string_lossy().as_bytes())
        ))
    }

    fn read_public_data(game_dir: &Path) -> AppResult<serde_json::Value> {
        let path = Self::accounts_path(game_dir)?;
        let data = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                AppError::Generic(format!(
                    "Invalid accounts file {} (left untouched): {e}",
                    path.display()
                ))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"accounts":{},"default":null})
            }
            Err(error) => return Err(error.into()),
        };
        if !data["accounts"].is_object()
            || !(data["default"].is_null() || data["default"].is_string())
        {
            return Err("Invalid accounts schema; refusing to overwrite it".into());
        }
        Ok(data)
    }

    fn save_public_data(game_dir: &Path, data: &serde_json::Value) -> AppResult<()> {
        let path = Self::accounts_path(game_dir)?;
        util::atomic_write(&path, &serde_json::to_vec_pretty(data)?)?;
        Ok(())
    }

    fn validate_offline_name(username: &str) -> AppResult<()> {
        if !(1..=16).contains(&username.len())
            || !username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(
                "Offline username must contain 1–16 ASCII letters, digits or underscores".into(),
            );
        }
        Ok(())
    }

    /// Public account metadata only (never tokens), for frontends that must not
    /// touch the OS keychain just to render an account list.
    pub fn public_metadata(game_dir: &Path) -> AppResult<serde_json::Value> {
        let mut data = Self::read_public_data(game_dir)?;
        // Defensive: a legacy file may still carry credential fields before
        // migration. Frontends only need names, UUIDs and expiry times.
        if let Some(accounts) = data["accounts"].as_object_mut() {
            for account in accounts.values_mut() {
                if let Some(object) = account.as_object_mut() {
                    object.remove("access_token");
                    object.remove("refresh_token");
                }
            }
        }
        Ok(data)
    }

    /// Add/replace the offline account without opening a credential store.
    pub fn set_offline_metadata(game_dir: &Path, username: &str) -> AppResult<()> {
        Self::validate_offline_name(username)?;
        let mut data = Self::read_public_data(game_dir)?;
        data["accounts"]["offline"] = serde_json::json!({"type":"offline","username":username,"uuid":util::offline_uuid(username)});
        data["default"] = "offline".into();
        Self::save_public_data(game_dir, &data)
    }

    /// Switch the default account; credentials and expiry are untouched.
    pub fn set_default_metadata(game_dir: &Path, key: &str) -> AppResult<()> {
        let mut data = Self::read_public_data(game_dir)?;
        if !data["accounts"].get(key).is_some_and(|v| v.is_object()) {
            return Err(format!("No saved account named '{key}'").into());
        }
        data["default"] = serde_json::Value::String(key.to_string());
        Self::save_public_data(game_dir, &data)
    }

    /// Remove local metadata and best-effort delete stored Microsoft secrets.
    /// Deletion never prompts, so this is safe to call from a full-screen UI.
    pub fn logout_metadata(game_dir: &Path) -> AppResult<()> {
        let dir = Self::resolve_dir(game_dir)?;
        let mut data = Self::read_public_data(&dir)?;
        if data["accounts"].get("msa").is_some() {
            let store = FallbackStore::new(&dir);
            let key = Self::secret_key_for(&dir)?;
            store.delete(&key).map_err(AppError::Generic)?;
            store
                .delete("msa_refresh_token")
                .map_err(AppError::Generic)?;
        }
        data = serde_json::json!({"accounts":{},"default":null});
        Self::save_public_data(&dir, &data)
    }

    fn with_store(game_dir: &Path, secrets: Box<dyn SecretStore>) -> AppResult<Self> {
        let game_dir = game_dir
            .canonicalize()
            .or_else(|_| util::absolute_path(game_dir))?;
        let path = game_dir.join(ACCOUNTS_FILE);
        let data = match fs::read(&path) {
            Ok(bytes) => {
                let data: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                    AppError::Generic(format!(
                        "Invalid accounts file {} (left untouched): {e}",
                        path.display()
                    ))
                })?;
                if !data["accounts"].is_object()
                    || !(data["default"].is_null() || data["default"].is_string())
                {
                    return Err("Invalid accounts schema; refusing to overwrite it".into());
                }
                data
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"accounts":{},"default":null})
            }
            Err(error) => return Err(error.into()),
        };
        let secret_key = format!(
            "msa:{:x}",
            Sha1::digest(game_dir.to_string_lossy().as_bytes())
        );
        let mut manager = Self {
            path,
            data,
            secret_key,
            secrets,
        };
        manager.migrate_plaintext()?;
        Ok(manager)
    }

    fn migrate_plaintext(&mut self) -> AppResult<()> {
        let Some(account) = self.data["accounts"].get("msa") else {
            return Ok(());
        };
        if account.get("access_token").is_none() && account.get("refresh_token").is_none() {
            return Ok(());
        }
        let credentials = serde_json::json!({
            "access_token":account["access_token"].as_str().unwrap_or(""),
            "refresh_token":account["refresh_token"].as_str().unwrap_or(""),
        });
        // A failed migration leaves the existing file untouched rather than losing credentials.
        self.secrets
            .set(&self.secret_key, &credentials.to_string())
            .map_err(|e| {
                AppError::Generic(format!("Cannot migrate credentials to a secure store: {e}"))
            })?;
        if let Some(account) = self.data["accounts"]["msa"].as_object_mut() {
            account.remove("access_token");
            account.remove("refresh_token");
        }
        self.save()?;
        // Older releases also stored a shared global refresh token.
        if let Err(error) = self.secrets.delete("msa_refresh_token") {
            crate::warn_msg!("Could not remove legacy keychain credential: {error}");
        }
        Ok(())
    }

    fn save(&self) -> AppResult<()> {
        util::atomic_write(&self.path, &serde_json::to_vec_pretty(&self.data)?)?;
        Ok(())
    }

    pub fn set_msa(
        &mut self,
        username: &str,
        uid: &str,
        access_token: &str,
        refresh_token: &str,
        expires_at: f64,
    ) -> AppResult<()> {
        if access_token.is_empty() || refresh_token.is_empty() {
            return Err("Authentication returned empty credentials".into());
        }
        let credentials =
            serde_json::json!({"access_token":access_token,"refresh_token":refresh_token});
        self.secrets
            .set(&self.secret_key, &credentials.to_string())
            .map_err(|e| AppError::Generic(format!("Cannot save Microsoft credentials: {e}")))?;
        self.data["accounts"]["msa"] = serde_json::json!({"type":"msa","username":username,"uuid":uid,"expires_at":expires_at});
        self.data["default"] = "msa".into();
        self.save()
    }

    pub fn set_offline(&mut self, username: &str) -> AppResult<()> {
        if !(1..=16).contains(&username.len())
            || !username
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(
                "Offline username must contain 1–16 ASCII letters, digits or underscores".into(),
            );
        }
        self.data["accounts"]["offline"] = serde_json::json!({"type":"offline","username":username,"uuid":util::offline_uuid(username)});
        self.data["default"] = "offline".into();
        self.save()
    }

    pub fn get_default(&self) -> Option<serde_json::Value> {
        let key = self.data["default"].as_str().unwrap_or("");
        let mut account = self.data["accounts"]
            .get(key)
            .or_else(|| self.data["accounts"].get("msa"))
            .or_else(|| self.data["accounts"].get("offline"))?
            .clone();
        if account["type"] == "msa" {
            match self.secrets.get(&self.secret_key) {
                Ok(Some(value)) => {
                    if let Ok(credentials) = serde_json::from_str::<serde_json::Value>(&value) {
                        account["access_token"] = credentials["access_token"].clone();
                        account["refresh_token"] = credentials["refresh_token"].clone();
                    }
                }
                Err(error) => {
                    crate::warn_msg!("Cannot read stored credentials: {error}");
                }
                _ => {}
            }
        }
        Some(account)
    }

    pub fn accounts(&self) -> &serde_json::Value {
        &self.data["accounts"]
    }
    pub fn default_key(&self) -> Option<String> {
        self.data["default"].as_str().map(String::from)
    }

    pub fn clear(&mut self) -> AppResult<()> {
        if self.data["accounts"].get("msa").is_some() {
            self.secrets
                .delete(&self.secret_key)
                .map_err(AppError::Generic)?;
            self.secrets
                .delete("msa_refresh_token")
                .map_err(AppError::Generic)?;
        }
        self.data = serde_json::json!({"accounts":{},"default":null});
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, rc::Rc};
    #[derive(Clone, Default)]
    struct MemoryStore(Rc<RefCell<HashMap<String, String>>>);
    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            Ok(self.0.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(key);
            Ok(())
        }
    }

    struct Unavailable;
    impl SecretStore for Unavailable {
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            Err("locked".into())
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
    }

    #[test]
    fn secrets_are_not_written_to_json_and_logout_deletes_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let mut manager = AccountManager::with_store(dir.path(), Box::new(store.clone())).unwrap();
        manager
            .set_msa("Steve", "uuid", "secret-access", "secret-refresh", 123.0)
            .unwrap();
        let saved = fs::read_to_string(&manager.path).unwrap();
        assert!(!saved.contains("secret-") && !saved.contains("refresh_token"));
        assert_eq!(
            manager.get_default().unwrap()["access_token"],
            "secret-access"
        );
        manager.clear().unwrap();
        assert!(store.0.borrow().is_empty());
        assert!(manager.get_default().is_none());
    }

    #[test]
    fn legacy_plaintext_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(ACCOUNTS_FILE), br#"{"accounts":{"msa":{"type":"msa","access_token":"old-access","refresh_token":"old-refresh"}},"default":"msa"}"#).unwrap();
        let manager =
            AccountManager::with_store(dir.path(), Box::<MemoryStore>::default()).unwrap();
        assert_eq!(
            manager.get_default().unwrap()["refresh_token"],
            "old-refresh"
        );
        assert!(!fs::read_to_string(&manager.path)
            .unwrap()
            .contains("old-refresh"));
    }

    #[test]
    fn malformed_account_file_is_not_silently_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ACCOUNTS_FILE);
        fs::write(&path, b"not json").unwrap();
        assert!(AccountManager::with_store(dir.path(), Box::<MemoryStore>::default()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"not json");
    }

    #[test]
    fn offline_account_validation_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            AccountManager::with_store(dir.path(), Box::<MemoryStore>::default()).unwrap();
        for name in ["", "../../a", "a b", "用户名", "abcdefghijklmnopq"] {
            assert!(manager.set_offline(name).is_err());
        }
        manager.set_offline("Steve_01").unwrap();
        let loaded = AccountManager::with_store(dir.path(), Box::<MemoryStore>::default()).unwrap();
        assert_eq!(loaded.get_default().unwrap()["username"], "Steve_01");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&manager.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unavailable_keychain_never_falls_back_to_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = AccountManager::with_store(dir.path(), Box::new(Unavailable)).unwrap();
        assert!(manager
            .set_msa("Steve", "uuid", "access-secret", "refresh-secret", 1.0)
            .is_err());
        assert!(!manager.path.exists());
        let legacy = br#"{"accounts":{"msa":{"type":"msa","access_token":"old"}},"default":"msa"}"#;
        fs::write(&manager.path, legacy).unwrap();
        assert!(AccountManager::with_store(dir.path(), Box::new(Unavailable)).is_err());
        assert_eq!(fs::read(&manager.path).unwrap(), legacy);
    }

    #[test]
    fn encrypted_file_store_round_trip_hides_secrets() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = EncryptedFileStore::new(dir.path());
            store.set("msa:abc", "top-secret-value").unwrap();
        }
        let raw = fs::read_to_string(dir.path().join(SECRETS_FILE)).unwrap();
        assert!(!raw.contains("top-secret-value"));
        let reopened = EncryptedFileStore::new(dir.path());
        assert_eq!(
            reopened.get("msa:abc").unwrap().as_deref(),
            Some("top-secret-value")
        );
        assert_eq!(reopened.get("missing").unwrap(), None);
        reopened.delete("msa:abc").unwrap();
        assert_eq!(reopened.get("msa:abc").unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(SECRETS_KEY_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn encrypted_file_store_rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::new(dir.path());
        store.set("k", "v").unwrap();
        let path = dir.path().join(SECRETS_FILE);
        let mut blob = hex::decode(fs::read_to_string(&path).unwrap().trim()).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        fs::write(&path, hex::encode(blob)).unwrap();
        assert!(EncryptedFileStore::new(dir.path()).get("k").is_err());
    }

    #[test]
    fn keychain_failure_falls_back_to_encrypted_file_with_consent() {
        let dir = tempfile::tempdir().unwrap();
        let store = FallbackStore::with_parts(
            Box::new(Unavailable),
            EncryptedFileStore::new(dir.path()),
            Consent::Always,
        );
        let mut manager = AccountManager::with_store(dir.path(), Box::new(store)).unwrap();
        manager
            .set_msa("Steve", "uuid", "secret-access", "secret-refresh", 1.0)
            .unwrap();
        assert_eq!(
            manager.get_default().unwrap()["refresh_token"],
            "secret-refresh"
        );
        assert!(!fs::read_to_string(&manager.path)
            .unwrap()
            .contains("secret-"));
        let raw = fs::read_to_string(dir.path().join(SECRETS_FILE)).unwrap();
        assert!(!raw.contains("secret-access") && !raw.contains("secret-refresh"));
        assert!(dir.path().join(SECRETS_KEY_FILE).exists());
    }

    #[test]
    fn public_metadata_is_secret_free_and_editable_without_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ACCOUNTS_FILE);
        let initial = AccountManager::public_metadata(dir.path()).unwrap();
        assert!(initial["accounts"].as_object().unwrap().is_empty());
        assert!(!path.exists());

        AccountManager::set_offline_metadata(dir.path(), "Steve_01").unwrap();
        let data = AccountManager::public_metadata(dir.path()).unwrap();
        assert_eq!(data["default"], "offline");
        assert_eq!(data["accounts"]["offline"]["username"], "Steve_01");
        assert_eq!(
            data["accounts"]["offline"]["uuid"],
            util::offline_uuid("Steve_01")
        );

        // Even a legacy file that still contains token fields must not expose
        // them through the metadata API used by the TUI.
        let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        raw["accounts"]["offline"]["access_token"] = "secret-access".into();
        raw["accounts"]["offline"]["refresh_token"] = "secret-refresh".into();
        fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
        let public = AccountManager::public_metadata(dir.path()).unwrap();
        assert!(public["accounts"]["offline"].get("access_token").is_none());
        assert!(public["accounts"]["offline"].get("refresh_token").is_none());

        let before = fs::read(&path).unwrap();
        assert!(AccountManager::set_default_metadata(dir.path(), "missing").is_err());
        assert_eq!(before, fs::read(&path).unwrap());
        AccountManager::set_default_metadata(dir.path(), "offline").unwrap();
        assert!(AccountManager::set_offline_metadata(dir.path(), "../bad").is_err());
        AccountManager::logout_metadata(dir.path()).unwrap();
        let cleared = AccountManager::public_metadata(dir.path()).unwrap();
        assert!(cleared["accounts"].as_object().unwrap().is_empty());
        assert!(cleared["default"].is_null());
    }

    #[test]
    fn malformed_metadata_is_never_overwritten_by_ui_helpers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ACCOUNTS_FILE);
        fs::write(&path, b"broken json").unwrap();
        assert!(AccountManager::public_metadata(dir.path()).is_err());
        assert!(AccountManager::set_offline_metadata(dir.path(), "Steve").is_err());
        assert!(AccountManager::set_default_metadata(dir.path(), "offline").is_err());
        assert!(AccountManager::logout_metadata(dir.path()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken json");
    }

    #[test]
    fn declined_fallback_still_refuses_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = FallbackStore::with_parts(
            Box::new(Unavailable),
            EncryptedFileStore::new(dir.path()),
            Consent::Never,
        );
        let mut manager = AccountManager::with_store(dir.path(), Box::new(store)).unwrap();
        assert!(manager
            .set_msa("Steve", "uuid", "secret-access", "secret-refresh", 1.0)
            .is_err());
        assert!(!manager.path.exists());
        assert!(!dir.path().join(SECRETS_FILE).exists());
        assert!(!dir.path().join(SECRETS_KEY_FILE).exists());
    }
}
