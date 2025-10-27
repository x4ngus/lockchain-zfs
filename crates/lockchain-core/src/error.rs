use std::path::PathBuf;
use thiserror::Error;

/// Result alias for core operations.
pub type LockchainResult<T> = Result<T, LockchainError>;

#[derive(Error, Debug)]
pub enum LockchainError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML config parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("YAML config parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("configuration error: {0}")]
    InvalidConfig(String),

    #[error("dataset `{0}` is not declared in policy")]
    DatasetNotConfigured(String),

    #[error("no key source configured for dataset `{0}`")]
    MissingKeySource(String),

    #[error("failed to decode hex key at {path}: {reason}")]
    InvalidHexKey { path: PathBuf, reason: String },

    #[error("provider error: {0}")]
    Provider(String),
}
