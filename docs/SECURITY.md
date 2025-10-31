# Security Brief

This playbook covers disclosure, hardening, and the sanctioned break-glass procedure. 

---

## Coordinated Disclosure

- **Primary channel:** `security@lockchain.run` (TLS enforced).  
- **Response target:** acknowledge within 48 hours, status updates every 7 days, mitigation within 14 days for high-impact issues.  
- **What we need:** summary, impact, reproduction steps, anonymised logs (`LOCKCHAIN_LOG_FORMAT=plain` helps humans), and any external disclosure deadlines.

We prefer private reports. Public GitHub issues wait until the fix ships.

## Support Window

| Track | Status | Notes |
| --- | --- | --- |
| `main` | 🟢 fully supported | First to receive patches and advisories. |
| Latest 2 tags | 🟢 backported fixes | Signed `.deb` releases updated as needed. |
| Older tags | 🔴 security updates not guaranteed | Plan an upgrade path. |

Changelog entries call out CVEs or internal advisory IDs so compliance teams can trace remediation.

## Hardening Playbook

1. **Dedicated service account** — Run everything as `lockchain`. Packaging scripts create the user and `/var/lib/lockchain`.  
2. **Config custody** — `/etc/lockchain-zfs.toml` must be `640` owned by `root:lockchain`.  
3. **Key hygiene** — Key files live at `/run/lockchain/key.hex` with enforced `0400`; validate occasionally.  
4. **USB enforcement** — Keep `lockchain-key-usb` enabled so every stick is normalised and fingerprinted before use.  
5. **Strict unlock policy** — Automation should prefer `lockchain unlock --strict-usb` to block silent fallback use.  
6. **Structured telemetry** — Leave logs in JSON (`LOCKCHAIN_LOG_FORMAT=json`) for SIEM-friendly ingestion unless actively debugging.  
7. **Mount discipline** — Mount vault media read-only where possible; let the tooling handle writes during normalisation.

## Least Privilege in Practice

- Delegate just the required ZFS verbs:

```bash
sudo zfs allow lockchain load-key,key tank/secure
```

- If you must use sudo, limit it to the known commands and validate with `visudo -cf`:

```
# /etc/sudoers.d/lockchain
lockchain ALL=(root) NOPASSWD:/usr/sbin/zfs load-key *, \
    /usr/sbin/zfs key -l *, \
    /usr/bin/lockchain-cli unlock *, \
    /usr/bin/lockchain-cli breakglass *
```

- Run `lockchain-ui`, the daemon, and any automation as the `lockchain` user to avoid accidental root pivots.

## Break-Glass Procedure

When USB access is lost and the organisation authorises emergency recovery, use the guarded break-glass flow. It broadcasts `[LC4000]` audit events and demands explicit confirmation.

```bash
lockchain validate -f /etc/lockchain-zfs.toml
lockchain breakglass tank/secure --output /root/tank-secure.key
```

Checklist:

1. CLI confirms dataset name and requires typing `BREAKGLASS`. Press Enter at either prompt to abort.  
2. Supply the emergency passphrase manually or via `--passphrase`.  
3. The tool derives the raw 32-byte key, writes it with `0400`, and logs the action.  
4. Use the key immediately (`zfs load-key`) and destroy the file (`shred && rm`) after use.  
5. Log reviewers should see `[LC4000] break-glass recovery invoked` with dataset context.

`--force` exists for scripted DR plans, but we expect it to be guarded by the same approvals you’d require for a production failover.

## Reporting Channels

- **Email:** security@lockchain.run  
- **Matrix:** `#lockchain-security:matrix.org` (tag a maintainer for encrypted DM)  
- **PGP fingerprint:** `2E58 5AC5 98E4 0AC2 8A53  45B0 7D6F B21E 54D0 9F73`

We coordinate with upstream ZFS communities when issues cross project boundaries.

## Researcher Cred

Researchers who help secure LockChain are credited in the changelog unless anonymity is requested. We celebrate every responsible disclosure.
