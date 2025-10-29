# Contributing // LockChain ZFS

> _"We maintain the perimeter the way others maintain gardens."_  

Welcome to the crew. This document explains how to collaborate without breaking the neon trance the project lives in. Read it once, bookmark it, and keep your PRs sharp.

---

## Signal Codes

- **Rust**: stable toolchain (`rustup toolchain install stable`)
- **Coverage**: core crate ≥ 70% (`cargo tarpaulin --ignore-tests --workspace`)
- **Style**: `cargo fmt` + `cargo clippy --all-targets -- -D warnings`
- **Docs**: README + ADRs must stay fresh; any significant change ships with words

## First Contact

1. Fork the repo and clone your fork.
2. Run the smoke test to prove your environment:
   ```bash
   cargo test -p lockchain-zfs --test unlock_smoke
   ```
3. Install dependencies for optional surfaces:
   - `libudev-dev` (or distro equivalent) for `lockchain-key-usb`
   - `pkg-config`, `python3` for simulated provider tests

## Branch Ritual

Branches are named `feature/<codename>` or `fix/<issue>`. Keep commits focused and signed off if your workflow requires it.

Before opening a PR:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo tarpaulin --workspace --ignore-tests  # optional, but include the result in your PR
```

## Review Expectations

- Every PR needs at least one test. Unit, integration, or smoke — dealer’s choice.
- Keep log output JSON-friendly by default; respect existing logging abstractions.
- Touching a config file? Update `docs/` or the ADRs as part of the change.
- Try not to squash reviewers’ comments by force-pushing; rebase at the end.

## DocOps

The docs live under `docs/` and follow a neon-with-purpose tone. When you add a new subsystem or break an assumption, drop an ADR (`docs/adr/ADR-00X-*.md`) plus a callout in the README if user-facing.

## Issue Protocol

- **Bug report**: include dataset/CLI output, log snippet (with `LOCKCHAIN_LOG_FORMAT=plain` if helpful), and kernel/ZFS versions.
- **Feature request**: open a discussion first if it impacts provider behaviour or security posture.
- **Security findings**: see [`docs/SECURITY.md`](SECURITY.md) for the encrypted channel.

## Release Cadence

1. Update `docs/CHANGELOG.md`.
2. Tag releases `vX.Y.Z`.
3. Attach binary artefacts only when the CLI surface changes.

## Appreciation

Thanks for helping keep the vaults locked. Add yourself to `docs/CONTRIBUTORS.md` (feel free to create it if you’re first) and drop into the discussions to keep the neon chat alive.

— The LockChain maintainers 🛡️
