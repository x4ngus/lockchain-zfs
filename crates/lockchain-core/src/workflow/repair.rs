//! System integration repair flow: installs units and enables them as needed.

use super::{event, WorkflowEvent, WorkflowLevel, WorkflowReport};
use crate::config::LockchainConfig;
use crate::error::{LockchainError, LockchainResult};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SYSTEMD_DIR_ENV: &str = "LOCKCHAIN_SYSTEMD_DIR";
const SYSTEMCTL_PATH_ENV: &str = "LOCKCHAIN_SYSTEMCTL";
const SYSTEMCTL_SKIP_ENV: &str = "LOCKCHAIN_SKIP_SYSTEMCTL";
const RUN_DIR: &str = "/run/lockchain";

/// Repair the host integration by ensuring systemd units exist and are enabled.
pub fn repair_environment(config: &LockchainConfig) -> LockchainResult<WorkflowReport> {
    let mut events = Vec::new();

    let skip_systemctl = env::var_os(SYSTEMCTL_SKIP_ENV).is_some();

    let systemd_dir = systemd_dir();
    if let Err(err) = fs::create_dir_all(&systemd_dir) {
        return Err(LockchainError::Io(std::io::Error::new(
            err.kind(),
            format!(
                "unable to create systemd directory {}: {err}",
                systemd_dir.display()
            ),
        )));
    }

    if let Err(err) = fs::create_dir_all(RUN_DIR) {
        events.push(event(
            WorkflowLevel::Warn,
            format!("Could not ensure {RUN_DIR} exists ({err}). USB watcher may create it later."),
        ));
    }

    install_mount_unit(config, &systemd_dir, &mut events)?;

    if skip_systemctl {
        events.push(event(
            WorkflowLevel::Warn,
            "LOCKCHAIN_SKIP_SYSTEMCTL set – skipping systemctl enable actions.",
        ));
    } else if let Some(systemctl) = systemctl_path() {
        reload_systemd(&systemctl, &mut events);
        enable_unit(&systemctl, "run-lockchain.mount", &mut events);
        enable_unit(&systemctl, "lockchain-zfs.service", &mut events);
        enable_unit(&systemctl, "lockchain-key-usb.service", &mut events);
        for dataset in &config.policy.datasets {
            if let Some(unit) = escaped_dataset_unit(dataset) {
                enable_unit(&systemctl, &unit, &mut events);
            } else {
                events.push(event(
                    WorkflowLevel::Warn,
                    format!(
                        "Unable to derive systemd instance name for dataset {dataset}; run `systemctl enable lockchain-zfs@$(systemd-escape --template=lockchain-zfs@.service {dataset})` manually."
                    ),
                ));
            }
        }
    } else {
        events.push(event(
            WorkflowLevel::Warn,
            "systemctl not found; unable to enable units automatically.",
        ));
    }

    Ok(WorkflowReport {
        title: "System integration repair".into(),
        events,
    })
}

/// Ensure the run-lockchain mount unit exists with the correct token selector.
fn install_mount_unit(
    config: &LockchainConfig,
    systemd_dir: &Path,
    events: &mut Vec<WorkflowEvent>,
) -> LockchainResult<()> {
    let what = usb_identifier(config)?;
    let path = systemd_dir.join("run-lockchain.mount");

    let content = format!(
        r#"[Unit]
Description=Mount LockChain key USB
DefaultDependencies=no
Before=local-fs-pre.target
After=systemd-udev-settle.service
Requires=systemd-udev-settle.service

[Mount]
What={what}
Where={where_dir}
Type=auto
Options=ro,nosuid,nodev,noexec,x-systemd.device-timeout=5s

[Install]
WantedBy=local-fs-pre.target
"#,
        what = what,
        where_dir = RUN_DIR
    );

    fs::write(&path, content)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Installed mount unit at {}", path.display()),
    ));
    Ok(())
}

/// Build a systemd-friendly identifier for the USB key device.
fn usb_identifier(config: &LockchainConfig) -> LockchainResult<String> {
    if let Some(uuid) = config.usb.device_uuid.as_ref() {
        if !uuid.trim().is_empty() {
            return Ok(format!("/dev/disk/by-uuid/{uuid}"));
        }
    }
    if let Some(label) = config.usb.device_label.as_ref() {
        if !label.trim().is_empty() {
            return Ok(format!("/dev/disk/by-label/{label}"));
        }
    }
    Err(LockchainError::InvalidConfig(
        "usb.device_uuid or usb.device_label must be configured before repairing systemd units"
            .into(),
    ))
}

/// Run `systemctl daemon-reload` and surface any warnings.
fn reload_systemd(systemctl: &Path, events: &mut Vec<WorkflowEvent>) {
    match Command::new(systemctl).args(["daemon-reload"]).output() {
        Ok(result) if result.status.success() => {
            events.push(event(WorkflowLevel::Info, "systemd daemon reloaded."))
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            events.push(event(
                WorkflowLevel::Warn,
                format!("systemctl daemon-reload failed: {}", stderr.trim()),
            ));
        }
        Err(err) => events.push(event(
            WorkflowLevel::Warn,
            format!("systemctl daemon-reload failed: {err}"),
        )),
    }
}

/// Enable a unit and log whether it succeeded.
fn enable_unit(systemctl: &Path, unit: &str, events: &mut Vec<WorkflowEvent>) {
    let output = Command::new(systemctl).args(["enable", unit]).output();
    match output {
        Ok(result) if result.status.success() => {
            events.push(event(WorkflowLevel::Info, format!("Enabled {unit}")))
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            events.push(event(
                WorkflowLevel::Warn,
                format!("systemctl enable {unit} failed: {}", stderr.trim()),
            ));
        }
        Err(err) => events.push(event(
            WorkflowLevel::Warn,
            format!("systemctl enable {unit} failed: {err}"),
        )),
    }
}

/// Use `systemd-escape` to form the instance unit name for a dataset.
fn escaped_dataset_unit(dataset: &str) -> Option<String> {
    if let Some(path) = systemd_escape_path() {
        if let Ok(output) = Command::new(path)
            .args(["--template=lockchain-zfs@.service", dataset])
            .output()
        {
            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// Honor the override environment variable or fall back to the systemd dir.
fn systemd_dir() -> PathBuf {
    env::var_os(SYSTEMD_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/systemd/system"))
}

/// Locate the `systemctl` binary, checking overrides first.
fn systemctl_path() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os(SYSTEMCTL_PATH_ENV) {
        return Some(PathBuf::from(explicit));
    }
    for candidate in ["/bin/systemctl", "/usr/bin/systemctl"] {
        let path = Path::new(candidate);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

/// Find a usable `systemd-escape` helper.
fn systemd_escape_path() -> Option<PathBuf> {
    for candidate in ["/bin/systemd-escape", "/usr/bin/systemd-escape"] {
        let path = Path::new(candidate);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CryptoCfg, Fallback, LockchainConfig, Policy, RetryCfg, Usb};
    use std::env;
    use tempfile::tempdir;

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

    fn sample_config(path: PathBuf) -> LockchainConfig {
        LockchainConfig {
            policy: Policy {
                datasets: vec!["tank/secure".into()],
                zfs_path: None,
                zpool_path: None,
                binary_path: None,
                allow_root: false,
            },
            crypto: CryptoCfg { timeout_secs: 5 },
            usb: Usb {
                key_hex_path: "/run/lockchain/key.hex".into(),
                expected_sha256: None,
                device_label: Some("LOCKCHAINKEY".into()),
                device_uuid: Some("UUID-TEST".into()),
                device_key_path: "key.hex".into(),
                mount_timeout_secs: 10,
            },
            fallback: Fallback::default(),
            retry: RetryCfg::default(),
            path,
            format: crate::config::ConfigFormat::Toml,
        }
    }

    #[test]
    fn repair_creates_mount_unit() {
        let temp = tempdir().unwrap();
        let _dir_guard = EnvGuard::set(SYSTEMD_DIR_ENV, temp.path().to_string_lossy());
        let _skip_guard = EnvGuard::set(SYSTEMCTL_SKIP_ENV, "1");

        let config_path = temp.path().join("config.toml");
        let config = sample_config(config_path);

        let report = repair_environment(&config).expect("repair should succeed");
        assert_eq!(report.title, "System integration repair");
        let mount_unit = temp.path().join("run-lockchain.mount");
        assert!(mount_unit.exists());
        let content = fs::read_to_string(mount_unit).expect("read unit file");
        assert!(content.contains("LockChain key USB"));
        assert!(content.contains("/dev/disk/by-uuid/UUID-TEST"));
    }
}
