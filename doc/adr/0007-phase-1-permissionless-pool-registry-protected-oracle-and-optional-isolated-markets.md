# 7. Phase 1 permissionless pool registry, protected oracle, and optional isolated markets

Date: 2026-09-03

## Status

Proposed

Amends [2. Pool registry as deployment, configuration, and optional-backstop routing layer](0002-pool-registry-as-deployment-configuration-and-optional-backstop-routing-layer.md)

Depends on [3. Protocol fee on borrow interest; defer origination and flash-loan fees](0003-protocol-fee-on-borrow-interest-with-deferred-origination-and-flash-loan-fees.md)

Depends on [4. Permissioned pools via pool-local allow-list, permission manager, and optional hook](0004-permissioned-pools-via-pool-local-allow-list-permission-manager-and-optional-hook.md)

Depends on [5. Reserve roles and isolated (Morpho-style) pools with no rehypothecation of collateral](0005-reserve-roles-and-isolated-morpho-style-pools-with-no-rehypothecation-of-collateral.md)

## Context

Phase 1 needs one production profile over the cumulative greenfield decisions in ADRs 0002–0005:

- one registry tracks every protocol pool, enforces protocol-wide deployment and live-configuration policy, and supplies ADR 0003's globally controlled fee-collection state;
- anyone may create a conforming pool without a registry-governance proposal or creator allow-list, while Templar-operated frontends remain free to list only reviewed pools;
- every pool uses the protected Soroban proxy-oracle system in `../contracts/contract/proxy-oracle/soroban` rather than a caller-selected direct price feed;
- a creator may choose ADR 0005's isolated no-rehypothecation configuration; and
- every Phase 1 pool keeps an enabled backstop, but the backstop receives no borrow-interest take because `backstop_take_rate` is fixed at zero.

This is a Phase 1 release profile and cumulative ABI amendment, not a replacement for the detailed economics, admission, isolation, lifecycle, or accounting decisions in its predecessors.

The existing decisions supply most of the required behavior:

- ADR 0002 defines the registry as deployer, membership authority, configuration-policy authority, lifecycle and upgrade coordinator. Its point views include `is_pool`, `pool_lifecycle`, `pool_implementation`, and `backstop_of`; the complete pool inventory is reconstructed from the canonical `pool/deploy` event stream rather than an unbounded on-chain `list_pools` vector (`0002:398-451,1029-1114`).
- ADR 0002's `deploy` calls `admin.require_auth()` (`0002:500-501`). Here `admin` is the selected pool admin and becomes a constructor authorizer (`0002:529-534`); it is not the registry admin. The existing text does not require registry-admin authorization or a governance proposal for deployment.
- ADR 0003 adds the positive protocol interest fee, the five-field `MinConfig`, protected permissionless `collect_fees`, and one atomic registry `FeeCollectionState`. It explicitly applies to every production pool, including permissioned and isolated pools (`0003:131-155,816-960,1003-1172`).
- ADR 0004 adds optional participant admission through a pool-local permission manager and hook. Those controls govern pool use, not who may deploy a pool (`0004:128-228,230-300,836-864`).
- ADR 0005 defines `isolated: bool`, the exact collateral-only/lend-only topology, immutable reserve roles, and the no-rehypothecation state and transaction-flow guarantees. “Morpho-style” means only the directed one-collateral/one-loan topology; it does not claim Morpho Blue's singleton, immutability, governance, accounting, callback, or flash-loan design (`0005:15-23,166-231,233-297`).

The oracle boundary is not yet decided by those ADRs. ADRs 0002–0005 carry a caller-supplied `oracle: Address`, but do not constrain that address. The vendored Blend pool stores one oracle address and calls the SEP-40 `decimals()` and `lastprice(Asset::Stellar(reserve))` methods for every reserve. Missing, non-positive, or more-than-24-hour-old prices fail with `PoolError::InvalidPrice` (`vendor/blend-contracts-v2/pool/src/pool/pool.rs:100-130`).

Templar's Soroban proxy runtime aggregates multiple source contracts, applies source freshness and circuit breakers, and exposes a fail-closed normalized cache through `aggregated_latest(asset)` (`../contracts/contract/proxy-oracle/soroban/contract/src/lib.rs:300-318,370-401`). The existing `Sep40Adapter` converts that cache to SEP-40, but each deployed adapter has one immutable `(parent_oracle, asset)` pair and returns `None` for every other asset (`../contracts/contract/proxy-oracle/soroban/sep40-adapter-contract/src/lib.rs:41-80,169-212`). It proves one-reserve Blend interoperability in `../contracts/contract/proxy-oracle/soroban/integration-tests/tests/blend_e2e.rs:130-256`, but one such adapter cannot serve ADR 0005's two-reserve isolated pool or an open multi-reserve pool.

Phase 1 therefore needs one pool-compatible multi-asset SEP-40 boundary over the existing proxy runtime. Pointing Blend directly at the normalized runtime would require a wider rewrite of every pool price conversion. Allowing each creator to supply an arbitrary SEP-40 oracle would make the protected proxy optional and defeat the requirement.

Constraints and assumptions:

- `vendor/` remains read-only here. Blend changes are upstream work in `Templar-Protocol/blend-contracts-v2`; proxy-oracle changes are work in `../contracts`.
- This is the same greenfield pre-freeze stack as ADRs 0002–0005. No non-disposable pool may use an interim constructor, deploy signature, storage layout, or event ABI.
- All Phase 1 pools have `Some(backstop)` from construction. ADR 0002's optional `None` route remains a protocol capability outside this Phase 1 product profile; Phase 1 does not deploy it.
- Every Phase 1 pool has `backstop_take_rate == 0` for its lifetime. The backstop remains active for its non-interest-take responsibilities, including bad-debt and liquidation settlement. Phase 1 does not claim that the backstop is removed or economically unused.
- ADR 0003's positive protocol interest fee remains mandatory. Origination and flash-loan fees remain deferred.
- Official frontend listing is off-chain product curation. It is not registry membership, reserve listing, ADR 0004 participant admission, or an on-chain certification bit.
- The proxy oracle is a liveness dependency. A stale, blocked, missing, evicted, or unrefreshed protected price fails closed and may stop every pool action that needs that asset price.
- Every optimized Wasm remains at or below `min(131072, active_network.contractMaxSizeBytes)`. The proxy runtime and governance use that limit; the pool adapter retains the proxy package's stricter 32 KiB budget unless a later reviewed decision changes it.
- ADR 0002 also requires an Accepted issuance/emissions disposition before production genesis. No such ADR exists at this draft. This ADR neither invents that decision nor weakens the gate.

In scope:

- the Phase 1 subset and dependency order of ADRs 0002–0005;
- permissionless registry deployment and separate permissioned frontend listing;
- a canonical pool-compatible SEP-40 adapter over Templar's proxy runtime;
- mandatory enabled backstop with zero backstop interest take;
- optional ADR 0005 isolated pools; and
- release, monitoring, failure, and rollback requirements for those choices.

Out of scope:

- positive backstop interest take in Phase 1;
- a no-backstop Phase 1 product;
- BLND issuance/emissions economics;
- origination or flash-loan fees;
- an on-chain frontend listing or certification registry;
- changing ADR 0004's participant-permission semantics;
- changing ADR 0005's reserve-role or no-rehypothecation semantics;
- equivalence with Morpho Blue beyond ADR 0005's directed two-asset topology; and
- migration of any non-disposable predecessor layout.

Principles:

1. Permissionless creation means no creator allow-list or registry-governance approval; it does not mean bypassing protocol configuration, oracle, backstop, fee, or constructor authorization.
2. Registry membership is objective on-chain fact. Official frontend listing is a separate, revocable off-chain product decision and never changes pool validity or discoverability.
3. Every price used by a Phase 1 pool traverses Templar's proxy aggregation, freshness, and circuit-breaker boundary and fails closed; no creator-selectable direct-oracle escape hatch exists.
4. Phase 1 retains the backstop while assigning it zero borrow-interest take. Protocol revenue follows ADR 0003, and no label may describe the pool as backstop-free.
5. ADR 0005 is the sole authority for the optional no-rehypothecation guarantee. This ADR binds it; it does not paraphrase or weaken it.

Decision drivers, ranked:

1. Enforce one non-bypassable Phase 1 safety and revenue profile while keeping pool creation open.
2. Preserve exact predecessor economics, lifecycle, admission, and no-rehypothecation invariants without a parallel implementation convention.
3. Minimize new contract and public-surface weight, especially on pool hot paths and under the Soroban Wasm limit.

## Decision

### Cumulative Phase 1 profile

Phase 1 binds ADRs 0002, 0003, 0004, and 0005 in that dependency order. ADR 0002 remains the registry, lifecycle, compatibility, event-replay, and upgrade authority. ADR 0003 remains the fee and supplier-accounting authority. ADR 0004 remains the optional participant-admission authority. ADR 0005 remains the reserve-role and optional isolated no-rehypothecation authority.

The production-v1 manifest required by ADR 0002 must name this ADR and every predecessor it binds only after each is `Accepted`. It must also name the separately Accepted issuance/emissions disposition required by ADR 0002. Until then, all deployed addresses are disposable and no Phase 1 mainnet genesis is authorized.

The cumulative production ABI materializes the final shapes from ADRs 0002–0005 plus the amendments below. It does not retain a client for an ADR-0002-only, caller-oracle, optional-backstop, or positive-backstop-take deployment signature.

### Permissionless creation and pool discovery

Any address may invoke `registry.deploy` by selecting a pool-admin address whose authorization is present. Deployment requires no registry-admin signature, governance proposal, creator permit, creator allow-list, or frontend listing record. The registry's global `onboarding_paused` control still blocks every new deployment under ADR 0002.

`admin.require_auth()` remains mandatory because it proves the selected pool admin consents to the deployment and constructor authorization tree. It must not be reinterpreted as registry-governance permission. A transaction source or fee payer may differ from the selected pool admin.

The registry deploys only conforming pools, writes membership atomically, and emits ADR 0002/0004's cumulative `pool/deploy` event. Event replay plus point queries is the complete protocol pool inventory. Phase 1 adds no unbounded `list_pools` export and no on-chain `listed`, `official`, `approved`, or certification flag.

Templar-operated frontends maintain their own allow-list of pools they choose to display. Listing or delisting changes no registry state and grants no authority. A conforming unlisted pool remains discoverable from registry events and callable directly. Frontends must distinguish “registered” from “listed by this interface” and must not describe listing as protocol certification.

### Mandatory backstop and zero take

Every Phase 1 deployment supplies one enabled, compatible `backstop: Address`; the public Phase 1 deploy signature does not accept `Option<Address>`. The registry passes `Some(backstop)` to the cumulative pool constructor and completes ADR 0002's atomic initial activation before membership and the deploy event commit.

Every Phase 1 pool is constructed with `backstop_take_rate = 0`. The caller does not supply that value. The cumulative Phase 1 `MinConfig` retains ADR 0003's five fields but requires `max_backstop_take == 0`. Registry construction and every `SetMinConfig` proposal reject any other value. `check_config`, `check_fees`, deployment, attachment/finalization code retained for compatibility, and live `update_pool` validation reject a positive backstop take. Because every Phase 1 pool starts at zero, no grandfathered positive value exists.

Zero take means borrow-interest accrual assigns no new `backstop_credit` or backstop fee remainder. ADR 0003's protocol allocation, supplier value, ownerless accrual, default waterfall, collection availability, and event rules remain unchanged with effective `g = 0`. The backstop still performs the non-interest-take behavior retained by ADR 0002 and the cumulative implementation. Removing it, deploying `None`, or introducing a positive take requires a later deliberate ADR and a new product profile.

### Canonical protected pool oracle

Phase 1 adds one `PoolOracleAdapter` contract to `../contracts/contract/proxy-oracle/soroban`. It is a thin multi-asset SEP-40 view over the existing Soroban proxy runtime; it performs no source IO, aggregation, independent freshness policy, or circuit-breaker evaluation.

Its immutable public configuration is:

~~~rust
struct PoolOracleConfig {
    parent_oracle: Address,
    owner: Address,
    decimals: u32,
    resolution: u32,
    base: Asset,
}

fn config(e: Env) -> PoolOracleConfig
fn interface_version(e: Env) -> u32
fn storage_version(e: Env) -> u32
~~~

It implements SEP-40 `PriceFeedTrait` as follows:

- `decimals`, `resolution`, and `base` return the immutable configured values;
- `assets()` returns the parent runtime's current `registered_assets()`;
- `lastprice(asset)` calls only the parent runtime's `aggregated_latest(asset)` and converts the returned `NormalizedPrice` to the configured decimals using the proxy package's checked conversion;
- `price` and `prices` use only `aggregated_history` and preserve the existing adapter's exact-timestamp and oldest-first behavior; and
- an unknown asset, missing or non-`Accepted` cache, stale cache, blocked feed, missing storage, or conversion overflow returns `None`. There is no direct-source fallback and no last-known-good bypass after the parent freshness window expires.

The adapter accepts `Asset::Stellar` and `Asset::Other` because the runtime is multi-purpose, but Phase 1 pool reserve reads use `Asset::Stellar(reserve_address)` exactly as the Blend pool does. `decimals <= 18`, `resolution != 0`, and checked exponent conversion remain mandatory.

`parent_oracle`, `decimals`, `resolution`, and `base` have no setter. Changing any of them requires a new adapter and therefore a later registry/profile decision; this prevents a live metadata change from silently rescaling `min_collateral`, health, liquidation, or auction values. The adapter may retain the proxy package's owner-authorized same-address Wasm upgrade surface during the maintenance period. Its owner must equal the proxy runtime's governance owner, and the release manifest binds both current Wasm hashes and the parent address. Upgrade and ownership changes require the proxy governance's existing typed process, full adapter/Blend canary, and event/readback verification before dependent pools resume routing.

The registry constructor adds immutable `pool_oracle: Address` state and a `pool_oracle() -> Address` view. Before writing genesis state it reads the adapter configuration and requires:

- nonzero interface and storage versions compatible with the current pool implementation;
- `parent_oracle` equals the reviewed Templar Soroban proxy runtime;
- the adapter owner equals that runtime's governance owner;
- adapter `base` equals the runtime's `source_base()`;
- valid decimals and resolution; and
- successful SEP-40 interface calls against the built Wasm.

The registry's genesis event appends the canonical pool-oracle address. No creator may substitute another oracle. The Phase 1 deploy signature removes both `oracle` and `backstop_take_rate`:

~~~text
registry.deploy(
  admin, name, salt,
  max_positions, min_collateral, backstop, interest_fee,
  permission_manager, hook, isolated
) -> Address

pool.__constructor(
  admin, name, pool_oracle, 0,
  max_positions, min_collateral, interest_fee, registry,
  Some(backstop), blnd_id, permission_manager, hook, isolated
)
~~~

The registry supplies `pool_oracle()` and zero take to the constructor. The pool constructor retains ADR 0002's `admin` plus registry authorization and verifies the supplied oracle equals `registry.pool_oracle()` before any state write. A direct constructor invocation, caller-selected direct oracle, single-feed adapter, or mismatched canonical adapter fails atomically without membership or deployment event.

The proxy runtime remains the source of truth for feed configuration. Every production reserve asset must have a governed `ProxyConfig` with a nonempty source set, `min_sources >= 1`, `max_age_secs = Some(age)`, and a positive age strictly below the pool's inherited 24-hour outer rejection window. Important feeds require multiple independent sources unless the release manifest records why only one source exists and marks the pool non-production. Breaker history must cover every installed rule; production breakers must be enforced before pool setup exit.

`refresh` and runtime/adapter TTL extension remain permissionless keeper operations. The release runbook assigns at least two independent keepers, refreshes each pool asset more frequently than its configured `max_age_secs`, runs TTL extension at least weekly, and alerts before half of either interval elapses without success. Manual trip, breaker trip, source quorum failure, stale cache, and TTL loss intentionally make dependent pool price reads fail closed.

### Optional isolated no-rehypothecation pools

The creator retains ADR 0005's trailing `isolated: bool` choice.

- `isolated == false` selects ADR 0005's open structural profile. ADR 0004's `(permission_manager, hook)` tuple independently decides participant admission.
- `isolated == true` selects ADR 0005's exact one-collateral-only plus one-lend-only topology, immutable role boundaries, setup-exit requirements, exposure disclosure, and state/flow no-rehypothecation guarantees. It does not create Morpho Blue equivalence.

The registry does not duplicate reserve roles, isolated identity, or certification in deployment metadata. Indexers read `is_isolated()` and the reserve factors from the pool as ADR 0005 requires. Phase 1's canonical multi-asset pool oracle must return protected prices for both isolated assets before setup exit.

## Alternatives

1. **Keep caller-supplied oracle and rely on frontend review.** Smallest contract diff, but direct callers could deploy registered pools that bypass proxy freshness and breakers. Rejected on driver 1.
2. **Point Blend directly at the normalized proxy runtime.** Avoids another adapter contract, but changes the pool from fixed-decimal SEP-40 prices to exponent-form prices throughout health, liquidation, auction, and minimum-collateral calculations. Larger and riskier than one thin view contract. Rejected on drivers 2 and 3.
3. **Use the existing single-asset `Sep40Adapter`.** Already implemented and tested with Blend, but it returns `None` for every asset except its immutable configured asset. It cannot serve ADR 0005's two-asset isolated pool or an open multi-reserve pool. Rejected as non-conforming.
4. **Deploy one existing adapter per reserve and store an oracle map in the pool.** Reuses current adapters but adds per-reserve addresses, storage, configuration, and calls to every pool valuation path. The multi-asset adapter is smaller and preserves the existing one-oracle pool model. Rejected on driver 3.
5. **Require registry-governance approval for every pool deployment.** Simplifies frontend curation but makes creation permissioned and couples deployment throughput to governance. Protocol-wide policy checks already bound valid creation. Rejected on driver 1.
6. **Put official listing state on-chain.** Creates a second governance surface and invites clients to mistake product curation for protocol validity. Off-chain allow-lists are reversible and sufficient. Rejected on principles 1 and 2.
7. **Deploy Phase 1 pools without a backstop.** ADR 0002 supports that protocol state, but it conflicts with the selected Phase 1 product requirement. Rejected.
8. **Keep the backstop and allow a positive take.** Preserves the inherited revenue share but conflicts with the selected zero-take profile and duplicates ADR 0003's protocol revenue claim. Rejected.
9. **Restate ADR 0005's isolation rules here.** Makes this ADR self-contained at the cost of creating two authorities for the same invariant. Binding ADR 0005 is smaller and prevents semantic drift. Rejected on driver 2.

## Consequences

Anyone can create a conforming registered pool, but nobody can create a Phase 1 registry pool with an arbitrary oracle, no backstop, a positive backstop take, or parameters outside current registry policy. Registry governance controls policy and incident containment, not ordinary deployment approval.

Official interfaces may present a reviewed subset without changing on-chain membership. Users and integrators must inspect the registry and pool directly rather than treating one frontend's list as complete or authoritative.

The pool hot path keeps its existing SEP-40 calls. The new adapter adds one proxy-runtime call per distinct uncached asset price, the same delegation shape already exercised by the single-asset adapter. No registry call, source call, or fee-recipient call is added solely by this ADR to ordinary pool actions.

Every Phase 1 pool retains backstop operational dependencies while allocating zero borrow-interest take to it. Backstop capital therefore needs an economics and issuance disposition outside this ADR; the zero take must not be presented as removal of bad-debt or liquidation reliance.

The canonical oracle concentrates pool price liveness in one governed runtime and adapter. This removes creator bypass but raises the impact of keeper, governance, upgrade, or storage failure. Fail-closed behavior is intentional: the recovery path is restore/refresh/rearm/upgrade under the reviewed oracle process, not a direct-source fallback.

Production genesis remains blocked until ADRs 0002–0006 and the mandatory issuance/emissions disposition are merged and human-accepted. This ADR's consensus review is not human acceptance.

### Acceptance criteria

1. `registry.deploy` succeeds with authorization from the selected pool admin and no registry-admin authorization, creator allow-list, governance proposal, or frontend record. Missing pool-admin authorization fails before any deploy, membership write, backstop activation, or event.
2. Deployment while `onboarding_paused` fails with ADR 0002's `Paused` result and no state or event. Permissionless creation does not bypass incident containment.
3. Every successful Phase 1 deployment has `is_pool(pool) == true`, `backstop_of(pool) == Some(selected_backstop)`, pool `get_backstop() == Some(selected_backstop)`, `bstop_rate == 0`, and pool oracle equal to `registry.pool_oracle()`. Exactly one cumulative deploy event commits after initial backstop activation and membership.
4. The Phase 1 deploy client has no `oracle`, `backstop_take_rate`, or optional-backstop argument. Direct pool construction, a substituted oracle, `None` backstop, disabled/incompatible backstop, or positive take commits no pool membership or deploy event.
5. Registry construction and `SetMinConfig` accept `max_backstop_take == 0` and reject every positive value. Every attempted live transition from zero to positive take fails before reserve settlement or configuration write. ADR 0003 `check_fees(interest_fee, 0)` and positive protocol-fee policy remain enforced.
6. With positive borrow interest and zero backstop take, generated traces allocate no new backstop credit or backstop fee remainder, preserve ADR 0003's protocol and supplier identities, and allow protected protocol collection only under ADR 0003's atomic fee state and liquidity formula.
7. Genesis-to-tip replay of cumulative registry events reconstructs exactly every registered pool, implementation, initial permission tuple, backstop relationship, lifecycle, policy, fee-collection state, and canonical pool-oracle address. No on-chain frontend-listing state exists.
8. A frontend-listing implementation can omit or later delist a registered pool without any registry transaction. Direct registry/event discovery still finds it, and pool calls remain unchanged. UI/API copy distinguishes registered from listed and never calls listing protocol certification.
9. The optimized `PoolOracleAdapter` implements the SEP-40 wire consumed by Blend. Built-Wasm tests prove at least two distinct `Asset::Stellar` reserves return correctly scaled accepted prices through one adapter and that `assets()` reflects the parent runtime's registered assets.
10. Unknown, removed, stale, blocked, source-quorum-failed, storage-missing, and conversion-overflow cases return `None` through adapter `lastprice`; the real Blend pool converts each to its existing `InvalidPrice` failure with no successful health, borrow, liquidation, or auction transition using a fallback value.
11. Adapter configuration is immutable and read back exactly. Registry genesis rejects parent, owner, base, version, decimals, resolution, or interface mismatch. Pool construction rejects an oracle unequal to `registry.pool_oracle()`.
12. Oracle governance configures protected prices for every listed reserve before setup exit. An isolated pool proves accepted prices for its exact collateral and lend assets, then proves manual trip and freshness expiry each stop a normal borrow and liquidation-price path until governance recovery and a successful refresh.
13. Two independent keepers exercise permissionless refresh and TTL extension. Testnet evidence covers healthy refresh, one keeper absent, both keepers absent past warning thresholds, stale fail-closed behavior, TTL restoration, manual trip, breaker trip, source quorum loss, rearm, and recovery without direct-source fallback.
14. ADR 0005's complete isolated acceptance suite passes unchanged with the Phase 1 canonical oracle, mandatory backstop, and zero take. `isolated == false` remains available, and ADR 0004 admission tuples remain independent of structural isolation.
15. The cumulative manifest names only existing Accepted ADRs in dependency order, includes the separately Accepted issuance/emissions disposition, binds implementation commits and optimized Wasm hashes, freezes the cumulative specs/clients/events/errors, and rejects every interim layout. Today this criterion is intentionally unmet.
16. The implementation runs ADRs 0002–0005's applicable unit/property, Kani, actual-Wasm, replay, resource, and testnet gates. Every optimized artifact is measured against the live effective network limit; proxy runtime/governance remain within 128 KiB and the pool adapter within 32 KiB.

### Verification strategy

Run the smallest proving checks first and report every level that ran:

1. **Static/spec checks:** `PATH="$PWD/bin:$PATH" adr --robot lint`; generated contract specs show the exact Phase 1 constructor/deploy changes and no caller oracle, take-rate, or optional-backstop input.
2. **Proxy unit/integration checks:** from `../contracts`, run the proxy package's existing kernel, runtime, governance, adapter, and integration tests plus new multi-asset adapter and registry-construction cases; then run `just -f contract/proxy-oracle/soroban/justfile audit-ready`.
3. **Blend unit/property checks:** build before test as required by the vendored workspace. Exercise permissionless deploy authorization, canonical-oracle equality, mandatory initial activation, zero-take policy, fee accounting at `g = 0`, and unchanged ADR 0004/0005 properties.
4. **Bounded model checking:** prove deployment atomicity across adapter check, pool construction, backstop activation, membership, and event; prove `max_backstop_take == 0` is invariant under every Phase 1 configuration transition; retain ADR 0003's zero-`g` arithmetic and ADR 0005's state/flow harnesses.
5. **Actual-Wasm compatibility and replay:** instantiate optimized registry, pool, backstop, proxy runtime, governance, and pool adapter. Decode with final generated clients; replay raw XDR; reject missing, duplicate, reordered, or mismatched genesis/deploy/oracle events.
6. **Testnet smoke:** install manifest-bound Wasms; verify on-chain hashes; initialize runtime/governance/adapter/registry; deploy an open and an isolated pool permissionlessly; configure at least two reserve feeds; refresh; exercise a normal borrow and liquidation-price path; trip and expire feeds; prove fail-closed behavior; recover; collect a protocol fee; read back all registry/pool/oracle state.
7. **Frontend separation:** against the deployed Testnet registry, show a registered pool absent from the official allow-list remains event-discoverable and directly callable, then list/delist it without an on-chain transaction.

### Risks and mitigations

| Risk | Owner and trigger | Prevention/detection | Recovery |
| --- | --- | --- | --- |
| Arbitrary creator bypasses protected pricing | registry implementation owner; deploy input or constructor substitution | remove caller oracle; constructor equality check; actual-Wasm auth/spec tests | block release; upgrade before any non-disposable pool |
| Canonical proxy or adapter outage stops many pools | oracle operations; refresh/TTL/source/governance failure | two keepers, freshness/TTL alerts, fail-closed probes, staged upgrades | restore footprint, refresh/rearm, or governed same-address repair; never direct-feed fallback |
| Adapter metadata change rescales pool economics | oracle governance; decimals/base/resolution mutation | immutable metadata; no setter; manifest readback | keep pools frozen; deploy/review a new profile rather than repoint silently |
| Backstop take becomes positive | registry/pool implementation owner; policy or update drift | exact zero policy at constructor, proposal, deploy, check, and update; invariant test | reject transition; if deployed, freeze risk and use reviewed repair |
| Zero take is misrepresented as no backstop | product/operator; UI or risk disclosure | explicit route/take display and ADR 0002/0005 evidence | correct disclosure; stop listing until accurate |
| Frontend allow-list is mistaken for protocol membership | frontend/indexer owner; listing change | separate data models and copy; compare allow-list with event replay | correct UI/API; registry state is unchanged |
| Permissionless deployment becomes spam or indexing load | indexer/operator; many valid deployments | event pagination, local indexing, frontend allow-list; no unbounded on-chain vector | rate-limit off-chain surfaces; never add creator permission merely for UI load |
| Isolated pool is marketed as full Morpho equivalence | product owner; launch copy | use ADR 0005's bounded terminology and evidence | correct claims and delist until reviewed |
| Mandatory predecessor or issuance decision is absent | release owner; attempted genesis | manifest accepts only existing Accepted ADRs | block genesis; complete human acceptance and missing ADR |
| Cumulative Wasm exceeds network limits | implementation owner; added adapter/validation surface | measure every optimized artifact and attribute delta | delete/narrow code or revise architecture; never weaken checks |

### Pre-mortem

1. **A permissionless creator deploys a registered pool whose prices do not traverse Templar's proxy.** Early sign: deploy spec still contains `oracle`, or pool config differs from `registry.pool_oracle()`. Mitigation: remove the argument and assert equality before writes. Recovery: treat every such address as disposable; no migration claim.
2. **The shared proxy stops refreshing and all listed pools fail price-dependent actions.** Early sign: keeper-success age approaches the shortest feed age, `RefreshFailure` rises, or adapter `lastprice` becomes `None`. Mitigation: independent keepers, half-window alerts, manual-trip and stale drills. Recovery: preserve fail-closed state, restore TTL/source/quorum, refresh, verify, then resume frontend routing.
3. **A pool is launched as “no backstop” because take is zero.** Early sign: UI hides the backstop address or risk documents omit bad-debt dependence. Mitigation: display both route and take rate; acceptance reads `Some(backstop)` and zero separately. Recovery: correct disclosures and stop listing until the actual risk model is shown.

Follow-ups: draft the mandatory issuance/emissions disposition; implement the cumulative contracts only after the required ADRs are Accepted; produce the Phase 1 production manifest and frontend listing policy in their owning repositories; decide any later no-backstop or positive-backstop-take product as a separate deliberate ADR.
