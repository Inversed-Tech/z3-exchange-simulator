# Project Scope — Z3 Exchange Simulator

## Objective

The Z3 stack (Zebra, Zaino, Zallet) is the Zcash Foundation's replacement for the
deprecated `zcashd` reference client. For this transition to succeed in production, the
stack needs to hold up under the operational load of the parties that actually move
volume: exchanges.

The problem is that exchanges do not share their internal transaction and custody
infrastructure. Without representative load data, Z3 is likely to be evaluated by
integrators only after something breaks in production. The only external data point
available at the time of this engagement described an account base of 12–15 users with
fewer than ten monthly transactions — useful context, but not a stress test.

**This project builds a reproducible, open-source exchange simulator that stress-tests Z3
under realistic exchange-scale load, measures where the stack starts to degrade, and
hands the results — along with the tool itself — back to the Foundation and the wider
Zcash developer community.**

The engagement produces two things:
1. A **findings report** tied to specific pinned commits of each Z3 component.
2. A **testing framework** that the Foundation, the component teams, and future
   contributors can run in their own CI pipelines indefinitely after handover.

---

## Engagement priorities

**Realistic exchange-scale load.** The simulation models account populations in the
thousands. The design ceiling is Binance-scale usage: tens of thousands of ZEC-holding
users.

**Open source and reusable.** The output is a testing framework runnable by the
Foundation, by the Zebra, Zaino, and Zallet teams, and by ecosystem developers in
their own pipelines.

**Findings tied to specific commits.** Every result in the findings report references
the pinned commits of each Z3 component, making results reproducible and verifiable.

**Regtest-focused, isolated environments.** All testing runs in regtest mode against
controlled local networks. No mainnet usage, no real funds, no real user data.

---

## Simulator architecture

The simulator has five functional parts:

### 1. Load generator
Provisions thousands of wallet accounts across transparent and shielded address types,
with configurable activity distributions (fraction active, transaction frequency per
account). Seeded deterministically so any run can be reproduced exactly.

### 2. Exchange emulation
Models how a typical cryptocurrency exchange would operate on the Z3 stack:
- Assigning deposit addresses to users
- Detecting and crediting incoming deposits (confirmation tracking)
- Processing withdrawals (constructing and broadcasting transactions)
- Hot-wallet sweeps (consolidating many deposit UTXOs into one address)
- Balance tracking across accounts
- Fee estimation (noting: fees are now auto-computed by Zallet via ZIP 317)

### 3. Z3 integration
Drives the Z3 Docker Compose stack through its unified RPC Router endpoint (`:8181`
in regtest). The router transparently forwards calls to the correct backend:
- **Zebra** — block processing, chain state, regtest block generation
- **Zallet** — wallet operations: account creation, address derivation, transaction
  construction and signing, balance queries, async shielded transaction tracking
- **Zaino** — indexing layer; covered directly via its zcashd-style JSON-RPC mirror
  (regtest `:28237`, outside the router). Its lightwalletd `CompactTxStreamer` gRPC
  surface (`:28137`) is documented but out of scope for this engagement.

Every RPC call is recorded (method, backend, latency, success/failure) for the
compatibility matrix and per-method latency histograms.

### 4. Observability
Captures, during each run:
- Per-RPC latency (P50, P95, P99 per method)
- Mempool size over time and saturation events
- Memory and CPU profiles for each Z3 process
- Failure traces and error codes
- ZK proof generation time for shielded transactions

Output format: JSONL files written incrementally per run, plus a human-readable
summary. See [`docs/architecture/observability.md`](architecture/observability.md).

### 5. Scenario harness
Parameterises workload shapes and exposes them via CLI and CI-friendly interfaces.
Required scenarios (minimum):
- **Steady-state** — constant TPS, baseline exchange behaviour
- **Ramp** — linearly increasing TPS, find the inflection point
- **Burst** — spike then recovery, model sudden volume events
- **Mixed** — steady load with a shielded transaction mix, exercise full shielded RPC
  surface

Each scenario is a YAML file. Seed, account count, duration, TPS target, and
transparent/shielded ratio are all configurable. See
[`docs/scenarios/scenario-design.md`](scenarios/scenario-design.md).

---

## Testing methodology

- All testing runs in **regtest mode**, in isolated networks, with deterministic seeding
  where useful.

- **Load-curve experiments** sweep TPS and account count upward until a latency SLO
  is breached or the stack crashes. The inflection points and degradation modes are
  recorded, not just the final failure state.

- **Every RPC method** listed in the Foundation's RFP is exercised. Where a `zcashd`
  equivalent exists, behavioural parity is recorded in the compatibility matrix.
  Deviations are characterised and documented — they are not automatically treated as
  bugs, since Z3 intentionally changes some behaviours.

- **Mempool** is monitored by polling `getrawmempool` / `getmempoolinfo` through the
  router. Z3's gRPC streaming interfaces that replace zcashd's ZMQ — Zebra's
  `Indexer.mempool_change()` and Zaino's `GetMempoolStream` — are documented but out of
  scope for this engagement.

- **All account and transaction data is synthetic.** Generated inside the harness,
  seeded deterministically, never sourced from real users. Safe to ship in the public
  repository.

---

## Scope

### In scope

- Simulator architecture, implementation, and deployment scripts
- Full RPC coverage matrix for the methods named in the RFP — transparent and shielded
- Regtest execution environment and isolated network setup
- Scenario library: steady-state, ramp, burst, and mixed-transaction-type workloads
- Synthetic account and transaction data generators
- Performance measurement and reporting harness
- Open-source repository with README, user manual, and API documentation
- Findings report pinned to the agreed commits of Zebra, Zaino, and Zallet

### Out of scope

- Security auditing of the Z3 stack (a separate dedicated audit partner is in place)
- Deployment against live mainnet
- Remediation of any issues identified — reporting is in scope; fixing is the component
  teams' responsibility
- Direct engagement with exchange partners

### Conditional (requires explicit agreement)

- ZSA-specific load scenarios, if Zcash Shielded Assets are live on the target pinned
  commits
- Extending the harness beyond Zallet to additional wallet implementations

---

## Deliverables

**1. Open-source GitHub repository**
Released under the **MIT License**. Contains:
- Simulator source code (Rust)
- Deployment and setup scripts
- Scenario library (YAML configs)
- Synthetic data generators
- Full documentation (README, user manual, API docs)

**2. Findings report** (PDF + Markdown)
Pinned to the agreed commit hashes of Zebra, Zaino, and Zallet. Contains:
- Load-curve results for each scenario (TPS vs. latency, inflection points, degradation
  modes)
- RPC compatibility matrix (each method: success rate, latency, parity with zcashd,
  deviations)
- Identified issues ranked by severity and reproducibility
- Recommendations for the Zebra, Zaino, and Zallet teams

**3. Final demonstration**
The simulator is run end-to-end against a fresh Z3 deployment, live, for the Foundation
and any invited observers.

---

## 12-week timeline

Three 4-week phases, with a midpoint checkpoint at Week 8.

| Phase | Weeks | Focus |
|---|---|---|
| Phase 1: Foundation | 1–4 | Kickoff; Zebra harness; Zaino and Zallet integration; first end-to-end transaction |
| Phase 2: Scaling | 5–8 | Thousands-scale accounts; scenario library; observability; mempool notifications; midpoint checkpoint |
| Phase 3: Findings | 9–12 | Full load-curve runs; RPC matrix; findings report; final demonstration |

### Week by week

**Phase 1 — Foundation**

- **Week 1** Kickoff; commit hashes confirmed; development and regtest environments
  provisioned; repository skeleton in place.
- **Week 2** Load-generator skeleton; Zebra regtest harness online; first `getblockchaininfo`
  call succeeds.
- **Week 3** Zaino integration; initial transparent RPC coverage verified end-to-end.
- **Week 4** Zallet integration; shielded RPC methods; first complete end-to-end
  transaction through the full stack (deposit → confirm → withdrawal).

**Phase 2 — Scaling and Scenarios**

- **Week 5** Account provisioning at thousands-scale; activity distribution tuning.
- **Week 6** Full scenario library implemented: steady-state, ramp, burst, mixed-type.
- **Week 7** Observability harness; latency histograms; mempool notification tests.
- **Week 8** Midpoint checkpoint: live demo, technical memo with prioritised findings,
  alignment call with the Foundation.

**Phase 3 — Findings and Handover**

- **Week 9** Full load-curve runs across all scenarios; data collection.
- **Week 10** RPC compatibility matrix finalised.
- **Week 11** Findings report drafted; repository documentation completed.
- **Week 12** Final demonstration; repository handover; report delivered.

---

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Z3 codebase evolves during the engagement | Commits are pinned at engagement start and fixed for its duration; findings are anchored to these specific versions |
| Assumed load profile turns out to be unrepresentative | The harness is fully parameterised — the Foundation and component teams can re-run with any profile after handover |
| Scope pressure towards security review | Security auditing is explicitly excluded; any such request is treated as a separate engagement |
| RPC incompatibilities surface late | The compatibility matrix is front-loaded in Weeks 3 and 4, not held for end-of-project analysis |
| ZSA scenarios become in scope mid-engagement | Treated as conditional scope; requires explicit agreement before any ZSA work begins |

---

## Z3 component notes

**Zallet repository:** https://github.com/zcash/wallet

**Zallet account model differs from zcashd.** `getnewaddress` and `z_getnewaddress`
do not exist. The workflow is:
1. Create an account: `z_getnewaccount`
2. Derive an address from it: `z_getaddressforaccount`

Each synthetic exchange user maps to one Zallet account.

**Several zcashd methods do not exist in Z3:**
`getbalance`, `z_getbalance`, `sendtoaddress`, `gettransaction`, `getmempoolentry`,
`createrawtransaction`, `signrawtransaction`. See the "Removed or replaced from zcashd"
section in [`docs/rpc/rpc-coverage-matrix.md`](rpc/rpc-coverage-matrix.md).

**Shielded transactions are fully implemented** at the pinned Zallet commit. Both
Sapling and Orchard pools are supported. `z_sendmany` handles all four flow types:
T→T, T→Z, Z→T, Z→Z.

**Fee computation:** `z_sendmany`'s `fee` parameter must be `null`. The fee is always
auto-computed via ZIP 317. The simulator reads the actual fee from the transaction
result after the fact.

---

## Open items

| Item | Status |
|---|---|
| Specific TPS and account count targets for load scenarios | Confirmed — calibrated from initial load runs; see `configs/scenarios/*.yaml` |
| Z3 pinned commit | Confirmed — `main` @ `dfb9d0ea` (frozen for the engagement) |
| Zaino coverage | Confirmed — JSON-RPC mirror (`:28237`); gRPC `CompactTxStreamer` out of scope |
