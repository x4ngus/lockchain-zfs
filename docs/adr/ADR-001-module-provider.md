# ADR-001: Provider Interface & Modular Core

**Status**: Accepted (Cycle 2024-Q4)

## Context

LockChain ZFS must orchestrate encrypted datasets across diverse environments: headless servers, emergency recovery laptops, and future service deployments. ZFS remains the storage substrate, yet the integration layer (CLI calls, daemon APIs, mocks) must remain replaceable without disrupting workflows. USB-first key management, optional fallbacks, and consistent observability are non-negotiable.

## Forces

- **Security control** — Key material handling must stay consistent independent of provider swap-outs.  
- **Test coverage** — We require mock providers to hit coverage targets and Python-backed stubs for integration smoke tests.  
- **Multiple execution surfaces** — CLI, daemon, UI, and automation pipelines expect identical behaviour.  
- **Operational ergonomics** — Logging, configuration parsing, and error handling should behave uniformly to simplify runbooks.

## Decision

Adopt a layered architecture:

1. **`lockchain-core`** — Houses configuration loaders, the `ZfsProvider` trait, unlock workflow orchestration, keyfile utilities, and logging bootstrapper. The crate remains IO-agnostic beyond explicit service calls.  
2. **Provider implementations** — `lockchain-zfs` shells out to native `zfs`/`zpool`, handles binary discovery, exit-code mapping, and integration fixtures. Future providers (RPC, libzfs) implement the same trait.  
3. **Edges** — `lockchain-cli`, `lockchain-daemon`, `lockchain-key-usb`, and `lockchain-ui` pair `lockchain-core` with a provider implementation, honour shared logging, and respect configuration overrides.  
4. **Documentation & style** — ADRs capture major decisions; documentation maintains the project’s neon-forward aesthetic to keep tooling recognisable in operations centres.

## Consequences

**Benefits**
- Provider swaps remain low-risk; integration points rely solely on the trait contract.  
- Unit tests mock providers to maintain ≥ 70 % coverage, while integration tests exercise real binaries.  
- Observability remains uniform: JSON logs, structured error codes, and deterministic workflow events.  
- USB key normalisation stays centralised within the core workflow, avoiding surface drift.

**Trade-offs**
- Additional crates increase compile time and complexity for newcomers.  
- Provider-specific integration tests depend on Python tooling, increasing contributor prerequisites.

## Follow-up

- Anticipate ADR-002 to cover remote providers or daemon-specific APIs once those land.  
- Periodically review provider contract ergonomics as we integrate new surfaces.  
- Continue curating documentation with the subtle neon theme to maintain brand continuity and operator familiarity.

— LockChain maintainers
