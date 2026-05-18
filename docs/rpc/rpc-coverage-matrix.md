# RPC Coverage Matrix

> Status: Draft — to be fully populated in Week 1 (Task T6).

Tracks which RPC methods the simulator exercises, which Z3 component serves each method,
zcashd behavioral parity, and test status.

**Important:** The authoritative list of required RPC methods comes from the RFP. That
list has not yet been received. All "Required by RFP?" cells are TBD until confirmed.

## Matrix

| Method | Category | Component | Required by RFP? | zcashd equivalent? | Transparent/Shielded | Implemented? | Tested? | Parity status | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | chain info | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Basic chain status |
| `getblockcount` | chain info | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Current block height |
| `getblock` | block lookup | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Fetch block by hash or height |
| `getrawtransaction` | tx lookup | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Fetch raw transaction data |
| `getrawmempool` | mempool | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Needed for mempool saturation signals |
| `sendrawtransaction` | tx broadcast | Zebra/Zaino | TBD | Yes | Both | No | No | TBD | Broadcast a signed transaction |
| `getnewaddress` | wallet/address | Zallet | TBD | Yes | Transparent | No | No | TBD | Generate new transparent address |
| `z_getnewaddress` | wallet/address | Zallet | TBD | Yes | Shielded | No | No | TBD | Generate new shielded address |
| `getbalance` | wallet/balance | Zallet | TBD | Yes | Transparent | No | No | TBD | Query transparent balance |
| `z_getbalance` | wallet/balance | Zallet | TBD | Yes | Shielded | No | No | TBD | Query shielded balance |
| `z_sendmany` | tx creation | Zallet | TBD | Yes | Both | No | No | TBD | Create and send shielded/transparent tx |
| `z_getoperationstatus` | tx creation | Zallet | TBD | Yes | Both | No | No | TBD | Check async operation status |

## Categories

- **chain info** — blockchain state queries
- **block lookup** — fetching block data
- **tx lookup** — fetching transaction data
- **mempool** — mempool inspection and notification
- **wallet/address** — address generation
- **wallet/balance** — balance queries
- **tx creation** — transaction construction and signing
- **tx broadcast** — transaction submission
- **notifications** — RPC client notification subscriptions (mempool changes, per RFP)
- **shielded** — z_ prefixed methods specific to shielded operations

## Open questions

- What is the complete RFP method list? This is the critical missing input for this matrix.
- Which component (Zebra, Zaino, or Zallet) serves each method? Routing must be verified
  through integration testing rather than assumed.
- Which zcashd methods are not yet implemented in Z3?
