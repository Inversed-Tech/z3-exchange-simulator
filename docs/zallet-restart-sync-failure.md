# Zallet Crash-Loop After a Container Restart

This document is a factual record of a specific, repeatedly observed problem. It
contains only direct observations: log excerpts, commands run, and results
obtained. It deliberately does **not** contain any hypothesis about the cause,
any assessment of which component is at fault, or any proposed fix. It is
written for handoff to an independent reviewer.

---

## Environment

- Deployment: the Z3 Docker Compose stack (`external/z3`), regtest network,
  brought up via `docker compose --env-file .env.regtest up -d`.
- Component versions in use (the "override" set recorded in `z3-commits.lock`):
  - Zebra `v6.0.0` (image `zfnd/zebra:6.0.0`)
  - Zaino `0.6.0` (image `zingodevops/zainod:0.6.0-no-tls`)
  - Zallet `v0.1.0-beta.1` (image `z3sim/zallet:v0.1.0-beta.1`, built locally
    from the official release tarball)
- Containers involved: `zebra`, `zaino`, `zallet`, `rpc-router`,
  `cookie-permissions`.
- Host: macOS, arm64 (Apple Silicon); the above images run under x86_64
  emulation (`DOCKER_PLATFORM=linux/amd64`).

---

## Summary of the observed behavior

After a Z3 environment has been running, has had one or more of the
simulator's test scenarios executed against it, and is then stopped
(`docker compose down`, which removes the containers but not the Docker
volumes) and started again (`docker compose up -d`, reusing the same
volumes) — the `zallet` container has, in multiple observed instances, gone
into a continuous restart loop. In this state it does not stay running long
enough to serve RPC requests reliably.

This has not been observed on the first start of a freshly initialized
environment (empty Docker volumes, wallet just created). In every observed
instance, the crash-loop followed at least one prior successful startup
during which the simulator had already created accounts and attempted
transactions.

Only the `zallet` container has been observed to enter this state. In every
observed instance, `zebra`, `zaino`, `rpc-router`, and `cookie-permissions`
remained in a running/healthy state throughout.

---

## Sequence of actions that preceded each observed instance

The following sequence of actions preceded the crash-loop in every observed
instance:

1. A Z3 environment is started against empty/freshly initialized Docker
   volumes (no prior chain or wallet data).
2. One or more simulator scenario runs are executed against this
   environment. Each run creates synthetic accounts and dispatches a number
   of transactions via Zallet's JSON-RPC interface (`z_sendmany` and
   related calls). In every observed instance, not all dispatched
   transactions in the preceding run(s) were reported as confirmed by the
   simulator (i.e., the preceding run's confirmed count was less than its
   attempted count).
3. The environment is stopped: `docker compose --env-file .env.regtest
   down` (without the `-v` flag — the named Docker volumes, including the
   Zallet wallet database and the Zebra chain state, are preserved).
4. The environment is started again: `docker compose --env-file
   .env.regtest up -d`, reusing the volumes from step 3. No wallet
   reinitialization step is performed between step 3 and step 4.
5. Following this restart, the `zallet` container's logs show the sequence
   described in the next section, ending with the process exiting. Docker's
   restart policy then restarts the container, which repeats the same
   sequence again. This repeats continuously (a restart loop) until the
   container is stopped or the underlying Docker volumes are removed and
   recreated.

---

## Zallet log output observed at the point of failure

The following sequence of log lines was observed, in this order, in every
instance where the failure occurred (specific numbers — chain height,
request count, transaction ID — varied between instances; three separate
instances are quoted below for reference).

Common sequence (abbreviated; full lines include additional structured
fields not reproduced here):

```
INFO zallet_core::commands::sync: Latest block height is 0
INFO zallet_core::components::sync: Initial boundary between recovery and steady-state sync is 0
INFO zallet_core::components::sync: Steady-state sync task started
INFO zallet_core::components::sync: History recovery sync task started
INFO zallet_core::commands::start: Spawned Zallet tasks
INFO zallet_core::components::sync: <N> transaction data requests to service
INFO zallet_core::components::sync: Getting status of <txid>
INFO zallet_core::commands::start: Wallet data-requests sync task exited wallet_sync_result=Err(Error(Context { kind: Sync, ... source: Some(Chain(Backend(ChainIndexError { kind: InternalServerError, message: "critical error in backing block source: could not fetch transaction data: unexpected error response from server: RPC Error (code: -5): No such mempool or main chain transaction", source: Some(BlockchainSourceError(UnrecoverableWithSource { message: "could not fetch transaction data: unexpected error response from server: RPC Error (code: -5): No such mempool or main chain transaction", source: UnexpectedErrorResponse(RpcError { code: -5, message: "No such mempool or main chain transaction", data: None }) })) }))) }))
INFO zallet_core::commands::start: Exiting Zallet because an ongoing task exited; asking other tasks to stop
INFO zallet_core::commands::start: All tasks have been asked to stop, waiting for remaining tasks to finish
INFO zallet_core::commands::start: Shutting down Zallet
```

### Instance 1

- Zebra chain height at the time of the log: 130
- Reported request count: "232 transaction data requests to service"
- Transaction ID referenced: `ed3cde7a4256af5071dd903d099b3d49021966145d7bcd001958ef63795fa81b`
- This instance occurred before the mitigation described later in this
  document was implemented.

### Instance 2

- Zebra chain height at the time of the log: 47
- Reported request count: "1095 transaction data requests to service"
- Transaction ID referenced: `3632b791e88eafbd418b314234eab64f61234d0396024d15cdcdad1a72dba826`
- This instance occurred before the mitigation described later in this
  document was implemented, on a separately re-initialized environment from
  Instance 1.

### Instance 3

- Zebra chain height at the time of the log: 52
- Reported request count: "1087 transaction data requests to service"
- Transaction ID referenced: `f0eb6bdfbad1d9fc2809ecdbf6002d3a4ac341704e7b0ceed41d7434d3bbad17`
- This instance occurred *after* the mitigation described later in this
  document was implemented and active. It occurred when a test process,
  having just completed one scenario run successfully (see "Mitigation
  attempted" below), brought the environment back up to begin a second,
  separate scenario run.

In all three instances, the container's status as reported by `docker ps`
was `Restarting (0) <N> seconds ago`, and this status recurred continuously
on subsequent checks (i.e., the container was observed to be repeating the
cycle rather than staying up).

---

## Effect observed on the client / test-runner side

While the `zallet` container was in this state, requests made through the
shared RPC router (which forwards calls to `zallet` or `zebra` depending on
method) did not receive successful responses for Zallet-routed methods.

- One observed integration-test failure, with the environment in this
  state, produced the following error from the simulator's own setup code:
  ```
  Setup("failed to resolve hot wallet: funding step `z_listaccounts`: RPC response parse error: error decoding response body")
  ```
- One direct manual request (using `curl`, sent straight to the Zallet
  container's published port rather than through the router) produced:
  ```
  * Empty reply from server
  ```

---

## Recovery method used

In every instance, the following steps were used to restore the environment
to a working state:

1. `docker compose --env-file .env.regtest down -v` (this removes the
   containers **and** the named Docker volumes — including the Zallet
   wallet database and the Zebra chain data).
2. Re-run the project's environment setup scripts against the now-empty
   volumes: `scripts/regtest-init.sh` (from `external/z3`), followed by
   `scripts/dev/regtest-miner-setup.sh` (from the simulator repository
   root).
3. `docker compose --env-file .env.regtest up -d`.

Following these steps, the environment was observed to start cleanly in
every instance, with `zallet` reaching a stable running state (no restart
loop), until the sequence described under "Sequence of actions that
preceded each observed instance" was repeated again.

No other recovery method was attempted or observed to succeed.

---

## Mitigation attempted

A change was made to the simulator's own run-orchestration code
(`src/scenarios/runner/mod.rs`), altering the timing of two things relative
to each other within a single scenario run:

- Previously: three background processes (one that periodically triggers
  new block production, one that periodically records wallet balance and
  related metrics, and one that monitors the mempool) were each signaled to
  stop as soon as the run stopped dispatching new transactions. The run
  then continued, separately, to wait for any already-dispatched
  transactions to reach a final status (success, failure, or an internal
  timeout) — a wait that was observed, in one recorded run, to continue for
  approximately 89 seconds after the last block-height sample had been
  recorded.
- Changed to: the three background processes are now signaled to stop only
  after that waiting period ends. In addition, immediately before those
  processes are signaled to stop, the code now makes one additional
  request for 5 more blocks to be produced.

This change was tested by executing three of the simulator's test
scenarios in sequence, within a single test invocation (`cargo test
--test integration -- --ignored --test-threads=1 test_flow_check`). Each
scenario independently starts the environment, runs its scenario, and stops
the environment (per the sequence described earlier in this document) before
the next scenario's test begins.

**Result:** The first scenario in the sequence completed and reported a
result (a run summary was produced; 4 of 60 attempted transactions were
reported as confirmed). When the test process for the second scenario then
started the environment again (reusing the volumes left by the first
scenario, per the standard sequence above), the crash-loop described in this
document occurred — this is "Instance 3" above. The second and third
scenarios in the sequence did not produce a run result; both test processes
reported a setup failure with the error message quoted under "Effect
observed on the client / test-runner side."

---

## What has not been determined or tested

The following questions have not been investigated, and this document takes
no position on them:

- Whether the crash-loop occurs following a run in which 100% of dispatched
  transactions were reported as confirmed. No such run has been observed;
  every run that preceded an observed crash-loop instance had a nonzero
  count of non-confirmed transactions.
- Whether the specific transaction ID referenced in each instance's "Getting
  status of `<txid>`" log line corresponds to a transaction the simulator
  itself had dispatched, and if so, what that transaction's recorded
  outcome was in the simulator's own output for the preceding run. This
  cross-reference has not been performed.
- Whether the crash-loop occurs after an idle period with no environment
  restart, given the same or a similar prior transaction history.
- Whether a larger or smaller number of additional blocks (the mitigation
  described above used 5) changes the outcome.
- Whether the crash-loop is deterministic (i.e., occurs on every restart
  following any run with unconfirmed transactions) or occurs only under
  some subset of conditions not yet isolated.
- Whether restarting the `zallet` container alone (leaving `zebra`/`zaino`
  running) produces the same result as restarting the full Compose stack.

---

## Resolution (2026-07-31)

Root-caused to upstream `zcash/zallet`: the `data_requests` sync task treated
any chain-backend error on a persisted `TransactionDataRequest` (including a
plain "not found" response) as fatal, and because the request is regenerated
from the wallet database on every startup, one stale request produced a
permanent crash-loop. Filed and fixed upstream — `zcash/zallet#599`, fixed by
`zcash/zallet#598`, shipped in `v0.1.0-beta.2`.

The simulator's pinned Zallet version was bumped from `v0.1.0-beta.1` to
`v0.1.0-beta.2` (`z3-commits.lock`, `scripts/dev/zallet-release-image/build.sh`).
Verified directly against this document's own evidence: starting the
`v0.1.0-beta.2` image against the Docker volumes left behind by the "Instance
3" crash-loop (1087 pending transaction data requests, txid
`f0eb6bdfbad1d9fc2809ecdbf6002d3a4ac341704e7b0ceed41d7434d3bbad17`) produced a
logged warning and a retry instead of a fatal exit; the process reached the
chain tip and stayed up (`RestartCount=0`), and `z_listaccounts` — the RPC
call that failed client-side during the crash-loop — responded normally. A
second, independent check (fresh `smoke` scenario run, stack stopped with
volumes preserved, stack restarted) reproduced the same clean, stable
outcome.
