# T3 Review Notes

## 1. `UnspentNote` does not match what Zallet actually returns (blocker)

The `UnspentNote` struct in `src/rpc/mod.rs` was modelled after the old zcashd
`z_listunspent` response shape. Zallet's response is different. Running `z_list_unspent`
against a real Zallet instance will always fail with a parse error because serde cannot
find the expected fields.

Specifically, the struct has four mismatches:

| Field in our struct | What Zallet actually sends | Problem |
|---|---|---|
| `amount: f64` | `value` | Wrong field name — serde will not find `amount` |
| `account: String` | `account_uuid` | Wrong field name — serde will not find `account` |
| `address: String` | `address` is `Option<String>` | Nullable in Zallet; our required `String` will fail when absent |
| `spendable: bool` | (field does not exist) | Zallet sends no `spendable` field at all |

Because `amount`, `account`, and `spendable` are all required (non-Option) fields,
deserialization fails on the first one serde can't find. The method call does not
panic — it returns `RpcError::Parse("missing field …")` — but it will never produce a
usable result.

**A note on which Zallet commit to check against.** The short answer is that the bug exists regardless of which version you look at, but the commit trail is worth knowing:

- `docs/integration/zallet.md` previously recorded commit `05926f3f` — this is likely
  what you (Charlotte) was working from when writing the struct.
- `z3-commits.lock` (the authoritative source in this repo) has `6fc85f68` — a newer
  candidate commit.
- The Z3 Docker Compose file (ZcashFoundation/z3, `dev` branch) actually runs the image
  tag `electriccoinco/zallet:v0.1.0-alpha.3`, which corresponds to git commit
  `f0db32d23de36b9a8e0c48b4438d22ab076aca58` — a third, different version.

All three Zallet commits have identical `UnspentOutput` struct shapes. The struct has
used `value` (not `amount`) and `account_uuid` (not `account`) throughout all of them.
So you might have been working from an older commit (`05926f3f`) that matches
neither z3-commits.lock nor the Docker image Z3 actually runs — but the code does not
match any of the three versions...

I corrected the integration docs (`docs/integration/zallet.md` and `docs/integration/zebra.md`) to align with z3-commits.lock. We may need to further update the file z3-commits.lock itself once the Z3 team confirms the final pinned state — the Docker image is currently at `v0.1.0-alpha.3` (`f0db32d2`) while the lock file lists `6fc85f68` as its candidate.

**Source — struct verified at all three Zallet commits:**
- [`list_unspent.rs` @ `05926f3f`](https://github.com/zcash/wallet/blob/05926f3f3ec1b1d90348ae899628cc0e28547ef3/zallet/src/components/json_rpc/methods/list_unspent.rs) (Charlotte's working commit, per old integration doc)
- [`list_unspent.rs` @ `6fc85f68`](https://github.com/zcash/wallet/blob/6fc85f68cf5ebe456160c6518255a83129e7d21c/zallet/src/components/json_rpc/methods/list_unspent.rs) (current z3-commits.lock candidate)
- [`list_unspent.rs` @ `v0.1.0-alpha.3` (`f0db32d2`)](https://github.com/zcash/wallet/blob/v0.1.0-alpha.3/zallet/src/components/json_rpc/methods/list_unspent.rs) (what Z3 Docker actually runs)

In all three: `value: JsonZec`, `account_uuid: String`, `address: Option<String>`, no
`spendable` field.

**Suggested replacement:**

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct UnspentNote {
    pub txid: String,
    pub confirmations: u32,
    pub address: Option<String>,
    #[serde(rename = "account_uuid")]
    pub account: String,          // keeps the public name stable for T5
    pub value: f64,               // ZEC float, as returned by the RPC
    #[serde(rename = "valueZat")]
    pub value_zat: u64,           // same amount in zatoshis — T5 should use this directly
}
```

Using `value_zat` (zatoshis, integer) in T5 also avoids the `value * 1e8` float
conversion that the existing comment warns about.

---

## 2. `call` and `call_nullable` duplicate ~50 lines of core logic

The HTTP round-trip, latency measurement, error classification, and `record_rpc_call`
call are copy-pasted verbatim between the private `call` method (lines 374–446) and
`call_nullable` (lines 450–518). The comment above `call` says it is "the single
chokepoint all public methods go through" — but it isn't, because `call_nullable` is a
separate copy.

Any future change to how `RpcCall` is recorded (adding a field, changing the timestamp
logic) requires two parallel edits, and it is easy to update one and forget the other.

One clean fix: make `call_nullable` delegate to `call` by asking serde to deserialize
into `Option<T>` — a JSON null result deserializes naturally into `None`. That collapses
the two paths back into one.

---

## 3. Timeout is hardcoded, no retry

`RpcClient::new` builds the HTTP client with a fixed 30-second timeout. The T3 spec
says the timeout should be configurable. Under load, or when running slower Zallet
operations, 30 seconds may be too short or too long depending on the scenario.

No retry logic is implemented either (the spec says "if implemented", so this is lower
priority). At minimum, the timeout should be a parameter to `RpcClient::new` so
different scenarios can tune it.

---

## Minor gaps (non-blocking, good to add)

- `z_get_operation_result` has no response-parsing test, even though it is a critical
  stress-test method for the withdrawal flow. Adding one test mirroring the existing
  `z_get_operation_status` success test would be enough.
- `get_tx_out` (the null-result path) has no test verifying that `RpcCall` is recorded
  correctly when the response is `Ok(None)`.
