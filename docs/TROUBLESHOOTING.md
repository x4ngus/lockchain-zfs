# Troubleshooting Field Manual

This manual aggregates the most common operational issues encountered during deployments. Each scenario lists observable symptoms, root-cause checks, and commands to restore service. Keep logs in JSON for automated analysis, but feel free to switch to plain text when following the steps below.

---

## 1. Unlock Workflow Fails Immediately

**Symptoms**
- `lockchain unlock` returns `[LC5001] provider error`
- Systemd units report `Failed to load key` on boot

**Checks**
1. Confirm the dataset is listed in `policy.datasets`.
2. Verify the USB key file exists and has `0400` permissions:
   ```bash
  ls -l /run/lockchain/key.hex
   ```
3. Run the doctor workflow for a consolidated report:
   ```bash
   lockchain doctor
   ```

**Remediation**
- If the key file is missing, reinsert the USB stick and ensure `lockchain-key-usb` is active.
- If the checksum mismatches, update `usb.expected_sha256` after validating the key material.

---

## 2. USB Key Not Detected

**Symptoms**
- `lockchain-key-usb` logs show `device ... skipped`
- Doctor workflow warns about missing UUID or label

**Checks**
1. Inspect udev properties for the inserted device:
   ```bash
   udevadm info --query=property --name=/dev/sdX1
   ```
2. Confirm `usb.device_label` or `usb.device_uuid` in config matches reality.

**Remediation**
- Update configuration to reflect the label/UUID reported by udev.
- If the device requires mounting, ensure `mount_timeout_secs` allows enough time for the filesystem to appear.

---

## 3. Self-Test Cannot Build Ephemeral Pool

**Symptoms**
- `lockchain self-test` emits `[LC5300]` errors about missing binaries

**Checks**
1. Confirm `zfs` and `zpool` binaries exist at the configured paths.
2. Run:
   ```bash
   which zfs zpool
   ```

**Remediation**
- Install `zfsutils-linux` and ensure the config points to the binaries.
- Verify the user running the test has permissions to create loopback devices (may require `sudo` on certain hosts).

---

## 4. Systemd Units Report `inactive (dead)`

**Symptoms**
- `systemctl status lockchain-zfs.service` shows the unit exiting immediately

**Checks**
1. View logs:
   ```bash
   journalctl -u lockchain-zfs.service --no-pager
   ```
2. Validate configuration syntax:
   ```bash
   lockchain validate -f /etc/lockchain-zfs.toml
   ```

**Remediation**
- Address configuration validation errors.
- Run `lockchain repair` to reinstall and enable mount/unlock units.
- Reload systemd units after changes:
  ```bash
  sudo systemctl daemon-reload
  sudo systemctl restart lockchain-zfs.service
  ```

---

## 5. Break-Glass Output Not Accepted by ZFS

**Symptoms**
- Manually invoked `zfs load-key` rejects the derived key with `key incorrect`

**Checks**
1. Ensure the fallback configuration includes both `passphrase_salt` and `passphrase_xor`.
2. Confirm the passphrase used matches the documented recovery phrase.

**Remediation**
- Re-run `lockchain breakglass` and verify the prompts are satisfied exactly.
- Rotate fallback material (rerun provisioning in safe mode) if the configuration is stale.

---

## 6. Need Human-Readable Logs Quickly

Use environment overrides to flip logging format without code changes:
```bash
export LOCKCHAIN_LOG_FORMAT=plain
lockchain unlock ...
```
Remember to reset to JSON afterwards to keep pipeline ingestion consistent.

---

If the issue persists beyond these scenarios, capture the command output, relevant log entries, and configuration snippets, then open an issue or start a discussion. For security-sensitive findings, always follow the process in [`docs/SECURITY.md`](SECURITY.md). Keep the glow, keep the rigor.
