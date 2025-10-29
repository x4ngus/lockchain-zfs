![CI (Ubuntu 25.10+, Rust)][def]

<div align="center">

# LockChain ZFS  
_Neon-grade crypto orchestration for your datasets._

</div>

## Signal Beacon

- **Status:** ![CI (Ubuntu 25.10+, Rust)][def]  
- **Rust edition:** 2021  
- **Core coverage target:** ≥ 70% (`cargo tarpaulin --workspace --ignore-tests`)  
- **Docs:** cyberpunk-flavoured, but production ready. Start at [`docs/`](docs) or the ADR log.

## Night City Quickstart

```bash
# 1. Fetch the repo and toolchains
git clone https://github.com/x4ngus/lockchain-zfs.git
cd lockchain-zfs

# 2. Run the smoke test (simulated pool, no root required)
cargo test -p lockchain-zfs --test unlock_smoke

# 3. Wire up your config
sudo install -m 640 packaging/systemd/lockchain-zfs.toml /etc/lockchain-zfs.toml

# 4. Bring neon online
cargo run -p lockchain-cli -- unlock
```

Want richer UX? Plug in `lockchain-key-usb` to monitor the vault stick and pipe alerts through your own stack.

### Privilege Model

- All services ship with a dedicated `lockchain` user/group. Packaging helpers create it automatically (`packaging/install-systemd.sh`).
- Configuration files should be readable by that group (`sudo chgrp lockchain /etc/lockchain-zfs.toml && sudo chmod 640 …`).
- Delegate only the required ZFS verbs or add a minimal sudoers drop-in (see `docs/SECURITY.md`).
- Run `lockchain-ui` and any bespoke tooling as the `lockchain` user to avoid surprise sudo escalations.

## The Stack

| Module | Purpose | Notes |
| --- | --- | --- |
| `lockchain-core` | Config, service orchestration, provider contract | Houses the key loader, checksum enforcement, logging bootstrapper |
| `lockchain-zfs` | System provider riding the native `zfs`/`zpool` CLIs | Integration tests simulate a dev pool via Python stubs |
| `lockchain-cli` | Ops console for unlock/status/list flows | JSON logging toggle via `LOCKCHAIN_LOG_FORMAT` |
| `lockchain-key-usb` | udev listener + key normaliser | Watches for configured label/UUID, rewrites legacy hex → raw |
| `lockchain-daemon` | Long-running orchestrator (USB + unlock cadence + health) | Tokio service with `/health` endpoint, consumes `lockchain-core` + `lockchain-zfs` |
| `docs/adr` | Architecture Decision Records | ADR-001 explains the module/provider split |

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

- Leave `zfs_path`/`zpool_path` unset to auto-discover common locations.  
- `device_label` / `device_uuid` lock the USB watcher onto the correct token.  
- `device_key_path` identifies the payload inside the mounted volume.  
- `mount_timeout_secs` controls how long we wait for `/proc/mounts` to expose the stick after insertion.
- `retry` tunes exponential backoff for unlock attempts (daemon/TUI/CLI reuse the same policy).

**Environment overrides**

| Variable | Effect |
| --- | --- |
| `LOCKCHAIN_KEY_PATH` | Replaces `usb.key_hex_path` (handy for tests or when systemd drops the file elsewhere) |
| `LOCKCHAIN_LOG_LEVEL` | Default log filter (`info` unless overridden) |
| `LOCKCHAIN_LOG_FORMAT` | `json` (default) or `plain` |
| `LOCKCHAIN_KEY_USB_MOUNTS_PATH` | Test-only override for the mount table when driving the USB watcher |
| `LOCKCHAIN_CONFIG` | Path override used by the daemon and USB watcher (`/etc/lockchain-zfs.toml` default) |
| `LOCKCHAIN_HEALTH_ADDR` | Bind address for the daemon’s health endpoint (`127.0.0.1:8787`) |

## Operator Moves

- `lockchain unlock --strict-usb` — refuse fallback paths, require the vault stick.  
- `lockchain unlock --prompt-passphrase` — call out to `systemd-ask-password` using the configured XOR blob.  
- `lockchain status` — get the live keystatus for every dataset in `policy.datasets`.  
- `lockchain list-keys` — friendly table showing dataset ↔ encryption root ↔ status.  
- `lockchain-key-usb` — run as a service to enforce USB key rotation and rewrite legacy hex files in-place.
- `lockchain tui` — keyboard-only interface for browsing datasets, unlocking with retries, and supplying passphrases on the fly.
- `lockchain validate -f /path/to/config` — run the static validator; `--schema` prints the JSON schema consumed by tooling.
- `lockchain-daemon` — supervise USB events, schedule unlock attempts, and expose health for your orchestrator (`cargo run -p lockchain-daemon`).
- Errors across binaries include `[LCxxxx]` codes; log collectors can pivot off the `LC` namespace for automated triage.

## Building in the Glow

- `cargo test -p lockchain-core` — high coverage target, includes keyfile and service guard-rails.  
- `cargo test -p lockchain-zfs` — includes the unlock smoke test backed by fake binaries.  
- `cargo test -p lockchain-key-usb` — requires `libudev-dev` (or equivalent) to compile.  
- `cargo fmt && cargo clippy --all-targets` — standard hygiene checks.

See [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) for the full build + review protocol and [`docs/SECURITY.md`](docs/SECURITY.md) for vuln reporting.

## Lore Drops

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — higher-level overview of the service boundaries.  
- [`docs/adr/ADR-001-module-provider.md`](docs/adr/ADR-001-module-provider.md) — why we double down on the provider abstraction.  
- [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) — playbook for rough nights.

Stay sharp, keep your datasets encrypted, and enjoy the glow.

[def]: https://github.com/x4ngus/lockchain-zfs/actions/workflows/ci.yml/badge.svg
