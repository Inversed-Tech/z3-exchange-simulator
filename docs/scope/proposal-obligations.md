# Proposal Obligations

Derived directly from the approved project proposal (Inversed / Zcash Foundation, April 2026).
This is the team's single reference for deliverables, constraints, timeline, risks, and open
questions. Do not add obligations here that are not in the proposal.

---

## Project objective

Build a reproducible, open-source exchange simulator that exercises the Z3 Zcash stack
(Zebra, Zaino, Zallet) under realistic exchange-scale load. The engagement produces two
outputs: a findings report tied to pinned commits of each Z3 component, and a testing
framework the Foundation and component teams can run in their own pipelines indefinitely.

Four priorities shape the work:

- **Realistic exchange-scale load.** Account populations in the thousands; Binance-scale
  usage (tens of thousands of ZEC-holding users) as the design ceiling.
- **Open source and reusable.** Runnable by the Foundation, Zebra/Zaino/Zallet teams, and
  ecosystem developers in CI. Not a one-off consulting deliverable.
- **Findings tied to a specific commit.** The report references pinned commits of each Z3
  component, agreed at kickoff, so findings are reproducible.
- **Regtest-focused, isolated environments.** All testing runs in regtest mode against
  controlled networks.

---

## In scope

- Simulator architecture, implementation, and deployment scripts.
- Full RPC coverage matrix for the methods named in the RFP, transparent and shielded.
- Regtest execution environment and isolated network setup.
- Scenario library covering at minimum: steady-state, ramp, burst, and
  mixed-transaction-type workloads.
- Synthetic account and transaction data generators.
- Performance measurement and reporting harness.
- Open-source repository with README, user manual, and API documentation.
- Findings report pinned to specific commits of Zebra, Zaino, and Zallet, agreed at kickoff.

---

## Out of scope

- **Security auditing of the Z3 stack.** The ecosystem has a dedicated audit partner. Any
  security audit work would be a separate engagement.
- **Deployment against live mainnet.** All testing is regtest-only.
- **Remediation of identified issues.** Reporting is in scope; fixes are the component
  teams' responsibility.
- **Direct engagement with exchange partners.**

---

## Conditional scope

The following are in scope only on express agreement of all parties:

- **ZSA-specific load scenarios** — if ZSAs (Zcash Shielded Assets) are live on the
  target commit.
- **Additional wallet implementations** — extending the harness beyond Zallet to other
  wallets in the ecosystem.

---

## Deliverables

### 1. Open-source GitHub repository

Released under a permissive license (MIT or Apache 2.0, to be confirmed with the
Foundation). Contains:

- simulator source code,
- deployment scripts,
- scenario library,
- synthetic data generators,
- full documentation (README, user manual, API docs).

### 2. Findings report

Delivered in both PDF and Markdown. Pinned to the agreed commit hashes of Zebra, Zaino,
and Zallet. Contains:

- load-curve results for each scenario,
- RPC compatibility matrix,
- identified issues ranked by severity and reproducibility,
- recommendations for the component teams.

### 3. Final demonstration

The simulator is run end-to-end against a fresh Z3 deployment for the Foundation and any
invited observers.

---

## Timeline

The engagement is 12 weeks, organized into three 4-week phases. A midpoint checkpoint
at Week 8 recalibrates against findings before the final phase.

| Phase | Weeks | Focus |
|---|---|---|
| Phase 1: Foundation | 1–4 | Kickoff; commit hashes pinned; Zebra regtest harness; Zaino integration; first end-to-end transaction through the full Z3 stack |
| Phase 2: Scaling | 5–8 | Thousands-scale account provisioning; scenario library; observability and mempool notification tests; midpoint checkpoint at Week 8 |
| Phase 3: Findings and Handover | 9–12 | Full load-curve runs; RPC compatibility matrix; findings report; final demonstration and repository handover |

### Phase 1: Foundation (Weeks 1–4)

| Week | Deliverable |
|---|---|
| Week 1 | Kickoff call; commit hashes pinned; development and regtest environments provisioned |
| Week 2 | Load-generator skeleton; Zebra regtest harness online |
| Week 3 | Zaino integration; initial transparent RPC coverage |
| Week 4 | Zallet integration; shielded RPC methods; first end-to-end transaction through the full stack |

### Phase 2: Scaling and Scenarios (Weeks 5–8)

| Week | Deliverable |
|---|---|
| Week 5 | Account provisioning at thousands-scale; distribution tuning |
| Week 6 | Scenario library: steady-state, ramp, burst, mixed-type |
| Week 7 | Observability harness; latency histograms; mempool notification tests |
| Week 8 | **Midpoint checkpoint.** Live demo; technical memo with prioritised findings; alignment call with the Foundation |

### Phase 3: Findings and Handover (Weeks 9–12)

| Week | Deliverable |
|---|---|
| Week 9 | Full load-curve runs; data collection |
| Week 10 | RPC compatibility matrix finalised |
| Week 11 | Findings report drafted; repository documentation completed |
| Week 12 | Final demonstration; repository handover; report delivered |

---

## Risks and mitigations

Taken directly from the proposal's risk table.

| Risk | Mitigation |
|---|---|
| Z3 codebase evolves during the engagement | Commit-pinning at kickoff; weekly rebase review; findings anchored to explicit commit hashes |
| Assumed load profile turns out to be unrepresentative | Harness is parameterised throughout; Foundation and ecosystem teams can re-run with any profile after handover |
| Scope pressure towards security review | Security auditing is explicitly excluded. Any such work would be scoped as a separate engagement |
| Single-lead continuity risk | Aurel Nicolas is named as backup; the agent stack preserves institutional knowledge in code and documentation |
| RPC incompatibilities surface late | Compatibility matrix is front-loaded in Weeks 3 and 4 rather than held for end-of-project analysis |

---

## Assumptions

Taken directly from the proposal (Section 12). These are flagged for confirmation at or
before the kickoff call.

1. **Commit hashes** for Zebra, Zaino, and Zallet will be pinned at kickoff.
2. **Load profile.** Thousands-of-accounts scale is assumed as the baseline. Specific TPS
   and account targets will be confirmed at kickoff. Where public on-chain data permits,
   observable ZEC volumes will also inform the targets.
3. **Regtest mode** is the testing environment for this engagement.
4. **Zaino** is assumed to be in scope for RPC coverage alongside Zebra and Zallet. This
   is open to adjustment.
5. **License** for the public repository is assumed to be MIT or Apache 2.0, pending the
   Foundation's preference.
6. **Contracting mechanism** is assumed to be direct Foundation commissioning. Alternative
   mechanisms (e.g. CCG) may adjust the timeline and payment structure.
7. **Security disclosure triage** (mentioned in the proposal's Extensions section) is a
   separate potential engagement. It is not priced or scoped in this proposal.

---

## Open questions

Questions that must be resolved at or shortly after kickoff. Tracked in
[`docs/reports/week1-status.md`](../reports/week1-status.md).

| # | Question | Owner | Status |
|---|---|---|---|
| 1 | What are the confirmed pinned commits for Zebra, Zaino, and Zallet? | Foundation / component teams | TBD |
| 2 | What is the Zallet repository URL? | Foundation / Zallet team | TBD |
| 3 | What is the full RFP method list for the RPC coverage matrix? | Foundation | TBD |
| 4 | Which RPC methods are mandatory for the Week 4 end-to-end transaction? | Foundation | TBD |
| 5 | Which component serves each RPC method (Zebra, Zaino, or Zallet)? | Component teams | TBD |
| 6 | What qualifies as the Week 4 "first end-to-end transaction"? | Foundation | TBD |
| 7 | Is transparent-only acceptable for Week 4 if shielded support is not yet available? | Foundation | TBD |
| 8 | What license does the Foundation prefer: MIT or Apache 2.0? | Foundation | TBD |
| 9 | Which GitHub organization should host the public repository? | Foundation | Resolved: Inversed-Tech |
| 10 | What is the preferred weekly reporting format for the Foundation? | Foundation | TBD |
| 11 | What are the specific TPS and account-count targets? | Foundation / kickoff | TBD |

---

## Collaboration structure

As specified in the proposal:

- **Slack** — day-to-day communication, async updates, quick questions.
- **Notion** — shared project dashboard: decisions, progress, emerging findings.
- **GitHub** — all code, experiments, benchmarks, and technical documentation. Public from
  Week 1. Repository: https://github.com/Inversed-Tech/z3-exchange-simulator
- **Bi-weekly progress memos** — summarising completed work, findings, and next sprint's
  priorities.
- **Midpoint checkpoint call** at Week 8 to review progress and align on Phase 3.
- **Ad-hoc calls** to unblock or recalibrate.

### Team contacts

| Name | Role | Contact |
|---|---|---|
| Oded Goffer | Project lead | oded@inversed.tech |
| Charlotte Léonard | Research and development engineer | charlotte@inversed.tech |
| Aurel Nicolas | Technical advisor and continuity backup | aurel@inversed.tech |
