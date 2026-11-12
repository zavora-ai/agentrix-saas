# W3 · Control plane: what node and deploy need

Owner: W3 (control plane). Crates: `agentrix-operator` (operator scope, pages, ledger, grants),
`agentrix-billing` (to come). Design: [docs/08-control-plane.md](../../docs/08-control-plane.md);
concepts: [concept/control-plane/](../../concept/control-plane/).

## Node wiring (coordinator)

One line in `app.rs`, with one extra piece: the ops host needs its **own approvals**, because
WebAuthn checks the page's origin.

```rust
// A second AuthState for the ops origin. RP id stays the platform host, so the passkeys people
// registered on agentrix.website work on ops.agentrix.website too.
let ops_host = agentrix_operator::ops_host(&config.platform_host());
let ops_origin = agentrix_operator::ops_origin(&config.origin, &ops_host);
let mut ops_config = AuthConfig::for_origin(ops_origin)?;
ops_config.rp_id = config.platform_host();
let ops_auth = AuthState::new(ops_config, pool.clone(), Sealer::from_key(key_file(&config.seal_key_path())?))?;
let ops_ctx = NodeContext { approvals: Arc::new(/* the same adapter as ctx.approvals, over */ ops_auth.approvals()), ..ctx.clone() };
mounted.push(agentrix_operator::mount(&ops_ctx)?);
// The owner's side of support access, on the platform host, with the platform's approvals.
mounted.push(agentrix_operator::mount_platform(&ctx)?);
```

- `mount_platform` adds `/support-access/{grant}` (the owner grants, declines or ends support
  access with their passkey or code) and `/support-access.json` (what waits for the signed-in
  owner, for the console's lane) on the platform host.
- `holds::active(pool, site_id)` tells the edge whether a site is on platform hold and how to
  serve it (`ReadOnly`: answers and forms paused; `Notice`: the neutral notice page). Until it
  is wired, set nothing; once it is, set `AGENTRIX_HOLDS_ENFORCED=1` so the console stops
  saying "recorded, not enforced on this build".
- `mount` returns `HostMatch::Exact(ops host)` (so node routes the host and the TLS ask says yes)
  and one task, `operator-roles`, that applies the roles file at start and every 5 minutes.
- Migration `crates/agentrix-node/src/db/migrations/0300_operator.sql` (W3's range): operator_roles,
  operator_sessions, operator_ledger, support_grants, support_access, break_glass,
  platform_holds. Additive; no foreign keys to other workstreams' tables.
- Settings, all optional:

| Variable | Default | What |
|---|---|---|
| `AGENTRIX_OPS_HOST` | `ops.` + the platform host | The operator console's host |
| `AGENTRIX_OPERATORS_FILE` | `$AGENTRIX_DATA_DIR/agentrix-operator/operators.txt` | The roles file |
| `AGENTRIX_CELL` | `local\|This machine\|single node` | The cell for the Fleet page: `id\|place\|profile\|product,product`, e.g. `atl-0\|Atlanta\|single node\|VX1,Object Storage` |

## The roles file (deploy, owned by the control-plane session)

Plain text, one person per line, `#` starts a comment. The email must already have an account on
the platform (register at agentrix.website with a passkey first).

```text
# Zavora operators. Changes here reach the console within 5 minutes; each is a ledger entry.
person@zavora.ai   op.admin op.sre director
other@zavora.ai    op.release
```

Roles: `op.admin`, `op.sre`, `op.release`, `op.finance`, `op.support`, `op.partners`,
`op.security`, `op.auditor` (08 §2.3) and `director` (Owner on the operator charter, 08 §5.3).
Roles the file grants are revoked when the file stops listing them; roles an `op.admin` grants in
the console are left alone. Proposed path on the control VM: `/etc/agentrix/operators.txt`,
mode 0644, root-owned, passed as `AGENTRIX_OPERATORS_FILE` in node's unit. It holds emails only,
no secrets.

## Caddy

None: node answers the TLS ask for the ops host through `Mounted::hosts`.

## Local preview

```sh
cargo run -p agentrix-operator --example ops-preview -- <scratch dir> --seed
```

Serves http://127.0.0.1:18300 over a scratch database migrated with node's schema files, with
made-up preview records. It approves sign-ins with a fixed code, so it is never wired into node.

## Tables and owners (for 03-data.md's catalogue)

| Table | Owner now | Later |
|---|---|---|
| operator_roles, operator_sessions | W3 (`agentrix-operator`) | W6's memberships and sessions in Horizon 2 (agreed with W6, Sun 8 Nov) |
| support_grants, support_access, break_glass | W3 | W6 (`agentrix-identity`) in Horizon 2, same agreement |
| operator_ledger, platform_holds | W3 | W3 |
