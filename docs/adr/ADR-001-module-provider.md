# ADR-001: Provider Interface & Modular Core

> _Status: Accepted (Cycle 2024-Q4)_  

## Scene

We’re shipping an encryption orchestrator that has to survive on any crew’s hardware: headless servers, rescue laptops, future daemon builds. ZFS is the substrate, but the way we talk to it (native CLI vs. daemon vs. mocked provider) must stay swappable. The project also wants USB-first workflows, optional fallbacks, and an observability story that works in the dark.

## Forces

- **Security first** — Key material can’t leak when a provider changes or an integration test swaps binaries.
- **Testability** — We need mock providers for high coverage and Python stubs for smoke tests.
- **Multiple surfaces** — CLI today, UI/daemon tomorrow. Shared logic must live in a neutral crate.
- **Ops ergonomics** — Logs, config parsing, and key loading need consistent behaviour across binaries.

## Decision

We introduced a layered architecture:

1. **`lockchain-core`** — Holds configuration loaders, the `ZfsProvider` trait, the unlock service, keyfile utilities, and logging bootstrapper. It exposes zero IO besides what the service orchestrator requests.
2. **Provider implementations** — `lockchain-zfs` shells out to native `zfs`/`zpool` with detection, exit-code mapping, and simulated fixtures for integration tests. Future providers (RPC, libzfs) slot into the trait.
3. **Edges** — `lockchain-cli` and `lockchain-key-usb` depend on `lockchain-core` + one provider. They adopt the shared logging format and respect configuration overrides.
4. **Documentation** — ADRs log decisions, the README introduces modules, and cyberpunk-styled docs keep the experience consistent with the product vibe.

## Consequences

- **Pros**
  - Easy to swap providers for different distros or future libzfs bindings.
  - Unit tests hit 70%+ coverage by mocking the provider while integration tests smoke the CLI binaries.
  - Logging and config handling stays uniform; observability pipelines just ingest JSON.
  - USB key normalisation is shared between service and daemon, preventing drift.
- **Cons**
  - Slightly higher crate count and CI build time.
  - Provider specific tests rely on Python stubs, so contributors need `python3`.

## Follow-up Signals

- Potential ADR-002 once a daemon or remote provider lands.
- Keep documentation neon-flavoured; it’s part of the brand and helps ops teams remember the flow.

— Authored by the LockChain maintainers, under neon lights.
