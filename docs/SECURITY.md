# Security Dispatch // LockChain ZFS

LockChain protects encrypted ZFS datasets in hostile environments. Treat every finding with the urgency of a breached perimeter.

---

## Coordinated Disclosure

Please email the maintainers at **security@lockchain.run**. The mailbox enforces TLS and is monitored by the core team.

When reporting:

1. Include a short summary and CVSS-style impact if possible.
2. Provide reproduction steps, configs, and anonymised logs (set `LOCKCHAIN_LOG_FORMAT=plain` for human-readable output).
3. Mention any public deadlines we should be aware of.

We aim to respond within **48 hours**, keep you updated every **7 days**, and ship a fix or mitigation within **14 days**. Critical findings get expedited patches and embargoed advisories as needed.

## Supported Versions

| Version | Status |
| --- | --- |
| `main` | 🟢 actively supported |
| Tagged releases (last 2) | 🟢 receive security patches |
| Anything older | 🔴 update recommended |

Security fixes land on `main` first, then are backported. Always review the changelog before upgrading in sensitive environments.

## Hardening Checklist

- Run `lockchain-key-usb` with a dedicated unprivileged user.
- Mount USB vault media read-only where possible; let the daemon rewrite legacy hex files automatically.
- Keep `LOCKCHAIN_LOG_FORMAT=json` to feed SIEM pipelines clean data.
- Protect `/run/lockchain/key.hex` with `0o400` permissions; the code enforces this, but double-check.
- Use `--strict-usb` in automated unlock workflows to avoid unexpected fallback prompts.

## Least Privilege Execution

- Run all LockChain services and the UI as the dedicated `lockchain` user. Packaging helpers already create `/var/lib/lockchain` with the right owner.
- Ensure `/etc/lockchain-zfs.toml` (and any secrets) are group-readable by `lockchain` only (e.g. `chmod 640`, `chgrp lockchain`).
- Delegate ZFS operations via `zfs allow lockchain load-key, key` or provide a minimal sudoers entry:

```
# /etc/sudoers.d/lockchain
lockchain ALL=(root) NOPASSWD:/usr/sbin/zfs load-key *, \
    /usr/sbin/zfs key -l *, \
    /usr/bin/lockchain-cli unlock *, \
    /usr/bin/lockchain-cli breakglass *
```

Validate with `visudo -cf /etc/sudoers.d/lockchain` and avoid granting a blanket `NOPASSWD:ALL`.

## Break-Glass Recovery

When normal USB/fallback workflows fail, a controlled “break-glass” flow is available. It **logs an audit entry `LC4000`**, demands explicit confirmation, and derives the fallback key to a file you specify.

```
lockchain validate -f /etc/lockchain-zfs.toml   # sanity check first
lockchain breakglass tank/secure --output /root/tank-secure.key
```

Steps:

1. Run the command locally as root. The tool displays a stern warning and asks you to type the dataset name, then the word `BREAKGLASS`. Press Enter at either prompt to abort.
2. Provide the emergency passphrase when prompted (or pass `--passphrase`).
3. The CLI writes the raw 32-byte key to the path you supplied with permissions `0400`. Use it immediately (e.g. `zfs load-key`) and destroy it (`shred && rm`) as soon as possible.
4. Check system logs for the `[LC4000] break-glass recovery invoked…` entry for auditing purposes.

You can bypass the prompts with `--force`, but only do this inside automated DR playbooks.

## Reporting Channels

- **Email (primary):** security@lockchain.run  
- **Matrix (secondary):** `#lockchain-security:matrix.org` (mentioning a maintainer initiates encrypted DM)
- **PGP:** fingerprint `2E58 5AC5 98E4 0AC2 8A53  45B0 7D6F B21E 54D0 9F73`

Do not open public GitHub issues for vulnerabilities until a patch is released.

## Hall of Fame

We credit researchers in the release notes unless anonymity is requested. Every valid report earns a place in the changelog and our eternal thanks.

Stay vigilant, keep the vault sealed.
