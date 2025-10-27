![CI (Ubuntu 25.10+, Rust)][def]

# LockChain ZFS

LockChain ZFS provides a modular foundation for managing encrypted ZFS datasets with
USB-first key workflows and guarded passphrase fallbacks. The workspace is organised
into focused crates that separate configuration, service orchestration, provider
implementations, and user interfaces (CLI/UI).

## Workspace Overview
- `lockchain-core`: shared configuration loader, service layer, and the `ZfsProvider`
  trait used by higher-level components.
- `lockchain-zfs`: production provider that shells out to the native `zfs` CLI while
  honouring service contracts defined in `lockchain-core`.
- `lockchain-cli`: command-line entrypoint offering unlock, status inspection, and key
  listing flows built on the core services.
- `lockchain-ui`, `lockchain-daemon`, etc.: future-facing surfaces that consume the
  same services (not yet expanded in this iteration).

## Configuration

LockChain reads a TOML or YAML file that matches the reference schema below. The
defaults align with the legacy `zfs_beskar_key` layout, so existing deployments can
move over without rewriting policies.

```toml
[policy]
datasets = ["rpool/ROOT"]
zfs_path = "/sbin/zfs"

[crypto]
timeout_secs = 10

[usb]
key_hex_path = "/run/lockchain/key.hex"
expected_sha256 = "optional sha256 of the decoded raw key"

[fallback]
enabled = true
askpass = true
askpass_path = "/usr/bin/systemd-ask-password"
passphrase_salt = "hex salt emitted during init"
passphrase_xor = "hex xor blob emitted during init"
passphrase_iters = 250000
```

## CLI Usage

```
lockchain unlock [--dataset tank/secure] [--strict-usb] [--prompt-passphrase] [--key-file /path/to/raw.key]
lockchain status [--dataset tank/secure]
lockchain list-keys
```

- `unlock` loads key material from the configured USB hex file. If the file is missing
  and fallback is enabled, supply `--prompt-passphrase` (interactive) or `--passphrase`
  to derive the key material instead. A raw 32-byte override can be provided through
  `--key-file`.
- `status` reports the keystatus of the specified dataset, or of every dataset in
  `policy.datasets` when no argument is given.
- `list-keys` prints a table of managed datasets, their encryption roots, and their
  current keystatus.

## Development Notes

- Core service behaviours ship with unit tests that exercise the mocked provider
  contract; run them via `cargo test -p lockchain-core`.
- The concrete provider has lightweight unit coverage (`cargo test -p lockchain-zfs`)
  and mirrors the legacy `zfs` CLI logic, including descendant retry semantics.
- Full-workspace tests may require system libraries such as `libdbus-1` for the GUI
  crate; install the relevant packages if you need to exercise those crates locally.

[def]: https://github.com/x4ngus/lockchain-zfs/actions/workflows/ci.yml/badge.svg
