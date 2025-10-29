use crate::error::{LockchainError, LockchainResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const KEY_PATH_ENV: &str = "LOCKCHAIN_KEY_PATH";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Policy {
    pub datasets: Vec<String>,

    #[serde(default)]
    pub zfs_path: Option<String>,

    #[serde(default)]
    pub zpool_path: Option<String>,

    #[serde(default)]
    pub binary_path: Option<String>,

    #[serde(default)]
    pub allow_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Usb {
    #[serde(default = "default_usb_key_path")]
    pub key_hex_path: String,

    #[serde(default)]
    pub expected_sha256: Option<String>,

    #[serde(default)]
    pub device_label: Option<String>,

    #[serde(default)]
    pub device_uuid: Option<String>,

    #[serde(default = "default_usb_device_key_path")]
    pub device_key_path: String,

    #[serde(default = "default_usb_mount_timeout_secs")]
    pub mount_timeout_secs: u64,
}

fn default_usb_key_path() -> String {
    "/run/lockchain/key.hex".to_string()
}

fn default_usb_device_key_path() -> String {
    "key.hex".to_string()
}

fn default_usb_mount_timeout_secs() -> u64 {
    10
}

impl Default for Usb {
    fn default() -> Self {
        Self {
            key_hex_path: default_usb_key_path(),
            expected_sha256: None,
            device_label: None,
            device_uuid: None,
            device_key_path: default_usb_device_key_path(),
            mount_timeout_secs: default_usb_mount_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryCfg {
    #[serde(default = "default_retry_attempts")]
    pub max_attempts: u32,

    #[serde(default = "default_retry_base_delay")]
    pub base_delay_ms: u64,

    #[serde(default = "default_retry_max_delay")]
    pub max_delay_ms: u64,

    #[serde(default = "default_retry_jitter")]
    pub jitter_ratio: f64,
}

fn default_retry_attempts() -> u32 {
    3
}

fn default_retry_base_delay() -> u64 {
    500
}

fn default_retry_max_delay() -> u64 {
    5_000
}

fn default_retry_jitter() -> f64 {
    0.1
}

impl Default for RetryCfg {
    fn default() -> Self {
        Self {
            max_attempts: default_retry_attempts(),
            base_delay_ms: default_retry_base_delay(),
            max_delay_ms: default_retry_max_delay(),
            jitter_ratio: default_retry_jitter(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LockchainConfig {
    pub policy: Policy,

    #[serde(default)]
    pub crypto: CryptoCfg,

    #[serde(default)]
    pub usb: Usb,

    #[serde(default)]
    pub fallback: Fallback,

    #[serde(default)]
    pub retry: RetryCfg,

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

    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if self.policy.datasets.is_empty() {
            issues.push("policy.datasets must contain at least one dataset".to_string());
        }

        let mut seen = std::collections::HashSet::new();
        for ds in &self.policy.datasets {
            if ds.trim().is_empty() {
                issues.push("policy.datasets contains an empty dataset entry".to_string());
            }
            if !seen.insert(ds) {
                issues.push(format!("duplicate dataset entry detected: {ds}"));
            }
        }

        if let Some(expected) = &self.usb.expected_sha256 {
            if expected.len() != 64 || hex::decode(expected).is_err() {
                issues.push("usb.expected_sha256 must be a 64-character hex string".to_string());
            }
        }

        if self.fallback.enabled {
            if self.fallback.passphrase_salt.is_none() {
                issues.push(
                    "fallback.enabled is true but fallback.passphrase_salt is missing".to_string(),
                );
            }
            if self.fallback.passphrase_xor.is_none() {
                issues.push(
                    "fallback.enabled is true but fallback.passphrase_xor is missing".to_string(),
                );
            }
        }

        if self.retry.max_attempts == 0 {
            issues.push("retry.max_attempts must be at least 1".to_string());
        }
        if self.retry.base_delay_ms == 0 {
            issues.push("retry.base_delay_ms must be greater than 0".to_string());
        }
        if self.retry.max_delay_ms < self.retry.base_delay_ms {
            issues.push(
                "retry.max_delay_ms must be greater than or equal to retry.base_delay_ms"
                    .to_string(),
            );
        }
        if !(0.0..=1.0).contains(&self.retry.jitter_ratio) {
            issues.push("retry.jitter_ratio must be between 0.0 and 1.0".to_string());
        }

        issues
    }

    pub fn key_hex_path(&self) -> PathBuf {
        if let Ok(override_path) = env::var(KEY_PATH_ENV) {
            if !override_path.is_empty() {
                return PathBuf::from(override_path);
            }
        }
        PathBuf::from(&self.usb.key_hex_path)
    }

    pub fn zfs_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.crypto.timeout_secs)
    }

    pub fn zfs_binary_path(&self) -> Option<PathBuf> {
        self.policy.zfs_path.as_ref().map(PathBuf::from)
    }

    pub fn zpool_binary_path(&self) -> Option<PathBuf> {
        self.policy.zpool_path.as_ref().map(PathBuf::from)
    }

    pub fn retry_config(&self) -> &RetryCfg {
        &self.retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl Into<String>) -> Self {
            let prev = env::var(key).ok();
            env::set_var(key, value.into());
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(prev) = &self.prev {
                env::set_var(self.key, prev);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn key_path_respects_env_override() {
        let config = LockchainConfig {
            policy: Policy {
                datasets: vec!["tank/secure".into()],
                zfs_path: None,
                zpool_path: None,
                binary_path: None,
                allow_root: false,
            },
            crypto: CryptoCfg { timeout_secs: 1 },
            usb: Usb::default(),
            fallback: Fallback::default(),
            retry: RetryCfg::default(),
            path: PathBuf::new(),
        };

        let guard = EnvGuard::set(KEY_PATH_ENV, "/tmp/override.key");
        assert_eq!(config.key_hex_path(), PathBuf::from("/tmp/override.key"));
        drop(guard);
        assert_eq!(config.key_hex_path(), PathBuf::from(default_usb_key_path()));
    }
}
