//! Account metadata on disk; Microsoft credentials only in the OS keychain.
use crate::error::{AppError, AppResult};
use crate::util;
use sha1::{Digest, Sha1};
use std::fs;
use std::path::{Path, PathBuf};

const ACCOUNTS_FILE: &str = "launcher_accounts.json";

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

pub struct AccountManager {
    path: PathBuf,
    data: serde_json::Value,
    secret_key: String,
    secrets: Box<dyn SecretStore>,
}

impl AccountManager {
    pub fn new(game_dir: &Path) -> AppResult<Self> {
        Self::with_store(game_dir, Box::new(Keychain))
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
        self.secrets.set(&self.secret_key, &credentials.to_string())
            .map_err(|e| AppError::Generic(format!("Cannot migrate credentials to OS keychain: {e}. Unlock/configure your keychain and retry.")))?;
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
        self.secrets.set(&self.secret_key, &credentials.to_string())
            .map_err(|e| AppError::Generic(format!("Cannot save Microsoft credentials to OS keychain: {e}. No plaintext fallback is used.")))?;
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
                    crate::warn_msg!("Cannot read OS keychain: {error}");
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
    use std::{cell::RefCell, collections::HashMap, rc::Rc};
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
}
