# RPC Method Scope — Z3 Exchange Simulator

RPC method list organised by test category.

---

## Test categories

**Stress-test** — exercised under full load. These methods are exchange-relevant,
state-size-sensitive, mempool-sensitive, or likely to degrade with account, transaction,
UTXO, note, or block volume. Latency histograms and failure rates are recorded for each.

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

## Smoke-test / compatibility methods

### Zebra (via RPC Router)

| Method | What it does |
|---|---|
| `generate` | Mine N blocks on demand — regtest chain control |
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
| `invalidateblock` | Mark a block as invalid — chain reorganization testing |
| `reconsiderblock` | Undo a previous `invalidateblock` |
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

## Total: 35 stress-test methods + 24 smoke-test methods

---

## Routing note

All calls go through the Z3 RPC Router at `:8181` (regtest). The router transparently
forwards each method to the correct backend (Zebra or Zallet) based on the method name.
The simulator does not call Zebra or Zallet directly.

Zaino is not a direct JSON-RPC target. It runs as a library inside Zallet (for indexing)
and as a separate gRPC container for light clients. Its latency is implicit in Zallet
method response times.

---

*Full coverage matrix with parity status and implementation tracking:
[`docs/rpc/rpc-coverage-matrix.md`](rpc-coverage-matrix.md)*
