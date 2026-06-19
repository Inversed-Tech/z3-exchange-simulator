# RPC Method Scope — Z3 Exchange Simulator

RPC method list organised by test category.

---

## Test categories

**Stress-test** — exercised under full load. These methods are exchange-relevant,
state-size-sensitive, mempool-sensitive, or likely to degrade with account, transaction,
UTXO, note, or block volume. Latency histograms and failure rates are recorded for each.

**Regtest-control** — deterministic chain-manipulation methods that only exist in
regtest (`generate`, `invalidateblock`, `reconsiderblock`). They shape the test rather
than being part of the workload under measurement, so they are driven by scenario logic
(setup, block production, reorg scenarios) and are **excluded from the stress latency
histograms** to keep the workload statistics clean. The Foundation specifically asked
for these to be included; they are first-class and scenario-driven, not passive
references.

**Smoke-test / compatibility** — called once or a small number of times to verify
presence and basic correctness. Not subjected to load. Used to populate the RPC
compatibility matrix.

---

## Stress-test methods

### Zebra (via RPC Router)

| Method | What it does |
|---|---|
| `getblockchaininfo` | Node health and chain identification — primary smoke signal at start of every run |
| `getblockcount` | Current block height — used to count confirmations on deposits |
| `getbestblockhash` | Hash of the current chain tip |
| `getbestblockheightandhash` | Chain tip height and hash in one call (Zebra-specific) |
| `getblock` | Fetch a full block — used to detect incoming deposits |
| `getblockhash` | Get a block's hash by height |
| `getblockheader` | Block header without the full transaction list |
| `getblocktemplate` | Block template for mining — required to drive regtest block production |
| `getrawmempool` | All pending transaction IDs — core mempool saturation signal |
| `getmempoolinfo` | Mempool size, byte count, and fee statistics |
| `getrawtransaction` | Fetch a transaction by its ID |
| `gettxout` | Check whether a specific unspent output still exists on-chain |
| `getaddressbalance` | Transparent balance for one or more addresses |
| `getaddresstxids` | All transactions involving a transparent address |
| `getaddressutxos` | All unspent outputs for a transparent address |
| `getpeerinfo` | Connected peer list — used to verify regtest network state |
| `sendrawtransaction` | Broadcast a signed transaction to the network |
| `submitblock` | Submit a mined block — required for regtest block production |
| `z_gettreestate` | Sapling and Orchard commitment tree state at a given block |
| `z_getsubtreesbyindex` | Subtree roots for the note commitment tree — shielded state size signal |

### Zallet (via RPC Router)

| Method | What it does |
|---|---|
| `z_getnewaccount` | Create a new wallet account — one per synthetic exchange user |
| `z_getaddressforaccount` | Derive a Unified Address (deposit address) from an account |
| `z_listaccounts` | List all accounts in the wallet |
| `z_getaccount` | Return details for a specific account |
| `listaddresses` | List all wallet addresses grouped by source |
| `z_gettotalbalance` | Combined transparent + shielded total balance |
| `z_sendmany` | Create and broadcast a transaction — transparent and shielded outputs in one call |
| `z_getoperationstatus` | Check whether a shielded transaction has finished ZK proof generation |
| `z_getoperationresult` | Retrieve the result (txid) of a completed shielded operation |
| `z_listoperationids` | List all pending and completed async operation IDs |
| `z_listunspent` | List unspent shielded notes |
| `z_listtransactions` | List transactions filterable by account and block range |
| `z_getnotescount` | Count of unspent notes in the wallet — shielded state size signal |
| `z_viewtransaction` | Decode and return full details of a wallet transaction |
| `z_recoveraccounts` | Recover accounts from the wallet seed — used during wallet reset scenarios |
| `getrawtransaction` | Fetch a transaction by its ID (Zallet's wallet-aware variant) |

> **Note on fees:** `z_sendmany`'s `fee` parameter must be `null`. The fee is always
> auto-computed via ZIP 317. The simulator cannot pre-specify fees.

> **Note on async proving:** Shielded transactions use an async operation pattern.
> `z_sendmany` returns an operation ID immediately; `z_getoperationstatus` and
> `z_getoperationresult` are used to track completion.

---

## Regtest-control methods

### Zebra (via RPC Router)

| Method | What it does |
|---|---|
| `generate` | Mine N blocks on demand — block production for setup and confirmations |
| `invalidateblock` | Mark a block as invalid — drives chain-reorganization scenarios |
| `reconsiderblock` | Undo a previous `invalidateblock` — restores the invalidated branch |

These are exercised by scenario logic (warmup block production, deposit/withdrawal
confirmation, and a dedicated reorg scenario) and are excluded from stress latency
histograms.

---

## Smoke-test / compatibility methods

### Zebra (via RPC Router)

| Method | What it does |
|---|---|
| `validateaddress` | Confirm a transparent address is valid |
| `z_validateaddress` | Confirm a shielded address is valid |
| `z_listunifiedreceivers` | List the individual receivers within a Unified Address |
| `getblocksubsidy` | Block reward and miner fee at a given height |
| `getdifficulty` | Current proof-of-work difficulty |
| `getinfo` | Basic node information |
| `getmininginfo` | Mining-related node statistics |
| `getnetworkhashps` | Estimated network hash rate |
| `getnetworkinfo` | Network connections and protocol version |
| `getnetworksolps` | Estimated network solutions per second |
| `getpeerinfo` | (also stress-tested; listed here for completeness) |
| `addnode` | Add a peer to the node's address book |
| `ping` | Ping all connected peers |
| `rpc.discover` | OpenRPC service discovery |
| `stop` | Graceful node shutdown |

### Zallet (via RPC Router)

| Method | What it does |
|---|---|
| `getwalletinfo` | Wallet status and metadata |
| `z_listunifiedreceivers` | List the individual receivers within a Unified Address |
| `walletlock` | Lock the wallet |
| `walletpassphrase` | Unlock the wallet with the passphrase |
| `rpc.discover` | OpenRPC service discovery |
| `stop` | Graceful wallet shutdown |
| `help` | List available RPC methods |

---

## Total: 36 stress-test + 3 regtest-control + 21 smoke/compatibility methods

(`getrawtransaction` and `z_listunifiedreceivers` appear under both backends.)

---

## Routing note

All calls go through the Z3 RPC Router at `:8181` (regtest, the only network with a
router). The router requires HTTP Basic Auth (default `zebra` / `zebra`) and transparently
forwards each method to the correct backend (Zebra or Zallet) based on the method name.
The simulator does not call Zebra or Zallet directly.

Zaino is covered separately, outside the router: the simulator points a dedicated client
at Zaino's zcashd-style JSON-RPC mirror (regtest `:28237`), tagging those calls
`Backend::Zaino`. Zaino's lightwalletd `CompactTxStreamer` gRPC surface (`:28137`) is
documented but out of scope for this engagement. See
[`docs/integration/zaino.md`](../integration/zaino.md).

---

*Full coverage matrix with parity status and implementation tracking:
[`docs/rpc/rpc-coverage-matrix.md`](rpc-coverage-matrix.md)*
