# Installation Playbook

This runbook distils the steps needed to deploy LockChain on Ubuntu/Debian hosts with `systemd` and ZFS tooling available.

---

## 1. Pre-flight Checks

1. Confirm the host has `zfsutils-linux` (or your distro’s equivalent).  
2. Make sure you have root or the ability to escalate once.  
3. Decide whether you’ll install from source or drop in the signed `.deb`.

Keep `/var/lib/lockchain` reserved for the service account; the packaging scripts create it, but it never hurts to verify.

## 2. Bring in the Bits

### Option A — Build from Source

```bash
git clone https://github.com/lockchain-org/lockchain-zfs.git
cd lockchain-zfs
cargo build --release
sudo install -Dm755 target/release/lockchain-cli /usr/bin/lockchain-cli
sudo install -Dm755 target/release/lockchain-daemon /usr/bin/lockchain-daemon
sudo install -Dm755 target/release/lockchain-key-usb /usr/bin/lockchain-key-usb
sudo install -Dm755 target/release/lockchain-ui /usr/bin/lockchain-ui
```

### Option B — Consume the Signed Package

```bash
curl -LO https://github.com/lockchain-org/lockchain-zfs/releases/latest/download/lockchain-zfs_0.1.9-1_amd64.deb
curl -LO https://github.com/lockchain-org/lockchain-zfs/releases/latest/download/lockchain-zfs_0.1.9-1_amd64.deb.asc
curl -LO https://github.com/lockchain-org/lockchain-zfs/releases/latest/download/SHA256SUMS
curl -LO https://github.com/lockchain-org/lockchain-zfs/releases/latest/download/SHA256SUMS.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum --check SHA256SUMS
sudo apt install ./lockchain-zfs_0.1.9-1_amd64.deb
```

Swap the version number when new releases ship. The installer creates the service user, pulls in systemd units, and nudges `update-initramfs`.

## 3. Wire the Configuration

The control file lives at `/etc/lockchain-zfs.toml`. Start from a clean slate:

```bash
sudo install -Dm640 /dev/null /etc/lockchain-zfs.toml
sudo chgrp lockchain /etc/lockchain-zfs.toml
sudo ${EDITOR:-vi} /etc/lockchain-zfs.toml
```

Populate the essentials:

- `policy.datasets` — every dataset you expect to unlock.  
- `usb.device_label` or `usb.device_uuid` — how we recognise the vault stick.  
- `usb.expected_sha256` — golden checksum of the raw 32-byte key.  
- `retry.*` — adjust patience for unlock retries (defaults: 3 attempts, 500 ms base, 5 s ceiling, 0.1 jitter).  
- `fallback.*` — only if you allow passphrase recovery; stash the `salt`/`xor` values from provisioning.

Validate early and often:

```bash
lockchain validate -f /etc/lockchain-zfs.toml
```

## 4. Provision or Refresh the USB Token

Use the CLI to normalise the USB token and refresh the initramfs templates:

```bash
sudo lockchain init --dataset tank/secure --device /dev/sdX1
sudo lockchain doctor
sudo lockchain repair
sudo lockchain self-test --dataset tank/secure --strict-usb
```

`lockchain init` wipes (or validates, when `--safe` is set) the token, writes fresh raw key material, configures fallback secrets, and installs the dracut module. `lockchain doctor` runs diagnostics and remediation, while `lockchain repair` reinstalls/enables the mount and unlock units if needed. Finish with `lockchain self-test` to prove the key can unlock an ephemeral pool before touching production datasets.

## 5. Lock Down Identity & Permissions

The `lockchain` user is the only account that should read the key material or run the services.

```bash
sudo id lockchain || sudo useradd --system --home /var/lib/lockchain --shell /usr/sbin/nologin lockchain
sudo install -d -o lockchain -g lockchain /var/lib/lockchain
sudo chgrp lockchain /etc/lockchain-zfs.toml
sudo chmod 640 /etc/lockchain-zfs.toml
```

Delegate the minimum ZFS verbs. Either use `zfs allow`:

```bash
sudo zfs allow lockchain load-key,key yourpool/encrypted
```

…or drop a narrowly scoped sudoers file:

```
# /etc/sudoers.d/lockchain
lockchain ALL=(root) NOPASSWD:/usr/sbin/zfs load-key *, \
    /usr/sbin/zfs key -l *, \
    /usr/bin/lockchain-cli unlock *, \
    /usr/bin/lockchain-cli breakglass *
```

Always validate with `visudo -cf /etc/sudoers.d/lockchain`.

## 6. Deploy the Services

The repo ships helper scripts and units; the Debian package installs them automatically. For source builds:

```bash
cd lockchain-zfs
sudo packaging/install-systemd.sh
```

Enable the core daemon and any dataset unlock templates:

```bash
sudo systemctl enable --now lockchain-zfs.service
sudo systemctl enable lockchain-zfs@tank-secure.service
sudo systemctl enable lockchain-zfs@tank-workload.service
```

(`lockchain repair` enables these units automatically, but the commands are shown here for clarity.)

Need USB event enforcement? Bring the watcher online:

```bash
sudo systemctl enable --now lockchain-key-usb.service
```

Reload systemd if you tweak units by hand:

```bash
sudo systemctl daemon-reload
```

## 7. Confidence Checks

### Health Endpoint

```bash
curl -s http://127.0.0.1:8787
```

Expect `OK`. Change the bind address with `LOCKCHAIN_HEALTH_ADDR`.

### Logs

```bash
sudo journalctl -u lockchain-zfs.service -f
sudo journalctl -u lockchain-key-usb.service -f
```

Logs default to JSON. Set `LOCKCHAIN_LOG_FORMAT=plain` if you want human-friendly output for troubleshooting.

### Workflow Smoke Test

Run the self-test from the Control Deck (Self-test directive) or via CLI:

```bash
lockchain self-test --dataset tank/secure --strict-usb
```

You should see `[OK] Self-test unlock` style messages confirming the path and a teardown notice at the end.

## 8. Maintenance & Removal

- Re-run `lockchain validate` after any config change.  
- Rotate the USB key? Forge a new one through the Control Deck or run `lockchain init` (see docs/workflow).  
- To uninstall, disable units and purge binaries/config:

```bash
sudo systemctl disable --now lockchain-zfs.service lockchain-key-usb.service
sudo apt remove lockchain-zfs           # or rm /usr/bin/lockchain-*
sudo rm -rf /var/lib/lockchain /etc/lockchain-zfs.toml
```

## 9. Operational Notes

- Signed packages and checksums back every deployment; verify before installation when operating under strict governance.  
- Use the Control Deck’s Doctor directive or `lockchain doctor` CLI workflow as part of acceptance testing.  
- Schedule periodic key self-tests (Control Deck or CLI) to prove the USB material still unlocks the pool.  
- Keep the glow subtle but present—consistent theming in terminals and UI helps operators quickly identify the LockChain surfaces.
