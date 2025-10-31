//! Self-healing and diagnostic workflows that keep Lockchain deployments healthy.

use super::{event, repair_environment, WorkflowEvent, WorkflowLevel, WorkflowReport};
use crate::config::LockchainConfig;
use crate::error::LockchainResult;
use crate::keyfile::{read_key_file, write_raw_key_file};
use crate::provider::{DatasetKeyDescriptor, KeyState, ZfsProvider};
use crate::service::LockchainService;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const JOURNAL_SAMPLE_LINES: usize = 20;
const DEFAULT_SERVICES: &[&str] = &[
    "lockchain-key-usb.service",
    "lockchain-zfs.service",
    "run-lockchain.mount",
];

const INITRAMFS_TOOLS: &[&str] = &["dracut", "update-initramfs", "lsinitrd", "lsinitramfs"];

/// Aggregates the raw results from the self-heal pass before we build a report.
#[derive(Default)]
struct SelfHealOutcome {
    events: Vec<WorkflowEvent>,
    warnings: usize,
    errors: usize,
    key_valid: bool,
    checksum_match: bool,
    updated_config: Option<LockchainConfig>,
}

/// Run non-destructive checks and attempt to repair obvious issues automatically.
pub fn self_heal<P>(config: &LockchainConfig, provider: P) -> LockchainResult<WorkflowReport>
where
    P: ZfsProvider + Clone,
{
    let outcome = run_self_heal(config, provider)?;
    Ok(WorkflowReport {
        title: "Self-heal diagnostics".into(),
        events: outcome.events,
    })
}

/// Wraps `self_heal` with deeper inspections and actionable remediation tips.
pub fn doctor<P>(config: &LockchainConfig, provider: P) -> LockchainResult<WorkflowReport>
where
    P: ZfsProvider + Clone,
{
    let outcome = run_self_heal(config, provider.clone())?;
    let SelfHealOutcome {
        events: heal_events,
        key_valid,
        checksum_match,
        updated_config,
        ..
    } = outcome;
    let mut events = Vec::new();
    let mut remedies = Vec::new();

    events.push(event(
        WorkflowLevel::Info,
        "Self-heal baseline diagnostics follow.",
    ));
    events.extend(heal_events.into_iter());

    if !key_valid {
        remedies.push("Re-import USB key material or re-run the provisioning directive.".into());
    }
    if !checksum_match {
        remedies.push("Update usb.expected_sha256 to match on-disk key material.".into());
    }

    events.push(event(
        WorkflowLevel::Info,
        "Inspecting lockchain-key-usb journal tail.",
    ));
    if let Some(remedy) = audit_journal("lockchain-key-usb.service", &mut events) {
        remedies.push(remedy);
    }

    events.push(event(
        WorkflowLevel::Info,
        "Evaluating systemd units required for boot flow.",
    ));
    for unit in DEFAULT_SERVICES {
        if let Some(remedy) = audit_systemd_unit(unit, &mut events) {
            remedies.push(remedy);
        }
    }

    events.push(event(
        WorkflowLevel::Info,
        "Verifying initramfs tooling presence.",
    ));
    remedies.extend(audit_initramfs_tooling(&mut events));

    events.push(event(
        WorkflowLevel::Info,
        "Reapplying system integration policies.",
    ));
    let repair_cfg = updated_config.as_ref().unwrap_or(config);
    match repair_environment(repair_cfg) {
        Ok(report) => events.extend(report.events.into_iter()),
        Err(err) => {
            events.push(event(
                WorkflowLevel::Warn,
                format!("System integration repair failed: {err}"),
            ));
            remedies.push("Run lockchain repair with elevated privileges.".into());
        }
    }

    if !remedies.is_empty() {
        events.push(event(
            WorkflowLevel::Warn,
            format!("Remediation actions suggested: {}", remedies.join(" | ")),
        ));
    }

    let (warnings, errors) = count_levels(&events);
    let summary_level = if errors > 0 {
        WorkflowLevel::Error
    } else if warnings > 0 {
        WorkflowLevel::Warn
    } else {
        WorkflowLevel::Success
    };
    events.push(event(
        summary_level,
        format!("Doctor summary :: warnings={} errors={}", warnings, errors),
    ));

    Ok(WorkflowReport {
        title: "System doctor diagnostics".into(),
        events,
    })
}

/// Core implementation shared by doctor/self-heal flows so we only probe the system once.
fn run_self_heal<P>(config: &LockchainConfig, provider: P) -> LockchainResult<SelfHealOutcome>
where
    P: ZfsProvider + Clone,
{
    let mut outcome = SelfHealOutcome::default();
    let mut cfg = config.clone();
    let mut config_dirty = false;
    let key_path = cfg.key_hex_path();

    let metadata = match fs::metadata(&key_path) {
        Ok(meta) => {
            let mode = meta.permissions().mode() & 0o777;
            outcome.events.push(event(
                WorkflowLevel::Info,
                format!(
                    "Key file located at {} (mode {:o})",
                    key_path.display(),
                    mode
                ),
            ));
            if mode != 0o400 {
                match fs::set_permissions(&key_path, fs::Permissions::from_mode(0o400)) {
                    Ok(_) => outcome.events.push(event(
                        WorkflowLevel::Warn,
                        format!(
                            "Key file permissions were {:o}; tightened to 0400 for compliance.",
                            mode
                        ),
                    )),
                    Err(err) => outcome.events.push(event(
                        WorkflowLevel::Error,
                        format!(
                            "Key file permissions {:o}; failed to set 0400 ({err}).",
                            mode
                        ),
                    )),
                }
            }
            Some(meta)
        }
        Err(err) => {
            outcome.events.push(event(
                WorkflowLevel::Error,
                format!(
                    "Key file {} missing or unreadable ({err})",
                    key_path.display()
                ),
            ));
            None
        }
    };

    if metadata.is_some() {
        match read_key_file(&key_path) {
            Ok((key, converted)) => {
                if converted {
                    match write_raw_key_file(&key_path, &key[..]) {
                        Ok(_) => outcome.events.push(event(
                            WorkflowLevel::Warn,
                            "Normalised legacy hex key to raw 32-byte format on disk.",
                        )),
                        Err(err) => outcome.events.push(event(
                            WorkflowLevel::Error,
                            format!("Failed to rewrite key as raw bytes ({err})."),
                        )),
                    }
                }

                if key.len() == 32 {
                    outcome.key_valid = true;
                    outcome.events.push(event(
                        WorkflowLevel::Success,
                        "Key material validated as raw 32-byte payload.",
                    ));
                } else {
                    outcome.events.push(event(
                        WorkflowLevel::Error,
                        format!(
                            "Key material must be 32 bytes; detected {} bytes.",
                            key.len()
                        ),
                    ));
                }

                let digest = hex::encode(Sha256::digest(&key[..]));
                if let Some(expected) = &config.usb.expected_sha256 {
                    if expected.eq_ignore_ascii_case(&digest) {
                        outcome.checksum_match = true;
                        outcome.events.push(event(
                            WorkflowLevel::Success,
                            "usb.expected_sha256 matches on-disk key material.",
                        ));
                    } else {
                        cfg.usb.expected_sha256 = Some(digest.clone());
                        config_dirty = true;
                        outcome.events.push(event(
                            WorkflowLevel::Warn,
                            format!(
                                "usb.expected_sha256 mismatch: config={} actual={digest}",
                                expected
                            ),
                        ));
                    }
                } else {
                    outcome.events.push(event(
                        WorkflowLevel::Warn,
                        format!(
                            "Computed key SHA-256={digest}; usb.expected_sha256 not configured."
                        ),
                    ));
                }
            }
            Err(err) => outcome.events.push(event(
                WorkflowLevel::Error,
                format!("Unable to decode key file {} ({err})", key_path.display()),
            )),
        }
    }

    if let Some(label) = &cfg.usb.device_label {
        outcome.events.push(event(
            WorkflowLevel::Info,
            format!("Configured USB label requirement: {label}"),
        ));
    } else {
        outcome.events.push(event(
            WorkflowLevel::Warn,
            "usb.device_label not set; relying on generic mount discovery.",
        ));
    }

    if let Some(uuid) = &cfg.usb.device_uuid {
        outcome.events.push(event(
            WorkflowLevel::Info,
            format!("Configured USB UUID requirement: {uuid}"),
        ));
    } else {
        outcome.events.push(event(
            WorkflowLevel::Warn,
            "usb.device_uuid not set; ensure label-based matching is resilient.",
        ));
    }

    let service = LockchainService::new(Arc::new(cfg.clone()), provider.clone());
    match service.list_keys() {
        Ok(snapshot) => {
            for DatasetKeyDescriptor {
                dataset,
                encryption_root,
                state,
            } in snapshot
            {
                match state {
                    KeyState::Available => outcome.events.push(event(
                        WorkflowLevel::Success,
                        format!("{dataset} :: {encryption_root} reports available"),
                    )),
                    KeyState::Unavailable => outcome.events.push(event(
                        WorkflowLevel::Warn,
                        format!("{dataset} :: {encryption_root} remains locked"),
                    )),
                    KeyState::Unknown(detail) => outcome.events.push(event(
                        WorkflowLevel::Warn,
                        format!("{dataset} :: status unknown ({detail})"),
                    )),
                }
            }
        }
        Err(err) => outcome.events.push(event(
            WorkflowLevel::Error,
            format!("Unable to enumerate dataset status ({err})"),
        )),
    }

    if cfg.fallback.enabled {
        let salt = cfg.fallback.passphrase_salt.is_some();
        let xor = cfg.fallback.passphrase_xor.is_some();
        if salt && xor {
            outcome.events.push(event(
                WorkflowLevel::Info,
                "Fallback passphrase material present.",
            ));
        } else {
            outcome.events.push(event(
                WorkflowLevel::Warn,
                "Fallback enabled but salt/xor material incomplete.",
            ));
        }
    } else {
        outcome.events.push(event(
            WorkflowLevel::Info,
            "Fallback passphrase disabled by configuration.",
        ));
    }

    if config_dirty {
        match cfg.save() {
            Ok(_) => outcome.events.push(event(
                WorkflowLevel::Info,
                format!("Persisted configuration updates to {}", cfg.path.display()),
            )),
            Err(err) => outcome.events.push(event(
                WorkflowLevel::Warn,
                format!("Failed to persist configuration updates ({err})"),
            )),
        }
        outcome.updated_config = Some(cfg);
    }

    let (warnings, errors) = count_levels(&outcome.events);
    outcome.warnings = warnings;
    outcome.errors = errors;
    Ok(outcome)
}

/// Sample a service's journal tail and flag any warnings or errors we spot.
fn audit_journal(service: &str, events: &mut Vec<WorkflowEvent>) -> Option<String> {
    let output = Command::new("journalctl")
        .args([
            "-u",
            service,
            "-n",
            &JOURNAL_SAMPLE_LINES.to_string(),
            "--no-pager",
        ])
        .output();

    match output {
        Ok(output) => {
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr);
                events.push(event(
                    WorkflowLevel::Warn,
                    format!(
                        "journalctl -u {service} returned exit code {} ({detail})",
                        output.status
                    ),
                ));
                return Some(format!(
                    "Investigate journald availability and ensure {service} is logging."
                ));
            }

            let text = String::from_utf8_lossy(&output.stdout);
            if text.trim().is_empty() {
                events.push(event(
                    WorkflowLevel::Warn,
                    format!("No recent journal entries for {service}."),
                ));
                return Some(format!(
                    "Restart {service} or ensure logging is configured."
                ));
            }

            let mut errors = 0;
            let mut warnings = 0;
            for line in text.lines() {
                let lower = line.to_lowercase();
                if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
                    errors += 1;
                } else if lower.contains("warn") || lower.contains("degrade") {
                    warnings += 1;
                }
            }

            let snippet: Vec<&str> = text
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<&str>>()
                .into_iter()
                .rev()
                .collect();

            let level = if errors > 0 {
                WorkflowLevel::Error
            } else if warnings > 0 {
                WorkflowLevel::Warn
            } else {
                WorkflowLevel::Info
            };

            events.push(event(
                level,
                format!(
                    "{} journal tail ({} lines): {}",
                    service,
                    JOURNAL_SAMPLE_LINES,
                    snippet.join(" | ")
                ),
            ));

            if errors > 0 {
                Some(format!(
                    "{service} journal contains {errors} error entries; run `journalctl -u {service}` for detail."
                ))
            } else if warnings > 0 {
                Some(format!(
                    "{service} journal includes warnings; review recent events."
                ))
            } else {
                None
            }
        }
        Err(err) => {
            events.push(event(
                WorkflowLevel::Warn,
                format!("journalctl not available ({err})."),
            ));
            Some("Install systemd-journal tools or review alternative logging backend.".into())
        }
    }
}

/// Inspect a systemd unit's state and suggest follow-up when it's unhealthy.
fn audit_systemd_unit(unit: &str, events: &mut Vec<WorkflowEvent>) -> Option<String> {
    let output = Command::new("systemctl")
        .args([
            "show",
            unit,
            "-p",
            "LoadState",
            "-p",
            "ActiveState",
            "-p",
            "UnitFileState",
        ])
        .output();

    match output {
        Ok(output) => {
            if !output.status.success() {
                let detail = String::from_utf8_lossy(&output.stderr);
                events.push(event(
                    WorkflowLevel::Warn,
                    format!("systemctl show {unit} failed: {detail}"),
                ));
                return Some(format!(
                    "Ensure {unit} is installed and systemd is available."
                ));
            }

            let text = String::from_utf8_lossy(&output.stdout);
            let mut load = "unknown";
            let mut active = "unknown";
            let mut unit_file = "unknown";
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("LoadState=") {
                    load = rest;
                } else if let Some(rest) = line.strip_prefix("ActiveState=") {
                    active = rest;
                } else if let Some(rest) = line.strip_prefix("UnitFileState=") {
                    unit_file = rest;
                }
            }

            let mut severity = WorkflowLevel::Info;
            let mut remedy = None;

            if load != "loaded" {
                severity = WorkflowLevel::Error;
                remedy = Some(format!(
                    "{unit} is not loaded (LoadState={load}); reinstall or re-enable the unit."
                ));
            } else if active != "active" && active != "activating" {
                severity = WorkflowLevel::Warn;
                remedy = Some(format!(
                    "{unit} is not active (ActiveState={active}); review `systemctl status {unit}`."
                ));
            } else if unit_file != "enabled" && unit_file != "static" {
                severity = WorkflowLevel::Warn;
                remedy = Some(format!(
                    "{unit} is not enabled (UnitFileState={unit_file}); run `systemctl enable {unit}`."
                ));
            }

            events.push(event(
                severity,
                format!("{unit}: LoadState={load} ActiveState={active} UnitFileState={unit_file}"),
            ));
            remedy
        }
        Err(err) => {
            events.push(event(
                WorkflowLevel::Warn,
                format!("systemctl not available to inspect {unit} ({err})."),
            ));
            Some("Systemd not present; validate service management manually.".into())
        }
    }
}

/// Confirm the expected initramfs utilities are present in PATH.
fn audit_initramfs_tooling(events: &mut Vec<WorkflowEvent>) -> Vec<String> {
    let mut remedies = Vec::new();
    let mut available = false;

    for tool in INITRAMFS_TOOLS {
        if let Some(path) = search_path(tool) {
            available = true;
            events.push(event(
                WorkflowLevel::Info,
                format!("{tool} detected at {}", path.display()),
            ));
        } else {
            events.push(event(
                WorkflowLevel::Warn,
                format!("{tool} not found in PATH."),
            ));
            remedies.push(format!(
                "Install `{tool}` or ensure initramfs refresh tooling is available."
            ));
        }
    }

    if !available {
        remedies.push(
            "Neither dracut nor initramfs-tools were detected; initramfs rebuilds will fail."
                .into(),
        );
    }

    remedies
}

/// Minimal PATH lookup that honours absolute or relative binary hints.
fn search_path(binary: &str) -> Option<PathBuf> {
    if binary.contains('/') {
        let path = Path::new(binary);
        if path.exists() {
            return Some(path.to_path_buf());
        }
        return None;
    }

    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(binary);
            if candidate.exists() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

/// Count how many warnings and errors we collected.
fn count_levels(events: &[WorkflowEvent]) -> (usize, usize) {
    let mut warnings = 0;
    let mut errors = 0;
    for event in events {
        match event.level {
            WorkflowLevel::Warn => warnings += 1,
            WorkflowLevel::Error => errors += 1,
            _ => {}
        }
    }
    (warnings, errors)
}
