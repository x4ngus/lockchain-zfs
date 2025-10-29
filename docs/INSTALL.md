# Installing LockChain ZFS

Welcome to the deployment runbook. These steps assume a Debian/Ubuntu server with root access and `systemd`.

## 1. Build or Install Packages

### Option A — From Source

```bash
git clone https://github.com/lockchain-org/lockchain-zfs.git
cd lockchain-zfs
cargo build --release
sudo install -Dm755 target/release/lockchain-cli /usr/bin/lockchain-cli
sudo install -Dm755 target/release/lockchain-daemon /usr/bin/lockchain-daemon
sudo install -Dm755 target/release/lockchain-key-usb /usr/bin/lockchain-key-usb
```

### Option B — Debian Package

Use `cargo deb` or grab the published `.deb` and run `sudo dpkg -i lockchain-zfs_*.deb`.

## 2. Configuration File

Create `/etc/lockchain-zfs.toml` and tailor it to your datasets, USB vault label, and checksum.

```bash
sudo install -Dm640 /dev/null /etc/lockchain-zfs.toml
sudo ${EDITOR:-vi} /etc/lockchain-zfs.toml
```

Essential fields to populate:

- `policy.datasets`: list every dataset you want unlocked
- `usb.device_label` or `usb.device_uuid`: identifies the vault stick
- `usb.expected_sha256`: checksum of the raw 32-byte key
- `retry.*`: optional tweaks for the exponential backoff used by the daemon/CLI (defaults: 3 attempts, 500ms base delay, 0.1 jitter)

Refer back to the README’s “Configuration Blueprint” for the full schema.

## 3. System User & Permissions

The services run as the dedicated `lockchain` user. The helper script (or Debian package postinst) will create it automatically, but you can double-check:

```bash
sudo id lockchain || sudo useradd --system --home /var/lib/lockchain --shell /usr/sbin/nologin lockchain
sudo install -d -o lockchain -g lockchain /var/lib/lockchain
sudo chgrp lockchain /etc/lockchain-zfs.toml
sudo chmod 640 /etc/lockchain-zfs.toml
```

Grant the user only the ZFS operations it needs. You can either delegate via `zfs allow` or create a sudoers drop-in such as:

```
# /etc/sudoers.d/lockchain
lockchain ALL=(root) NOPASSWD:/usr/sbin/zfs load-key *, \
    /usr/sbin/zfs key -l *, \
    /usr/bin/lockchain-cli unlock *, \
    /usr/bin/lockchain-cli breakglass *
```

Adjust the command list to your environment and validate with `visudo -cf /etc/sudoers.d/lockchain`.

## 4. Systemd Units

The repo ships helper units that run the daemon, USB watcher, and dataset unlock flows.

```bash
cd lockchain-zfs
sudo packaging/install-systemd.sh
```

The script copies the units into `/etc/systemd/system/` and enables `lockchain-zfs.service` so the daemon starts on boot.

Enable unlock jobs for each dataset with the templated unit:

```bash
sudo systemctl enable lockchain-zfs@tank-secure.service
sudo systemctl enable lockchain-zfs@tank-working.service
```

These units run `lockchain-cli unlock --dataset <name>` during boot and remain in the `enabled` state for manual replays.

Finally reload and start everything:

```bash
sudo systemctl daemon-reload
sudo systemctl start lockchain-zfs.service
sudo systemctl start lockchain-zfs@tank-secure.service
```

Validate the configuration whenever you make changes:

```bash
lockchain validate -f /etc/lockchain-zfs.toml
```

## 4. USB Watcher (Optional but Recommended)

`lockchain-daemon` already integrates a placeholder watcher; for full USB enforcement, deploy the dedicated watcher and point it at the same configuration file.

```bash
sudo install -Dm755 target/release/lockchain-key-usb /usr/bin/lockchain-key-usb
sudo systemctl enable --now lockchain-key-usb.service  # create your own unit or reuse packaging/systemd definitions as they land
```

## 5. Verifying the install

```bash
# health endpoint (default 127.0.0.1:8787)
curl -s http://127.0.0.1:8787

# tail logs (JSON by default)
sudo journalctl -u lockchain-zfs.service -f
```

If the response is `OK`, the daemon is happy. Use `LOCKCHAIN_HEALTH_ADDR` to bind to a different interface if required.

## 6. Removal

Disable the services and remove the binaries/config when you’re ready to decommission:

```bash
sudo systemctl disable lockchain-zfs.service lockchain-zfs@*.service
sudo rm /usr/bin/lockchain-{cli,daemon,key-usb}
sudo rm /etc/lockchain-zfs.toml
```

Stay encrypted, stay neon.
