use crate::command::{CommandRunner, Output};
use lockchain_core::config::LockchainConfig;
use lockchain_core::error::{LockchainError, LockchainResult};
use lockchain_core::provider::{DatasetKeyDescriptor, KeyState, KeyStatusSnapshot, ZfsProvider};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_ZFS_PATHS: &[&str] = &[
    "/sbin/zfs",
    "/usr/sbin/zfs",
    "/usr/local/sbin/zfs",
    "/bin/zfs",
];

/// Concrete `ZfsProvider` that shells out to the native `zfs` CLI.
pub struct SystemZfsProvider {
    runner: CommandRunner,
}

impl SystemZfsProvider {
    pub fn from_config(config: &LockchainConfig) -> LockchainResult<Self> {
        let timeout = config.zfs_timeout();
        if let Some(path) = config.zfs_binary_path() {
            return Self::with_path(path, timeout);
        }
        Self::discover(timeout)
    }

    pub fn with_path(path: PathBuf, timeout: Duration) -> LockchainResult<Self> {
        if !path.exists() {
            return Err(LockchainError::InvalidConfig(format!(
                "zfs binary not found at {}",
                path.display()
            )));
        }
        Ok(Self {
            runner: CommandRunner::new(path, timeout),
        })
    }

    pub fn discover(timeout: Duration) -> LockchainResult<Self> {
        for candidate in DEFAULT_ZFS_PATHS {
            let p = Path::new(candidate);
            if p.exists() {
                return Self::with_path(p.to_path_buf(), timeout);
            }
        }
        Err(LockchainError::InvalidConfig(format!(
            "unable to locate zfs binary; tried {:?}",
            DEFAULT_ZFS_PATHS
        )))
    }

    fn run(&self, args: &[&str], input: Option<&[u8]>) -> LockchainResult<Output> {
        self.runner.run(args, input)
    }

    fn run_checked(&self, args: &[&str]) -> LockchainResult<Output> {
        let out = self.run(args, None)?;
        if out.status != 0 {
            return Err(LockchainError::Provider(format!(
                "{} {} failed: {}",
                self.runner.binary().display(),
                args.join(" "),
                out.stderr.trim()
            )));
        }
        Ok(out)
    }

    fn get_property(&self, dataset: &str, property: &str) -> LockchainResult<String> {
        let out = self.run_checked(&["get", "-H", "-o", "value", property, dataset])?;
        Ok(out.stdout.trim().to_string())
    }

    fn load_key(&self, dataset: &str, key: &[u8]) -> LockchainResult<()> {
        let out = self.run(&["load-key", "-L", "prompt", dataset], Some(key))?;
        if out.status != 0 {
            let stderr = out.stderr.trim();
            if stderr.contains("Key already loaded") {
                return Ok(());
            }
            return Err(LockchainError::Provider(format!(
                "zfs load-key {} failed: {}",
                dataset, stderr
            )));
        }
        Ok(())
    }

    fn keystatus(&self, dataset: &str) -> LockchainResult<KeyState> {
        let out = self.run_checked(&["get", "-H", "-o", "value", "keystatus", dataset])?;
        Ok(Self::parse_keystatus(out.stdout.trim()))
    }

    fn parse_keystatus(value: &str) -> KeyState {
        match value {
            "available" => KeyState::Available,
            "unavailable" | "absent" | "missing" => KeyState::Unavailable,
            other if other.is_empty() || other == "-" || other == "none" => {
                KeyState::Unknown(other.to_string())
            }
            other => KeyState::Unknown(other.to_string()),
        }
    }
}

impl ZfsProvider for SystemZfsProvider {
    fn encryption_root(&self, dataset: &str) -> LockchainResult<String> {
        self.get_property(dataset, "encryptionroot")
    }

    fn locked_descendants(&self, root: &str) -> LockchainResult<Vec<String>> {
        let list = self.run_checked(&["list", "-H", "-r", "-o", "name,encryptionroot", root])?;
        let mut same_root = HashSet::new();
        for line in list.stdout.lines() {
            let mut parts = line.split('\t');
            if let (Some(name), Some(enc_root)) = (parts.next(), parts.next()) {
                if enc_root == root {
                    same_root.insert(name.to_string());
                }
            }
        }

        let status =
            self.run_checked(&["get", "-H", "-r", "-o", "name,value", "keystatus", root])?;
        let mut locked = Vec::new();
        for line in status.stdout.lines() {
            let mut parts = line.split('\t');
            if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                let trimmed = value.trim();
                if same_root.contains(name)
                    && trimmed != "available"
                    && !trimmed.is_empty()
                    && trimmed != "-"
                    && trimmed != "none"
                {
                    locked.push(name.to_string());
                }
            }
        }
        locked.sort();
        Ok(locked)
    }

    fn load_key_tree(&self, root: &str, key: &[u8]) -> LockchainResult<Vec<String>> {
        self.load_key(root, key)?;
        let mut unlocked = vec![root.to_string()];

        let pending = self
            .locked_descendants(root)?
            .into_iter()
            .filter(|ds| ds != root)
            .collect::<Vec<_>>();
        for ds in &pending {
            self.load_key(ds, key)?;
            unlocked.push(ds.clone());
        }

        let stubborn = self.locked_descendants(root)?;
        if stubborn.iter().any(|ds| ds == root) {
            return Err(LockchainError::Provider(format!(
                "encryption root {} remained locked after load-key",
                root
            )));
        }
        let stubborn_descendants: Vec<String> =
            stubborn.into_iter().filter(|ds| ds != root).collect();
        if !stubborn_descendants.is_empty() {
            return Err(LockchainError::Provider(format!(
                "descendants still locked after retries: {}",
                stubborn_descendants.join(", ")
            )));
        }

        Ok(unlocked)
    }

    fn describe_datasets(&self, datasets: &[String]) -> LockchainResult<KeyStatusSnapshot> {
        let mut snapshot = Vec::with_capacity(datasets.len());
        for ds in datasets {
            let encryption_root = self.encryption_root(ds)?;
            let state = self.keystatus(ds)?;
            snapshot.push(DatasetKeyDescriptor {
                dataset: ds.clone(),
                encryption_root,
                state,
            });
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keystatus_handles_basic_cases() {
        assert!(matches!(
            SystemZfsProvider::parse_keystatus("available"),
            KeyState::Available
        ));
        assert!(matches!(
            SystemZfsProvider::parse_keystatus("unavailable"),
            KeyState::Unavailable
        ));
        assert!(matches!(
            SystemZfsProvider::parse_keystatus(""),
            KeyState::Unknown(_)
        ));
    }
}
