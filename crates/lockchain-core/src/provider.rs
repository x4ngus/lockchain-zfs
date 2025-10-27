use crate::error::LockchainResult;

/// Normalised keystatus for a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyState {
    Available,
    Unavailable,
    Unknown(String),
}

/// High-level descriptor for dataset encryption metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetKeyDescriptor {
    pub dataset: String,
    pub encryption_root: String,
    pub state: KeyState,
}

/// Snapshot of keystatus information for a group of datasets.
pub type KeyStatusSnapshot = Vec<DatasetKeyDescriptor>;

/// Abstraction over ZFS key-management commands.
///
/// Implementations are expected to provide a thin, testable surface over the
/// underlying system interface (CLI, RPC, etc.), so higher-level services can
/// be exercised without invoking real ZFS binaries.
pub trait ZfsProvider {
    /// Resolve the encryption root responsible for `dataset`.
    fn encryption_root(&self, dataset: &str) -> LockchainResult<String>;

    /// Return datasets under `root` (including the root itself) that still
    /// report a sealed keystatus.
    fn locked_descendants(&self, root: &str) -> LockchainResult<Vec<String>>;

    /// Attempt to load a key for `root` and any descendants that share it.
    /// Returns the datasets confirmed to have accepted the key, in the order
    /// they were processed (root is always first).
    fn load_key_tree(&self, root: &str, key: &[u8]) -> LockchainResult<Vec<String>>;

    /// Describe the keystatus for the provided dataset list. Implementations
    /// should return entries for each dataset in the input slice, preserving
    /// that order.
    fn describe_datasets(&self, datasets: &[String]) -> LockchainResult<KeyStatusSnapshot>;
}
