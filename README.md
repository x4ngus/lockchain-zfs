# LockChain ZFS

> A modern, cyberpunk security tool with Rust-based GUI (iced.rs), modular encryption providers, and native ZFS auto-unlock integration.

## Features
- USB token-based ZFS key unlock
- Passphrase fallback at boot
- GUI and CLI parity
- Systemd & initramfs integration
- Modular design for future LUKS/TPM providers

## Getting Started
```bash
cargo build --release
sudo dpkg -i target/debian/lockchain-zfs_0.1.0_amd64.deb
Then run lockchain-ui or lockchain-cli init.
