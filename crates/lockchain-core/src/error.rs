use std::path::PathBuf;
use thiserror::Error;

/// Result alias for core operations.
pub type LockchainResult<T> = Result<T, LockchainError>;

#[derive(Error, Debug)]
pub enum LockchainError {
    #[error("[LC1000] io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("[LC1001] toml config parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("[LC1002] yaml config parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("[LC1100] configuration error: {0}")]
    InvalidConfig(String),

    #[error("[LC1200] dataset `{0}` is not declared in policy")]
    DatasetNotConfigured(String),

    #[error("[LC1201] no key source configured for dataset `{0}`")]
    MissingKeySource(String),

    #[error("[LC1300] failed to decode hex key at {path}: {reason}")]
    InvalidHexKey { path: PathBuf, reason: String },

    #[error("[LC2000] provider error: {0}")]
    Provider(String),

    #[error("[LC3000] unlock retries exhausted after {attempts} attempts: {last_error}")]
    RetryExhausted { attempts: u32, last_error: String },
}

impl LockchainError {
    pub fn code(&self) -> &'static str {
        match self {
            LockchainError::Io(_) => "LC1000",
            LockchainError::Toml(_) => "LC1001",
            LockchainError::Yaml(_) => "LC1002",
            LockchainError::InvalidConfig(_) => "LC1100",
            LockchainError::DatasetNotConfigured(_) => "LC1200",
            LockchainError::MissingKeySource(_) => "LC1201",
            LockchainError::InvalidHexKey { .. } => "LC1300",
            LockchainError::Provider(_) => "LC2000",
            LockchainError::RetryExhausted { .. } => "LC3000",
        }
    }
}
