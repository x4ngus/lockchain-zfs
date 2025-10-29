# Changelog // LockChain ZFS

> _"Version numbers are just coordinates through neon space."_  

All notable changes to this project will be documented here. The cadence follows semantic versioning once we cross the `v1.x` threshold; until then we log every milestone release to keep ops crews in sync with the glow.

---

## v0.1.9 — Control Deck Ignition (2025-10-28)

**Highlights**
- Debuted the `lockchain-ui` control deck powered by Iced 0.13 with neon-styled dashboards, passphrase prompts, and a guided break-glass recovery flow.
- Unified USB key handling across the stack by reusing the hardened raw/hex loader, permission tightening, and checksum validation.
- Extended the retry/backoff controls and error taxonomy so the daemon, CLI, and UI surface uniform LCxxxx diagnostics.

**Operational polish**
- Refined JSON logging defaults with `LOCKCHAIN_LOG_LEVEL` overrides and clarified privilege guidance in the README/SECURITY docs.
- Baked in `resolver = "2"` and a workspace-wide `v0.1.9` version stamp to prep for upstream tagging and packaging.

**Tooling & Docs**
- Added this changelog to keep future release notes tight and traceable.
- Synced documentation passes across README, INSTALL, CONTRIBUTING, and ARCHITECTURE to match the current surfaces.

Stay tuned for the next neon drop — and tag your release `v0.1.9` once you cut the build.
