use crate::settings::AppData;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use directories::ProjectDirs;
use rand::RngCore;
use std::fs;
use std::path::PathBuf;

const DATA_FILE: &str = "accounts.enc";
const KEY_FILE: &str = "key.bin";

pub struct SecureStorage {
    data_path: PathBuf,
    key_path: PathBuf,
}

impl SecureStorage {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "steamaccountmanager", "Steam Account Manager")
            .context("Kon data directory niet bepalen")?;
        let data_dir = dirs.data_dir().to_path_buf();
        fs::create_dir_all(&data_dir).context("Kon data directory niet aanmaken")?;
        Ok(Self {
            data_path: data_dir.join(DATA_FILE),
            key_path: data_dir.join(KEY_FILE),
        })
    }

    pub fn with_paths(data_path: PathBuf, key_path: PathBuf) -> Result<Self> {
        if let Some(parent) = data_path.parent() {
            fs::create_dir_all(parent).context("Kon data directory niet aanmaken")?;
        }
        Ok(Self {
            data_path,
            key_path,
        })
    }

    pub fn load(&self) -> Result<AppData> {
        if !self.data_path.exists() {
            return Ok(AppData::default());
        }
        let encrypted = fs::read(&self.data_path).context("Kon data niet lezen")?;
        if encrypted.len() < 12 {
            anyhow::bail!("Ongeldig databestand");
        }
        let key = self.load_or_create_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).context("Kon cipher niet initialiseren")?;
        let nonce = Nonce::from_slice(&encrypted[..12]);
        let plaintext = cipher
            .decrypt(nonce, &encrypted[12..])
            .map_err(|_| anyhow::anyhow!("Kon data niet ontsleutelen"))?;
        let mut data: AppData = match serde_json::from_slice(&plaintext) {
            Ok(data) => data,
            Err(_) => {
                let store: crate::accounts::AccountStore =
                    serde_json::from_slice(&plaintext).context("Kon data niet parsen")?;
                AppData {
                    accounts: store,
                    settings: crate::settings::AppSettings::default(),
                }
            }
        };
        for account in &mut data.accounts.accounts {
            account.sync_search_fields();
        }
        Ok(data)
    }

    pub fn save(&self, data: &AppData) -> Result<()> {
        let key = self.load_or_create_key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).context("Kon cipher niet initialiseren")?;
        let plaintext = serde_json::to_vec(data).context("Kon data niet serialiseren")?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|_| anyhow::anyhow!("Kon data niet versleutelen"))?;
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        fs::write(&self.data_path, output).context("Kon data niet opslaan")?;
        Ok(())
    }

    fn load_or_create_key(&self) -> Result<[u8; 32]> {
        if self.key_path.exists() {
            let raw = fs::read(&self.key_path).context("Kon sleutel niet lezen")?;
            let decoded = STANDARD.decode(raw).context("Kon sleutel niet decoderen")?;
            if decoded.len() != 32 {
                anyhow::bail!("Ongeldige sleutellengte");
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            return Ok(key);
        }
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        let encoded = STANDARD.encode(key);
        fs::write(&self.key_path, encoded).context("Kon sleutel niet opslaan")?;
        Ok(key)
    }

    pub fn data_dir_display(&self) -> String {
        self.data_path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{account_from_form, AccountFormInput};
    use crate::settings::AppData;
    use tempfile::TempDir;

    #[test]
    fn encrypted_roundtrip_preserves_accounts_and_settings() {
        let dir = TempDir::new().unwrap();
        let storage =
            SecureStorage::with_paths(dir.path().join("accounts.enc"), dir.path().join("key.bin"))
                .unwrap();
        let mut data = AppData::default();
        data.accounts.add(account_from_form(&AccountFormInput {
            username: "tester".into(),
            password: "secret".into(),
            alias: "Main".into(),
            ..Default::default()
        }));
        data.settings.register_country = "NL".into();
        storage.save(&data).unwrap();
        let loaded = storage.load().unwrap();
        assert_eq!(loaded.accounts.accounts.len(), 1);
        assert_eq!(loaded.accounts.accounts[0].username, "tester");
        assert_eq!(loaded.settings.register_country, "NL");
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = TempDir::new().unwrap();
        let storage =
            SecureStorage::with_paths(dir.path().join("missing.enc"), dir.path().join("key.bin"))
                .unwrap();
        let data = storage.load().unwrap();
        assert!(data.accounts.accounts.is_empty());
    }
}
