//! Workflow orchestration for provisioning, diagnostics, repair, and drills.

mod diagnostics;
mod provisioning;
mod repair;
mod self_test;

use crate::config::LockchainConfig;
use crate::error::{LockchainError, LockchainResult};
use crate::provider::ZfsProvider;
use crate::service::{LockchainService, UnlockOptions};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

pub use diagnostics::{doctor, self_heal};
pub use provisioning::{forge_key, ForgeMode, ProvisionOptions};
pub use repair::repair_environment;
pub use self_test::self_test;

/// Severity levels used when reporting workflow events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowLevel {
    Info,
    Success,
    Warn,
    Error,
    Security,
}

/// Single line of output produced by a workflow step.
#[derive(Debug, Clone)]
pub struct WorkflowEvent {
    pub level: WorkflowLevel,
    pub message: String,
}

/// Aggregated report returned by any workflow entry point.
#[derive(Debug, Clone)]
pub struct WorkflowReport {
    pub title: String,
    pub events: Vec<WorkflowEvent>,
}

/// Convenience constructor that wraps the repeated boilerplate.
pub(crate) fn event(level: WorkflowLevel, message: impl Into<String>) -> WorkflowEvent {
    WorkflowEvent {
        level,
        message: message.into(),
    }
}

/// Exercise the unlock path end-to-end and capture everything we learned.
pub fn drill_key<P>(
    config: &LockchainConfig,
    provider: P,
    dataset: &str,
    strict_usb: bool,
) -> LockchainResult<WorkflowReport>
where
    P: ZfsProvider + Clone,
{
    let mut events = Vec::new();
    let service = LockchainService::new(Arc::new(config.clone()), provider.clone());
    let mut options = UnlockOptions::default();
    options.strict_usb = strict_usb;
    let report = service.unlock_with_retry(dataset, options)?;

    if report.already_unlocked {
        events.push(event(
            WorkflowLevel::Info,
            format!(
                "Encryption root {} already unlocked",
                report.encryption_root
            ),
        ));
    } else {
        events.push(event(
            WorkflowLevel::Success,
            format!(
                "Unlocked {} ({} datasets)",
                report.encryption_root,
                report.unlocked.len()
            ),
        ));
    }

    let locked_post = provider.locked_descendants(&report.encryption_root)?;
    if locked_post.iter().any(|ds| ds == &report.encryption_root) {
        events.push(event(
            WorkflowLevel::Warn,
            "Root still reports locked descendants after drill — investigate key content.",
        ));
    } else {
        events.push(event(
            WorkflowLevel::Info,
            "All descendants report unlocked after drill.",
        ));
    }

    Ok(WorkflowReport {
        title: format!("Drilled unlock sequence for {dataset}"),
        events,
    })
}

/// Recover fallback key material and write it to disk with the right permissions.
pub fn recover_key<P>(
    config: &LockchainConfig,
    provider: P,
    dataset: &str,
    passphrase: &[u8],
    output_path: &Path,
) -> LockchainResult<WorkflowReport>
where
    P: ZfsProvider + Clone,
{
    let mut events = Vec::new();
    let service = LockchainService::new(Arc::new(config.clone()), provider);
    let key = service
        .derive_fallback_key(passphrase)
        .map_err(|err| LockchainError::InvalidConfig(err.to_string()))?;
    crate::keyfile::write_raw_key_file(output_path, &key)?;
    let digest = hex::encode(Sha256::digest(&key[..]));
    events.push(event(
        WorkflowLevel::Security,
        format!(
            "Derived fallback key for {dataset} and wrote to {}",
            output_path.display()
        ),
    ));
    events.push(event(
        WorkflowLevel::Info,
        format!("SHA-256 of derived key: {digest}"),
    ));
    fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o400))?;
    Ok(WorkflowReport {
        title: format!("Recovered key material for {dataset}"),
        events,
    })
}
