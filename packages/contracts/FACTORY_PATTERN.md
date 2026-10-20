# Factory Pattern: WASM Size, Deployment Cost, and Storage

This document compares the monolithic `call_registry` design with the
`prediction_market_factory` + per-market `prediction_market` pattern.

## Architecture

| Component                   | Role                                                                          |
| --------------------------- | ----------------------------------------------------------------------------- |
| `prediction_market_factory` | Deploys market instances, tracks `call_id → Address`, emits `MarketDeployed`  |
| `prediction_market`         | One market per contract: escrow, stakes, resolution hooks                     |
| `outcome_manager` (shared)  | Single oracle quorum layer; resolves any factory-deployed market by `call_id` |

### Outcome manager: shared vs per-market

**Chosen: single shared `outcome_manager`.**

| Approach                 | Pros                                                                      | Cons                                                      |
| ------------------------ | ------------------------------------------------------------------------- | --------------------------------------------------------- |
| **Shared** (implemented) | One oracle set, one admin surface, oracle votes keyed by global `call_id` | All markets share pause/upgrade risk                      |
| **Per-market**           | Isolated oracle config per market                                         | N deployments, N admin keys, fragmented liquidity signals |

The shared instance accepts any market address via `submit_outcome(registry, …)` and
validates against the configured factory with `set_factory` + `validate_market_registry`.

## WASM size (release build)

Measure after building:

```bash
cd packages/contracts
cargo build --release --target wasm32-unknown-unknown -p call-registry -p prediction-market -p prediction-market-factory
ls -la target/wasm32-unknown-unknown/release/*.wasm
```

Typical expectations (optimized with `opt-level = "z"`, LTO):

| Contract                    | Approx. size | Notes                                           |
| --------------------------- | ------------ | ----------------------------------------------- |
| `call_registry`             | ~80–120 KB   | Governance, SEP-10, shares, indexes, pagination |
| `prediction_market`         | ~25–45 KB    | Single market only; no global indexes           |
| `prediction_market_factory` | ~15–25 KB    | Deploy + enumeration                            |

The lightweight market WASM is uploaded **once**; each `deploy_market` reuses the same hash.

## Deployment cost

Soroban charges for:

1. **WASM upload** — paid once per unique hash (factory + market template).
2. **Contract deployment** — `deploy_v2` per market instance (CPU + rent for instance storage).
3. **Per-stake storage** — grows in the **market** contract, not the factory.

### Monolithic registry (N markets)

- 1 contract deploy
- All `Call(id)` entries in **one** contract's persistent storage
- Hot-path storage contention on single instance (64 KB instance cap risk at scale)
- Upgrade affects all markets atomically

### Factory pattern (N markets)

- 1 factory deploy + N market deploys
- Higher **up-front** deploy cost (N × deploy_v2)
- Storage **sharded** across N contracts — no cross-market blast radius
- Per-market WASM upgrade possible (deploy new hash, factory `set_market_wasm_hash`)
- Factory instance storage: `O(N)` address entries only (~32 B + overhead per market)

### Rule of thumb

| Scale                                       | Prefer                                     |
| ------------------------------------------- | ------------------------------------------ |
| &lt; 50 markets, single operator            | Monolithic may be cheaper overall          |
| 50+ markets, public permissionless creation | Factory — isolation and TTL management win |
| Regulated / upgradeable per product         | Factory                                    |

## Storage implications

### Monolithic `call_registry` per market

Persistent keys per call (approximate):

- `Call(id)` — full `Call` struct + nested `Map`s
- `CallStakers`, `StakerCalls`, `UserStake`, `CreatorStats`, IPFS keys
- Global `Config`, counters, governance state **shared**

All markets compete for the same contract's storage budget and TTL extension calls.

### Factory + `prediction_market`

**Factory** (instance storage):

- `MarketCounter`, `MarketList` (`Vec<Address>`), `Market(id) → Address`, `Config`

**Each market** (instance storage only):

- `Config`, `Call` (single), `UserStake(staker, position)` entries

No `CallStakers` / `StakerCalls` / creator reputation / governance in the market template —
indexers should read `MarketDeployed` events and per-market `stake_added` events.

### Savings per market

Removing from the per-market contract vs full registry:

- No per-call persistent key prefix (`Call(id)` in persistent tier)
- No global staker/creator indexes
- No share-token deploy path (optional in registry)
- No SEP-10 / governance / admin mutation surface

Estimated **60–70% less per-market state** than a fully featured registry row.

## Events

Factory emits:

```
("prediction_market_factory", "MarketDeployed")
  → (call_id, market_address, creator, stake_token, end_ts)
```

Indexers should treat `market_address` as the escrow contract and `call_id` as the global
oracle message key (unchanged for `outcome_manager`).

## Operational checklist

1. Upload `prediction_market` WASM to network; record `BytesN<32>` hash.
2. Deploy `prediction_market_factory`; `initialize(admin, outcome_manager, wasm_hash, min_stake)`.
3. Deploy **one** `outcome_manager`; `set_factory(factory_address)`.
4. `whitelist_token` on factory for each SAC stake token.
5. Users call `deploy_market(creator, MarketInitArgs)` — not direct market deploy.

## Measuring live costs

Use Soroban transaction simulation (`stellar contract invoke --simulate`) for:

- `deploy_market` CPU/memory budget
- First `stake_on_call` on a fresh market
- `submit_outcome` + `claim_payout` cross-contract path (3 contracts)

Compare against the same flow on monolithic `call_registry` for your target `N`.

## Deterministic addresses

Markets are deployed with `deploy_v2` from the factory address and the salt `sha256("market:" || call_id)`.
`predict_market_address(call_id)` returns that address without deploying, so an indexer or client can
know where market `N` will live before (or without) calling `deploy_market`.
