//! USB watcher that copies key material from removable media into place.

use anyhow::{bail, Context, Result};
use clap::Parser;
use hex::encode as hex_encode;
use lockchain_core::{
    keyfile::{read_key_file, write_raw_key_file},
    logging, LockchainConfig,
};
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use udev::{Device, Enumerator, MonitorBuilder};

const DEFAULT_CONFIG_PATH: &str = "/etc/lockchain-zfs.toml";
const MOUNTS_OVERRIDE_ENV: &str = "LOCKCHAIN_KEY_USB_MOUNTS_PATH";

/// Command-line options for the USB watcher service.
#[derive(Parser, Debug)]
#[command(
    name = "lockchain-key-usb",
    version,
    about = "USB key watcher for LockChain deployments."
)]
struct Args {
    /// Path to the LockChain configuration file.
    #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,
}

/// Top-level entry: wrap run() and map errors to logs + exit codes.
fn main() {
    if let Err(err) = run() {
        error!("{err:?}");
        std::process::exit(1);
    }
}

/// Load configuration, prime the daemon, and start monitoring udev events.
fn run() -> Result<()> {
    logging::init("info");

    let args = Args::parse();
    let config = Arc::new(
        LockchainConfig::load(&args.config)
            .with_context(|| format!("failed to load config {}", args.config.display()))?,
    );

    info!(
        "USB key watcher started (dest path: {})",
        config.key_hex_path().display()
    );

    let daemon = UsbKeyDaemon::new(config);
    daemon.scan_existing()?;
    daemon.event_loop()
}

/// Tracks the currently mounted USB device so we can clean up on removal.
#[derive(Debug)]
struct ActiveDevice {
    devpath: String,
    devnode: PathBuf,
    #[allow(dead_code)]
    mount_point: PathBuf,
    #[allow(dead_code)]
    source_path: PathBuf,
}

/// Handles device discovery, checksum verification, and file synchronisation.
struct UsbKeyDaemon {
    config: Arc<LockchainConfig>,
    active: Mutex<Option<ActiveDevice>>,
}

impl UsbKeyDaemon {
    /// Construct a daemon with shared configuration.
    fn new(config: Arc<LockchainConfig>) -> Self {
        Self {
            config,
            active: Mutex::new(None),
        }
    }

    /// Look for already-mounted USB devices that match policy.
    fn scan_existing(&self) -> Result<()> {
        let mut enumerator = Enumerator::new()?;
        enumerator.match_subsystem("block")?;
        enumerator.match_property("DEVTYPE", "partition")?;
        enumerator.match_property("ID_BUS", "usb")?;

        for device in enumerator.scan_devices()? {
            self.try_import(&device)?;
        }
        Ok(())
    }

    /// Block on udev events and react to arrivals and removals.
    fn event_loop(&self) -> Result<()> {
        let mut monitor = MonitorBuilder::new()?.match_subsystem("block")?.listen()?;

        loop {
            if let Some(event) = monitor.next() {
                let device = event.device();
                if let Err(err) = self.process_device(&device) {
                    warn!(
                        "handling event for {} failed: {err:?}",
                        device_syspath(&device)
                    );
                }
            } else {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    /// Dispatch the udev event to either import or cleanup handlers.
    fn process_device(&self, device: &Device) -> Result<()> {
        let action = device.action().and_then(os_str_to_str).unwrap_or("change");
        match action {
            "add" | "change" | "bind" => self.try_import(device),
            "remove" | "unbind" => {
                self.handle_removal(device);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Validate the device, verify content, and copy key material into place.
    fn try_import(&self, device: &Device) -> Result<()> {
        if !self.device_matches(device) {
            return Ok(());
        }

        let devpath = device.devpath().to_string_lossy().to_string();
        {
            let active = self.active.lock().unwrap();
            if matches!(
                active.as_ref(),
                Some(current)
                    if current.devpath == devpath
            ) {
                debug!("device {} already active, skipping import", devpath);
                return Ok(());
            }
        }

        let devnode = device
            .devnode()
            .ok_or_else(|| anyhow::anyhow!("device {} missing devnode", devpath))?
            .to_path_buf();

        let mount_point = self.wait_for_mount(&devnode)?;
        let source_path = mount_point.join(&self.config.usb.device_key_path);

        let (key, converted) = match read_key_file(&source_path) {
            Ok(result) => result,
            Err(err) => {
                warn!("failed to decode key at {}: {err}", source_path.display());
                self.clear_destination();
                return Ok(());
            }
        };

        if let Some(expected) = &self.config.usb.expected_sha256 {
            let digest = Sha256::digest(&key);
            let checksum = hex_encode(digest);
            if !expected.eq_ignore_ascii_case(&checksum) {
                warn!(
                    "checksum mismatch for {}: expected {}, got {}",
                    source_path.display(),
                    expected,
                    checksum
                );
                self.clear_destination();
                return Ok(());
            }
        }

        if converted {
            info!(
                "normalised legacy hex key from {} before writing destination",
                source_path.display()
            );
        }

        let dest = self.config.key_hex_path();
        write_raw_key_file(&dest, &key).map_err(|err| anyhow::anyhow!(err))?;
        info!(
            "copied key material from {} to {}",
            source_path.display(),
            dest.display()
        );

        let mut guard = self.active.lock().unwrap();
        *guard = Some(ActiveDevice {
            devpath,
            devnode,
            mount_point,
            source_path,
        });

        Ok(())
    }

    /// Tear down state when the matching USB device disappears.
    fn handle_removal(&self, device: &Device) {
        let mut guard = self.active.lock().unwrap();
        if guard.is_none() {
            return;
        }

        let matches = {
            let active = guard.as_ref().unwrap();
            let devpath = device.devpath().to_string_lossy();
            let devnode = device.devnode().map(|p| p.to_path_buf());

            if devpath == active.devpath {
                true
            } else if let Some(node) = devnode {
                node == active.devnode
            } else {
                false
            }
        };

        if matches {
            info!(
                "device {} removed; clearing destination key",
                device_syspath(device)
            );
            self.clear_destination();
            *guard = None;
        }
    }

    /// Remove the destination key to avoid stale material lingering.
    fn clear_destination(&self) {
        let dest = self.config.key_hex_path();
        match fs::remove_file(&dest) {
            Ok(_) => info!("removed destination key {}", dest.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => warn!("failed to remove destination key {}: {err}", dest.display()),
        }
    }

    /// Poll /proc/mounts until the device shows up or we time out.
    fn wait_for_mount(&self, devnode: &Path) -> Result<PathBuf> {
        let timeout = Duration::from_secs(self.config.usb.mount_timeout_secs);
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(path) = find_mount_point(devnode)? {
                return Ok(path);
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out waiting for {} to mount ({}s)",
                    devnode.display(),
                    timeout.as_secs()
                );
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// Check whether the udev device aligns with our configured label/UUID.
    fn device_matches(&self, device: &Device) -> bool {
        if device.property_value("DEVTYPE").and_then(os_str_to_str) != Some("partition") {
            return false;
        }

        if device.property_value("ID_BUS").and_then(os_str_to_str) != Some("usb") {
            return false;
        }

        if let Some(expected) = &self.config.usb.device_label {
            let label = device.property_value("ID_FS_LABEL").and_then(os_str_to_str);
            if label.map(|value| value != expected).unwrap_or(true) {
                return false;
            }
        }

        if let Some(expected) = &self.config.usb.device_uuid {
            let uuid = device.property_value("ID_FS_UUID").and_then(os_str_to_str);
            if uuid.map(|value| value != expected).unwrap_or(true) {
                return false;
            }
        }

        true
    }
}

/// Provide a human-readable path for logging udev devices.
fn device_syspath(device: &Device) -> String {
    device.syspath().to_string_lossy().into_owned()
}

/// Convenience helper for zero-copy OsStr → &str conversions.
fn os_str_to_str(value: &OsStr) -> Option<&str> {
    value.to_str()
}

/// Locate the mountpoint for a block device by scanning the mount table.
fn find_mount_point(devnode: &Path) -> Result<Option<PathBuf>> {
    let mounts = read_mount_table()?;
    let devnode_str = devnode.to_string_lossy();
    Ok(parse_mounts(&mounts, devnode_str.as_ref()))
}

/// Read `/proc/mounts` or its override for testing purposes.
fn read_mount_table() -> Result<String> {
    if let Ok(path) = env::var(MOUNTS_OVERRIDE_ENV) {
        return Ok(fs::read_to_string(&path).with_context(|| format!("read mounts file {path}"))?);
    }
    Ok(fs::read_to_string("/proc/mounts").context("read /proc/mounts")?)
}

/// Parse the mount table content and return a matching mountpoint path.
fn parse_mounts(mounts: &str, devnode: &str) -> Option<PathBuf> {
    for line in mounts.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let device = match parts.next() {
            Some(value) => value,
            None => continue,
        };
        let mountpoint = match parts.next() {
            Some(value) => value,
            None => continue,
        };
        if device == devnode {
            return Some(PathBuf::from(unescape_mount_field(mountpoint)));
        }
    }
    None
}

/// Convert fstab-style escaped fields back into display strings.
fn unescape_mount_field(input: &str) -> String {
    let mut chars = input.chars().peekable();
    let mut output = String::with_capacity(input.len());

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut oct = String::new();
            for _ in 0..3 {
                if let Some(next) = chars.peek() {
                    if !next.is_ascii_digit() {
                        break;
                    }
                }
                if let Some(next) = chars.next() {
                    oct.push(next);
                }
            }
            if oct.len() == 3 {
                if let Ok(value) = u8::from_str_radix(&oct, 8) {
                    output.push(value as char);
                    continue;
                }
            }
            output.push('\\');
            output.push_str(&oct);
        } else {
            output.push(ch);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

    #[test]
    fn parse_mounts_finds_matching_device() {
        let snapshot = "/dev/sdb1 /media/LOCK\\040CHAIN ext4 rw 0 0\n";
        let mount = parse_mounts(snapshot, "/dev/sdb1").unwrap();
        assert_eq!(mount, PathBuf::from("/media/LOCK CHAIN"));
    }

    #[test]
    fn find_mount_point_honours_override() {
        let dir = tempdir().unwrap();
        let mount_file = dir.path().join("mounts");
        fs::write(
            &mount_file,
            "/dev/sdb1 /media/lockchain ext4 rw,relatime 0 0\n",
        )
        .unwrap();

        let _guard = EnvGuard::set(
            MOUNTS_OVERRIDE_ENV,
            mount_file.to_string_lossy().into_owned(),
        );

        let result = find_mount_point(Path::new("/dev/sdb1")).unwrap();
        assert_eq!(result, Some(PathBuf::from("/media/lockchain")));
    }

    #[test]
    fn unescape_mount_field_decodes_octals() {
        assert_eq!(
            unescape_mount_field("/media/LOCK\\040CHAIN"),
            "/media/LOCK CHAIN"
        );
        assert_eq!(unescape_mount_field("/mnt/keys"), "/mnt/keys");
    }
}
