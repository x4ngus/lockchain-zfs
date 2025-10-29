# Architecture Overview

This document outlines the service boundaries inside the LockChain workspace,
with a focus on the ZFS provider contract shared across the crates and the way
long-running services compose with the USB watcher.

## Core Concepts

- **Configuration (`LockchainConfig`)**: Loaded from TOML or YAML, the config
  describes the managed datasets, ZFS binary location, USB key path, and
  fallback parameters. It is the canonical source of runtime settings.
- **Service Layer (`LockchainService`)**: Implements domain operations such as
  `unlock`, `status`, and `list_keys`. It consumes any implementation of the
  provider contract and exposes a testable, dependency-injected API to the CLI,
  daemon, or UI surfaces.
- **Provider Interface (`ZfsProvider`)**: Abstracts the mechanics of invoking
  ZFS. Each crate can supply its own implementation (system commands, RPC, or
  mocks) while respecting the same behaviour contract.

## Provider Contract

The `ZfsProvider` trait (defined in `lockchain-core`) encapsulates the minimal
set of operations the service layer requires:

```rust
pub trait ZfsProvider {
    /// Resolve the encryption root responsible for `dataset`.
    fn encryption_root(&self, dataset: &str) -> LockchainResult<String>;

    /// Return datasets under `root` (including the root itself) that still
    /// report a sealed keystatus.
    fn locked_descendants(&self, root: &str) -> LockchainResult<Vec<String>>;

    /// Attempt to load a key for `root` and any descendants that share it.
    /// Returns the datasets confirmed to have accepted the key, in order.
    fn load_key_tree(&self, root: &str, key: &[u8]) -> LockchainResult<Vec<String>>;

    /// Describe the keystatus for each dataset. Implementations should preserve
    /// the input order in the returned snapshot.
    fn describe_datasets(&self, datasets: &[String]) -> LockchainResult<KeyStatusSnapshot>;
}
```

### Behaviour Guidelines

- **Deterministic Ordering**: Methods that return dataset lists are expected to
  preserve deterministic ordering. The core service relies on this to present
  predictable outputs in the CLI, and mock tests assert sorted results.
- **Error Semantics**: Use descriptive `LockchainError::Provider` messages when
  an underlying command fails. Callers differentiate between configuration
  issues (e.g., `DatasetNotConfigured`) and runtime failures.
- **Separation of Concerns**: Provider implementations must not read user
  configuration directly; pass in the necessary data via the service layer.
  This keeps tests simple and ensures alternate implementations (for example,
  a future ZFS daemon) stay drop-in compatible.

## Implementations

| Implementation | Crate | Notes |
| --- | --- | --- |
| `SystemZfsProvider` | `lockchain-zfs` | Shells out to the native `zfs`/`zpool` CLI with timeout controls and exit-code mapping. |
| `MockProvider` | `lockchain-core` tests | In-memory implementation validating service behaviour. |
| `DaemonService` | `lockchain-daemon` | Long-running process orchestrating USB events, scheduled unlocks, and health reporting. |

When adding new providers, ensure they satisfy the contract above and write
integration or unit tests that demonstrate compliance. The unit tests in
`lockchain-core` serve as a compatibility suite; they will fail early if the
contract is broken.

## Services Model

LockChain now ships with two continuously running components:

- **lockchain-key-usb** — a udev-backed watcher that normalises vault sticks,
  rewrites legacy hex keys to raw bytes and mirrors them into `/run/lockchain/`
  with strict permissions. It can run standalone or alongside the daemon.
- **lockchain-daemon** — a Tokio service that loads configuration, creates a
  `LockchainService<SystemZfsProvider>`, and performs scheduled unlock attempts
  for the target datasets. It exposes a minimal HTTP health endpoint (default
  `127.0.0.1:8787`) returning `OK` / `DEGRADED`, ready for consumption by systemd
  health checks or external monitors.

The daemon will eventually subscribe directly to USB events from
`lockchain-key-usb` via shared crates; today it broadcasts health by marking the
state ready once a watcher is active and successful unlocks occur. This design
keeps the core logic reusable while enabling future RPC or REST entry points.
