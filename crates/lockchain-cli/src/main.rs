use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use lockchain_core::{
    config::Policy,
    provider::{DatasetKeyDescriptor, KeyState},
    LockchainConfig, LockchainService, UnlockOptions,
};
use lockchain_zfs::SystemZfsProvider;
use rpassword::prompt_password;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

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

#[derive(Subcommand, Debug)]
enum Commands {
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

    /// Show keystatus information for a dataset (or all managed datasets).
    Status {
        /// Dataset to inspect; defaults to all configured datasets.
        dataset: Option<String>,
    },

    /// List the managed datasets and their current key status.
    ListKeys,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    let config =
        Arc::new(LockchainConfig::load(&cli.config).with_context(|| {
            format!("failed to load configuration from {}", cli.config.display())
        })?);
    let provider = SystemZfsProvider::from_config(&config)?;
    let service = LockchainService::new(config.clone(), provider);

    match cli.command {
        Commands::Unlock {
            dataset,
            strict_usb,
            passphrase,
            prompt_passphrase,
            key_file,
        } => {
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

            let report = service.unlock(&target, options)?;
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
            let snapshot = service.list_keys()?;
            print_key_table(snapshot);
        }
    }

    Ok(())
}

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
