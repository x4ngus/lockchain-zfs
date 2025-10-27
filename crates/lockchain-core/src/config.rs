use crate::error::{LockchainError, LockchainResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub datasets: Vec<String>,

    #[serde(default)]
    pub zfs_path: Option<String>,

    #[serde(default)]
    pub binary_path: Option<String>,

    #[serde(default)]
    pub allow_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoCfg {
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    10
}

impl Default for CryptoCfg {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usb {
    #[serde(default = "default_usb_key_path")]
    pub key_hex_path: String,

    #[serde(default)]
    pub expected_sha256: Option<String>,
}

fn default_usb_key_path() -> String {
    "/run/lockchain/key.hex".to_string()
}

impl Default for Usb {
    fn default() -> Self {
        Self {
            key_hex_path: default_usb_key_path(),
            expected_sha256: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fallback {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub askpass: bool,

    #[serde(default)]
    pub askpass_path: Option<String>,

    #[serde(default)]
    pub passphrase_salt: Option<String>,

    #[serde(default)]
    pub passphrase_xor: Option<String>,

    #[serde(default = "default_passphrase_iters")]
    pub passphrase_iters: u32,
}

fn default_passphrase_iters() -> u32 {
    250_000
}

impl Default for Fallback {
    fn default() -> Self {
        Self {
            enabled: true,
            askpass: true,
            askpass_path: Some("/usr/bin/systemd-ask-password".to_string()),
            passphrase_salt: None,
            passphrase_xor: None,
            passphrase_iters: default_passphrase_iters(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockchainConfig {
    pub policy: Policy,

    #[serde(default)]
    pub crypto: CryptoCfg,

    #[serde(default)]
    pub usb: Usb,

    #[serde(default)]
    pub fallback: Fallback,

    #[serde(skip)]
    pub path: PathBuf,
}

impl LockchainConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> LockchainResult<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        let mut cfg = if matches!(path.extension().and_then(|ext| ext.to_str()), Some(ext) if ext.eq_ignore_ascii_case("toml"))
        {
            toml::from_str::<Self>(&contents)?
        } else {
            serde_yaml::from_str::<Self>(&contents)?
        };

        cfg.path = path.to_path_buf();

        if cfg.policy.datasets.is_empty() {
            return Err(LockchainError::InvalidConfig(
                "policy.datasets must list at least one dataset".to_string(),
            ));
        }

        Ok(cfg)
    }

    pub fn contains_dataset(&self, dataset: &str) -> bool {
        self.policy.datasets.iter().any(|d| d == dataset)
    }

    pub fn key_hex_path(&self) -> PathBuf {
        PathBuf::from(&self.usb.key_hex_path)
    }

    pub fn zfs_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.crypto.timeout_secs)
    }

    pub fn zfs_binary_path(&self) -> Option<PathBuf> {
        self.policy.zfs_path.as_ref().map(PathBuf::from)
    }
}
