# 8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim

Date: 2026-09-03

## Status

Accepted

The historical
[`2026-09-08-adr-0008-0009-hardened-bridge.json`](reviews/2026-09-08-adr-0008-0009-hardened-bridge.json)
receipt remains evidence for its recorded blobs, but its consensus authority was withdrawn by
[`2026-09-09-adr-0008-0009-consensus-withdrawal.json`](reviews/2026-09-09-adr-0008-0009-consensus-withdrawal.json).
This revision requires a fresh Raladr Architect and Critic review before human acceptance.

Amended 2026-09-16 after the adversarial review recorded in
[`adr-0008-0011-gate-reopen-record.md`](reviews/adr-0008-0011-gate-reopen-record.md): restriction 7
now names the same-reserve supply setoff and its `debt_setoff` event, the declared no-denominator
deviation from stock `default_liabilities`, and the trapped pool `set_emissions_config`; section 4
adds the live-read caller obligation for disable transitions; sections 8 and 13 add orphan-cash and
settlement-window disclosure. The amendment requires the same fresh review before acceptance.

Amends [2. Pool registry as deployment, configuration, and optional-backstop routing layer](0002-pool-registry-as-deployment-configuration-and-optional-backstop-routing-layer.md)

Amends [3. Protocol fee on borrow interest; defer origination and flash-loan fees](0003-protocol-fee-on-borrow-interest-with-deferred-origination-and-flash-loan-fees.md)

Backstop emissions are governed by
[11. Disable activation-shim backstop emissions while preserving its ABI](0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md).
This ADR selects that exact six-trap backstop profile; both ADRs require human acceptance before
activation.

Migration is governed by
[9. Migrate ADR-0008 legacy pools to registry-native successors](0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md).
ADR 0009 is not a launch gate, but it must be accepted before a registry cutover starts.

Amended by [14. Use a multisig operator account as the ADR-0008 pool admin instead of an immutable policy contract](0014-use-a-multisig-operator-account-as-the-adr-0008-pool-admin-instead-of-an-immutable-policy-contract.md)

## Context

Templar wants to serve limited near-term lending demand before the Precept registry and permanent
pool profile are ready. Stock Blend requires a backstop even when Templar does not want to operate an
economic backstop. The proposed deposit is therefore an **activation shim**: its two assets have no
market value, it earns no protocol take, and it is not insurance or junior capital.

That design can be defensible only as a small, temporary, isolated bridge. It is not a path to one
shared $100M fault domain. The immutable shim still participates in activation and status reads.
Operations can detect its failure but cannot repair or repoint a deployed pool.

The source base is `vendor/blend-contracts-v2` commit
`6cf6dd1ca712ea3cf17a767fe49b7da01f80b909`, identical in its executable contract/build tree to
upstream Blend v2.0.0 release commit `c19abee5b9be4f49e0cda9057e87d343e5dcc095`.
The factory remains byte-identical to that base. The pool receives a focused security fork, and the
backstop receives ADR 0011's focused six-trap emissions fork.

Existing audit work does not name the complete final release tree. This ADR therefore claims
upstream v2.0.0 provenance plus explicitly reviewed Templar deltas, not “audited byte-for-byte.” A
report-to-release coverage assessment and focused review of the exact final diffs are launch gates.

Important stock behavior:

- The factory permanently binds one backstop to every pool it deploys. Pool, factory, and backstop
  have no useful upgrade or repoint path.
- Stock status changes read `backstop.pool_data` before handling emergency status 4. A broken
  backstop can therefore block the stock emergency freeze.
- Status 4 blocks supply, collateral supply, and borrow while preserving repay, withdraw,
  liquidation auction creation/fill, and `bad_debt(user)`.
- Pool activation depends on synthetic Comet LP geometry, not the economic value supporting loans.
- Comet v1.0.0 has a public one-time initializer and fixed initial LP supply. Deploy then initialize
  is front-runnable; deploy and exact initialization must occur in one top-level transaction.
- Stock bad-debt auctions require callers and worthless LP gives third parties no economic reason
  to fill them. This profile must therefore remove the shim from real loss processing.
- LP shares are transferable and public backstop deposit can create foreign shares. Sharing a Comet
  or backstop therefore shares failure and retirement state.
- Zero `bstop_rate` does not disable public reserve `gulp`; positive balance drift can become real
  backstop credit and an interest-auction lot.
- The pool has no queued-reserve getter. Any retirement proof must rely on an admin that owns every
  queue transition from genesis and fail closed when its queue state is unresolved.
- Direct pool participation remains permissionless. Templar controls only official funding,
  product routing, and its own authorities.

In scope: exact pool and backstop profiles; activation geometry; immutable authority; oracle and TTL
boundaries; synthetic-asset custody; exposure caps; deterministic supplier-loss processing; release
evidence; operations; disclosure; and migration.

Out of scope: insurance, junior capital, a v1 fee, permissioned users, a permanent retained-backstop
profile, changing liquidation-auction economics, choosing market risk parameters, or promising a
fixed date on which independent users will have exited.

Principles:

1. Put failures that can lose or strand value behind contract invariants, not operator promises.
2. Keep each executable diff small and independently reviewable.
3. Give every immutable shim one small fault domain and one prebuilt exit.
4. Keep evidence readable: reviewed Markdown, Git history, standard checksums, transactions, and
   live readbacks rather than a second policy language.
5. Never describe synthetic activation value as economic protection.

Decision drivers, ranked:

1. Bound supplier-loss, availability, and retirement exposure when any immutable shim component fails.
2. Minimize executable delta without misrepresenting inherited audit coverage.
3. Keep the bridge maintainable until the last direct user exits, including funded migration and tail work.

ADR 0008, ADR 0009, and ADR 0011 form one review bundle. This ADR's only recovery for immutable
failure is the exact intake brake, terminal classes, and migration path defined by ADR 0009, while
ADR 0011 makes the selected activation-shim emissions boundary enforceable. Reviewing launch
without either exit or the exact backstop profile would leave its principal safety claims
untestable.

## Decision

### 1. Authorize only a capped isolated pilot

ADR 0008 authorizes legacy pools outside the Precept production registry. Each pool has its own
factory, Comet, backstop, synthetic assets, policy admin, and share custodian. No two pools share any
of those contracts or assets.

The launch envelope is:

- each reserve's stock `supply_cap` is set in asset units so supplied principal valued at the
  greater of the approved live and stress prices cannot exceed **25,000 USD**;
- the sum of those sealed per-reserve caps at the same prices cannot exceed **50,000 USD per
  pool**; and
- activation tooling permits at most **100,000 USD** of those sealed capacities across all live
  ADR-0008 pools.

The per-reserve limit is enforced by stock Blend at every supply action. The sealed reserve set and
its `supply_cap` values make the per-pool limit an immutable consequence of pool configuration. The
global limit remains an operational deployment-count gate because separate permissionless pools
share no on-chain authority.

The deployment review groups aliases and economically equivalent assets before applying the global
cap. Splitting exposure across addresses, pools, issuers, or legal entities does not create
operational headroom. Caps may be lower for a market but may not be increased under this ADR. A
larger deployment requires the permanent registry-native profile.

These are principal-capacity limits, not hard limits on accrued claims, debt, fiat value, or tail
duration. Interest consumes remaining supply headroom and can grow existing claims above original
principal. Operations report current claims and debt and fund an indefinite repay/withdraw tail.

Each policy contract binds a sunset no later than 90 days after activation. At sunset anyone may
advance only the exact disable path described below. Sunset stops new risk; it does not erase
positions or end service obligations.

### 2. Use a seven-restriction pool fork

The pool starts from the pinned base and changes only these security boundaries:

1. **Independent absorbing freeze.** An authenticated request for status 4 is handled before any
   backstop construction or call, succeeds idempotently, and is irreversible. Every request to leave
   status 4 rejects before external effects.
2. **Zero take.** Initialization accepts only `bstop_rate = 0`; `update_pool` always rejects.
3. **Sealed configuration.** The complete reserve set, including each stress-priced `supply_cap`
   and a `max_util` strictly below 100%, is installed in setup status 6. After activation, reserve
   additions, re-enablement, and parameter changes reject both when queued and when executed. The
   only permitted reserve transition is the exact existing configuration with
   `enabled: true -> false`.
4. **No surplus auction path.** Reserve `gulp` never recognizes surplus or credits the backstop. It
   rejects except for the denominator-safe orphan settlement defined in restriction 7. Creating or
   filling an `InterestAuction` rejects. Existing cancellation dispatch remains unchanged.
   Unsolicited underlying remains physically present but unrepresented: it never increases reserve
   `b_supply`, accounting-derived exit headroom, or any official withdrawal cap. Raw token balance
   is only a separate transfer-sufficiency and reconciliation input.
5. **Strict pool price observations.** The pool rejects future-dated observations, observations
   older than 24 hours, nonpositive prices, and any oracle decimal scale other than seven before
   dependent effects. The proxy may impose stricter market limits.
6. **No flash loans.** `flash_loan` keeps its ABI and rejects before authorization, transfer,
   callback, storage, or event effects in every status.
7. **Zero-raw-collateral-aware direct residual-debt default.** After liquidation exhausts auctionable
   collateral, the bad-debt handler computes the stock `PositionData` under the strict current
   oracle and treats remaining collateral as economically absent only when aggregate
   `collateral_raw == 0`. This includes zero-rate positions and positive-rate b-token dust whose
   conversion or oracle multiplication floors to zero. The public `bad_debt(user)` entrypoint keeps
   the stock no-live-`UserLiquidation`-auction precondition. Before effects, the handler rejects
   unless every residual collateral b-token has no pool-emissions-map entry, reserve emission data,
   or pool-address user-emission data. The admin `set_emissions_config` entrypoint keeps its ABI
   and unconditionally rejects as its first operation, so no pool-emissions-map entry can ever be
   created on this fork. In one atomic operation, the handler snapshots every residual collateral
   owner, asset, and b-token amount; for each liability reserve, first burns the user's own
   noncollateral supply in that same reserve against that liability at the accrued `b_rate` and
   `d_rate` (same-user, same-reserve setoff, never cross-reserve netting) and emits
   `debt_setoff(asset, b_tokens_burned, d_tokens_repaid)`; defaults only the uncovered residual
   directly against that reserve's suppliers through the existing `default_liabilities` accounting
   path; then, only if some residual was actually defaulted, moves each snapshotted amount from the
   user's collateral position to a noncollateral supply position owned by the pool contract itself.
   A setoff that clears every liability leaves the user's collateral in place and creates no
   orphan. `default_liabilities` applies the exact stock ceil-rounded `b_rate` update whenever
   `b_supply > 0`; when the setoff has burned the last b-token of the reserve, the residual is
   cleared at an unchanged `b_rate`. This is a declared deviation from stock, which would divide by
   zero there; it is the only case in which defaulted debt produces no supplier haircut, and the
   differential must assert it explicitly rather than absorb it. The orphan transfer preserves
   reserve `b_supply` exactly, prevents value revival for the borrower, and emits
   `collateral_orphaned(user, asset, b_tokens)`. Any failure rolls back setoff, loss, and
   reclassification together. The pool-owned position has no private key or
   withdrawal path. In every pool status, permissionless `gulp(asset)` applies the same emissions
   absence check and rejects while that reserve has `d_supply > 0`; once debt is zero it removes only
   the pool-owned b-token position and emits `orphan_settled(asset, b_tokens)`. The public call
   returns zero in the stock underlying-token-delta unit, so its unchanged wrapper also emits stock
   `gulp(asset, 0)`; with no orphan it returns zero and emits only that stock zero event. It never
   transfers tokens, recognizes surplus, or credits the backstop. Liability loss still emits stock
   `defaulted_debt(asset, d_tokens)`, never stock `bad_debt`; setoff repayment emits only
   `debt_setoff`. Creating or filling a
   `BadDebtAuction` rejects before effects.

Everything else, including ordinary liquidation auctions, remains base behavior. The implementation
must be an allowlisted diff, preserve existing storage and wire formats, and keep each optimized Wasm
at or below `min(131072, active_network.contractMaxSizeBytes)`.

### 3. Use ADR 0011's six-trap backstop fork

The backstop preserves the pinned ABI but unconditionally raises `BackstopError::BadRequest` as the
first operation of `distribute`, `gulp_emissions`, `add_reward`, `remove_reward`, `claim`, and
`drop`, exactly as defined by ADR 0011. No public caller can create reward-zone, distribution,
backfill, accrual, allowance, token-movement, Comet, emitter, or claim effects through those exports.
The read-only `reward_zone` getter and the ordinary deposit, queue, dequeue, withdrawal, donation,
and status paths remain available.

Fresh setup creates no pool emissions map or reserve/user emission data, and the pool fork traps
`set_emissions_config` (restriction 7), so no admin route can create one later. The retained
ordinary backstop hooks therefore take their stock no-data return; the policy additionally exposes
neither `set_emissions_config` nor an arbitrary call.
Activation and every orphan create/settle operation read and require the relevant b-token
configuration/data to remain absent. Pool `gulp_emissions` must roll back at the trapped backstop
call without creating a pool claim. Any contradictory read or event closes routes, blocks orphan
processing, and permanently retires the cohort.

The factory is byte-identical to the pinned base. Each deployment record names the exact pool and
backstop fork artifacts and their independent maintenance obligations.

### 4. Make the policy contract the pool admin from construction

A small, immutable, non-upgradeable `v1_policy_admin` contract is the pool admin from deployment. Its
constructor binds one operator G-account, the exact factory and deterministic pool identity, pool
constructor arguments, ordered reserve configurations, oracle, sunset, and immutable
`retirement_sink`.

The policy exposes only:

- one-time deterministic pool deployment and setup;
- one-time activation from status 6 to status 0 after exact live readback and with no policy-owned
  queue outstanding;
- operator-authenticated status 4;
- operator-authenticated `contain(asset)` before sunset and permissionless `expire(asset)` at or
  after sunset, both limited to the exact `enabled: true -> false` transition;
- cancellation at status 4 of an exact pending disable created by the policy;
- read-only bound/live reserve state and policy-owned queue phase; and
- operator-authenticated `propose_retirement()`.

The policy has no arbitrary call, upgrade, rescue, token transfer, reserve addition, parameter
change, re-enable, or status-downgrade capability. It is the only authority able to queue reserve
changes, records every queue it creates, and remains fail-closed until each is observed executed or
cancelled and live reserve configuration agrees. A TTL-expired or otherwise unresolved queue record
blocks activation and retirement; it is never inferred away.

Caller obligation for every disable: the pool accepts a post-activation reserve change only when
the queued configuration equals the live stored configuration in every field, including the
pool-assigned `index`, with `enabled` flipped to false. The policy must therefore build each
`contain`/`expire` configuration from a live `get_reserve` read at queue time, never from its bound
constructor copy, whose `index` predates reserve creation. A mismatch reverts with
`InvalidReserveMetadata` and would leave the all-reserves-disabled fallback brake unreachable. The
pool's types do not enforce this; the policy review must check it explicitly.

`propose_retirement()` succeeds only after the policy live-reads status 4 or all reserves disabled,
all policy-owned queues resolved, and every configured reserve matching its bound terminal state. It
then calls the pool's actual `propose_admin(retirement_sink)` entrypoint. Before the stock temporary
proposal expires, anyone may call the sink's only method, `accept_pool_admin(pool)`; the sink
authorizes the nested `pool.accept_admin()` call as itself. There is no assumed `set_admin`
entrypoint and no off-chain inspection of private pool storage.
The bound terminal state checks represented accounting, not raw reserve-token zero or a USD
rounding budget. The policy does not try to verify off-chain reference-model attribution; the
operator's reviewed terminal record supplies that evidence before invoking the on-chain handoff.

The pool fork, rather than policy correctness, enforces zero take, sealed configuration, disabled
surplus/interest-auction and bad-debt-auction paths, direct supplier default, strict price
observations, no flash loans, and absorbing status 4.

### 5. Construct a non-economic shim without transferable control

One pool gets one Comet, one backstop, one synthetic BLND leg, one synthetic USDC leg, and one share
custodian. No contract or asset is reused across pools. Synthetic-token admins, Comet controller,
policy operator, deployment authority, and share custodian are distinct recoverable authorities with
no small common signer set controlling every boundary.

A reviewed one-use helper atomically deploys and initializes exact Comet bytes with exact
constructor values. The transaction fails as a unit on any authorization, identity, or readback
mismatch. Separate deploy then init is forbidden. After seeding:

- no ceremony account retains synthetic legs or LP;
- no external holder remains authorized for either leg;
- Comet control is frozen to the approved at-rest posture; and
- one recoverable custodian owns all intended backstop shares.

The seed is the minimum exact geometry that passes Blend activation after integer floors. Its
values, LP supply, rounding margin, and Comet artifact hash are derived and tested, not copied from a
fixture. No symmetric seed or inflated synthetic “value” is permitted.

The activation shim is disclosed as worthless plumbing. It is never counted as TVL, insurance,
reserve capital, or expected auction consideration.

### 6. Use the oracle as an independent debt-increase brake

Every reserve uses a reviewed Templar proxy oracle with a canonical USDC base, seven-decimal output,
explicit freshness, minimum positive price, deviation, breaker, governance, and TTL policy. The pool
enforces the universal temporal and decimal boundary; the proxy enforces stricter market risk.

A tripped or stale proxy must reject debt-increasing or health-dependent actions before effects. It
is an independent product brake, not a substitute for status 4 and not a reason to abandon repay,
withdraw, liquidation, or migration support. Redundant funded keepers maintain oracle and contract
TTLs while any position or open liquidation auction remains.

Before listing a reserve, property tests prove that one d-token unit at the lowest permitted oracle
output cannot floor stock debt value to zero. Stock withdraw and collateral-withdraw enforce
utilization below 100%, so `b_supply` is nonzero whenever `d_supply` is nonzero, but sealed
`max_util` does not preserve free cash against withdrawals. For every sequence of direct defaults,
tests prove the stock per-event update
`b_rate = max(b_rate - ceil(default_amount * 10^12 / b_supply), 0)` and its cumulative rounding
whenever `b_supply > 0`, and prove that the declared no-denominator case (restriction 7) preserves
`b_rate` exactly.
For every sequence, tests cover both a full default after free cash has been withdrawn to the stock
limit and cumulative rounding. Either can leave unauctionable collateral: `b_rate` may reach zero,
or a positive-rate b-token position's asset conversion or strict-oracle multiplication may floor to
zero. Stock withdrawal cannot burn a zero-asset-unit position, and a lot with aggregate
`collateral_raw == 0` cannot form a liquidation auction. Restriction 7 therefore transfers all such
residual collateral of a defaulting user to a pool-owned noncollateral position without changing
`b_supply`. Later rate or price movement cannot revive borrower value and remaining reserve debt
retains a nonzero loss/accrual denominator. Once `d_supply` reaches zero, terminal `gulp` removes the
orphaned position without transfer or backstop credit; any physical surplus becomes abandoned value.
Residual balances belonging to users without defaulted liabilities must convert to zero underlying
asset units to enter the disclosed rounding-loss terminal class; any withdrawable unit must exit.

### 7. Release through ordinary reviewed evidence

Each pool has one readable Markdown deployment record reviewed in a protected pull request. It
contains:

- exact source commits, complete allowlisted diffs, lockfiles, toolchain and build commands;
- optimized Wasm checksums and byte sizes for pool, factory, backstop, helper, policy, and sink;
- generated-interface checks and proof that the factory equals the pinned base;
- deployed contract and asset IDs, constructor values, transaction hashes, ledger sequences, and
  live code, admin, reserve, oracle, cap, status, queue, custody, and TTL readbacks;
- the audit coverage assessment and focused reviews for uncovered base risk and each final fork;
- the direct frontend route or exact vault/adapter build and its focused review;
- exposure calculation, stress prices, authority map, sunset, operators, keepers, runbooks, service
  levels, migration class, and named security/operations approvals; and
- testnet and disposable-mainnet canary evidence against the exact release Wasm.

Factory deploy events do not confer official identity. Only the exact deterministic pool address in
the reviewed record may enter official inventory or routing.

Git history records changes. Standard SHA-256 checksums bind bytes. On-chain transactions and live
readbacks prove execution. A bespoke JSON schema, signing envelope, hash chain, vector corpus, or
ADR-embedded verifier is not part of this decision. Later tooling may automate checks, but its
private format is not architecture and its output never overrides readable evidence or live state.

Any missing evidence, checksum mismatch, unreviewed executable delta, stale readback, or authority
mismatch blocks deployment and official routing.

### 8. Operate fail closed and preserve loss-processing liveness

Before activation, Templar assigns a contract-capable primary and backup responder, redundant funded
liquidation/TTL/oracle keepers, independent signing authorities, and funded migration and
indefinite-tail owners. Every alert has a service level and exercised runbook. Official routes close
on stale monitoring, code/admin/configuration drift, unexplained accounting change, record gap,
oracle failure, policy or contract TTL risk, unexpected queue, operational pool/global valuation
breach, or unresolved upstream security issue. Reaching a sealed reserve `supply_cap` closes that
reserve's official supply route, not unrelated pool actions.

Each pool has an aggregate **25-USD operating tripwire** for model-attributed rounding surplus,
not an enforceable lifetime bound or a retirement ceiling. Reconcile exact base-unit deltas against
the pinned reference model separately from donations and orphan-settlement value. Value each
reserve's attributed rounding at the greater approved live/stress price, rounding each conversion
up to seven-decimal USD before summing. An observed or forecast sum above 25 USD closes official
intake and starts containment; stale/unavailable valuation also closes intake under the existing
monitoring rule. Keep safe repay/withdraw/liquidation and migration support available.
Archive the breach, valuation inputs and ledger, attribution, investigation, and corrective action;
later price decreases never erase the incident. Review and reconcile further retirement-path
rounding and revaluation through terminal handoff. A maximum-single-transition launch test does not
prove every exit sequence fits lifetime headroom. No action is rejected solely to keep an already
retiring pool below this tripwire, and no surplus-recognition or transfer authority is added.

Status 4 is the primary containment path. If a non-getter pool failure prevents it while policy
authorization and transaction inclusion remain healthy, the policy may queue and execute the exact
disable for every reserve after the stock timelock. Only all reserves disabled with no unresolved
policy queue is a fallback intake brake. Operator loss, policy TTL failure, failed inclusion, or
foreign queue state leaves containment pending behind closed product routes and the oracle brake; it
does not create a fictional on-chain fallback deadline.

Liquidation keepers promptly create and fill eligible liquidation auctions. They alert on fill
latency against auction block age and on utilization approaching 100% while an underwater position
exists. They never create a lot with aggregate stock `collateral_raw == 0`. A full fill that leaves
only zero-raw collateral invokes the direct-default handler. Otherwise keepers complete or recreate
eligible liquidation until the position has no auctionable collateral. If a now-zero-raw lot is
already live, they preserve it until the stock stale-auction deletion path is available, delete it,
and then call permissionless `bad_debt(user)`. That public entrypoint rejects while an auction is
live. The modified handler recomputes stock `PositionData`, proves aggregate `collateral_raw == 0`
and every residual collateral b-token's emissions state absent, snapshots all such collateral,
defaults all remaining liabilities, then atomically reclassifies those b-tokens as pool-owned
noncollateral supply without changing aggregate `b_supply`;
`collateral_orphaned` records the exact move. The transaction rolls back as a unit. Later underlying
accrual belongs only to the unreachable pool-owned position. After that reserve's `d_supply` reaches
zero, a keeper calls terminal `gulp(asset)` to remove only the orphaned position; `orphan_settled`
records the b-token amount, while the return and unchanged stock `gulp` event report zero underlying
tokens and no tokens move. Settlement is permissionless and conditioned only on `d_supply == 0` at
the instant of the call, including a transient zero between a full repay and a new borrow inside one
ledger; keepers and reconciliation must not assume a stable debt-free state and must attribute an
`orphan_settled` event whenever it appears. Until settlement, the cash behind the orphan is ordinary
reserve cash: other suppliers may withdraw against it under the stock utilization limit, so abandoned
value is first-come-first-served rather than reserved. That is disclosed, not prevented.
The pool writes residual liability loss to the affected suppliers; there is no bad-debt auction,
backstop draw, treasury cure budget, synthetic-LP disposal, emissions claim, or surplus-credit step.
Archival and reconciliation treat each focused `debt_setoff`, stock `defaulted_debt`, focused
`collateral_orphaned` and `orphan_settled`, and stock `gulp(asset, 0)` event as one explained loss
sequence and prove the corresponding positions, aggregate supply (reduced only by `debt_setoff`
b-tokens, unchanged by orphaning, reduced again only at settlement), liabilities (reduced by
`debt_setoff` d-tokens plus `defaulted_debt`), supplier rates, pool-emissions absence, and physical
surplus. A `b_supply` or `d_supply` change inside `bad_debt` or a final fill that no `debt_setoff`,
`defaulted_debt`, or `collateral_orphaned` event explains is an unexplained accounting change and
closes routes. This is loss accounting, not insurance or compensation.

Liquidation keepers use separate filler accounts with no unrelated pool positions and preflight the
exact post-fill position against the possible same-transaction `b_rate` loss. A reverted final fill
leaves the auction live and pages a second funded filler; no path bypasses the stock health check.
Because `defaulted_debt` has no user topic, each archived transaction binds the called user or
liquidation owner to exact before/after position and reserve deltas rather than inferring identity
from that event alone.

From the pool deployment ledger onward, gap-detected archival and point-read reconciliation cover
positions, reserves, liquidation auctions, policy transitions, backstop shares/q4w/tokens, asset
authorization, Comet, oracle, and official routes. Missing history is recovered and reconciled
before routing resumes.

While any cohort is live, a named fork owner reviews every intervening upstream commit and relevant
security/RustSec advisory at least every seven days. Changes to `User::default_liabilities`,
`remove_liabilities`, or the pool bad-debt handler are explicitly fork-sensitive. An overdue or
incomplete checkpoint closes routes and blocks new cohorts. Applicable fixes receive a new reviewed
artifact for future cohorts; existing immutable pools are contained and migrated, not patched in
place.

### 9. Define the tail honestly

A cohort is never reactivated after status 4, a completed all-disabled fallback, a code/configuration
integrity incident, or a shim incident. Recovery is migration.

Backstop retirement has two honest terminal classes:

1. **normal empty:** user and pool shares, q4w, and tokens are zero after the stock withdrawal delay;
   or
2. **foreign-state tombstone:** no Templar-represented claim remains, but an uncooperative holder or
   unsupported unsolicited transfer leaves exact archived shares, q4w, or tokens.

Pool retirement permits one explicit rounding-loss equivalence after every reserve has
`d_supply == 0` and every pool-owned orphan is settled: aggregate or position b-token supply and
collateral entries may remain only when each owner's exact balance converts through the live
`b_rate` to zero underlying asset units. `b_rate` may be zero or positive. Those balances are
unburnable, nonwithdrawable claims. The terminal record archives their exact owners, position kind,
amounts, conversion inputs/results, and originating loss transactions; they do not block drained
state or sink handoff. Every balance converting to at least one underlying asset unit must be
withdrawn, migrated, or otherwise reconciled before retirement. Every reserve requires zero
liability and no active auction.

Fully explained **unrepresented physical residue** is a separate terminal disposition, not a
rounding-loss b-token claim. After the other terminal predicates pass, exact model-attributed
rounding surplus may remain physically stranded, including above the 25-USD operating tripwire.
The terminal record preserves its rounding source, per-asset base units, model/version and
transaction-delta evidence, and any investigated tripwire incident. It records permanent
abandonment in place, not a donation reclassification, new claim, supplier compensation, `gulp`
credit, or authorized sweep. Unexplained units and any remaining withdrawable claim still block
retirement. Terminal admissibility uses exact units and claim reconciliation, not a current USD
price or remaining tripwire headroom; archive the last valid valuation and any unavailable-price
incident without inventing a fresh price. No oracle/TTL/keeper estate service is owed solely for
this residue after full terminal proof and sink ownership. The incident evidence remains retained;
later revaluation or unsolicited transfers neither erase it nor reactivate the estate.

Backstop liabilities/credit, pool-owned orphan positions, migration authority, official routes, and
positive-value user, vault, treasury, and protocol claims must reach zero before retirement;
rounding-loss b-tokens remain represented only as terminal records. The policy then proposes the
immutable sink and the sink accepts pool admin. Later unsolicited transfers create no user claim and
do not reactivate the product.

### 10. Launch and abandonment gates

Launch order is:

1. freeze the exact ADR bundle and obtain fresh Raladr approval, then human acceptance;
2. implement pool, ADR-0011 backstop profile, helper, policy, and sink in reviewed source pull
   requests;
3. complete focused security review and reproducible builds;
4. deploy and reconcile the isolated shim and contracts on public testnet;
5. run a disposable mainnet canary covering activation, ordinary lending, liquidation, direct
   supplier-loss accounting, containment, sunset disable, terminal classes, and migration metadata;
6. complete the Markdown deployment record and named approvals; and
7. merge official routing only after every prior gate passes.

Before implementation begins, owners publish a dated comparison showing that this bridge has a real
schedule advantage over waiting for the registry-native successor after counting shared-team work,
review availability, integrations, canaries, operations, migration capital, and indefinite tail.
The comparison accounts for the stock reserve-disable timelock: permissionless sunset expiry may
require roughly seven additional days before intake is fully disabled. It sets a hard abandonment
date. Missing that date abandons v1; sunk work is not a reason to launch or add another cohort.

If the permanent profile becomes ready first, Templar uses it and does not launch ADR 0008.

## Alternatives

1. **Wait for the registry-native successor.** Safest and lowest-maintenance. Chosen automatically if
   ADR 0008 has no demonstrated schedule advantage or misses its abandonment gate.
2. **Use a real economic backstop.** Preserves stock loss-processing assumptions but requires genuine
   junior capital and reviewed backstop economics. It is preferable if Templar can fund it.
3. **Use stock Blend with an operationally inert shim.** Smallest source delta, but status 4 remains
   dependent on the shim and public `gulp`, configuration, flash-loan, and emissions behavior remain
   policy rather than invariants. Rejected.
4. **Share one shim across many pools.** Reduces deployment work but creates a shared immutable fault
   and tombstone domain. Rejected even below the global cap.
5. **Adopt the broader PR-6 pool redesign immediately.** Better permanent architecture if fully
   reviewed, but a larger delivery delta than this seven-restriction bridge. The permanent successor
   should prefer it when ready.

## Consequences

This is no longer a stock-contract shortcut. It is a narrow, security-focused pool fork plus ADR
0011's six-trap backstop fork, fixed pilot caps, and a mandatory migration tail. Direct supplier-loss
accounting removes the fake LP from real loss processing. Human-readable release evidence is
reviewable without maintaining a second policy implementation inside the ADR.

The cost remains substantial: Templar owns forked pool and backstop artifacts, one policy and sink
per pool, distinct synthetic assets and authorities, continuous monitoring, redundant
keepers, focused security reviews, migration capital, and an indefinite direct-user tail. The
100,000 USD global principal cap does not make those obligations cheap. If the team cannot staff and
fund them, the correct outcome is not to launch.

Supplier loss remains possible. Liquidation remains stock behavior; any residual debt after
liquidation is socialized directly and atomically to the affected reserve's suppliers. The
activation shim provides no recovery value. A compromised operator can irreversibly freeze a healthy
pool, but cannot reopen it or change economic configuration. Immutable defects are handled by route
closure, containment, and migration.

### Acceptance criteria

1. **Exact scope.** The pool diff contains only the seven restrictions above and tests; the factory
   is byte-identical to the pinned base; and the backstop diff contains only ADR 0011's six
   first-operation traps and tests.
2. **Independent freeze.** Authenticated status 4 succeeds without a backstop call, is idempotent,
   and cannot be downgraded. Unauthorized calls and every exit from status 4 reject before effects.
3. **Sealed economics and loss path.** Only zero take and `max_util < 100%` initialize;
   `update_pool`, `set_emissions_config`, reserve addition/re-enable/mutation, nonterminal or
   surplus-processing reserve `gulp`, interest-auction create/fill, bad-debt-auction create/fill,
   and flash loans reject atomically. Every supply that would take total reserve supply above its bound stock `supply_cap`
   rejects, including after interest consumes headroom, and the sealed cap cannot change after
   activation. Bad-debt eligibility ignores collateral only when stock `PositionData` computed under
   the strict current oracle has aggregate `collateral_raw == 0`. The handler first burns the
   user's same-reserve noncollateral supply against each liability and emits `debt_setoff`; then
   proves every residual collateral b-token's pool, reserve, and pool-address emissions state is
   absent; applies every residual liability default; and, only when a residual was defaulted,
   atomically moves all residual collateral to pool-owned noncollateral supply without changing
   `b_supply` or transferring assets and emits `collateral_orphaned`. While `d_supply > 0`, the orphan preserves the denominator. Once
   `d_supply == 0`, `gulp(asset)` removes only the pool-owned position, emits
   `orphan_settled(asset, b_tokens)`, and returns/emits stock `gulp(asset, 0)` in underlying units.
   Each residual liability default applies the exact stock ceil-rounded `b_rate` update whenever
   `b_supply > 0`, preserves `b_rate` in the declared no-denominator case, emits `defaulted_debt`,
   never emits stock `bad_debt`, and never enters backstop state. Every step is
   atomic. A residual user balance is terminal only if it converts to zero underlying asset units.
4. **Price boundary.** Pool and proxy tests cover current, exact-24-hour, future, stale, nonpositive,
   wrong-decimal, below-minimum, deviating, and tripped observations and the one-d-token floor case.
5. **Policy authority.** Exact-source tests prove deterministic deployment, one-time activation,
   bound configuration, authenticated containment, permissionless post-sunset disable, policy-owned
   queue accounting, and `propose_admin` -> permissionless sink `accept_admin` before proposal TTL
   expiry. Unknown queue state blocks activation and retirement.
6. **Shim custody.** Atomic deploy-init, minimum activation geometry, distinct authorities, frozen
   Comet control, zero ceremony holdings, complete share custody, authorization, balance, and rounding
   reconciliation all pass.
7. **Backstop and emissions profile.** Accepted ADR 0011 and exact-artifact tests prove its behavior,
   zero lifetime emissions to the pool, absent pool/reserve/user b-token emission state, and ordinary
   deposit, queue, dequeue, withdrawal, donation, and status liveness.
8. **Exposure.** The deployment record includes each reserve's asset-unit `supply_cap`, valuation
   price, and live readback. Tests prove the reserve cap and sealed pool-cap sum. Activation tooling
   recomputes equivalence groups and pool/global totals and rejects every operational breach or
   split. Accrued claims may exceed principal caps, consume reserve headroom, and remain monitored.
   The rounding-surplus tripwire is distinct: crossing it, including during exits or by revaluation,
   triggers containment and investigation but does not make explained unrepresented residue
   terminally inadmissible. Tests do not claim a lifetime rounding bound from one-transition checks.
9. **Release evidence.** Independent clean builds reproduce every Wasm checksum and size; generated
   interface and allowlisted-diff checks match; the reviewed Markdown record binds exact source,
   bytes, addresses, configuration, authorities, transactions, live readbacks, and approvals.
10. **Canary and operations.** Public-testnet and disposable-mainnet runs exercise ordinary lending,
    liquidation including final-fill health reversion and an alternate funded filler, direct
    supplier-loss accounting, cross-reserve debt with zero aggregate raw collateral, positive-rate
    and zero-rate rounding dust, exact denominator-preserving orphan transfer, later rate/price
    revival under pool ownership, terminal orphan settlement with b-token event plus zero-unit stock
    return/event, rejection on any emissions state, stale-auction deletion before default, isolated
    status 4, all-disabled fallback, authority/TTL/inclusion/queue failures, both backstop terminal
    classes, the rounding-loss terminal class, and route closure.
    A retirement fixture containing only 25.01 USD of exactly model-attributed physical rounding
    surplus and no represented economic obligations must complete terminal proof and sink handoff
    without a transfer, source relabelling, or new claim. One unexplained base unit, a withdrawable
    claim, or another unmet terminal predicate must still prevent completion.
11. **Staffing and maintenance.** Named primary/backup responders, redundant keepers, independent
    authorities, seven-day upstream review, funded migration, and indefinite-tail ownership exist
    before production. Missing or stale coverage closes routes and blocks new cohorts.
12. **Migration preparation.** From-genesis inventory and migration metadata reconcile before
    activation; ADR 0009 is accepted before registry cutover; no new ADR-0008 cohort launches after
    the first cutover starts.
13. **Disclosure.** Every product surface states the exact caps, synthetic non-economic backstop,
    no insurance, direct post-liquidation supplier-loss behavior, forced transfer of all
    zero-raw collateral from a defaulting user to an unreachable pool-owned claim, later abandonment
    of that claim's value, that the cash behind an unsettled orphan stays withdrawable by other
    suppliers under the ordinary utilization limit so abandoned value is first-come-first-served,
    that anyone may settle the orphan the instant reserve debt touches zero, that a defaulting
    user's own same-reserve supply is consumed before any supplier loss, possible unburnable b-token dust that converts to zero underlying asset
    units for users without defaulted liabilities, the rounding-loss terminal, fork scope,
    oracle/TTL dependencies, one-way freeze, migration-only recovery, and unbounded direct-user tail.
14. **Current review.** `adr --robot lint` and `git diff --check` pass. A fresh sequential Raladr
    Architect and distinct Critic approve the same candidate commit and ADR blob IDs. Only a human
    accepts it.

### Verification strategy

This ADR adds no contract implementation. The expanded implementation test plan is:

- **Unit/property:** each pool restriction, policy transition, asset-unit supply-cap derivation and
  enforcement, sealed pool/global cap calculation, cap headroom after interest, shim geometry, and
  direct residual-debt default arithmetic/events across sequences, including cumulative ceiling
  rounding; same-reserve setoff (zero, partial, full) with `debt_setoff` attribution; the declared
  no-denominator `b_rate` preservation as an explicit differential witness; `b_rate == 0`; positive `b_rate` with b-token-to-asset floor zero; strict-oracle
  multiplication floor zero; aggregate `collateral_raw` boundaries; exact position
  reclassification; invariant `b_supply`; rollback; cross-reserve liabilities and cycles; later
  rate/price accrual; emissions absence/rejection; terminal orphan settlement; b-token versus
  underlying event units; stock-zero return/event compatibility; and the rounding-loss terminal;
- **Integration:** exact Soroban authorization, storage/TTL, event, balance, allowance, rollback, and
  cross-contract behavior plus base-versus-fork differential tests for unchanged fund and
  liquidation paths, explicit classification of the intended direct-default and terminal-`gulp`
  divergences, borrowers with unrelated liabilities and only zero-raw collateral, a positive-rate
  one-b-token dust witness, a last supplier with another user's debt outstanding, cyclic positions,
  stale-auction deletion before default, final-fill health reversion with an alternate funded
  filler, transaction-delta attribution of each userless `defaulted_debt`, and exact
  `collateral_orphaned`/`orphan_settled`/stock-`gulp(0)` attribution;
- **End-to-end/testnet:** `make build && make test`, optimized Wasm size and interface comparison,
  independent clean-build checksum reproduction, public-testnet ceremony, and disposable-mainnet
  activation/use/containment/migration/terminal rehearsal against exact release bytes; and
- **Observability:** exercised alerts for route, code/admin/configuration, queue, caps, accounting,
  pool-emissions state, `debt_setoff`, `defaulted_debt`, `collateral_orphaned`, `orphan_settled`, liquidation fill latency/block age, near-100% utilization
  with an underwater position, oracle/TTL, upstream review, and tail state.

The implementation owns any small scripts needed to check allowlists, hashes, exposure arithmetic,
or reconciliation. Those scripts are reviewed and tested with the implementation; they are not part
of this ADR's consensus boundary.

### Risks

| Risk | Prevention / detection | Recovery |
| --- | --- | --- |
| Shim or backstop failure blocks normal operation | one pool per cohort; independent status 4; exact canary and monitoring | close route, freeze/disable, migrate; never repoint or reactivate |
| Pool fork changes unintended behavior | seven-change allowlist; differential tests; exact diff review | block release or retire affected cohort |
| Synthetic asset or LP escapes custody | atomic init; distinct authorities; auth-required/revocable assets; continuous reconciliation | revoke where possible, freeze cohort, archive foreign state, migrate |
| Operator freezes a healthy pool | authenticated narrow policy; transaction alerting; no downgrade | disclose and migrate; status 4 remains irreversible |
| Policy, TTL, inclusion, or queue state fails | redundant keepers; TTL alerts; policy-owned queue ledger; fail-closed routing | restore exact dependency and contain, or keep a serviced tail; do not invent T0 |
| Oracle fails or lies | reviewed proxy, floor/deviation/breaker checks, independent keepers | trip product brake, close routing, preserve safe exits, migrate |
| Direct supplier default or orphan accounting misstates loss, misclassifies the zero-raw boundary, or breaks the reserve denominator | focused stock-PositionData/position change; boundary, oracle-rounding, ordering, rollback, cycle, revival, event, terminal-`gulp`, and differential tests | close routing, freeze cohort, reconcile claims, migrate; no insurance claim |
| Late liquidation concentrates loss on remaining suppliers | funded keepers; fill-latency/block-age alerts; default-event reconciliation | close routing, accelerate exits and migration; no synthetic-backstop claim |
| Same-transaction default makes a designated filler unhealthy | exact post-fill preflight; second funded filler; canary | preserve the live auction after reversion and use a qualified filler; never bypass health checks |
| Supplier run plus default/valuation rounding leaves zero-raw collateral | near-100% utilization alert; exact sequence, conversion, oracle-rounding, orphan, and cross-reserve liveness tests | settle pool-owned orphan after debt reaches zero, archive zero-asset-unit claims and abandoned surplus, complete sink handoff, migrate; no compensation |
| Upstream fix is missed | named fork owner and seven-day reviewed commit/advisory checkpoint | close routes; issue reviewed future artifact; migrate immutable cohorts |
| Independent holder extends the tail | honest inventory, direct support, no completion-date promise | maintain repay/withdraw/liquidation and monitoring until objective terminal state |
| Bridge costs exceed commercial value | pre-implementation comparison, hard abandonment date, fixed caps | abandon v1 and ship the permanent profile |

### Pre-mortem

1. **The focused fork changes an ordinary fund path.** Early warning: base/fork differential state,
   event, balance, or authorization mismatch outside the seven restrictions. Mitigation: allowlisted
   diff and full unchanged-path differential suite. Recovery: block release; if already deployed,
   close routes, freeze the isolated pool, and migrate.
2. **The policy cannot establish a durable intake brake.** Early warning: TTL risk, rejected status
   4, unresolved queue phase, or live reserve mismatch. Mitigation: redundant keepers, status-4 and
   fallback drills, and fail-closed policy state. Recovery: close product routes, trip the oracle
   brake, restore exact authority/TTL/inclusion, or maintain a serviced tail without claiming T0.
3. **A supplier run precedes full default.** Early warning: utilization approaches 100% while an
   underwater position or liquidation remains. Mitigation: route closure, keeper fill-latency
   response, exact sequence/conversion/oracle-rounding tests, and cross-reserve/cycle tests that
   preserve `b_supply` while debt remains. Recovery: recognize `defaulted_debt`, move all
   zero-raw collateral of the defaulting user to the pool-owned orphan, settle that orphan only
   after reserve debt is zero, archive other zero-asset-unit b-token dust and abandoned surplus,
   complete objective retirement and sink handoff, and migrate every withdrawable claim.
4. **The temporary bridge outlives its staffing and commercial case.** Early warning: missed
   upstream checkpoint, abandonment date, responder coverage, or migration funding. Mitigation:
   fixed caps, 90-day sunset, named owners, and pre-implementation comparison. Recovery: launch no
   new cohort, disable intake, and move all effort to the registry-native successor and tail.

### Invariants

- One ADR-0008 pool equals one isolated factory/Comet/backstop/assets/policy/custodian fault domain.
- Stock `supply_cap` bounds each reserve to 25,000 USD and the sealed reserve sum bounds each pool
  to 50,000 USD at approved greater-of-live/stress prices; operations bound global capacity to
  100,000 USD.
- Status 4 is absorbing and independent of the backstop.
- Economic configuration cannot grow after activation; `max_util` remains below 100%, and only
  exact reserve disablement remains.
- Residual debt after liquidation is first set off against the defaulting user's own same-reserve
  noncollateral supply (`debt_setoff`); only the uncovered remainder defaults directly to the
  affected reserve's suppliers through the exact stock ceil-rounded `b_rate` update whenever
  `b_supply > 0` (preserving `b_rate` in the declared no-denominator case), emits `defaulted_debt`,
  and never enters the backstop.
  Collateral is ignored for eligibility only when strict-oracle stock `PositionData` has aggregate
  `collateral_raw == 0` and every residual collateral b-token has no configured or accrued emissions.
  All such collateral moves atomically to a pool-owned noncollateral position while reserve
  `b_supply` remains unchanged. This prevents borrower revival without breaking the denominator
  while debt remains or creating an unclaimable emissions claim.
- Permissionless `gulp(asset)` never recognizes surplus. After proving emissions absence and
  `d_supply == 0`, it removes only the pool-owned orphan, emits its b-token amount in
  `orphan_settled`, returns zero, preserves stock `gulp(asset, 0)`, and moves no tokens or backstop
  credit.
- Pool b-token emissions configuration/data remains absent for the cohort's lifetime: the pool traps
  `set_emissions_config`, and ADR 0011 permits no positive pool emissions.
- After all debt and pool-owned orphans are gone, a user b-token balance is terminal only when its
  live conversion yields zero underlying asset units. Such zero-asset-unit supply or collateral is
  archived as a rounding-loss record; every withdrawable unit must exit.
- The 25-USD rounding tripwire governs operation, not terminal admissibility. Explained
  unrepresented residue is permanently abandoned in place without changing its source; only full
  terminal proof and sink ownership end estate service, not the breach or its investigation alone.
- Synthetic activation value is never represented as economic backstop value.
- Official evidence is readable and tied to standard hashes, reviewed Git commits, transactions, and
  live readbacks; automation cannot declare success against contradictory live state.
- A retired or integrity-failed cohort is never repaired and resumed. Its only supported direction is
  drain, sink handoff, and registry-native replacement.

### Follow-ups

- Human-review and accept ADR 0011's exact backstop profile with this ADR and ADR 0009.
- Implement and independently review the pool fork, ADR 0011 backstop fork, policy, helper, and sink.
- Complete the coverage assessment and direct-route or vault/adapter review.
- Accept and implement ADR 0009 before registry cutover.
- Amend the selected permanent-profile ADRs so ADR-0008 addresses remain out-of-registry legacy.
