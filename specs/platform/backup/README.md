# A backup for control.db (W8 proposal for the control-plane session)

Status: proposal, Sun 8 Nov 2026, by W8 (Platform core). `deploy/` belongs to the
"Agentrix WebAgent Control Plane" session; nothing here is in the repo yet.

## Why

`control.db` holds every account, claim, Move, decision, receipt index and usage row, and
`/var/lib/agentrix/keys/` holds control's receipt-signing key, the TOTP sealing key and the
release master key. Neither is backed up today: nothing in `deploy/` copies them, and
`provision.sh destroy` deletes the VM (03-data.md §6). Losing the keys ends the receipt chain and
every site's release log; losing the database loses every owner.

## What it does

`agentrix-backup` (a bash script; tested locally, shellcheck clean), every 15 minutes from a
systemd timer, as user `agentrix`:

1. `VACUUM INTO` a consistent copy of `control.db` (one read snapshot; node and the gateway keep
   writing, nothing is locked for long).
2. `PRAGMA integrity_check` on the copy; a failed check stops here, nothing is kept.
3. Bundles the copy with `keys/` (tar, zstd or gzip).
4. Encrypts it with **age** to public recipients in `/etc/agentrix/backup-recipients.txt`.
   Control holds public keys only: it can make copies, never open one. Without a recipient it
   refuses to make a copy rather than write one in the clear.
5. Writes a manifest beside it (`.json`: time, both SHA-256s, row counts of sites, accounts,
   moves, decisions, receipts and usage, migration versions; no personal data).
6. Uploads both to Object Storage when `objects.json` is configured: the releases bucket, path
   `backups/control/<file>`, SigV4 via `curl --aws-sigv4`, keys passed on stdin (`curl -K -`),
   never on a command line.
7. Keeps the newest 96 copies on control (`/var/lib/agentrix/backups/`, one day).

Exit 0: copied (and shipped); 1: no copy; 2: copied but the upload failed (next run retries).

RPO: 15 minutes. Size (W12, measured read-only at 07:15 EAT): control.db 2.7 MB, WAL 4 MB,
keys/ 16 KB. Each copy is at most a few MB, so up to about 250 MB a day at today's size, and
about 8 GB over 30 days: inside the bucket's flat $18 a month. Copies grow with the database
and never expire without the 30-day lifecycle rule below, which W12 recommends. The 96 copies
kept on control take the same order of space on its disk.

## To install (control-plane session)

1. **James makes the keys, off the host**, on his laptop: `age-keygen -o agentrix-backup-1.key`
   (and a second holder makes `-2`, for two-person recovery). Keep the `.key` files offline
   (password manager or paper). Only the `age1…` public lines go to control.
2. `deploy/`:
   - `deploy/backup/agentrix-backup` → installed as `/opt/agentrix/bin/agentrix-backup` (0755).
   - `deploy/systemd/agentrix-backup.service` and `.timer` → enabled with `systemctl enable --now agentrix-backup.timer`.
   - `deploy/env/backup-recipients.txt` (public `age1…` lines, one per holder) →
     `/etc/agentrix/backup-recipients.txt` (0644).
   - Optional `deploy/env/backup.env` → `/etc/agentrix/backup.env` (`AGENTRIX_BACKUP_KEEP=96`).
3. Host setup in `deploy.sh` (idempotent): `apt-get install -y age zstd` (`sqlite3` is already in
   cloud-init; `age` is in Ubuntu 24.04 universe), `install -d -o agentrix -g agentrix -m 0700
   /var/lib/agentrix/backups`.
4. Once (James's yes; W12 recommends it): a lifecycle rule on the bucket expiring
   `backups/control/` after 30 days.
5. Smoke: `sudo systemctl start agentrix-backup && journalctl -u agentrix-backup -n 5` shows
   "made … (… bytes, integrity ok)" and "shipped … to Object Storage".

Nothing restarts node. Rollback: `systemctl disable --now agentrix-backup.timer`.

## Restore drill (on a machine holding a private key)

```sh
f=control-20261108T041303Z.tar.zst.age
age -d -i agentrix-backup-1.key "$f" > bundle.tar.zst
shasum -a 256 bundle.tar.zst        # equals bundle_sha256 in $f.json
zstd -d bundle.tar.zst && tar -xf bundle.tar   # control.db and keys/
sqlite3 control.db 'PRAGMA integrity_check'    # ok; row counts match the manifest
```

To put it back on control: stop `agentrix-node` and `agentrix-models`, move the current
`control.db*` and `keys/` aside, copy the restored ones in (owner `agentrix`, group `agentrix`,
0660 for the db, 0700 for keys/), start models then node. A monthly drill on a scratch machine
should open the newest copy and compare its counts with the manifest (08 §3.9).

## Later (Horizon 2)

WAL shipping with Litestream (seconds of RPO, its own age encryption), per-site stores backed
up the same way, keys in `agentrix-keys` with their own escrow, and the erasure ledger replayed
after any restore (ADR-028). The status page can then show "Backups: last copy N minutes ago".
