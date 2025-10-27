use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockChainError {
#[error("I/O error: {0}")]
Io(#[from] std::io::Error),
#[error("Invalid configuration")]
InvalidConfig,
#[error("ZFS command failed")]
ZfsFailure,
}
