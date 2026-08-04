# Regtest Bootstrap Fails Permanently on Fresh Volumes: Miner Address Cannot Pay Orchard Before NU5 Activation

**Status:** Confirmed via direct reproduction. Patched locally in this checkout's
`external/z3` clone; the patch has not been upstreamed (see "Delivery note" below).
**Applies to:** `external/z3`'s `scripts/regtest-init.sh`, upstream commit `dd84312`
(pinned Z3 meta-repo commit `dfb9d0eae6d3f67a3c184e0d8fcb1166e7740724`, per
`z3-commits.lock`), on the regtest override stack (Zebra `v6.0.0` / Zaino `0.6.0` /
Zallet `v0.1.0-beta.2`).
**Repo:** this is upstream Zcash Foundation code
(`external/z3`, `git remote` = `https://github.com/ZcashFoundation/z3`), not code
authored by this simulator project.

---

## Impact

A genuinely fresh regtest environment — fresh `z3-regtest-{chain,cookie,zaino,zallet}`
Docker volumes, i.e. what anyone gets by wiping volumes or running `make clone-z3` +
`regtest-init.sh` on a clean machine — could not mine a single block. `regtest-init.sh`'s
own first mining call panicked Zebra; the chain stayed at height 0 forever, and every
downstream operation (wallet balance, account provisioning, funding) failed as a direct
consequence. This is a stronger, more fundamental failure than the simulator-side
provisioning-concurrency and warmup-timeout issues fixed earlier in this engagement — those
only manifest once a chain already exists past this point.

This was not caught earlier in the engagement because every test session, including the
ones that produced the two fixes above, reused Docker volumes whose chain had already
survived past the point described below, from before the current `.env.regtest` mining
address was set. It surfaced only when a fresh-volume run was performed specifically to
validate those other fixes under worst-case conditions.

---

## Reproduction

Environment: `external/z3` volumes removed (`docker volume rm z3-regtest-chain
z3-regtest-cookie z3-regtest-zaino z3-regtest-zallet`), stack brought up fresh via
`docker compose --env-file .env.regtest up -d`.

```
$ curl -u zebra:zebra -X POST http://127.0.0.1:8181 \
    -d '{"jsonrpc":"1.0","id":"1","method":"generate","params":[1]}'
```

Zebra's log:

```
ERROR rpc_request{... rpc.method=generate ...}: zebra_rpc::methods::types::transaction:
Failed to add Orchard output: Cannot create Orchard transactions without an Orchard
anchor, or before NU5 activation

thread 'tokio-rt-worker' (31) panicked at zebra-rpc/src/methods/types/get_block_template/zip317.rs:73:18:
valid coinbase transaction template: CoinbaseConstruction("Could not construct output with miner reward")
```

The chain remains at height 0 after this; no partial block is committed. Calling
`generate([2])` — exactly what `regtest-init.sh` itself does as its first mining step —
panics identically, on the same first block. There is no way to reach height 2 with the
address configured at the time of reproduction.

---

## Root cause

`external/z3/config/regtest/zebra.toml` activates NU5 (Orchard) at height 2, not height 0:

```
[network.testnet_parameters.activation_heights]
...
NU5 = 2
```

Block 1 is necessarily mined before NU5 activates. Its coinbase output therefore cannot
pay an Orchard receiver — Zebra's block-template construction has no fallback and panics
the RPC-serving task outright rather than returning a JSON-RPC error.

At the time of reproduction, `external/z3/.env.regtest` set:

```
ZEBRA_MINING__MINER_ADDRESS=uregtest1msdytc8nl3txzjdqfynls5mnv0uf4kpszz3t5cdpwayq3dv6qs2w7z8k0u9jtn5gu38t9fzc42k967s82w5s8vx0t492nw3s4uhast2p
```

— a single-receiver Orchard-only unified address (the hot wallet account's own address
from a previous, already-bootstrapped wallet). This value was a local, uncommitted edit:
`git diff` against the pinned commit shows the file's original tracked default is

```
ZEBRA_MINING__MINER_ADDRESS=tmSRd1r8gs77Ja67Fw1JcdoXytxsyrLTPJm
```

a transparent (non-Orchard) address, which does not hit this problem — pre-NU5 blocks can
pay a transparent output. The local edit was made deliberately, alongside
`docker-compose.regtest.override.yml`, to route mining rewards directly into the hot
wallet's Orchard balance rather than its (at the time separately broken — see
`zallet-transparent-spending-bug.md`) transparent balance. That workaround fixed one
problem and introduced this one: it hardcoded a value that is only valid once the chain is
already past height 2, with no step anywhere that gets a fresh chain there in the first
place.

`scripts/regtest-init.sh`'s own comment ("Mining 2 blocks (NU5/Orchard activates at height
2 to match Zaino's regtest defaults)") shows the script's author was aware blocks 1-2
needed special handling, but the script mines them with whatever
`ZEBRA_MINING__MINER_ADDRESS` happens to be configured — there was never a code path that
mines the pre-activation blocks with something other than the final address.

---

## Fix

Patched `external/z3/scripts/regtest-init.sh` to bootstrap in two phases instead of one:

1. **Phase 1 (pre-NU5):** start Zebra with the address override exported directly in the
   script's own shell (taking precedence over whatever is in `.env.regtest` on disk),
   using the pinned repo's own transparent placeholder
   (`tmSRd1r8gs77Ja67Fw1JcdoXytxsyrLTPJm` — valid at every activation height, needs no
   wallet). Mine blocks 1-2 with it, exactly as before; this reward is negligible
   regtest-only value and is discarded.
2. **Phase 2 (post-NU5):** start Zallet, create the `hot_wallet` account via
   `z_getnewaccount`, and derive its real Orchard receiver via `z_listaccounts` +
   `z_listunifiedreceivers` — the same resolution path the simulator itself uses
   (`src/scenarios/runner/funding.rs::resolve_account`). Write that address into
   `.env.regtest`'s `ZEBRA_MINING__MINER_ADDRESS` so every subsequent `docker compose up
   -d` (block 3 onward, including the one the script's own closing instructions tell the
   operator to run) mines directly to the hot wallet.

The script's existing idempotency guard (skip if a wallet mnemonic already exists) now
also skips this whole bootstrap sequence, so re-running it against an already-initialized
wallet does not re-mine or overwrite a working address with a fresh derivation. Added `jq`
as a new script dependency, used to parse the account/receiver RPC responses.

### Validation

Ran the patched script against fresh volumes, then brought up the full stack and mined
past coinbase maturity:

```
$ ./scripts/regtest-init.sh
...
   Hot wallet Orchard receiver: uregtest1xwdx3v9x9t8xtl8v3fnx5tyz00s50jlpzxf6cl6804wgcjqqfrandjwkdflglm4ju4qezu78tzdaheg4cusxt94pz34v3d8fhykyaylp
==> Recording it as ZEBRA_MINING__MINER_ADDRESS in .env.regtest...
$ docker compose --env-file .env.regtest up -d
$ curl -u zebra:zebra ... '{"method":"generate","params":[110]}'
```

- Mined to height 112 with zero panics (`docker logs z3-regtest-zebra-1 | grep -c
  "panicked at"` → `0`).
- `z_getbalanceforaccount` on the hot wallet: `{"orchard":{"valueZat":68750000000}}`
  (687.5 ZEC, correctly matured).
- `z_sendmany` from the hot wallet succeeded (`opid-a8e39d83-740b-44e5-b0a0-3618874a34a7`)
  — the exact call that fails with `Insufficient balance (have 0, ...)` when the chain
  never got past this bootstrap step.

---

## Delivery note

The fix lives in `external/z3`, a separate clone of the upstream Zcash Foundation
repository (`git remote` = `https://github.com/ZcashFoundation/z3`) pinned via
`z3-commits.lock` and not tracked by this project's own git history. The patch is applied
locally to this checkout only — it does not persist across a fresh `make clone-z3`, the
same way the existing `docker-compose.regtest.override.yml` workaround (documented in its
own header comment) does not. Anyone re-cloning the pinned Z3 stack from scratch will hit
this bug again until `scripts/regtest-init.sh` is patched the same way, or the fix is
upstreamed to the Foundation's own repository.
