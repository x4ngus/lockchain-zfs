<div align="center">

# LockChain ZFS  
_Security lighting rig for encrypted storage._

</div>

## System Overview

- **Objective**: Deliver a unique unlock workflow for encrypted ZFS datasets.  
- **Surfaces**: CLI, daemon, UI, and USB watcher all route through the same workflow engine.  
- **Status**: ![CI (Ubuntu 25.10+, Rust)][def] · Rust 2021

LockChain allows an improved interaction with encrypted file system, without trivialising your security posture.

## Quick Start Guide

```bash
# 1. Fetch the repo and toolchains
git clone https://github.com/x4ngus/lockchain-zfs.git
cd lockchain-zfs

# 2. Run the glow-test (fake zfs provider, no root)
cargo test -p lockchain-zfs --test unlock_smoke

# 3. Stage a config the team can read
sudo install -Dm640 packaging/systemd/lockchain-zfs.toml /etc/lockchain-zfs.toml

# 4. Trigger the unlock sequence
cargo run -p lockchain-cli -- unlock
```

For a full control room perspective, point the Control Deck (`lockchain-ui`) at the same config or have `lockchain-key-usb` enforce key presence.
Follow up with `lockchain doctor` or `lockchain repair` to install the mount/unlock units and refresh system dependencies on your host.

## Module Lineup

| Module | Purpose | Notes |
| --- | --- | --- |
| `lockchain-core` | Policy engine, workflow orchestration, ZFS provider contract | Houses keyfile guards, checksum enforcement, JSON logging bootstrap |
| `lockchain-zfs` | System provider using native `zfs`/`zpool` binaries | Maps exit codes, parses stdout, backs the unlock smoke test |
| `lockchain-cli` | Operator console (unlock/status/list/validate/breakglass) | Structured error codes for SIEM correlation (`LCxxxx`) |
| `lockchain-key-usb` | udev watcher & key normaliser | Detects label/UUID, rewrites legacy hex → raw, mirrors to `/run/lockchain/` |
| `lockchain-daemon` | Long-running safety net | Watches USB, retries unlocks, runs health responder (`127.0.0.1:8787`) |
| `lockchain-ui` | Iced Control Deck | Keyboard-first TUI with directives for forge, self-test, doctor |
| `docs/adr` | Architecture Decisions | ADR-001 captures the provider strategy |

## Configuration Blueprint

```toml
[policy]
datasets = ["rpool/ROOT/blackice"]
zfs_path = "/sbin/zfs"
zpool_path = "/sbin/zpool"

[crypto]
timeout_secs = 10

[usb]
key_hex_path = "/run/lockchain/key.hex"
expected_sha256 = "optional sha256 of the decoded raw key"
device_label = "LOCKCHAIN"
# device_uuid = "optional blkid UUID"
device_key_path = "key.hex"
mount_timeout_secs = 10

[fallback]
enabled = true
askpass = true
askpass_path = "/usr/bin/systemd-ask-password"
passphrase_salt = "hex salt emitted during init"
passphrase_xor = "hex xor blob emitted during init"
passphrase_iters = 250000

[retry]
max_attempts = 3
base_delay_ms = 500
max_delay_ms = 5000
jitter_ratio = 0.1
```

**Environment Overrides**

| Variable | Intent | Effect |
| --- | --- | --- |
| `LOCKCHAIN_KEY_PATH` | Point to alternate key material | Overrides `usb.key_hex_path`. |
| `LOCKCHAIN_LOG_LEVEL` | Adjust verbosity | Default log filter (`info`). |
| `LOCKCHAIN_LOG_FORMAT` | Switch between JSON/plain logs | `json` (default) or `plain`. |
| `LOCKCHAIN_KEY_USB_MOUNTS_PATH` | Provide a mounts fixture for testing | Feeds the USB watcher with synthetic data. |
| `LOCKCHAIN_CONFIG` | Run a surface against a different config | Daemon + watcher default to `/etc/lockchain-zfs.toml`. |
| `LOCKCHAIN_HEALTH_ADDR` | Rebind the daemon health endpoint | Default `127.0.0.1:8787`. |

## Console Commands

- `lockchain init --dataset <ds>` — forge or refresh the USB token, rebuild dracut, and capture checksum updates.  
- `lockchain doctor` — run diagnostics with automatic remediation for config, systemd, and initramfs.  
- `lockchain repair` — reinstall/enable mount and unlock units when doctor suggests manual action.  
- `lockchain unlock --strict-usb` — require the vault stick; no silent fallbacks.  
- `lockchain self-test` — exercise an ephemeral pool to prove the current key still opens the vault.  
- `lockchain unlock --prompt-passphrase` — partner with `systemd-ask-password` when policy allows.  
- `lockchain status` — live keystatus for every dataset in `policy.datasets`.  
- `lockchain list-keys` — report encryption roots vs. datasets.  
- `lockchain-key-usb` — enforce USB insertion/removal rules, heal legacy key files.  
- `lockchain tui` — keyboard-only Control Deck for datasets, retries, and passphrases.  
- `lockchain validate -f /path/to/config` — static validator; `--schema` exports the JSON schema.  
- `lockchain-daemon` — schedule unlock attempts, stream health, surface warnings.  

All surfaces emit machine-readable error codes prefixed with `LC`, making SOC integration straightforward.

## Build & Quality Gates

- `cargo test -p lockchain-core` — keyfile, workflow, and fallback coverage.  
- `cargo test -p lockchain-zfs` — unlock smoke test with fake binaries.  
- `cargo test -p lockchain-key-usb` — requires `libudev-dev`.  
- `cargo fmt && cargo clippy --all-targets` — routine hygiene.  
- Packaging pipeline (`.github/workflows/release.yml`) builds signed `.deb` releases on Ubuntu 25.10+.

## Further Reading

- [`docs/INSTALL.md`](docs/INSTALL.md) — deployment runbook for operations and platform teams.  
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — component map and integration touchpoints.  
- [`docs/SECURITY.md`](docs/SECURITY.md) — hardening posture, disclosure process, break-glass guardrails.  
- [`docs/RELEASE.md`](docs/RELEASE.md) — how we ship signed packages.  
- [`docs/adr/ADR-001-module-provider.md`](docs/adr/ADR-001-module-provider.md) — strategy memo on the provider abstraction.

[def]: https://github.com/x4ngus/lockchain-zfs/actions/workflows/ci.yml/badge.svg
