//! Lockchain command-line interface: provisioning, maintenance, and unlock tooling.

use anyhow::{bail, ensure, Context, Result};
use clap::{Parser, Subcommand};
use lockchain_core::{
    config::Policy,
    keyfile::write_raw_key_file,
    logging,
    provider::{DatasetKeyDescriptor, KeyState},
    workflow::{self, ForgeMode, ProvisionOptions, WorkflowLevel, WorkflowReport},
    LockchainConfig, LockchainService, UnlockOptions,
};
use lockchain_zfs::SystemZfsProvider;
use log::warn;
use rpassword::prompt_password;
use schemars::schema_for;
use serde_json::to_string_pretty;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

mod tui;

/// Top-level command-line options shared by every subcommand.
#[derive(Parser, Debug)]
#[command(
    name = "lockchain",
    version,
    about = "Key management utilities for Lockchain ZFS deployments."
)]
struct Cli {
    /// Path to the Lockchain configuration file.
    #[arg(short, long, default_value = "/etc/lockchain-zfs.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

/// Subcommands covering the full lifecycle of a Lockchain deployment.
#[derive(Subcommand, Debug)]
enum Commands {
    /// Provision a USB token with raw key material and refresh initramfs assets.
    Init {
        /// Target dataset; defaults to the first entry in policy.datasets.
        dataset: Option<String>,

        /// USB block device (e.g. /dev/sdb1). When omitted, autodetect via label/UUID.
        #[arg(long)]
        device: Option<String>,

        /// Mountpoint used during provisioning.
        #[arg(long)]
        mount: Option<PathBuf>,

        /// Filename to write inside the mounted token (default: key.hex).
        #[arg(long)]
        filename: Option<String>,

        /// Optional fallback passphrase material to configure immediately.
        #[arg(long)]
        passphrase: Option<String>,

        /// Perform a non-destructive safety check instead of wiping the token.
        #[arg(long)]
        safe: bool,

        /// Force a wipe even in safe mode.
        #[arg(long)]
        force_wipe: bool,

        /// Skip initramfs rebuild after provisioning.
        #[arg(long)]
        no_rebuild: bool,
    },

    /// Run diagnostics and remediation to keep the environment healthy.
    Doctor,

    /// Unlock an encrypted dataset (and its descendants).
    Unlock {
        /// Target dataset; defaults to the first entry in policy.datasets.
        dataset: Option<String>,

        /// Require USB key material and skip fallback handling.
        #[arg(long)]
        strict_usb: bool,

        /// Provide a fallback passphrase directly on the command line.
        #[arg(long)]
        passphrase: Option<String>,

        /// Prompt interactively for the fallback passphrase.
        #[arg(long)]
        prompt_passphrase: bool,

        /// Provide raw key material via file (32-byte binary).
        #[arg(long)]
        key_file: Option<PathBuf>,
    },

    /// Perform a self-test using an ephemeral ZFS pool.
    SelfTest {
        /// Dataset to validate; defaults to the first entry in policy.datasets.
        dataset: Option<String>,

        /// Require the USB token and skip fallback handling during the drill.
        #[arg(long)]
        strict_usb: bool,
    },

    /// Reinstall mount/unlock systemd units and ensure services are enabled.
    Repair,

    /// Show keystatus information for a dataset (or all managed datasets).
    Status {
        /// Dataset to inspect; defaults to all configured datasets.
        dataset: Option<String>,
    },

    /// List the managed datasets and their current key status.
    ListKeys,

    /// Launch the interactive TUI unlocker.
    Tui,

    /// Validate a configuration file or emit the config schema.
    Validate {
        /// Path to the configuration file to validate.
        #[arg(short = 'f', long, default_value = "/etc/lockchain-zfs.toml")]
        file: PathBuf,

        /// Output the JSON schema instead of validating a file.
        #[arg(long)]
        schema: bool,
    },

    /// Derive the fallback key and write it to disk (emergency only).
    Breakglass {
        /// Dataset to target; defaults to the first entry in policy.datasets.
        dataset: Option<String>,

        /// File path to write the derived key material to.
        #[arg(short, long)]
        output: PathBuf,

        /// Provide the emergency passphrase directly.
        #[arg(long)]
        passphrase: Option<String>,

        /// Skip interactive confirmations.
        #[arg(long)]
        force: bool,
    },
}

/// Entry point: parse arguments and surface errors with an exit code.
fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

/// Dispatch to the requested subcommand and map results into rich output.
fn run() -> Result<()> {
    logging::init("info");
    let cli = Cli::parse();
    let config_path = cli.config.clone();

    match cli.command {
        Commands::Init {
            dataset,
            device,
            mount,
            filename,
            passphrase,
            safe,
            force_wipe,
            no_rebuild,
        } => {
            let mut config = LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?;
            let provider = SystemZfsProvider::from_config(&config)?;
            let target = resolve_dataset(dataset, &config.policy)?;
            let mut options = ProvisionOptions::default();
            options.usb_device = device;
            options.mountpoint = mount;
            options.key_filename = filename;
            options.passphrase = passphrase;
            options.force_wipe = force_wipe;
            options.rebuild_initramfs = !no_rebuild;
            let mode = if safe {
                ForgeMode::Safe
            } else {
                ForgeMode::Standard
            };
            let report = workflow::forge_key(&mut config, &provider, &target, mode, options)
                .map_err(anyhow::Error::new)?;
            print_report(report);
            return Ok(());
        }
        Commands::Doctor => {
            let config = LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?;
            let provider = SystemZfsProvider::from_config(&config)?;
            let report = workflow::doctor(&config, provider).map_err(anyhow::Error::new)?;
            print_report(report);
            return Ok(());
        }
        Commands::Validate { file, schema } => {
            if schema {
                let schema = schema_for!(LockchainConfig);
                println!("{}", to_string_pretty(&schema)?);
                return Ok(());
            }

            let cfg = LockchainConfig::load(&file)
                .with_context(|| format!("failed to load configuration from {}", file.display()))?;

            let issues = cfg.validate();
            if issues.is_empty() {
                println!(
                    "Configuration valid ({} datasets).",
                    cfg.policy.datasets.len()
                );
            } else {
                eprintln!("Configuration validation failed:");
                for issue in issues {
                    eprintln!("  - {issue}");
                }
                std::process::exit(1);
            }
            return Ok(());
        }
        Commands::Breakglass {
            dataset,
            output,
            passphrase,
            force,
        } => {
            let config = Arc::new(LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?);
            let provider = SystemZfsProvider::from_config(&config)?;
            let service = LockchainService::new(config.clone(), provider);

            let target = resolve_dataset(dataset, &config.policy)?;
            if !config.fallback.enabled {
                bail!("fallback recovery is not enabled in this configuration");
            }
            if config.fallback.passphrase_salt.is_none() || config.fallback.passphrase_xor.is_none()
            {
                bail!("fallback configuration is incomplete (salt/xor missing)");
            }

            if !force {
                println!("*** BREAK-GLASS RECOVERY ***");
                println!(
                    "This will derive the raw key for dataset `{}` and write it to {}.",
                    target,
                    output.display()
                );
                println!("Type the dataset name to continue or press Enter to abort:");
                print!("> ");
                io::stdout().flush().ok();
                let mut confirm_dataset = String::new();
                io::stdin().read_line(&mut confirm_dataset)?;
                if confirm_dataset.trim() != target {
                    println!("Break-glass aborted.");
                    return Ok(());
                }

                println!("Type BREAKGLASS to confirm this emergency action:");
                print!("> ");
                io::stdout().flush().ok();
                let mut confirm_phrase = String::new();
                io::stdin().read_line(&mut confirm_phrase)?;
                if confirm_phrase.trim() != "BREAKGLASS" {
                    println!("Break-glass aborted.");
                    return Ok(());
                }
            }

            let passphrase = match passphrase {
                Some(p) => p,
                None => prompt_password(format!("Emergency passphrase for {target}: "))?,
            };

            let key = service.derive_fallback_key(passphrase.as_bytes())?;
            write_raw_key_file(&output, &key)?;

            warn!(
                "[LC4000] break-glass recovery invoked for dataset {target}, output {}",
                output.display()
            );
            println!(
                "Emergency key material written to {} (permissions set to 0400). Remember to securely delete this file when finished.",
                output.display()
            );
            return Ok(());
        }
        Commands::SelfTest {
            dataset,
            strict_usb,
        } => {
            let config = LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?;
            let provider = SystemZfsProvider::from_config(&config)?;
            let target = resolve_dataset(dataset, &config.policy)?;
            let report = workflow::self_test(&config, provider, &target, strict_usb)
                .map_err(anyhow::Error::new)?;
            print_report(report);
            return Ok(());
        }
        Commands::Repair => {
            let config = LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?;
            let report = workflow::repair_environment(&config).map_err(anyhow::Error::new)?;
            print_report(report);
            return Ok(());
        }
        Commands::Unlock {
            dataset,
            strict_usb,
            passphrase,
            prompt_passphrase,
            key_file,
        } => {
            let config = Arc::new(LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?);
            let provider = SystemZfsProvider::from_config(&config)?;
            let service = LockchainService::new(config.clone(), provider);
            let target = resolve_dataset(dataset, &config.policy)?;
            let mut options = UnlockOptions::default();
            options.strict_usb = strict_usb;

            if let Some(path) = key_file {
                let key_bytes =
                    fs::read(&path).with_context(|| format!("read key file {}", path.display()))?;
                ensure!(
                    key_bytes.len() == 32,
                    "expected a 32-byte raw key in {}, found {} bytes",
                    path.display(),
                    key_bytes.len()
                );
                options.key_override = Some(key_bytes);
            }

            if let Some(pass) = passphrase {
                options.fallback_passphrase = Some(pass);
            } else if prompt_passphrase {
                let prompt = format!("Fallback passphrase for {}", target);
                let value = prompt_password(prompt)?;
                options.fallback_passphrase = Some(value);
            }

            let report = service.unlock_with_retry(&target, options)?;
            if report.already_unlocked {
                println!(
                    "Dataset {} (root {}) already has an available key.",
                    target, report.encryption_root
                );
            } else {
                println!(
                    "Unlocked encryption root {} via dataset {}.",
                    report.encryption_root, target
                );
                for ds in report.unlocked {
                    println!("  - {ds}");
                }
            }
        }
        Commands::Status { dataset } => {
            let config = Arc::new(LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?);
            let provider = SystemZfsProvider::from_config(&config)?;
            let service = LockchainService::new(config.clone(), provider);
            let datasets = match dataset {
                Some(ds) => vec![ds],
                None => config.policy.datasets.clone(),
            };

            for ds in datasets {
                let status = service.status(&ds)?;
                if status.root_locked {
                    println!(
                        "{} (root {}) is LOCKED.",
                        status.dataset, status.encryption_root
                    );
                    if status.locked_descendants.is_empty() {
                        println!("  No locked descendants reported.");
                    } else {
                        println!("  Locked descendants:");
                        for child in status.locked_descendants {
                            println!("    - {child}");
                        }
                    }
                } else {
                    println!(
                        "{} (root {}) is unlocked.",
                        status.dataset, status.encryption_root
                    );
                }
            }
        }
        Commands::ListKeys => {
            let config = Arc::new(LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?);
            let provider = SystemZfsProvider::from_config(&config)?;
            let service = LockchainService::new(config.clone(), provider);
            let snapshot = service.list_keys()?;
            print_key_table(snapshot);
        }
        Commands::Tui => {
            let config = Arc::new(LockchainConfig::load(&config_path).with_context(|| {
                format!(
                    "failed to load configuration from {}",
                    config_path.display()
                )
            })?);
            let provider = SystemZfsProvider::from_config(&config)?;
            let service = LockchainService::new(config.clone(), provider);
            tui::launch(config, service)?;
        }
    }

    Ok(())
}

/// Pretty-print a workflow report so humans can follow along.
fn print_report(report: WorkflowReport) {
    println!("{}", report.title);
    for event in report.events {
        println!("  [{}] {}", level_tag(event.level), event.message);
    }
}

/// Short tag used when printing workflow severity levels.
fn level_tag(level: WorkflowLevel) -> &'static str {
    match level {
        WorkflowLevel::Info => "INFO",
        WorkflowLevel::Success => "OK",
        WorkflowLevel::Warn => "WARN",
        WorkflowLevel::Error => "ERR",
        WorkflowLevel::Security => "SEC",
    }
}

/// Pick a dataset from CLI input or fall back to the first policy entry.
fn resolve_dataset(dataset: Option<String>, policy: &Policy) -> Result<String> {
    if let Some(ds) = dataset {
        return Ok(ds);
    }
    policy
        .datasets
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no datasets configured in policy.datasets"))
}

/// Render a simple table describing current key status across datasets.
fn print_key_table(snapshot: Vec<DatasetKeyDescriptor>) {
    println!("{:<32} {:<32} {}", "DATASET", "ENCRYPTION ROOT", "STATUS");
    for entry in snapshot {
        let status = match entry.state {
            KeyState::Available => "available".to_string(),
            KeyState::Unavailable => "locked".to_string(),
            KeyState::Unknown(value) => value,
        };
        println!(
            "{:<32} {:<32} {}",
            entry.dataset, entry.encryption_root, status
        );
    }
}
