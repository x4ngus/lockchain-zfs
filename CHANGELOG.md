# Changelog // LockChain ZFS

> _"Version numbers are just coordinates through space and time."_  

All notable changes to this project will be documented here. The cadence follows semantic versioning once we cross the `v1.x` threshold. Until then we will log every milestone release to keep users updated of what the latest version contains.

---

## v0.1.9 — Control Deck Ignition (2025-10-28)

**Highlights**
- Debuted the `lockchain-ui` control deck powered by Iced 0.13 with neon-styled dashboards.
- Unified USB key handling across the stack by reusing the hardened raw/hex loader, permission tightening, and checksum validation.
- Extended the retry/backoff controls and error taxonomy so the daemon, CLI, and UI surface uniform diagnostics.
- Brought feature parity with `zfs_beskar_key`: dracut/initramfs loader templates, mount/unlock systemd units, and the full CLI workflow (`lockchain init`, `doctor`, `repair`, `self-test`).

**Operational polish**
- Refined JSON logging defaults with `LOCKCHAIN_LOG_LEVEL` overrides and clarified privilege guidance in the README/SECURITY docs.
- Automated system integration repair so `lockchain doctor`/`lockchain repair` reinstall mount/unlock units and enable `lockchain-key-usb.service` during recovery.

**Tooling & Docs**
- Added this changelog to keep future release notes tight and traceable.
- Synced documentation passes across README, INSTALL, CONTRIBUTING, and ARCHITECTURE to match the current surfaces.

Stay tuned for the next drop — and tag your release `vX.Y.Z` once you cut the build.
