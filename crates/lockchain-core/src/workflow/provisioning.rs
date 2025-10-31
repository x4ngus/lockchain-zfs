//! Provisioning workflow that wipes, seeds, and configures the USB key token.

use super::{event, WorkflowEvent, WorkflowLevel, WorkflowReport};
use crate::config::{LockchainConfig, Usb};
use crate::error::{LockchainError, LockchainResult};
use crate::keyfile::write_raw_key_file;
use crate::provider::ZfsProvider;
use pbkdf2::pbkdf2_hmac;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

const LOCKCHAIN_LABEL: &str = "LOCKCHAINKEY";
const DEFAULT_MOUNTPOINT: &str = "/run/lockchain";
const DEFAULT_KEY_FILENAME: &str = "lockchain.key";
const PARTED_BINARIES: &[&str] = &["/sbin/parted", "/usr/sbin/parted", "/usr/bin/parted"];
const MKFS_BINARIES: &[&str] = &[
    "/sbin/mkfs.ext4",
    "/usr/sbin/mkfs.ext4",
    "/usr/bin/mkfs.ext4",
];
const BLKID_BINARIES: &[&str] = &["/sbin/blkid", "/usr/sbin/blkid", "/usr/bin/blkid"];
const LSBLK_BINARIES: &[&str] = &["/bin/lsblk", "/usr/bin/lsblk"];
const UDEVADM_BINARIES: &[&str] = &["/sbin/udevadm", "/usr/sbin/udevadm", "/usr/bin/udevadm"];
const MOUNT_BINARIES: &[&str] = &["/bin/mount", "/usr/bin/mount"];
const UMOUNT_BINARIES: &[&str] = &["/bin/umount", "/usr/bin/umount"];
const DRACUT_BINARIES: &[&str] = &["/usr/bin/dracut", "/usr/sbin/dracut"];
const UPDATE_INITRAMFS_BINARIES: &[&str] = &["/usr/sbin/update-initramfs"];
const LSINITRD_BINARIES: &[&str] = &["/usr/bin/lsinitrd", "/bin/lsinitrd"];

/// Determines whether provisioning wipes the token or leaves it intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeMode {
    Standard,
    Safe,
}

/// Caller-provided knobs that influence USB selection, mounting, and post-work.
#[derive(Debug, Clone)]
pub struct ProvisionOptions {
    pub usb_device: Option<String>,
    pub mountpoint: Option<PathBuf>,
    pub key_filename: Option<String>,
    pub passphrase: Option<String>,
    pub force_wipe: bool,
    pub rebuild_initramfs: bool,
}

impl Default for ProvisionOptions {
    fn default() -> Self {
        Self {
            usb_device: None,
            mountpoint: None,
            key_filename: None,
            passphrase: None,
            force_wipe: false,
            rebuild_initramfs: true,
        }
    }
}

/// Prepare the USB token, generate new key material, and refresh integration assets.
pub fn forge_key<P: ZfsProvider + Clone>(
    config: &mut LockchainConfig,
    provider: &P,
    dataset: &str,
    mode: ForgeMode,
    mut options: ProvisionOptions,
) -> LockchainResult<WorkflowReport> {
    let mut events = Vec::new();

    if !config.contains_dataset(dataset) {
        return Err(LockchainError::DatasetNotConfigured(dataset.to_string()));
    }

    let encryption_root = provider.encryption_root(dataset)?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Encryption root resolved to {encryption_root}"),
    ));

    let locked_descendants = provider.locked_descendants(&encryption_root)?;
    if locked_descendants.iter().any(|ds| ds == &encryption_root) {
        return Err(LockchainError::Provider(format!(
            "encryption root {encryption_root} is still locked; unlock before forging a new key"
        )));
    }

    let usb_device = resolve_usb_device(&options, config)?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Using USB device {usb_device}"),
    ));

    let (usb_disk, usb_partition) = derive_device_layout(&usb_device)?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Disk {usb_disk} partition {usb_partition} selected"),
    ));

    let safe_mode = matches!(mode, ForgeMode::Safe);

    if options.force_wipe || !safe_mode {
        wipe_usb_token(&usb_disk, &usb_partition)?;
        events.push(event(
            WorkflowLevel::Success,
            format!(
                "Reinitialised {} with label {}",
                usb_partition, LOCKCHAIN_LABEL
            ),
        ));
    } else {
        ensure_partition_label(&usb_partition)?;
        events.push(event(
            WorkflowLevel::Info,
            format!(
                "Safe mode: existing filesystem on {} validated for label {}",
                usb_partition, LOCKCHAIN_LABEL
            ),
        ));
    }

    settle_udev()?;

    let mountpoint = options
        .mountpoint
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MOUNTPOINT));
    let filename = options
        .key_filename
        .clone()
        .unwrap_or_else(|| DEFAULT_KEY_FILENAME.to_string());
    let key_path = mountpoint.join(&filename);

    fs::create_dir_all(&mountpoint)?;

    let mount_guard = MountGuard::mount(&usb_partition, &mountpoint)?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Mounted {} at {}", usb_partition, mountpoint.display()),
    ));

    let mut key_material = vec![0u8; 32];
    OsRng.fill_bytes(&mut key_material);
    write_raw_key_file(&key_path, &key_material)?;
    events.push(event(
        WorkflowLevel::Success,
        format!("Wrote key material to {}", key_path.display()),
    ));

    let digest = hex::encode(Sha256::digest(&key_material));

    mount_guard.sync()?; // flush writes before unmount
    drop(mount_guard); // unmount

    configure_fallback_passphrase(
        &mut events,
        config,
        options.passphrase.take(),
        &key_material,
    )?;

    let device_uuid = detect_partition_uuid(&usb_partition).ok().flatten();

    update_config(
        config,
        dataset,
        key_path.clone(),
        digest.clone(),
        device_uuid,
    )?;
    events.push(event(
        WorkflowLevel::Info,
        format!(
            "Config updated with key location {} and checksum {}",
            key_path.display(),
            digest
        ),
    ));

    install_dracut_module(&key_path, Some(&digest), &mut events)?;
    if options.rebuild_initramfs {
        rebuild_initramfs(&mut events)?;
        audit_initramfs(&mut events)?;
    } else {
        events.push(event(
            WorkflowLevel::Warn,
            "Initramfs rebuild skipped (rebuild=false). Ensure loader assets are regenerated manually.",
        ));
    }

    Ok(WorkflowReport {
        title: format!("Forged new key for {dataset}"),
        events,
    })
}

/// Determine which block device to operate on, using CLI options or config hints.
fn resolve_usb_device(
    options: &ProvisionOptions,
    config: &LockchainConfig,
) -> LockchainResult<String> {
    if let Some(device) = options.usb_device.as_ref() {
        return Ok(device.clone());
    }
    if let Some(label) = config.usb.device_label.as_ref() {
        if let Some(device) = device_from_label(label)? {
            return Ok(device);
        }
    }
    Err(LockchainError::InvalidConfig(
        "usb device not specified; pass device=/dev/sdX".to_string(),
    ))
}

/// Probe blkid for a device matching the requested filesystem label.
fn device_from_label(label: &str) -> LockchainResult<Option<String>> {
    for candidate in BLKID_BINARIES {
        if Path::new(candidate).exists() {
            let output = Command::new(candidate)
                .args(["-L", label])
                .stderr(Stdio::null())
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let device = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !device.is_empty() {
                        return Ok(Some(device));
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Work out the disk/partition pair we should operate on for the target device.
fn derive_device_layout(device: &str) -> LockchainResult<(String, String)> {
    let device_path = Path::new(device);
    if !device_path.exists() {
        return Err(LockchainError::InvalidConfig(format!(
            "device {device} not found"
        )));
    }

    let block_type = query_block_info(device, "TYPE")?;
    match block_type.as_str() {
        "disk" => {
            if let Some(existing) = existing_partition_for_disk(device)? {
                Ok((device.to_string(), existing))
            } else {
                Ok((device.to_string(), predict_partition_name(device)))
            }
        }
        "part" => {
            let parent = query_block_info(device, "PKNAME")?;
            if parent.is_empty() {
                Err(LockchainError::InvalidConfig(format!(
                    "unable to resolve parent disk for {device}"
                )))
            } else {
                Ok((format!("/dev/{}", parent.trim()), device.to_string()))
            }
        }
        other => Err(LockchainError::InvalidConfig(format!(
            "unsupported block type {other} for {device}"
        ))),
    }
}

/// Run `lsblk` for a single field and normalise the output.
fn query_block_info(device: &str, field: &str) -> LockchainResult<String> {
    let args = vec![
        OsString::from("-no"),
        OsString::from(field),
        OsString::from(device),
    ];
    let output = run_external(LSBLK_BINARIES, &args)?;
    if !output.status.success() {
        return Err(LockchainError::Provider(format!(
            "lsblk -no {} {} failed: {}",
            field,
            device,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Look for an existing partition on `disk` we can reuse.
fn existing_partition_for_disk(disk: &str) -> LockchainResult<Option<String>> {
    let args = vec![
        OsString::from("-P"),
        OsString::from("-nrpo"),
        OsString::from("PATH,TYPE"),
        OsString::from(disk),
    ];
    let output = run_external(LSBLK_BINARIES, &args)?;
    if !output.status.success() {
        return Ok(None);
    }

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut path = None;
        let mut kind = None;
        for part in line.split_whitespace() {
            if let Some((key, value)) = part.split_once('=') {
                let trimmed = value.trim_matches('"').to_string();
                match key {
                    "PATH" => path = Some(trimmed),
                    "TYPE" => kind = Some(trimmed),
                    _ => {}
                }
            }
        }
        if matches!(kind.as_deref(), Some("part")) {
            if let Some(path) = path {
                if Path::new(&path).exists() {
                    return Ok(Some(path));
                }
            }
        }
    }

    Ok(None)
}

/// Predict the first partition path a fresh GPT layout will produce.
fn predict_partition_name(disk: &str) -> String {
    let suffix_is_digit = Path::new(disk)
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.chars().last())
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);

    if suffix_is_digit {
        format!("{disk}p1")
    } else {
        format!("{disk}1")
    }
}

/// Verify a partition already bears the expected Lockchain filesystem label.
fn ensure_partition_label(partition: &str) -> LockchainResult<()> {
    let args = vec![
        OsString::from("-s"),
        OsString::from("LABEL"),
        OsString::from("-o"),
        OsString::from("value"),
        OsString::from(partition),
    ];
    let output = run_external(BLKID_BINARIES, &args)?;
    if !output.status.success() {
        return Err(LockchainError::InvalidConfig(format!(
            "unable to read label for {partition}: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let label = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if label != LOCKCHAIN_LABEL {
        return Err(LockchainError::InvalidConfig(format!(
            "partition {partition} bears label {label}; expected {LOCKCHAIN_LABEL}"
        )));
    }
    Ok(())
}

/// Repartition and format the USB device with a fresh ext4 filesystem.
fn wipe_usb_token(disk: &str, partition: &str) -> LockchainResult<()> {
    dismantle_mounts(disk)?;
    dismantle_mounts(partition)?;

    run_external(
        PARTED_BINARIES,
        &[
            OsString::from("-s"),
            OsString::from(disk),
            OsString::from("mklabel"),
            OsString::from("gpt"),
        ],
    )?;
    run_external(
        PARTED_BINARIES,
        &[
            OsString::from("-s"),
            OsString::from(disk),
            OsString::from("mkpart"),
            OsString::from("LOCKCHAIN_PART"),
            OsString::from("ext4"),
            OsString::from("1MiB"),
            OsString::from("100%"),
        ],
    )?;
    settle_udev()?;
    run_external(
        MKFS_BINARIES,
        &[
            OsString::from("-F"),
            OsString::from("-L"),
            OsString::from(LOCKCHAIN_LABEL),
            OsString::from(partition),
        ],
    )?;
    Ok(())
}

/// Give udev time to notice the new partition layout before we continue.
fn settle_udev() -> LockchainResult<()> {
    let result = run_external(UDEVADM_BINARIES, &[OsString::from("settle")]);
    if let Err(err) = result {
        return Err(LockchainError::Provider(format!(
            "udevadm settle failed: {err}"
        )));
    }
    Ok(())
}

/// Unmount any existing mountpoints tied to `target`.
fn dismantle_mounts(target: &str) -> LockchainResult<()> {
    let output = Command::new("lsblk")
        .args(["-nrpo", "NAME,MOUNTPOINT", target])
        .output()
        .map_err(|err| LockchainError::Provider(err.to_string()))?;

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split_whitespace();
        let name = parts.next();
        let mount = parts.next();
        if let (Some(_name), Some(mountpoint)) = (name, mount) {
            if !mountpoint.trim().is_empty() {
                run_external(UMOUNT_BINARIES, &[OsString::from(mountpoint)])?;
            }
        }
    }
    Ok(())
}

/// Capture the partition UUID so the daemon can detect the token later.
fn detect_partition_uuid(partition: &str) -> LockchainResult<Option<String>> {
    let args = vec![
        OsString::from("-s"),
        OsString::from("UUID"),
        OsString::from("-o"),
        OsString::from("value"),
        OsString::from(partition),
    ];
    let output = run_external(BLKID_BINARIES, &args)?;
    if !output.status.success() {
        return Ok(None);
    }
    let uuid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uuid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(uuid))
    }
}

/// Optionally seed fallback passphrase material based on supplied input.
fn configure_fallback_passphrase(
    events: &mut Vec<WorkflowEvent>,
    config: &mut LockchainConfig,
    passphrase: Option<String>,
    key_material: &[u8],
) -> LockchainResult<()> {
    if let Some(passphrase) = passphrase {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);

        let mut derived = Zeroizing::new(vec![0u8; key_material.len()]);
        pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), &salt, 250_000, &mut derived);

        let xor: Vec<u8> = key_material
            .iter()
            .zip(derived.iter())
            .map(|(a, b)| a ^ b)
            .collect();

        config.fallback.enabled = true;
        config.fallback.passphrase_salt = Some(hex::encode(salt));
        config.fallback.passphrase_xor = Some(hex::encode(xor));
        config.fallback.passphrase_iters = 250_000;
        events.push(event(
            WorkflowLevel::Security,
            "Fallback passphrase material generated.",
        ));
    } else {
        config.fallback.enabled = false;
        config.fallback.passphrase_salt = None;
        config.fallback.passphrase_xor = None;
        events.push(event(WorkflowLevel::Info, "Fallback passphrase disabled."));
    }
    Ok(())
}

/// Persist the new key metadata and sane defaults back into the config file.
fn update_config(
    config: &mut LockchainConfig,
    dataset: &str,
    key_path: PathBuf,
    checksum: String,
    device_uuid: Option<String>,
) -> LockchainResult<()> {
    if !config.policy.datasets.iter().any(|entry| entry == dataset) {
        config.policy.datasets.push(dataset.to_string());
    }

    let file_name = key_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(DEFAULT_KEY_FILENAME)
        .to_string();
    config.usb = Usb {
        key_hex_path: key_path.to_string_lossy().into_owned(),
        expected_sha256: Some(checksum),
        device_label: Some(LOCKCHAIN_LABEL.to_string()),
        device_uuid,
        device_key_path: file_name,
        mount_timeout_secs: config.usb.mount_timeout_secs.max(10),
    };

    if config.policy.binary_path.is_none() {
        config.policy.binary_path = Some("/usr/bin/lockchain-cli".to_string());
    }

    if config.fallback.askpass_path.is_none() {
        config.fallback.askpass_path = Some("/usr/bin/systemd-ask-password".to_string());
    }

    config.save()?;
    Ok(())
}

/// Scratch wrapper around `std::process::Output` for external command wrappers.
struct CommandOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: std::process::ExitStatus,
}

/// Try each binary in `candidates` until one executes successfully.
fn run_external(candidates: &[&str], args: &[OsString]) -> LockchainResult<CommandOutput> {
    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() {
            let output = Command::new(candidate)
                .args(args)
                .output()
                .map_err(|err| LockchainError::Provider(err.to_string()))?;
            return Ok(CommandOutput {
                stdout: output.stdout,
                stderr: output.stderr,
                status: output.status,
            });
        }
    }
    Err(LockchainError::Provider(format!(
        "none of {:?} are available on this system",
        candidates
    )))
}

/// RAII helper that unmounts the USB device when dropped.
struct MountGuard {
    mountpoint: PathBuf,
}

impl MountGuard {
    /// Mount the partition and return a guard that unmounts on drop.
    fn mount(partition: &str, mountpoint: &Path) -> LockchainResult<Self> {
        let mountpoint_str = mountpoint.to_string_lossy().into_owned();
        run_external(
            MOUNT_BINARIES,
            &[
                OsString::from("-o"),
                OsString::from("defaults"),
                OsString::from(partition),
                OsString::from(mountpoint_str),
            ],
        )?;
        Ok(Self {
            mountpoint: mountpoint.to_path_buf(),
        })
    }

    /// Flush pending writes to disk before unmounting.
    fn sync(&self) -> LockchainResult<()> {
        if let Err(err) = Command::new("sync").status() {
            return Err(LockchainError::Provider(err.to_string()));
        }
        Ok(())
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        let _ = run_external(
            UMOUNT_BINARIES,
            &[OsString::from(
                self.mountpoint.to_string_lossy().into_owned(),
            )],
        );
    }
}

/// Stage the dracut hook and systemd drop-ins that load the key during boot.
fn install_dracut_module(
    key_path: &Path,
    checksum: Option<&str>,
    events: &mut Vec<WorkflowEvent>,
) -> LockchainResult<()> {
    let ctx = DracutContext {
        mountpoint: key_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| DEFAULT_MOUNTPOINT.to_string()),
        key_path: key_path.to_string_lossy().into_owned(),
        checksum: checksum.map(|s| s.to_string()),
    };
    let module = DracutModule::install(&ctx)?;
    events.push(event(
        WorkflowLevel::Info,
        format!("Dracut module installed at {}", module.root.display()),
    ));
    Ok(())
}

/// Run whichever initramfs tool is available to pick up the new hook.
fn rebuild_initramfs(events: &mut Vec<WorkflowEvent>) -> LockchainResult<()> {
    if let Ok(_) = run_external(DRACUT_BINARIES, &[OsString::from("-f")]) {
        events.push(event(WorkflowLevel::Success, "Dracut rebuild completed."));
        return Ok(());
    }

    if let Ok(_) = run_external(UPDATE_INITRAMFS_BINARIES, &[OsString::from("-u")]) {
        events.push(event(
            WorkflowLevel::Success,
            "update-initramfs rebuild completed.",
        ));
        return Ok(());
    }

    Err(LockchainError::Provider(
        "neither dracut nor update-initramfs available".into(),
    ))
}

/// Inspect the generated initramfs to ensure our assets were included.
fn audit_initramfs(events: &mut Vec<WorkflowEvent>) -> LockchainResult<()> {
    for candidate in LSINITRD_BINARIES {
        if Path::new(candidate).exists() {
            let output = Command::new(candidate)
                .output()
                .map_err(|err| LockchainError::Provider(err.to_string()))?;
            if !output.status.success() {
                continue;
            }
            let manifest = String::from_utf8_lossy(&output.stdout);
            let missing = [
                "lockchain-load-key",
                "zfs-load-key.service.d/lockchain.conf",
            ];
            let mut absent = Vec::new();
            for needle in missing {
                if !manifest.contains(needle) {
                    absent.push(needle);
                }
            }
            if absent.is_empty() {
                events.push(event(
                    WorkflowLevel::Success,
                    "Initramfs audit confirmed lockchain loader assets are present.",
                ));
            } else {
                events.push(event(
                    WorkflowLevel::Warn,
                    format!("Initramfs audit missing assets: {}", absent.join(", ")),
                ));
            }
            return Ok(());
        }
    }
    events.push(event(
        WorkflowLevel::Warn,
        "lsinitrd not available; unable to audit initramfs contents.",
    ));
    Ok(())
}

/// Details required to render the dracut hook for this deployment.
struct DracutContext {
    mountpoint: String,
    key_path: String,
    checksum: Option<String>,
}

/// Represents the installed dracut module directory.
struct DracutModule {
    root: PathBuf,
}

impl DracutModule {
    /// Materialise the module files onto disk using the provided context.
    fn install(ctx: &DracutContext) -> LockchainResult<Self> {
        let module = determine_module_dir();
        fs::create_dir_all(&module)?;

        let script = module.join("lockchain-load-key.sh");
        let service = module.join("lockchain-load-key.service");
        let dropin_key_dir = module.join("zfs-load-key.service.d");
        let dropin_module_dir = module.join("zfs-load-module.service.d");
        let dropin_key = dropin_key_dir.join("lockchain.conf");
        let dropin_module = dropin_module_dir.join("lockchain.conf");
        let setup = module.join("module-setup.sh");

        fs::create_dir_all(&dropin_key_dir)?;
        fs::create_dir_all(&dropin_module_dir)?;

        write_template(&script, LOCKCHAIN_LOAD_KEY_TEMPLATE, ctx, 0o750)?;
        write_template(&service, LOCKCHAIN_SERVICE_TEMPLATE, ctx, 0o644)?;
        write_template(&dropin_key, LOCKCHAIN_DROPIN_TEMPLATE, ctx, 0o644)?;
        write_template(&dropin_module, LOCKCHAIN_ZFS_DROPIN_TEMPLATE, ctx, 0o644)?;
        write_template(&setup, LOCKCHAIN_MODULE_SETUP_TEMPLATE, ctx, 0o750)?;

        Ok(Self { root: module })
    }
}

/// Pick a sane destination directory for the dracut module.
fn determine_module_dir() -> PathBuf {
    let candidates = [
        PathBuf::from("/usr/lib/dracut/modules.d/90lockchain"),
        PathBuf::from("/lib/dracut/modules.d/90lockchain"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }

    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/usr/lib/dracut/modules.d/90lockchain"))
}

/// Render a template to disk with executable or config permissions as needed.
fn write_template(
    path: &Path,
    template: &str,
    ctx: &DracutContext,
    mode: u32,
) -> LockchainResult<()> {
    let rendered = template
        .replace("{{TOKEN_LABEL}}", LOCKCHAIN_LABEL)
        .replace("{{MOUNTPOINT}}", &ctx.mountpoint)
        .replace("{{KEY_PATH}}", &ctx.key_path)
        .replace(
            "{{KEY_SHA256}}",
            ctx.checksum.clone().unwrap_or_default().as_str(),
        )
        .replace("{{SERVICE_NAME}}", "lockchain-load-key.service")
        .replace("{{SCRIPT_NAME}}", "lockchain-load-key.sh")
        .replace("{{DROPIN_NAME}}", "lockchain.conf")
        .replace("{{DROPIN_DIR}}", "zfs-load-key.service.d")
        .replace("{{MODULE_DROPIN_DIR}}", "zfs-load-module.service.d")
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"));

    fs::write(path, rendered)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

const LOCKCHAIN_LOAD_KEY_TEMPLATE: &str = include_str!("../../templates/lockchain-load-key.sh");
const LOCKCHAIN_SERVICE_TEMPLATE: &str = include_str!("../../templates/lockchain-load-key.service");
const LOCKCHAIN_DROPIN_TEMPLATE: &str = include_str!("../../templates/lockchain-load-key.conf");
const LOCKCHAIN_ZFS_DROPIN_TEMPLATE: &str =
    include_str!("../../templates/lockchain-zfs-load-module.conf");
const LOCKCHAIN_MODULE_SETUP_TEMPLATE: &str =
    include_str!("../../templates/lockchain-module-setup.sh");
