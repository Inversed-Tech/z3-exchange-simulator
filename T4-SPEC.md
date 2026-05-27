# T4 — Synthetic Generators: Implementation Specification

Full specification for Track 4 of the Z3 Exchange Simulator WORK-TRACKS.md. Written
before implementation as the basis for code review. All design decisions, open questions
and their resolutions, structural ground truth from Z3 source research, detailed function
contracts, invariants, edge cases, and test requirements are documented here.

This document is the single source of truth for the T4 implementation. The reviewer
should flag any inconsistency between this spec and the existing codebase (T1, T2),
any violation of the WORK-TRACKS.md contracts, or any missing detail that would force
an implementation-time decision.

---

## Table of contents

1. [Context and placement in the work plan](#1-context-and-placement-in-the-work-plan)
2. [Design decisions — questions asked and answers given](#2-design-decisions--questions-asked-and-answers-given)
3. [Structural ground truth from Z3 source research](#3-structural-ground-truth-from-z3-source-research)
4. [Files to create or modify](#4-files-to-create-or-modify)
5. [Stage 1 — T1 additions: new config types](#5-stage-1--t1-additions-new-config-types)
6. [Stage 2 — Cargo.toml dependencies](#6-stage-2--cargotoml-dependencies)
7. [Stage 3 — Module structure and SyntheticPopulation](#7-stage-3--module-structure-and-syntheticpopulation)
8. [Stage 4 — AccountGenerator](#8-stage-4--accountgenerator)
9. [Stage 5 — TransactionIntentGenerator](#9-stage-5--transactionintentgenerator)
10. [Stage 6 — write\_fixtures](#10-stage-6--write_fixtures)
11. [Stage 7 — Test plan](#11-stage-7--test-plan)
12. [Integration contract for T7](#12-integration-contract-for-t7)
13. [Out of scope for T4](#13-out-of-scope-for-t4)
14. [Open items for the reviewer](#14-open-items-for-the-reviewer)

---

## 1. Context and placement in the work plan

**WORK-TRACKS.md track:** T4 — Synthetic Data Generators  
**Module:** `src/synthetic/`  
**Depends on:** T1 (data model types — complete)  
**Blocked by:** Nothing. T2 and T3 are parallel; T4 has no dependency on either.  
**Unblocks:** T5 (Exchange Emulation), T7 (Scenario Runner)  
**Branch:** `workplan-task-4-synthetic-generators`

### What T4 produces

T4 produces deterministic, seeded synthetic data that drives the rest of the simulator.
Given the same scenario config, running T4 twice always produces the same population and
the same sequence of transaction intents. This is the reproducibility guarantee the
proposal requires for findings to be attributable.

Three deliverables:

| Deliverable | Description | Consumed by |
|---|---|---|
| `SyntheticPopulation` | Collection of generated `Account` and `Wallet` records with fast lookup | T7 (provisioning), T5 (emulation), T8 (fixtures command) |
| `TransactionIntentGenerator` | On-demand generator of `TransactionIntent` values at the right flow-type distribution | T7 (TPS scheduler) |
| `write_fixtures()` | Writes `accounts.json` and `wallets.json` to disk for offline inspection | T8 (generate-fixtures subcommand) |

### Where T4 fits in a live run

```
ScenarioConfig (parsed YAML, validated by T7)
  │
  ▼
AccountGenerator::new(config: ScenarioConfig)         ← T4
AccountGenerator::generate_population()               ← T4
  │  produces SyntheticPopulation with empty wallet address lists
  ▼
[T7: for each account in population]
  z_getnewaccount(name)          → account_uuid       ← T3 (RPC)
  z_getaddressforaccount(uuid)   → unified_address     ← T3 (RPC)
  population.add_address(account_id, address)          ← T4 (address hook)
  │  population now has real Zallet-derived addresses
  ▼
TransactionIntentGenerator::new(&population, &config) ← T4
  │  configured with flow weights, amount range, distinct-pair logic
  ▼
[T7: TPS scheduler tick]
  generator.next_intent(run_id, &population)           ← T4
  │  produces one TransactionIntent per tick
  ▼
[T5: Exchange Emulation]
  run_deposit / run_withdrawal / run_sweep / run_balance_check
```

---

## 2. Design decisions — questions asked and answers given

These decisions were made explicitly before writing this spec. Each entry records the
question, the options considered, the answer chosen, and the rationale.

---

### Q1 — Which RNG algorithm?

**Question:** Should we use `rand::rngs::StdRng` or `rand_chacha::ChaCha8Rng`?

**Options considered:**
- `StdRng` — simple, no extra crate, but its internal algorithm is not guaranteed stable
  across `rand` crate versions. A `rand` upgrade could silently change the sequence,
  breaking reproducibility without any code change.
- `ChaCha8Rng` — algorithm is fixed by the ChaCha specification. The sequence for a
  given seed will be identical across all `rand_chacha` versions forever.

**Answer:** `ChaCha8Rng`.

**Rationale:** The proposal's reproducibility guarantee ("same seed → same output") is
a hard requirement for findings to be attributable to specific Z3 commits. `StdRng`
would satisfy it within a single binary but could silently break after a routine
dependency update. `ChaCha8Rng` makes the guarantee unconditional.

**Crate:** `rand_chacha = "0.3"` (compatible with `rand = "0.8"`).

---

### Q2 — How is ActivityProfile distributed?

**Question:** Should the distribution of `Low`/`Medium`/`High` activity profiles across
generated accounts be fixed in code or configurable per scenario?

**Options considered:**
- Fixed distribution (e.g. 50/35/15) — no new config fields, but inflexible.
- Configurable per scenario — requires adding a new struct to `ScenarioConfig` in T1.

**Answer:** Configurable per scenario, through a new `ActivityProfileConfig` struct
added to `ScenarioConfig` in T1.

**Rationale:** Different load scenarios (smoke, ramp, burst, mixed) may want different
population shapes. Making it configurable now avoids a T1 edit later.

**Impact on T1:** `ActivityProfileConfig` struct and a new field `activity_profiles` in
`ScenarioConfig`. See Stage 1.

**Default in smoke.yaml:** `low: 0.50, medium: 0.35, high: 0.15`.

---

### Q3 — Are self-sends (sender == recipient) allowed?

**Question:** When `TransactionIntentGenerator` picks a sender and recipient account,
can the same account appear as both?

**Options considered:**
- Allow self-sends — simplest, no rejection loop needed.
- Require distinct accounts — more realistic (exchanges don't send to themselves in normal flow), minor complexity.

**Answer:** Distinct accounts only — reject and re-draw if sender account equals recipient account.

**Rationale:** More realistic. The rejection loop terminates in one retry in the vast
majority of cases (probability of collision = 1/(n-1) where n ≥ 2).

**Edge case:** If there are fewer than 2 active accounts, `next_intent` returns `None`.

---

### Q4 — Is the zatoshi amount range configurable?

**Question:** Should the range `[min_zatoshis, max_zatoshis]` for transaction amounts
be a hardcoded constant or a per-scenario config field?

**Options considered:**
- Hardcoded constants in `src/synthetic/` — no T1 edit.
- Configurable per scenario — requires a new struct in T1.

**Answer:** Configurable per scenario, through a new `AmountRangeConfig` struct added
to `ScenarioConfig` in T1.

**Rationale:** Burst scenarios may want larger amounts to stress mempool and fee
estimation differently from smoke scenarios. Configurable now, avoids a later T1 edit.

**Impact on T1:** `AmountRangeConfig` struct and a new field `amounts` in `ScenarioConfig`.

**Default in smoke.yaml:** `min_zatoshis: 10000` (0.0001 ZEC), `max_zatoshis: 10000000`
(0.1 ZEC).

---

### Q5 — What does `write_fixtures` produce?

**Question:** Should the fixture output be one combined file or one file per entity type?

**Answer:** One JSON file per entity type in the output directory:
- `accounts.json` — JSON array of all `Account` records
- `wallets.json` — JSON array of all `Wallet` records

**Rationale:** Easier to inspect individual entity types, easier to diff across runs,
easier to use as isolated test fixtures in T3 or T5.

**Note:** Pre-generated intents are NOT written to disk. `TransactionIntent` values are
produced on demand by `TransactionIntentGenerator::next_intent`. Writing them all upfront
would produce a file whose size is proportional to `load_duration_seconds * target_tps`,
which can be very large.

---

### Q6 — Should `SyntheticPopulation` have fast-lookup indexes?

**Question:** Is sequential iteration over `Vec<Account>` sufficient, or should the
population support O(1) lookup by ID?

**Answer:** Both. `SyntheticPopulation` holds `Vec<Account>` and `Vec<Wallet>` for
ordered iteration, plus `HashMap<String, usize>` indexes for O(1) lookup by ID.

**Rationale:** T7 needs to look up a wallet by `account_id` when adding a Zallet-provisioned
address. Scanning the full `Vec` on every provisioning call would be O(n) per account,
totalling O(n²) for provisioning. With a `HashMap` index it is O(1) per account = O(n)
total.

---

### Structural decision — module layout

**Question:** Should T4 live in a single file `src/synthetic/mod.rs` or be split into
submodules?

**Answer:** Submodules. The directory layout is:

```
src/synthetic/
  mod.rs          Public API: re-exports, SyntheticPopulation struct, PopulationError
  generators.rs   AccountGenerator, TransactionIntentGenerator, seeding logic
  fixtures.rs     write_fixtures(), FixtureError
```

**Rationale:** T4's scope is large enough that a single file would exceed 700 lines.
Submodules make the boundaries clear for reviewers and future maintainers.

---

## 3. Structural ground truth from Z3 source research

Before T4 was designed, the source code of Zallet and Zebra at their pinned commits was
read to establish the exact data formats that the real Z3 stack produces. T4's synthetic
data must be structurally identical to what Z3 returns — same ID format, same address
prefixes, same field names — so that when T7 replaces placeholder values with real
Zallet-derived values, the types are fully compatible.

### Pinned commits inspected

| Component | Repository | Commit |
|---|---|---|
| Zallet | `zcash/wallet` | `6fc85f68cf5ebe456160c6518255a83129e7d21c` |
| Zebra | `ZcashFoundation/zebra` | `aba329d6dca884f6d42bb4d36bda0010a071c2fc` |

### Findings

#### Account ID format (`z_getnewaccount` response)

Source file: `zallet/src/components/json_rpc/methods/get_new_account.rs`

Zallet's `z_getnewaccount` returns:
```json
{ "account_uuid": "e0764683-855f-471d-b723-cf640a0ea262" }
```

The `account_uuid` is produced by `account_id.expose_uuid().to_string()` on a `uuid::Uuid`
value. The `uuid` crate's `Display` / `to_string()` always produces the canonical
hyphenated lowercase format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (36 characters, 4
hyphens, all lowercase hex).

The `account` field (ZIP 32 index) is always absent in Zallet's response at the pinned
commit (always `None`, skipped by `#[serde(skip_serializing_if = "Option::is_none")]`).

**Implication for T4:** All generated `account_id` and `wallet_id` strings must be
standard RFC-4122 UUIDs in the canonical hyphenated lowercase format. We generate them
by drawing 16 random bytes from the seeded `ChaCha8Rng` and using
`uuid::Builder::from_random_bytes(bytes).into_uuid().to_string()`, which correctly sets
version bits (4) and variant bits. This is structurally identical to Zallet's output.

#### `z_getaddressforaccount` response structure

Source file: `zallet/src/components/json_rpc/methods/get_address_for_account.rs`

```json
{
  "account_uuid": "e0764683-855f-471d-b723-cf640a0ea262",
  "diversifier_index": 0,
  "receiver_types": ["orchard", "sapling", "p2pkh"],
  "address": "uregtest1..."
}
```

The `address` field is a **Unified Address** containing all three receiver types. The
`receiver_types` list declares which receivers are embedded in the UA.

**Implication for T4:** T4's `Wallet` struct uses the existing T1 `Address` model, which
represents a single receiver. When T7 calls `add_address`, it passes one `Address` record
per receiver type decoded from the Unified Address. T4 is not responsible for decoding
Unified Addresses — that is T7's (or T3's) responsibility. T4 only stores whatever `Address`
records it receives via `add_address`.

Note: The `diversifier_index` field is not part of our T1 `Address` struct. This is
intentional — it is a Zallet-internal concept the simulator does not need to track.

#### Regtest address prefixes

| Type | Prefix | Source |
|---|---|---|
| Unified Address (regtest) | `uregtest1...` | `zcash_protocol` HRP constant `"uregtest"` |
| Transparent P2PKH (regtest) | `tm...` | Zebra `NetworkKind::Regtest` reuses testnet version bytes `[0x1d, 0x25]` |
| Transparent P2SH (regtest) | `t2...` | Zebra version bytes `[0x1c, 0xba]` |
| Sapling standalone (regtest) | `zregtestsapling1...` | `zcash_protocol` HRP constant `"zregtestsapling"` |

**Implication for T4:** Wallet `Address` records created at generation time are
placeholders (address lists are `vec![]`). When T4 generates fixture output, the address
lists in wallets are empty. Real addresses are set by T7 after Zallet provisioning.

If, in the future, T4 needs to produce synthetic placeholder address strings (e.g. for
fixtures with pre-populated wallets), the correct prefixes for regtest are `tm...`
(transparent) and `uregtest1...` (Unified). These must never be used in live runs — they
would be syntactically plausible-looking but cryptographically invalid.

#### Transaction ID format

Zebra's `Hash` type is a `[u8; 32]` displayed as 64 lowercase hex characters. Real txids:
`8974d08d1c5f9c860d8b629d582a56659a4a1dcb2b5f98a25a5afcc2a784b0f4`

**Implication for T4:** `TransactionIntent.intent_id` is an internal simulator concept
(UUID format). When `txid` fields appear in `Deposit`, `Withdrawal`, `Sweep` records, they
must be 64-character lowercase hex strings — but T4 never sets `txid` fields. Those are
set by T5 after real transactions are broadcast.

---

## 4. Files to create or modify

| File | Action | Stage |
|---|---|---|
| `src/data_model/mod.rs` | Modify — add 2 new structs, extend `ScenarioConfig`, update tests | Stage 1 |
| `configs/scenarios/smoke.yaml` | Rewrite — convert existing nested YAML keys to flat `ScenarioConfig` field names, and add `activity_profiles` and `amounts` sections | Stage 1 |
| `Cargo.toml` | Modify — add 3 dependencies | Stage 2 |
| `src/synthetic/mod.rs` | Modify (currently doc-comment stub) — add `SyntheticPopulation`, `PopulationError`, re-exports | Stage 3 |
| `src/synthetic/generators.rs` | Create — `AccountGenerator`, `TransactionIntentGenerator` | Stages 4–5 |
| `src/synthetic/fixtures.rs` | Create — `write_fixtures`, `FixtureError` | Stage 6 |

No other files are touched. `src/z3/mod.rs`, `src/metrics/mod.rs`, and `src/rpc/mod.rs`
are not modified. `src/main.rs` is not modified (CLI wiring is T8's job).

---

## 5. Stage 1 — T1 additions: new config types

### 5.1 New struct: `ActivityProfileConfig`

Added to `src/data_model/mod.rs` alongside the existing `FlowConfig` and
`ObservabilityConfig` structs.

```
pub struct ActivityProfileConfig {
    pub low_fraction: f64,
    pub medium_fraction: f64,
    pub high_fraction: f64,
}
```

**Invariant (enforced by T7, not T4):** `low_fraction + medium_fraction + high_fraction`
must equal `1.0` within floating-point tolerance (±1e-6). T4 trusts that the config it
receives has been validated. If the fractions do not sum correctly, `WeightedIndex::new`
will either succeed with a skewed distribution or return an error — both are acceptable
at this level because the root cause is an invalid config.

**Serde:** `#[derive(Debug, Clone, Serialize, Deserialize)]` — same as all other config
structs in T1. No `rename_all` needed because field names are already snake_case.

**Tests to add in `src/data_model/mod.rs`:**
- Roundtrip: construct an `ActivityProfileConfig`, serialize to JSON, deserialize, assert
  each field is identical.
- Wire format: assert that the JSON key names are `"low_fraction"`, `"medium_fraction"`,
  `"high_fraction"` (not renamed).

### 5.2 New struct: `AmountRangeConfig`

```
pub struct AmountRangeConfig {
    pub min_zatoshis: u64,
    pub max_zatoshis: u64,
}
```

**Invariant (enforced by T7, not T4):** `min_zatoshis <= max_zatoshis`. If violated,
`rng.gen_range(min..=max)` panics. Again, T4 trusts validated config.

**Note:** `min_zatoshis = 0` is technically valid at the type level but semantically
wrong (sending 0 ZEC is not a real transaction). Validation is T7's responsibility.

**Serde:** same pattern as above.

**Tests to add:**
- Roundtrip for `AmountRangeConfig`.
- Boundary: `min_zatoshis = 0, max_zatoshis = u64::MAX` roundtrips without overflow.

### 5.3 Modified struct: `ScenarioConfig`

Two new fields are added. Two existing fields receive `#[serde(default)]` annotations.
All other existing fields remain unchanged.

**New fields:**

```
pub struct ScenarioConfig {
    // ... all existing fields unchanged ...
    pub activity_profiles: ActivityProfileConfig,  // NEW
    pub amounts: AmountRangeConfig,                // NEW
}
```

**Placement of new fields:** Add after `flows: FlowConfig` (logically grouped with the
other workload-shape config).

**`#[serde(default)]` on `config_hash` and `source_path`:**

These two fields are program-computed at load time by T7 (config hash = SHA-256 of the
raw YAML bytes; source path = absolute path of the YAML file). They must not appear in
any user-authored YAML file. Adding `#[serde(default)]` makes them optional during
deserialization — they default to empty string — and T7 overwrites them immediately after
`serde_yaml::from_str(...)` returns:

```
#[serde(default)]
pub config_hash: String,
#[serde(default)]
pub source_path: String,
```

Without this annotation, deserializing any YAML file that omits `config_hash` or
`source_path` — including the rewritten `smoke.yaml` in §5.4 — would return a hard serde
error ("missing field `config_hash`") before T7 ever has a chance to populate them. This
pre-existed T4 as a latent bug; T4 is the correct place to fix it because T4 is the first
track that exercises YAML loading end-to-end.

**All other user-facing fields are required.** There is no `#[serde(default)]` on
`accounts_count`, `flows`, `activity_profiles`, `amounts`, etc. — a YAML file that omits
those fields fails loudly at deserialization, which is intentional.

**Impact on existing tests in `src/data_model/mod.rs`:**
The tests `scenario_config` and `scenario_config_hash_and_source_path_survive_roundtrip`
both construct `ScenarioConfig` inline. Both must be updated to include the two new fields.
Suggested values for the test fixture:
```
activity_profiles: ActivityProfileConfig { low_fraction: 0.5, medium_fraction: 0.35, high_fraction: 0.15 },
amounts: AmountRangeConfig { min_zatoshis: 10_000, max_zatoshis: 10_000_000 },
```

### 5.4 Updated `configs/scenarios/smoke.yaml`

**The existing `smoke.yaml` must be completely rewritten.** The current file uses nested
YAML keys (`accounts.count`, `load.duration_seconds`, `confirmations.deposit_required`)
that do not match the flat field names in `ScenarioConfig` (`accounts_count`,
`load_duration_seconds`, `confirmations_deposit_required`). Attempting to deserialize the
current file into `ScenarioConfig` would fail immediately. This inconsistency
pre-exists T4 but T4 is the first track that requires the config to be loadable —
resolving it here is the correct place.

**Option chosen:** Rewrite `smoke.yaml` to flat keys matching `ScenarioConfig` exactly,
and add the two new sections (`activity_profiles`, `amounts`). This is the complete new
content of the file:

```yaml
name: smoke
description: "Minimal smoke scenario for early simulator development and CI sanity checks."
seed: 42

accounts_count: 10
accounts_active_fraction: 0.5

load_duration_seconds: 60
load_target_tps: 1.0

flows:
  transparent_to_transparent: 1.0
  transparent_to_shielded: 0.0
  shielded_to_transparent: 0.0
  shielded_to_shielded: 0.0

confirmations_deposit_required: 3

observability:
  record_rpc_calls: true
  record_component_logs: true
  metric_sampling_interval_secs: 5
  mempool_saturation_threshold: 500

activity_profiles:
  low_fraction: 0.50
  medium_fraction: 0.35
  high_fraction: 0.15

amounts:
  min_zatoshis: 10000     # 0.0001 ZEC — minimum realistic transaction size
  max_zatoshis: 10000000  # 0.1 ZEC   — small amounts suitable for smoke testing
```

**Fields NOT included in smoke.yaml:** `config_hash` and `source_path` are computed
programmatically by T7 at load time (hash of the file content, absolute path). They are
not user-editable and must not appear in the YAML file. T7 is responsible for populating
them after deserialization.

**Note on YAML deserialization:** `config_hash` and `source_path` carry `#[serde(default)]`
(see §5.3) so they deserialize to empty string when absent — as they are here. All other
fields in `ScenarioConfig` are required; a YAML file that omits `activity_profiles`,
`amounts`, or any other user-facing field will fail deserialization with a clear error.

### 5.5 T1 edit: add `recipient_account_id` to `TransactionIntent`

**Why this T1 edit is required:** The review identified that the original
`TransactionIntent` struct has `sender_address` and `recipient_address` (address strings)
but no field for the recipient's `account_id`. This causes a correctness problem in the
`intent_generator_distinct_pair` test: the test asserts `sender_address !=
recipient_address`, but during unit tests wallet address pools are always empty, so both
addresses are `"unprovisioned:{account_id}"`. Two different accounts always have
different `account_id` values, so the distinct-pair logic is never exercised by address
comparison — the test passes vacuously.

Storing `recipient_account_id` in `TransactionIntent` solves this: the test can now
assert `intent.account_id != intent.recipient_account_id`, which directly verifies that
the distinct-pair selection loop produced two different accounts, regardless of address
provisioning state.

**Change to `src/data_model/mod.rs`:** Add one field to `TransactionIntent`:

```
pub struct TransactionIntent {
    pub intent_id: String,
    pub run_id: String,
    pub account_id: String,          // sender's account ID
    pub recipient_account_id: String, // NEW — recipient's account ID
    pub sender_address: String,
    pub recipient_address: String,
    pub amount_zatoshis: u64,
    pub fee_zatoshis: u64,
    pub flow_type: FlowType,
    pub status: TransactionStatus,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
}
```

**Impact on existing T1 tests:** Three tests in `src/data_model/mod.rs` construct
`TransactionIntent` with struct literal syntax (no `..` spread) and will produce a
compile error — "missing field `recipient_account_id`" — after the field is added:

| Line range | Test name |
|---|---|
| 350–367 | `transaction_intent` |
| 858–879 | `transaction_intent_submitted_at_option_roundtrips` |
| 914–932 | `zatoshi_amount_one_roundtrips` |

All three must be updated to include `recipient_account_id: "acc-2".into()` (or any
distinct, non-empty string that is different from the `account_id` used in that test).

**Semantics:** `account_id` is the sender; `recipient_account_id` is the recipient.
Both are always present for `T4`-generated intents (T4 always produces a distinct pair).
For intents that represent deposits (where the recipient is an external depositor not in
our account set), `recipient_account_id` may be set to a sentinel value such as
`"external"`. This usage is T5's responsibility and is out of scope for T4.

---

## 6. Stage 2 — Cargo.toml dependencies

Three dependencies are added to `[dependencies]`:

```toml
rand = "0.8"
rand_chacha = "0.3"
uuid = { version = "1", features = ["v4"] }
```

**Why `rand = "0.8"` and `rand_chacha = "0.3"`:**  
These two versions are designed as companions. `rand 0.8` defines the `SeedableRng`,
`Rng`, and distribution traits. `rand_chacha 0.3` implements `ChaCha8Rng` against those
traits. Using mismatched versions (e.g. `rand 0.9` with `rand_chacha 0.3`) will cause
trait bound failures at compile time.

**Why `uuid` with `features = ["v4"]`:**  
The `v4` feature enables `Uuid::new_v4()` and `Builder::from_random_bytes()`. Without
it, only UUID parsing and formatting are available — generation is disabled. We use
`Builder::from_random_bytes` to create v4 UUIDs from seeded random bytes rather than
`Uuid::new_v4()` (which would use the thread-local PRNG and break reproducibility).

**Coordination with T3:** T3 will also need `uuid` to generate `call_id` values. If T3
adds `uuid` independently on its branch, the merge will have both branches adding the
same `Cargo.toml` line. This is a trivial merge conflict (identical insertions) that git
resolves automatically. No coordination is needed beyond awareness.

**No new dev-dependencies:** `tempfile` and `wiremock` are already in `[dev-dependencies]`.
Fixture write tests will use `tempfile::TempDir` (already available).

---

## 7. Stage 3 — Module structure and SyntheticPopulation

### 7.1 `src/synthetic/mod.rs` — full content specification

The current content is a doc-comment stub. Replace with:

1. Module-level doc comment describing T4's purpose.
2. Submodule declarations: `pub mod generators;` and `pub mod fixtures;`.
3. Re-exports: `pub use generators::{AccountGenerator, TransactionIntentGenerator};`
   and `pub use fixtures::{write_fixtures, FixtureError};`.
4. `use` imports from `crate::data_model`.
5. `SyntheticPopulation` struct (public).
6. `PopulationError` enum (public).
7. `impl SyntheticPopulation` block.
8. Tests for `SyntheticPopulation` methods (inline `#[cfg(test)] mod tests`).

### 7.2 `SyntheticPopulation` struct

```
pub struct SyntheticPopulation {
    pub accounts: Vec<Account>,
    pub wallets: Vec<Wallet>,
    pub active_account_ids: Vec<String>,  // pre-filtered list of Active account IDs
    accounts_by_id: HashMap<String, usize>,     // account_id → index into accounts
    wallets_by_account: HashMap<String, usize>, // account_id → index into wallets
}
```

**`accounts` and `wallets` ordering:** Both slices are in the same order — `accounts[i]`
and `wallets[i]` correspond to the same account. This ordering is produced by
`AccountGenerator::generate_population` and must be maintained if the `Vec`s are ever
sorted or filtered.

**`active_account_ids`:** A flat `Vec<String>` of account IDs for accounts with
`status == AccountStatus::Active`. Pre-computed at generation time so that
`TransactionIntentGenerator` can select senders and recipients in O(1) without filtering
the full account list on every call.

**Private indexes:** `accounts_by_id` and `wallets_by_account` are private. Public access
is through the methods below. This prevents callers from inserting accounts into the
`HashMap` without updating the corresponding `Vec`, which would corrupt the structure.

### 7.3 `impl SyntheticPopulation`

#### `pub fn account_by_id(&self, id: &str) -> Option<&Account>`
Looks up an account by its `account_id`. Returns `None` if not found.  
Implementation: `self.accounts_by_id.get(id).map(|&i| &self.accounts[i])`.

#### `pub fn wallet_for_account(&self, account_id: &str) -> Option<&Wallet>`
Looks up the wallet for a given `account_id`. Returns `None` if not found.  
Implementation: `self.wallets_by_account.get(account_id).map(|&i| &self.wallets[i])`.

#### `pub fn add_address(&mut self, account_id: &str, address: Address) -> Result<(), PopulationError>`
Called by T7 after receiving a real Zallet-derived address for an account.

**Logic:**
1. Look up the wallet index via `self.wallets_by_account.get(account_id)`. If not found,
   return `Err(PopulationError::AccountNotFound(account_id.to_string()))`.
2. Match on `address.address_type`:
   - `AddressType::Transparent` → push to `wallets[i].transparent_addresses`
   - `AddressType::Sapling` or `AddressType::Orchard` → push to `wallets[i].shielded_addresses`
3. Return `Ok(())`.

**Why no `AddressTypeMismatch` error?** The initial spec included this error variant but
on review it is unnecessary — every `AddressType` has a clear destination. The error
variant should be removed from `PopulationError` to keep the API minimal.

#### `pub fn active_count(&self) -> usize`
Returns `self.active_account_ids.len()`. Convenience method for T7 and tests.

#### Constructor — not public
`SyntheticPopulation` is constructed only by `AccountGenerator::generate_population`.
There is no `SyntheticPopulation::new()` public constructor. This enforces that
populations are always generated through the proper seeded path.

To support testing in other modules, add a `#[cfg(test)]` constructor:
```
#[cfg(test)]
pub fn new_for_test(
    accounts: Vec<Account>,
    wallets: Vec<Wallet>,
) -> Self
```
This test constructor builds the indexes from the provided slices and computes
`active_account_ids` from accounts with `status == Active`. Used in T5 and T7 tests to
construct populations without going through `AccountGenerator`.

**Visibility scope note:** Because `new_for_test` is marked `#[cfg(test)]`, it is
compiled only in test builds and is therefore NOT accessible from files in the `tests/`
directory (integration tests). It is only accessible from inline `#[cfg(test)] mod tests`
blocks within the same crate. T5 and T7 tests that need a pre-built `SyntheticPopulation`
must use inline tests, or T7 must expose a separate test-utility function in a
`#[cfg(test)]` block in its own module. This is a standard Rust limitation and is not a
T4 bug — it is documented here to prevent confusion when T5/T7 tests are written.

### 7.4 `PopulationError` enum

```
pub enum PopulationError {
    AccountNotFound(String),  // account_id that was not found
}
```

**Implements:** `std::fmt::Display`, `std::error::Error`.

`Display`: `"No account found with id '{}'", account_id`.

---

## 8. Stage 4 — AccountGenerator

### 8.1 File: `src/synthetic/generators.rs`

This file contains both `AccountGenerator` and `TransactionIntentGenerator`. It is
one file because both generators share the seeding logic and the `next_uuid` helper
function.

### 8.2 Seeding strategy

**Primary concern:** `AccountGenerator` and `TransactionIntentGenerator` must be
independently reproducible. If they shared a single `ChaCha8Rng`, the intent sequence
would depend on how many accounts were generated (because each account generation
advances the shared RNG state). This would mean that changing `accounts_count` in the
scenario config also changes all intent values, which is surprising and fragile.

**Solution:** Two separate `ChaCha8Rng` instances, seeded from two independent values
derived from `config.seed`:

```
Account generator seed: config.seed
Intent generator seed:  config.seed XOR INTENT_SEED_SALT
```

Where `INTENT_SEED_SALT` is a compile-time constant chosen to be non-zero and
non-trivial, ensuring the two seeds are always distinct even for common seed values
(0, 1, etc.):

```
const INTENT_SEED_SALT: u64 = 0x9E37_79B9_7F4A_7C15;
```

This value is the fractional part of the golden ratio × 2^64, a standard mixing
constant. It guarantees that `config.seed XOR INTENT_SEED_SALT != config.seed` for all
possible seed values.

**Important:** The account population and the intent sequence are now fully independent.
Calling `generator.next_intent()` 1000 times does not change what `AccountGenerator`
would produce for a new run. Calling `AccountGenerator::generate_population()` does not
affect what `TransactionIntentGenerator` will produce.

### 8.3 `AccountGenerator` struct

```
pub struct AccountGenerator {
    rng: ChaCha8Rng,
    config: ScenarioConfig,
    profile_dist: WeightedIndex<f64>,
}
```

**`profile_dist`:** Pre-computed in `AccountGenerator::new` to avoid recomputing the
`WeightedIndex` on every account. If the fractions in `config.activity_profiles` are
invalid (e.g. all zero), `WeightedIndex::new` returns an error.

### 8.4 `AccountGenerator::new(config: ScenarioConfig) -> Result<Self, GeneratorError>`

**Steps:**
1. Build `profile_dist` from `[config.activity_profiles.low_fraction, medium_fraction, high_fraction]`.
   If `WeightedIndex::new` fails (all-zero weights or negative weights), return
   `Err(GeneratorError::InvalidActivityProfileWeights)`.
2. Seed `rng = ChaCha8Rng::seed_from_u64(config.seed)`.
3. Return `Ok(Self { rng, config, profile_dist })`.

**Why `Result<Self, ...>` and not panicking:** Returning a `Result` here means T7's
config validation can surface a clear error ("invalid activity profile weights") rather
than a panic. The error occurs at generator construction time, before any run phases
begin.

**Why `AccountGenerator` takes `config` by value while `TransactionIntentGenerator` takes
`config` by `&ScenarioConfig`:** `AccountGenerator` must own the config for the lifetime
of its RNG loop — it accesses `config.accounts_count`, `config.accounts_active_fraction`,
`config.activity_profiles`, and `config.seed` on every iteration. Storing a reference
would tie the generator's lifetime to the caller's `ScenarioConfig` binding without
benefit. `TransactionIntentGenerator`, on the other hand, only needs a few scalar values
at construction time (`config.seed`, `config.amounts.min_zatoshis`,
`config.amounts.max_zatoshis`, `config.flows.*`); it copies these into its own fields and
does not need the config struct after construction. Taking `&ScenarioConfig` keeps the API
honest about this — and callers who want to retain the config simply do not need to
`.clone()` it for the intent generator. Callers who want to pass the same `config` to
both generators should `.clone()` before calling `AccountGenerator::new`.

### 8.5 `AccountGenerator::generate_population(&mut self) -> Result<SyntheticPopulation, GeneratorError>`

Generates the full population in one call. Consumes the `AccountGenerator`'s RNG state.
May be called only once (calling it twice produces a second population with a different
RNG state, which is not the same as the first — this is documented behavior, not a bug).

**Return type is `Result`:** The function can fail if `accounts_count` exceeds
`usize::MAX` (on 32-bit platforms where `usize` is 32 bits, this would happen for counts
above ~4 billion). The simulator targets 64-bit Linux where this cannot occur in practice,
but the cast must be explicit and checked to avoid undefined behaviour on other platforms.

**Algorithm:**
```
let n = usize::try_from(self.config.accounts_count)
    .map_err(|_| GeneratorError::AccountCountTooLarge)?;
let mut accounts = Vec::with_capacity(n);
let mut wallets = Vec::with_capacity(n);
let mut accounts_by_id = HashMap::with_capacity(n);
let mut wallets_by_account = HashMap::with_capacity(n);
let mut active_account_ids = Vec::new();

for i in 0..n {
    let account_id = next_uuid(&mut self.rng);
    let wallet_id  = next_uuid(&mut self.rng);
    let status     = sample_account_status(&mut self.rng, self.config.accounts_active_fraction);
    let profile    = sample_activity_profile(&mut self.rng, &self.profile_dist);

    let account = Account {
        account_id: account_id.clone(),
        status: status.clone(),
        activity_profile: profile,
        wallet_id: wallet_id.clone(),
        created_at: Utc::now(),
    };
    let wallet = Wallet {
        wallet_id: wallet_id.clone(),
        account_id: account_id.clone(),
        transparent_addresses: vec![],
        shielded_addresses: vec![],
        created_at: Utc::now(),
    };

    if status == AccountStatus::Active {
        active_account_ids.push(account_id.clone());
    }
    accounts_by_id.insert(account_id.clone(), i);
    wallets_by_account.insert(account_id.clone(), i);
    accounts.push(account);
    wallets.push(wallet);
}

Ok(SyntheticPopulation { accounts, wallets, active_account_ids, accounts_by_id, wallets_by_account })
```

**RNG call order per account (critical for determinism):**
1. `next_uuid` for `account_id` (consumes 16 bytes = 128 RNG bits)
2. `next_uuid` for `wallet_id` (consumes 16 bytes)
3. `rng.gen::<f64>()` for `AccountStatus` sampling (1 RNG step)
4. `profile_dist.sample(&mut self.rng)` for `ActivityProfile` (1–2 RNG steps)

This order is fixed and must not change. Adding a new field to this sequence in the
future would break backward compatibility of the seed.

**`created_at` timestamps:** Use `Utc::now()` at generation time. Timestamps are NOT
seeded — they reflect the wall clock when the run executes. This is intentional: the
`created_at` field is used for ordering and human display, not for reproducibility. The
reproducibility guarantee applies to IDs, statuses, and profiles — not timestamps.

### 8.6 Helper functions (private to `generators.rs`)

#### `fn next_uuid(rng: &mut ChaCha8Rng) -> String`
```
let bytes: [u8; 16] = rng.gen();
uuid::Builder::from_random_bytes(bytes).into_uuid().to_string()
```
Produces a version-4, variant-1 UUID in hyphenated lowercase format:
`xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`. Identical in structure to Zallet's output.

#### `fn sample_account_status(rng: &mut ChaCha8Rng, active_fraction: f64) -> AccountStatus`
```
if rng.gen::<f64>() < active_fraction {
    AccountStatus::Active
} else {
    AccountStatus::Inactive
}
```
`gen::<f64>()` returns a value in `[0.0, 1.0)`. With `active_fraction = 1.0`, all
accounts are Active. With `active_fraction = 0.0`, all accounts are Inactive.
With `active_fraction = 0.5`, approximately half are Active on average.

#### `fn sample_activity_profile(rng: &mut ChaCha8Rng, dist: &WeightedIndex<f64>) -> ActivityProfile`
```
match dist.sample(rng) {
    0 => ActivityProfile::Low,
    1 => ActivityProfile::Medium,
    2 => ActivityProfile::High,
    _ => unreachable!(),
}
```
The `WeightedIndex` was built with exactly 3 weights, so the sample result is always
0, 1, or 2.

---

## 9. Stage 5 — TransactionIntentGenerator

### 9.1 `TransactionIntentGenerator` struct

```
pub struct TransactionIntentGenerator {
    rng: ChaCha8Rng,
    flow_dist: WeightedIndex<f64>,
    flow_variants: [FlowType; 4],
    min_amount: u64,
    max_amount: u64,
    // Snapshot of active account IDs taken at construction time.
    // If accounts become inactive during a run (not a current feature), this list
    // does not update. For T4's scope, the active set is fixed at generation time.
    active_account_ids: Vec<String>,
}
```

**`flow_variants`:** Fixed-order array `[TToT, TToZ, ZToT, ZToZ]` matching the weight
order in `FlowConfig`. The index returned by `flow_dist.sample` maps directly to this
array.

### 9.2 `TransactionIntentGenerator::new(population: &SyntheticPopulation, config: &ScenarioConfig) -> Result<Self, GeneratorError>`

**Steps:**
1. Build `flow_dist` from `[flows.transparent_to_transparent, transparent_to_shielded, shielded_to_transparent, shielded_to_shielded]`.
   Return `Err(GeneratorError::InvalidFlowWeights)` if `WeightedIndex::new` fails.
2. Derive sub-seed: `let sub_seed = config.seed ^ INTENT_SEED_SALT`.
3. Seed `rng = ChaCha8Rng::seed_from_u64(sub_seed)`.
4. Clone `population.active_account_ids` into `active_account_ids`.
5. Return `Ok(Self { rng, flow_dist, flow_variants: [TToT, TToZ, ZToT, ZToZ], min_amount: config.amounts.min_zatoshis, max_amount: config.amounts.max_zatoshis, active_account_ids })`.

**Why clone `active_account_ids`?** The generator holds its own copy so it is not
coupled to the lifetime of `SyntheticPopulation`. T7 may want to move or store the
generator separately from the population.

### 9.3 `TransactionIntentGenerator::next_intent(&mut self, run_id: &str, population: &SyntheticPopulation) -> Option<TransactionIntent>`

Returns `None` if fewer than 2 active accounts exist (cannot form a distinct pair).
Returns `Some(TransactionIntent)` otherwise.

**Algorithm:**

```
Step 1 — Guard
  if self.active_account_ids.len() < 2 { return None; }

Step 2 — Sample flow type
  let flow_type = self.flow_variants[self.flow_dist.sample(&mut self.rng)].clone();

Step 3 — Select distinct sender and recipient accounts
  let n = self.active_account_ids.len();
  let sender_idx = self.rng.gen_range(0..n);
  let recipient_idx = loop {
      let idx = self.rng.gen_range(0..n);
      if idx != sender_idx { break idx; }
  };
  let sender_account_id   = &self.active_account_ids[sender_idx];
  let recipient_account_id = &self.active_account_ids[recipient_idx];

Step 4 — Resolve addresses from population
  let sender_address   = resolve_address(population, sender_account_id,   &flow_type, Side::Sender);
  let recipient_address = resolve_address(population, recipient_account_id, &flow_type, Side::Recipient);

Step 5 — Sample amount
  let amount_zatoshis = self.rng.gen_range(self.min_amount..=self.max_amount);

Step 6 — Generate intent_id
  let intent_id = next_uuid(&mut self.rng);

Step 7 — Build TransactionIntent
  Some(TransactionIntent {
      intent_id,
      run_id: run_id.to_string(),
      account_id: sender_account_id.to_string(),
      recipient_account_id: recipient_account_id.to_string(),  // NEW field (see §5.5)
      sender_address,
      recipient_address,
      amount_zatoshis,
      fee_zatoshis: 0,        // Zallet auto-computes via ZIP 317
      flow_type,
      status: TransactionStatus::Pending,
      created_at: Utc::now(),
      submitted_at: None,
  })
```

**RNG call order per intent (critical for determinism):**
1. `flow_dist.sample` — flow type
2. `rng.gen_range(0..n)` — sender index
3. `rng.gen_range(0..n)` — recipient index (may repeat once on collision)
4. `rng.gen_range(min..=max)` — amount
5. `next_uuid` — intent_id (16 bytes)

This order is fixed.

### 9.4 Address resolution — `resolve_address` (private helper)

```
fn resolve_address(
    population: &SyntheticPopulation,
    account_id: &str,
    flow_type: &FlowType,
    side: Side,
) -> String
```

where `Side` is a private enum `{ Sender, Recipient }`.

**Logic:**

The flow type determines which address pool to draw from for each side:

| FlowType | Sender pool | Recipient pool |
|---|---|---|
| `TToT` | transparent | transparent |
| `TToZ` | transparent | shielded |
| `ZToT` | shielded | transparent |
| `ZToZ` | shielded | shielded |

```
let wallet = match population.wallet_for_account(account_id) {
    Some(w) => w,
    None => return format!("unprovisioned:{}", account_id),
};

let pool: &Vec<Address> = match (flow_type, side) {
    (TToT | TToZ, Sender)     => &wallet.transparent_addresses,
    (TToT | ZToT, Recipient)  => &wallet.transparent_addresses,
    (ZToT | ZToZ, Sender)     => &wallet.shielded_addresses,
    (TToZ | ZToZ, Recipient)  => &wallet.shielded_addresses,
};

if pool.is_empty() {
    return format!("unprovisioned:{}", account_id);
}

pool[0].address.clone()
```

**Why `pool[0]`?** During T4's scope (no live run), pools are always empty and the
placeholder is returned. During a live run (T7), T7 calls `add_address` before starting
intent generation, so each wallet has at least one address of each required type. Using
`pool[0]` is correct for the single-address case. A future enhancement (multiple deposit
addresses per account) would need to randomly select from the pool using the generator's
RNG — documented as a known limitation.

**Placeholder format:** `"unprovisioned:{account_id}"` — clearly synthetic, not a valid
Zcash address, will fail immediately if submitted to an RPC call. Used only in fixture
generation and dry-run scenarios.

### 9.5 `GeneratorError` enum

```
pub enum GeneratorError {
    InvalidActivityProfileWeights(String),  // underlying rand error message
    InvalidFlowWeights(String),
    AccountCountTooLarge,                   // accounts_count exceeds usize::MAX
}
```

Implements `Display` and `Error`. Used by `AccountGenerator::new`,
`AccountGenerator::generate_population`, and `TransactionIntentGenerator::new`.

`Display` for `AccountCountTooLarge`:
`"accounts_count exceeds the maximum value representable as usize on this platform"`.

In practice `AccountCountTooLarge` can only be returned on 32-bit platforms; the
simulator targets 64-bit Linux where `usize::MAX` = `u64::MAX` and the error is
unreachable. The check is present for correctness, not for expected usage.

---

## 10. Stage 6 — write\_fixtures

### 10.1 File: `src/synthetic/fixtures.rs`

Contains one public function and one error type.

### 10.2 `pub fn write_fixtures(population: &SyntheticPopulation, out_dir: &Path) -> Result<(), FixtureError>`

**What it writes:**
- `<out_dir>/accounts.json` — pretty-printed JSON array of all `Account` records
- `<out_dir>/wallets.json` — pretty-printed JSON array of all `Wallet` records

**What it does NOT write:**
- Transaction intents — generated on demand, not pre-computed.
- Manifest or summary — those are T6's responsibility.

**Algorithm:**
1. Create `out_dir` and all parent directories if they do not exist
   (`std::fs::create_dir_all(out_dir)`). If this fails, return `Err(FixtureError::Io(e))`.
2. Serialize `&population.accounts` to pretty JSON: `serde_json::to_string_pretty(...)`.
   If this fails, return `Err(FixtureError::Serialisation(e))`.
3. Write the JSON string to `out_dir/accounts.json`. If write fails, return `Err(FixtureError::Io(e))`.
4. Same for `&population.wallets` → `out_dir/wallets.json`.
5. Return `Ok(())`.

**Idempotency:** Calling `write_fixtures` twice with the same population and output
directory overwrites the files silently. This is correct behavior — the files are
deterministic given the population, so overwriting produces the same content.

**Synchronous I/O:** This function uses `std::fs` (synchronous), not `tokio::fs`. T4 has
no async context — it is purely synchronous Rust. If T8 calls this from an async context
it must either use `tokio::task::spawn_blocking` or accept the brief blocking call (for
fixture generation, which is a one-shot developer tool, blocking is acceptable).

### 10.3 `FixtureError` enum

```
pub enum FixtureError {
    Io(std::io::Error),
    Serialisation(serde_json::Error),
}
```

Implements `Display` and `Error`.

`Display`:
- `Io(e)`: `"Failed to write fixture file: {e}"`
- `Serialisation(e)`: `"Failed to serialise fixture data: {e}"`

---

## 11. Stage 7 — Test plan

All tests are inline in their respective files using `#[cfg(test)] mod tests { ... }`.
No external test files are created for T4. All tests pass without Docker, network, or
a live Z3 stack.

Before opening a PR, run:
```sh
cargo test          # all tests pass, including existing 63
cargo clippy -- -D warnings   # zero warnings
cargo fmt -- --check          # no drift
```

### 11.1 Tests for T1 additions (`src/data_model/mod.rs`)

| Test name | What it verifies |
|---|---|
| `activity_profile_config_roundtrip` | `ActivityProfileConfig` survives JSON serialise → deserialise with all values preserved |
| `activity_profile_config_field_names` | JSON keys are exactly `"low_fraction"`, `"medium_fraction"`, `"high_fraction"` |
| `amount_range_config_roundtrip` | `AmountRangeConfig` survives JSON roundtrip |
| `amount_range_config_u64_max` | `max_zatoshis = u64::MAX` roundtrips without overflow |
| Update `scenario_config` | Add new fields to existing test inline |
| Update `scenario_config_hash_and_source_path_survive_roundtrip` | Add new fields to existing test inline |

### 11.2 Tests for `SyntheticPopulation` (`src/synthetic/mod.rs`)

| Test name | What it verifies |
|---|---|
| `population_account_by_id_found` | Returns the correct `Account` when ID exists |
| `population_account_by_id_not_found` | Returns `None` for an unknown ID |
| `population_wallet_for_account_found` | Returns the correct `Wallet` for a known account ID |
| `population_wallet_for_account_not_found` | Returns `None` for an unknown account ID |
| `population_add_address_transparent` | Adds to `transparent_addresses`; shielded list stays empty |
| `population_add_address_shielded_sapling` | Adds Sapling address to `shielded_addresses` |
| `population_add_address_shielded_orchard` | Adds Orchard address to `shielded_addresses` |
| `population_add_address_unknown_account` | Returns `PopulationError::AccountNotFound` |
| `population_active_count` | Returns count of active accounts |
| `population_active_account_ids_all_active` | `active_fraction = 1.0` means `active_count == accounts_count` |
| `population_active_account_ids_none_active` | `active_fraction = 0.0` means `active_count == 0` |
| `population_new_for_test_builds_correct_indexes` | Construct a population via `new_for_test` with 3 accounts (2 Active, 1 Inactive); assert `account_by_id` returns the correct account for each known ID, `wallet_for_account` returns the correct wallet, and `active_count() == 2` |

### 11.3 Tests for `AccountGenerator` (`src/synthetic/generators.rs`)

| Test name | What it verifies |
|---|---|
| `account_generator_determinism` | Calling `generate_population()` twice with the same config (via two fresh generators) produces identical `account_id` sequences |
| `account_generator_population_size` | Population length equals `config.accounts_count` |
| `account_generator_wallet_account_correspondence` | `accounts[i].wallet_id == wallets[i].wallet_id` and `wallets[i].account_id == accounts[i].account_id` for all i |
| `account_generator_active_fraction_one` | `active_fraction = 1.0` → all accounts `Active` |
| `account_generator_active_fraction_zero` | `active_fraction = 0.0` → all accounts `Inactive` |
| `account_generator_active_fraction_half` | `active_fraction = 0.5`, N = 1000: roughly 45–55% are Active (within 3σ) |
| `account_generator_activity_profile_distribution` | N = 1000 accounts, profile fractions 0.5/0.35/0.15: each bin within 5% of target fraction |
| `account_generator_ids_are_uuids` | Each `account_id` and `wallet_id` matches the regex `[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}` |
| `account_generator_wallet_addresses_empty` | All generated wallets have `transparent_addresses` and `shielded_addresses` equal to `vec![]` |
| `account_generator_invalid_weights` | All-zero activity profile weights returns `Err(GeneratorError::InvalidActivityProfileWeights(_))` |
| `account_generator_negative_weights` | Negative weight (e.g. `low_fraction = -0.1`) returns `Err(GeneratorError::InvalidActivityProfileWeights(_))` |
| `account_generator_zero_accounts` | `accounts_count = 0` produces an empty population (Ok with zero-length vecs) without panicking |
| `account_generator_count_too_large` | Marked `#[cfg(target_pointer_width = "32")]`. On 32-bit targets, construct a config with `accounts_count = u64::MAX`, call `generate_population()`, assert `Err(GeneratorError::AccountCountTooLarge)`. On 64-bit (the primary target), `usize::MAX == u64::MAX` so the error path is provably unreachable and the test does not compile into the binary — avoiding a false assertion. The `AccountCountTooLarge` variant remains in the enum as a public API guarantee for 32-bit consumers; the gated test justifies its existence. |

### 11.4 Tests for `TransactionIntentGenerator` (`src/synthetic/generators.rs`)

| Test name | What it verifies |
|---|---|
| `intent_generator_determinism` | Same seed + same population → same first 100 intents (same intent_id sequence) |
| `intent_generator_none_with_zero_active` | Returns `None` when `active_count = 0` |
| `intent_generator_none_with_one_active` | Returns `None` when `active_count = 1` |
| `intent_generator_distinct_pair` | Over 1000 draws: no intent has `intent.account_id == intent.recipient_account_id` (directly tests the distinct-pair selection loop using account IDs, which are always set regardless of address provisioning state) |
| `intent_generator_flow_distribution` | 1000 draws with all-TToT config → 100% of intents are `TToT` |
| `intent_generator_flow_distribution_all_t_to_z` | 1000 draws with all-TToZ config → 100% of intents are `TToZ` |
| `intent_generator_flow_distribution_all_z_to_t` | 1000 draws with all-ZToT config → 100% of intents are `ZToT` |
| `intent_generator_flow_distribution_all_z_to_z` | 1000 draws with all-ZToZ config → 100% of intents are `ZToZ` |
| `intent_generator_flow_distribution_mixed` | 1000 draws with equal 25% weights → each flow type appears between 20% and 30% |
| `intent_generator_amount_in_range` | All drawn `amount_zatoshis` satisfy `min_zatoshis <= amount <= max_zatoshis` |
| `intent_generator_amount_fixed_range` | `min = max = 500_000` → all amounts equal 500_000 |
| `intent_generator_intent_id_is_uuid` | Each `intent_id` matches UUID v4 format regex |
| `intent_generator_fee_is_zero` | All generated intents have `fee_zatoshis = 0` |
| `intent_generator_status_is_pending` | All generated intents have `status = Pending` |
| `intent_generator_submitted_at_is_none` | All generated intents have `submitted_at = None` |
| `intent_generator_run_id_propagated` | `intent.run_id` equals the `run_id` argument passed to `next_intent` |
| `intent_generator_sender_is_active_account` | `intent.account_id` is always in `active_account_ids` |
| `intent_generator_rng_is_seeded_independently_of_account_generator` | Create two AccountGenerators with the same seed but different `accounts_count` (e.g. 5 and 10). Create a TransactionIntentGenerator for each resulting population. Draw the first intent from each generator. Assert that `intent.flow_type` is identical between the two. `flow_type` is always draw #1 in each intent (§9.3 RNG call order), made before any account-selection draw, so its value depends only on the intent sub-seed which is `config.seed ^ INTENT_SEED_SALT` — the same for both generators. Do NOT assert a sequence of multiple intents: after draw #2 (`gen_range(0..n)`, sender index), a collision in the recipient retry loop (draw #3) occurs with probability ≈20% for `n=5` and ≈10% for `n=10`. These are different events on the same underlying u64, so roughly 26% of the time exactly one generator collides and the two RNG streams shift by one u64. Every subsequent draw — including the `flow_type` of intent #2 — is then taken from a different stream offset. Over 10 intents, the probability of no divergence is ≈(0.74)^10 ≈ 4%, making any multi-intent assertion reliably flaky. |
| `intent_generator_invalid_flow_weights` | All-zero flow weights returns `Err(GeneratorError::InvalidFlowWeights(_))` |

### 11.5 Tests for `write_fixtures` (`src/synthetic/fixtures.rs`)

| Test name | What it verifies |
|---|---|
| `write_fixtures_creates_accounts_json` | After call, `out_dir/accounts.json` exists |
| `write_fixtures_creates_wallets_json` | After call, `out_dir/wallets.json` exists |
| `write_fixtures_account_count_matches` | `accounts.json` parses as a JSON array with length equal to population size |
| `write_fixtures_account_ids_match` | Deserialized `account_id` values in `accounts.json` match `population.accounts` |
| `write_fixtures_is_idempotent` | Calling `write_fixtures` twice with the same args produces identical file content |
| `write_fixtures_creates_output_dir` | Output directory does not need to exist before the call |
| `write_fixtures_pretty_json` | Output files contain newlines (i.e. are pretty-printed, not compact) |
| `write_fixtures_empty_population` | Empty population (zero accounts) → `accounts.json` contains `[]` and `wallets.json` contains `[]`; no error is returned |

All fixture tests use `tempfile::TempDir::new().unwrap()` as the output directory.

---

## 12. Integration contract for T7

This section defines exactly what T7 (Scenario Runner) must do to use T4 correctly.
It is written here so the T4 reviewer can confirm the T4 API makes T7's job straightforward.

### Provisioning phase (T7 calls into T4)

```rust
// 1. Build population from scenario config
let mut generator = AccountGenerator::new(config.clone())?;
let mut population = generator.generate_population()?;  // returns Result since §8.5

// 2. For each account, provision in Zallet and record real addresses
for account in &population.accounts {
    // Call z_getnewaccount to register account in Zallet
    let account_info = rpc_client.z_get_new_account(&account.account_id).await?;
    // The returned account_uuid should match account.account_id if T7 uses it as the name.
    // (Zallet generates its own UUID; T7 may need to store the mapping.)

    // Call z_getaddressforaccount to get a Unified Address for this account
    let ua_response = rpc_client.z_get_address_for_account(&account_info.account_uuid).await?;

    // Decode the Unified Address into individual receiver addresses (T7/T3 responsibility)
    // and call add_address for each receiver type:
    for (receiver_type, address_string) in decode_unified_address(&ua_response) {
        let address = Address {
            address_id: uuid::Uuid::new_v4().to_string(),
            wallet_id: account.wallet_id.clone(),
            address: address_string,
            address_type: receiver_type,
            purpose: AddressPurpose::Deposit,
            created_at: Utc::now(),
            last_used_at: None,
        };
        population.add_address(&account.account_id, address)?;
    }
}

// 3. Build intent generator — AFTER provisioning is complete
let mut intent_gen = TransactionIntentGenerator::new(&population, &config)?;

// 4. TPS scheduler loop (simplified)
let run_id = derive_run_id(&config);
loop {
    if let Some(intent) = intent_gen.next_intent(&run_id, &population) {
        tokio::spawn(async move { exchange_emulation::dispatch(intent).await });
    }
    tps_limiter.tick().await;
}
```

**Contract guarantees T7 can rely on:**
- `generate_population()` returns `Err(GeneratorError::AccountCountTooLarge)` only on
  32-bit targets when `accounts_count > u32::MAX`; on 64-bit Linux this path is
  unreachable. On 64-bit, the only failure modes are handled before `generate_population`
  is reached (invalid weights surface in `AccountGenerator::new`).
- `add_address` returns `Err` only if `account_id` is not in the population.
- `next_intent` returns `None` only when fewer than 2 active accounts exist.
- `next_intent` never panics.
- All returned `intent_id` values are globally unique (UUID v4 with seeded RNG — collision
  probability is negligible across any realistic number of intents).

**Contract T7 must uphold:**
- Provisioning (`add_address`) must complete for all accounts before `next_intent` is
  called in a live run. Calling `next_intent` before provisioning returns intents with
  `"unprovisioned:{account_id}"` addresses, which will cause T5's RPC calls to fail.
- T7 must validate `ScenarioConfig` (fraction sums, amount range sanity) before passing
  it to T4 constructors.

---

## 13. Out of scope for T4

The following items will not be implemented in T4 and should not appear in this branch.

| Item | Owner |
|---|---|
| Decoding Unified Addresses into receiver types | T7 or T3 |
| Balance tracking or spending limit enforcement | T5 |
| Actual RPC calls to Zallet for address generation | T3 |
| CLI wiring of `generate-fixtures` subcommand | T8 |
| `make generate-fixtures` Makefile target update | T8 |
| Async I/O in fixtures writer | Not needed (synchronous is correct) |
| Multiple deposit addresses per account | Future enhancement; document as known limitation |
| Hot-wallet account concept | T5 (Exchange Emulation) |
| `tests/fixtures/reference/` RPC response fixtures | T3 (should be created when T3 is implemented) |
| GitHub Actions CI config | T9 |

---

## 14. Open items for the reviewer

The following points are flagged for the reviewer to assess. Some may require
implementation corrections; others are confirmations that the design is sound.

1. **`created_at` timestamps are not seeded.** Accounts and wallets use `Utc::now()`
   for `created_at`. This means two runs with the same seed produce the same
   `account_id` values but different `created_at` values. Is this acceptable? The
   proposal's reproducibility guarantee is about transaction flows and load patterns,
   not about wall-clock timestamps. Confirm or correct.

2. **`next_intent` takes `&SyntheticPopulation` — lifetime coupling.** Callers must
   keep the population alive as long as the generator is in use. T7 will hold both
   in the same `RunState` struct, so this is fine. Confirm no lifetime issues arise
   in the T7 context.

3. **Retry loop for distinct pair selection.** The loop `loop { let idx = rng.gen_range(0..n); if idx != sender_idx { break idx; } }` is correct and terminates with
   probability 1.0 when `n >= 2`. However, with `n = 2`, there is exactly one valid
   recipient and the loop resolves in at most 2 draws on average. With `n = 1`, the
   loop would be infinite — guarded by the `< 2` check. Confirm the guard is
   sufficient and the loop terminates correctly.

4. **`write_fixtures` uses synchronous I/O.** T8 will call this from a CLI context
   that may or may not be async. If T8 uses Tokio, it should wrap this call in
   `tokio::task::spawn_blocking`. This is T8's responsibility, but the spec should
   note it explicitly. Confirm this note is sufficient or add a `async` variant.

5. ~~**`accounts_count` is `u64` in `ScenarioConfig` but `usize` is needed for `Vec::with_capacity`.** Casting `u64 → usize` is platform-dependent on 32-bit systems
   (would overflow for counts > 2^32). The simulator targets 64-bit Linux — confirm
   this cast is safe and add a defensive check if reviewers prefer.~~
   **Resolved (C3):** `usize::try_from(self.config.accounts_count).map_err(|_| GeneratorError::AccountCountTooLarge)?` replaces the bare `as usize` cast. `generate_population` now returns `Result<SyntheticPopulation, GeneratorError>`. See §8.5 and §9.5.

6. **Zallet account UUID vs simulator account UUID.** T7's provisioning loop calls
   `z_getnewaccount(name)` where `name` is the simulator's `account.account_id` (a
   UUID). Zallet generates and returns its own UUID (`account_uuid`). These two UUIDs
   are different — Zallet does not use the caller's name as its UUID. T7 must store
   the mapping `{ simulator_account_id → zallet_account_uuid }` to call
   `z_getaddressforaccount` with the right UUID. T4 does not need to store this
   mapping (it is T7's responsibility), but the spec should flag this to avoid T7
   accidentally using the simulator UUID where the Zallet UUID is required.

7. **`pool[0].address.clone()` for address selection.** If an account has multiple
   transparent or shielded addresses (because T7 called `add_address` multiple times),
   `next_intent` always uses the first one. This is a known limitation noted above.
   Confirm the reviewer accepts this as a deliberate scope boundary, not a bug.

8. **`FlowType` match exhaustiveness in `resolve_address`.** The match on
   `(flow_type, side)` must be exhaustive across all 4 FlowType × 2 Side combinations
   = 8 arms. The implementation outline above collapses them with `|` patterns. Confirm
   the Rust compiler will catch any missing arm.

9. **Statistical tests use hard-coded tolerances (5%, 3σ).** The distribution tests
   sample N = 1000 draws. At N = 1000 and true probability p = 0.5, the 3σ range is
   approximately [47%, 53%]. A tolerance of ±5% should be safely inside 3σ for all
   configured fractions down to 15%. Confirm or tighten the tolerance and minimum N.

10. ~~**`smoke.yaml` YAML deserialization structure.** The existing `smoke.yaml` uses a
    nested structure (`accounts.count`, `load.duration_seconds`) that differs from the
    flat `ScenarioConfig` field names (`accounts_count`, `load_duration_seconds`). This
    gap currently exists and would cause deserialization to fail — T4 does not implement
    YAML deserialization (that is T7's job), but if any existing code path attempts to
    deserialize `smoke.yaml` into `ScenarioConfig` directly, it will fail on the nested
    keys. **The reviewer should flag whether this inconsistency pre-exists in the
    codebase or whether T4 is expected to resolve it.**~~
    **Resolved (C1):** `smoke.yaml` is completely rewritten to use flat keys matching
    `ScenarioConfig` exactly, plus the two new sections. See §5.4 for the full new content.

---

*Document version: pre-implementation, written for review before any code is written.*  
*Branch: `workplan-task-4-synthetic-generators`*  
*All decisions in this document were made explicitly before writing began.*
