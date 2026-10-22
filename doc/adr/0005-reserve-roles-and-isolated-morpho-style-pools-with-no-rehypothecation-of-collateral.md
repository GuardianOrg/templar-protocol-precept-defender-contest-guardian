# 5. Reserve roles and isolated (Morpho-style) pools with no rehypothecation of collateral

Date: 2026-08-27

## Status

Proposed

Amends [2. Pool registry as deployment, configuration, and optional-backstop routing layer](0002-pool-registry-as-deployment-configuration-and-optional-backstop-routing-layer.md)

Amends [3. Protocol fee on borrow interest; defer origination and flash-loan fees](0003-protocol-fee-on-borrow-interest-with-deferred-origination-and-flash-loan-fees.md)

Amends [4. Permissioned pools via pool-local allow-list, permission manager, and optional hook](0004-permissioned-pools-via-pool-local-allow-list-permission-manager-and-optional-hook.md)

## Context

Templar needs a market shape with one collateral asset and one different loan asset. The
collateral must not be available for ordinary borrowing or flash lending from the pool. This ADR
uses **Morpho-style** only for that directed two-asset topology: Morpho Blue identifies a market
with one `loanToken` and one `collateralToken`
([Morpho Blue `IMorpho.sol`, commit `4418b82`](https://github.com/morpho-org/morpho-blue/blob/4418b82363e4a54a57a301bae057d83592a50a2f/src/interfaces/IMorpho.sol#L6-L12)).
It does not adopt Morpho Blue's singleton, immutability, governance, accounting, callback, or
flash-loan design. Blend remains a pooled Soroban protocol governed by ADRs 0002-0004.

The vendored baseline is Blend v2 at `vendor/blend-contracts-v2` commit `6cf6dd1`. It has the
factor vocabulary for reserve roles but does not enforce the required market shape:

- `ReserveConfig` publicly exposes `c_factor` and `l_factor`
  (`pool/src/storage.rs:42-59`). Existing validation bounds each factor by `SCALAR_7` but permits
  zero for either or both (`pool/src/pool/config.rs:169-184`; `pool/src/constants.rs:4-7`).
- Health calculation multiplies collateral value by `c_factor` and divides liability value by
  `l_factor` (`pool/src/pool/health_factor.rs:27-66`;
  `pool/src/pool/reserve.rs:186-204`). A zero collateral factor therefore contributes no health,
  while a debt position on a zero liability factor reaches division by zero rather than a
  deliberate admission error.
- Reserve admission currently checks only `enabled` for `Supply`, `SupplyCollateral`, and
  `Borrow` (`pool/src/pool/reserve.rs:142-156`). Those three ordinary actions invoke that check;
  withdrawal, collateral withdrawal, and repayment deliberately do not
  (`pool/src/pool/actions.rs:288-305,312-331,339-355,363-383,391-440`).
- The flash path creates liabilities and emits its event before transferring principal and calling
  the receiver; its existing enabled-only reserve check occurs after liability creation
  (`pool/src/pool/submit.rs:68-136`). Baseline Blend has no factor check anywhere on flash-borrow
  admission, so a zero-liability-factor asset can be flash-borrowed and repaid in one transaction
  while leaving no debt in committed state. The current ordering motivates this ADR's fail-before-
  effects rule; it is not the reason the baseline permits the asset.
- Ordinary submit first builds and validates the complete batch, then moves tokens. Its allowance
  path nets inputs and outputs by asset; the other path transfers every input before every output
  (`pool/src/pool/submit.rs:25-65,198-245`). A borrow/repay batch can therefore end with no debt or
  net transfer even though admitting the collateral asset as a loan asset violates the intended
  role.
- `SupplyCollateral` currently accepts a zero collateral factor, creating a position with no
  health value that can nevertheless enter a liquidation lot
  (`pool/src/pool/health_factor.rs:44-54`;
  `pool/src/auctions/user_liquidation_auction.rs:90-159`). `Supply` likewise accepts a zero
  liability factor although that reserve may not safely support debt.
- Regular borrow and flash borrow are the production entry paths that mint debt
  (`pool/src/pool/actions.rs:391-407`; `pool/src/pool/submit.rs:68-121`). Liquidation and bad-debt
  paths can transfer existing liabilities between users or reduce aggregate debt
  (`pool/src/auctions/user_liquidation_auction.rs:206-220`;
  `pool/src/pool/user.rs:204-249`; `pool/src/pool/bad_debt.rs:53-75,100-124`). They must remain
  usable for liquidation, repayment, and loss recovery rather than being mistaken for new loans.
- Debt is not a complete inventory of underlying-token egress. Withdrawals, collateral
  withdrawals, repayment refunds, backstop-interest auction fills, and ADR 0003 protocol-fee
  collection move accounted value without creating loan principal
  (`pool/src/pool/actions.rs:307-331,358-383,415-440`;
  `pool/src/pool/submit.rs:198-245`;
  `pool/src/auctions/backstop_interest_auction.rs:108-118`). No-rehypothecation must distinguish
  those flows from ordinary and flash loan principal.
- Reserve configuration is queued by the admin and later executed permissionlessly. Full metadata
  validation currently runs only at queue time. Execution re-runs only the transition predicate for
  an existing reserve and runs no full validation for a new reserve
  (`pool/src/pool/config.rs:66-91,98-166`). That transition predicate preserves decimals and forbids
  `l_factor > 0 -> 0`, but changing both factors can still switch a reserve's economic role
  (`pool/src/pool/config.rs:186-196`). Authoritative full execution-time validation is therefore a
  net-new requirement below, not a baseline property.
- The reserve list is append-only and capped at 30
  (`pool/src/storage.rs:458-496`; `pool/src/constants.rs:15-16`). A listed asset cannot be replaced
  by removing its slot.
- A pool starts in status 6 (setup). Permissionless `update_status` cannot leave setup; admin
  `set_status` is the release path (`pool/src/pool/config.rs:16-40`;
  `pool/src/pool/status.rs:11-117`; `pool/src/contract.rs:496-511`). That admin path emits the
  two-topic `(Symbol("set_status"), admin)` event through `set_status_admin`, whereas
  permissionless `update_status` emits the one-topic variant (`pool/src/events.rs:82-104`). ADR 0002
  preserves the lifecycle for pools with and without a backstop.
- `get_reserve(asset)` exposes the factors and reserve data, and `get_reserve_list()` exposes every
  listed asset (`pool/src/contract.rs:431-446`). Clients can therefore derive reserve roles without
  a second public role representation.

ADRs 0002-0004 are predecessors, not optional context:

- ADR 0002 supplies authenticated registry deployment, optional-backstop routing, timelocked
  maintenance upgrades, compatibility checks, recovery, and the one-way upgrade seal.
- ADR 0003 supplies the interest-only fee ABI and exact fungible supplier ownership through pooled
  value `V = P0 + Ks` shared pro rata across `N` b-tokens, while keeping current and deferred
  backstop credit, cash-backed unowned surplus, ownerless accrual, and protocol credit as distinct
  accounting buckets. This ADR does not redefine that accounting.
- ADR 0004 supplies the pool-local permission manager, automatic epoch invalidation on every
  manager transition, all-local-permits-before-hook admission order, the optional hook, the self-
  directed reserve-exit exception, root-first `HookAdmission` finality, and their final public ABI.

Constraints and assumptions:

- `vendor/` remains read-only here. Contract changes are separate upstream PRs to
  `Templar-Protocol/blend-contracts-v2`.
- This is one greenfield cumulative public/storage release with ADRs 0002-0004. No non-disposable
  pool may use an interim layout. Discovery of one blocks release and requires an explicit
  compatibility and migration ADR or a new-address pool.
- Every shipping optimized Wasm must remain at or below
  `min(131072, active_network.contractMaxSizeBytes)` on its deployment network. Measured
  before/after sizes are implementation and release evidence, not Raladr approval evidence.
- During the roughly one-year stabilization period, the no-rehypothecation guarantee is enforced
  by the attested current pool Wasm and ADR 0002's governed upgrade process. It is not a claim that
  the code is already immutable.
- This ADR decides observable protocol semantics, public ABI and events, authority, invariants,
  compatibility and migration boundaries, and operational release gates. It does **not** choose
  storage keys or layouts, registry metadata, internal role types, helpers or module placement,
  validation data structures, TTL mechanics, proof-harness names, runner phases, replay
  architecture, or evidence-directory layout. Equivalent implementations are valid when they
  satisfy the public contract, atomicity, invariants, and acceptance criteria below.

In scope:

- reserve roles derived from the public factors and their action semantics;
- one collateral-only reserve and one lend-only reserve per isolated pool;
- immutable isolated-pool identity and immutable listed-reserve roles;
- cumulative public construction and discovery interfaces;
- interaction with ADR 0003 accounting and ADR 0004 permissioning;
- static committed-exposure disclosure from the existing lend-reserve `supply_cap` and
  `max_util`, with the distinct transient flash boundary stated explicitly;
- release, upgrade, monitoring, and recovery gates for the new guarantees.

Out of scope:

- per-position submarkets within one pool;
- custody segregation in a separate vault;
- immutable factor magnitudes, interest curves, or oracle configuration;
- forbidding liquidation, repayment, or a user's own withdrawal;
- a live-pool migration for role-incompatible positions;
- equivalence with Morpho Blue beyond the stated directed two-asset topology;
- a dynamic oracle-priced backstop-capacity formula or a new debt-cap subsystem;
- registry implementation structure or an on-chain canary-certification subsystem.

Principles:

1. Public factors remain the single observable source of reserve role; no second role value may
   disagree with them.
2. No-rehypothecation needs both a committed-state invariant and a transaction-flow invariant.
   An end state with zero debt is not proof that no principal left and returned.
3. An isolated pool has exactly one directed collateral/loan pair whose asset roles never change
   after listing.
4. Permission managers, hooks, registries, and operator labels may restrict behavior but may not
   replace pool enforcement of the economic invariant.
5. Principal exit and debt reduction remain available during denial, misconfiguration, and
   incident response; maintenance upgrades remain evidence-bound until they are sealed.

Decision drivers, ranked:

1. **Exact exposure boundary.** An isolated pool must never lend or flash-lend its collateral.
2. **Small public and maintenance surface.** Reuse public factors, views, errors, and events unless
   a new observable is necessary.
3. **Safe cumulative lifecycle.** Queue races, setup release, ADR 0002 upgrades, ADR 0003
   accounting, and ADR 0004 admission policy must not create a bypass or false certification.

## Decision

### Reserve roles and action semantics

Every reserve has one role derived from its public factors:

| Factor predicate | Role | Entry actions allowed |
| --- | --- | --- |
| `c_factor > 0 && l_factor > 0` | `Both` | `Supply`, `SupplyCollateral`, and `Borrow` |
| `c_factor > 0 && l_factor == 0` | `CollateralOnly` | `SupplyCollateral` only |
| `c_factor == 0 && l_factor > 0` | `LendOnly` | `Supply` and `Borrow` |
| `c_factor == 0 && l_factor == 0` | invalid | none |

`LendOnly` names the supplier-facing use of the reserve; it is the isolated pool's borrowable loan
asset, not an asset restricted to lenders.

The invalid `0/0` configuration is rejected with `PoolError::InvalidReserveMetadata` in every
pool. The same existing code is the exact public failure for every role/topology configuration
violation introduced here: isolated `Both`, a duplicate isolated role, a third isolated reserve,
and any listed isolated-reserve role transition. At queue time, validation precedes creation of the
queued entry. At permissionless execution, the existing missing-entry and unlock-time checks retain
their order; full metadata, decimals, role, and then-current topology validation then completes
before deletion of the queued entry or mutation of reserve configuration, data, or the reserve
list. Failure therefore preserves the executable/cancellable queued proposal without relying on
transaction rollback to undo an early delete. The action matrix applies to open and isolated pools:

| Request | Required factor | Rejection when absent |
| --- | --- | --- |
| `Supply` | `l_factor > 0` | `PoolError::ReserveDisabled` |
| `SupplyCollateral` | `c_factor > 0` | `PoolError::ReserveDisabled` |
| regular or flash `Borrow` | `l_factor > 0` | `PoolError::ReserveDisabled` |
| `Withdraw` | no new role requirement | existing checks |
| `WithdrawCollateral` | no new role requirement | existing checks |
| `Repay` | no new role requirement | existing checks |

No new error code, public `ReserveRole` type, role getter, or role event is added. Clients derive
the table from `get_reserve(asset)`. Internal representation and the placement of validation are
implementation-owned and must not create a conflicting observable role.

For every transaction, however, the observable ordering is normative: a factor-invalid regular
or flash borrow fails atomically before debt creation, a committed `borrow` or `flash_loan` event,
loan-principal transfer, or flash-receiver callback. No request later in the batch, no net-zero
transfer result, and no ADR 0004 manager or hook decision can authorize a factor-invalid action.

The committed-state no-rehypothecation guarantee is:

~~~text
l_factor == 0  =>  d_supply == 0
~~~

The distinct transaction-flow guarantee is:

~~~text
l_factor == 0
  => no successful regular-borrow or flash-borrow admission
  => no loan-principal egress, committed borrow/flash event, or flash-receiver callback
~~~

The state predicate proves that the reserve has no outstanding debt-token claim. The flow
predicate rules out a fully repaid same-transaction loan that would be invisible in committed debt.
Together they are this ADR's bounded no-rehypothecation claim.

They do not forbid owner-authorized withdrawal, collateral withdrawal, liquidation transfer,
repayment refund, accounted backstop-interest transfer, ADR 0003 protocol-fee collection, token
contract behavior, or a governed code replacement before sealing. Liability transfers and default
may move or reduce existing debt but must not create net debt on a collateral-only reserve. Every
new production debt-creation or underlying-egress path must be classified against those
distinctions before release.

### Isolated-pool identity and directed pair

The pool adds one immutable construction value and public view:

~~~rust
fn is_isolated(e: Env) -> bool
~~~

There is no post-construction setter. The view returns the constructed value. A pool cohort that
predates this value or cannot serve the view is incompatible and fails closed; absence must never
be interpreted as `false`. Such a cohort needs a separately reviewed versioned migration based on
authoritative deployment evidence or a new-address pool. Internal storage and compatibility
metadata used to satisfy this requirement are implementation-owned.

Relative to ADR 0004, the cumulative greenfield construction ABI appends `isolated: bool` last and
changes no earlier argument:

~~~text
registry.deploy(
  admin, name, salt, oracle, backstop_take_rate,
  max_positions, min_collateral, backstop, interest_fee,
  permission_manager, hook, isolated
) -> Address

pool.__constructor(
  admin, name, oracle, bstop_rate, max_positions,
  min_collateral, interest_fee, registry, backstop, blnd_id,
  permission_manager, hook, isolated
)
~~~

Origination-fee and flash-loan-fee arguments remain absent. A successful deployment is atomic: the
returned pool's `is_isolated()` value equals the requested value, or no pool membership or
deployment event commits. How the registry establishes that postcondition is not decided here.

ADR 0004's `PoolDeployedEvent` remains unchanged. Indexers discover the pool address from that
event and read immutable isolated identity from the pool. This ADR adds no duplicate deployment
field or separate registry query for reserve roles, isolated identity, or canary state.

For `is_isolated() == true`:

1. Every newly listed reserve is `CollateralOnly` or `LendOnly`; `Both` is rejected with
   `InvalidReserveMetadata`.
2. The complete append-only reserve list, including disabled reserves, contains at most one asset
   of each role and therefore at most two assets. A duplicate role or third reserve returns
   `InvalidReserveMetadata`.
3. Once an asset is listed, later configuration may change factor magnitudes but may not cross a
   role boundary. An atomic two-factor swap is still a role change and returns
   `InvalidReserveMetadata`.
4. Queueing and execution both run full reserve-metadata, immutable-decimals, role-transition, and
   isolated-topology validation against the state they observe. The full execution-time validation
   is net-new and authoritative against then-current committed state for both new and existing
   reserves. If two same-role additions are queued while neither exists, at most one may commit; the
   other returns `InvalidReserveMetadata` before its queued entry is deleted, so it remains
   cancellable under the existing lifecycle.
5. An isolated pool may leave setup only when its committed state contains exactly one enabled
   reserve of each role and `get_permission_manager() == Some(_)`. Queued changes do not count.
   Failure returns `StatusNotAllowed` and changes neither status nor admission state.
6. After setup, either reserve may be disabled for incident response, but neither asset nor role is
   replaceable. Correcting a mistaken pair requires a new pool.

For `is_isolated() == false`, multi-reserve and `Both` configurations remain available. This ADR
uses **open pool** only for that structural identity; it uses **fully permissionless** for an ADR
0004 admission tuple of `(None, None)`. Open pools are not behaviorally untouched: the global
`0/0` rejection and factor-aware action matrix intentionally reject entries the baseline allowed.

### Static exposure envelope

The existing `LendOnly` reserve settings provide one simple release-time committed-exposure
envelope. Its domain is:

~~~text
P     = SCALAR_7, P > 0
S     = supply_cap, S >= 0
m     = max_util, 0 < m < P
E_cfg = floor(S * m / P)
~~~

Conditional on end-of-batch supplier value `V <= S`, the final `max_util` check bounds committed
lend-asset liability as `L <= E_cfg`. A negative `S` is outside the formula domain and blocks
isolated-pool release or approval of that configuration; this ADR does not otherwise add a new
reserve-metadata field or error. The baseline may accept `m == P`, but that saturated-utilization
configuration is ineligible for this finite envelope and cannot pass isolated-pool release or later
routing approval under this ADR. The restriction governs use of `E_cfg`; it does not narrow the
underlying reserve-metadata ABI.

`E_cfg` is evaluated as exact mathematical floor division using checked wider arithmetic before
narrowing to `i128`. Intermediate overflow, failed narrowing, or any unrepresentable result blocks
release/routing approval; the calculation never wraps or saturates. This matters even when the final
quotient is representable because `S * m` can exceed `i128::MAX`.

`E_cfg` does **not** bound per-action temporary debt or transient flash principal. The retained
flash path requires temporary utilization below 100% before principal transfer and callback, then
applies `max_util` to the resulting end state. A fully repaid flash may therefore transfer principal
greater than `E_cfg` while committing `L == 0`. This ADR preserves that atomic boundary and adds no
separate flash-principal cap. A deployment that requires one uniform committed-and-transient limit
needs a separate protocol decision.

`E_cfg` is also not a perpetual cap on aggregate `L`: interest can increase both supplier value and
liabilities after the last supply action, and default can change coverage. Release evidence therefore
records `S`, `m`, `E_cfg`, current `V` and `L`, the stress horizon and interest assumptions, and the
resulting stressed committed liability alongside the route's capacity assessment. It separately
discloses the temporary `< 100%` flash-utilization boundary and that transient atomic principal is
excluded from the backstop-capacity comparison. No UI, registry record, or release report may
present `E_cfg` alone as a guaranteed worst-case loss or flash-principal cap.

Before setup exit and before Templar authorizes any later `S`, `m`, or `E_cfg` increase, the registry
governance release policy assigns the isolated pool a static risk tier and checks the complete pair,
not either field in isolation. For `Some(backstop)`, evidence states the backstop-capacity method in
lend-asset units, the observed ledger inputs, the required stress buffer, and whether declared
capacity covers the stressed exposure. If it does not, release is blocked or the product is labeled
partially insured rather than covered. For `None`, the same stressed exposure is published as
explicitly uninsured. A dynamic oracle-based coverage formula is deferred until operational
evidence justifies its additional trust and maintenance surface.

This is a release/routing policy over existing reserve controls, not a new on-chain debt-cap,
flash-principal cap, or backstop-price subsystem. The existing `supply_cap` check gates new supply;
the temporary `< 100%` check gates flash principal before callback; and `max_util` gates committed
end-of-batch liability. Tightening the policy or exceeding its committed bound must never block
repayment, liquidation, bad-debt settlement, donation, protocol-fee collection, or a user's
withdrawal. A pool-admin configuration that bypasses the recorded tier is a page-worthy policy
violation: Templar stops new routing and risk-increasing admission until the pair is restored or
freshly approved, while risk-reducing paths remain live.

### Authority and release boundary

Existing authorities remain distinct:

- ADR 0002 registry authority authenticates construction and maintenance upgrades.
- The pool admin owns reserve configuration, status, manager, and hook transitions under the
  existing public interfaces.
- An ADR 0004 manager grants admission; a hook may reject a call. Neither can authorize an action
  forbidden by reserve role.
- Off-chain release operators decide whether the required canary evidence is complete and whether
  Templar-operated routing may begin. No contract state introduced here certifies that decision.

The mandatory isolated-pool release sequence is:

1. While status is 6, configure the exact enabled pair and install the intended release manager.
   Grant only the permissions needed for the named canary principals. If the declared final policy
   retains a hook, attach and review that exact hook before activation.
2. Leave setup through admin `set_status`. The pool re-evaluates the exact-pair and manager-presence
   preconditions in that transaction. While the manager remains attached, adversarial unpermitted
   supply, borrow, and flash attempts return `NotPermitted` under ADR 0004 before hook invocation or
   authorization, role validation, accounting, or callback. A 61st request remains ADR 0002
   `BadRequest` before either admission seam or role validation.
3. Against the exact optimized pool Wasm, configured assets, and configured oracle, record both
   price reads; supply lend liquidity and collateral; borrow the lend asset; repay; and remove the
   canary positions to self. For each user-action role-error canary, first install a current complete
   local permit; when a hook is retained, use its independently verified allowing path. The call
   must then reach role validation and return `ReserveDisabled`. Separately prove that a missing,
   stale, expired, or partial permit in combined mode returns `NotPermitted` without invoking or
   authorizing the hook. Configuration/topology canaries use the queue and execution paths and
   return `InvalidReserveMetadata` independently of participant admission. A permission or hook
   denial therefore cannot masquerade as role enforcement.
4. On failure, retain or restore the manager, freeze at status 4, preserve transaction evidence,
   and use the ADR 0004 self-exit path for cleanup. The pool is not routing-eligible.
5. On success, reconcile the declared final admission policy against the complete
   `(get_permission_manager(), get_hook())` tuple:
   - manager-only and manager-plus-hook policies retain or rotate the manager and remove or
     invalidate canary-only grants;
   - hook-only policy retains the canaried hook and calls `set_permission_manager(None)`;
   - fully permissionless policy detaches any launch-only hook while the manager still gates entry,
     then calls `set_permission_manager(None)` as the final admission mutation.
   Every actual manager transition automatically advances the epoch under ADR 0004; no
   invalidation boolean exists. The returned epoch, manager view, hook view, and stale-permit
   behavior must match the declared policy before off-chain routing begins.

Active status, registry membership, `is_isolated()`, permission-manager state, and hook state do
not individually **or collectively** certify canary completion or routing eligibility. Canary
completion is off-chain, transaction-bound release evidence tied to the network, pool address,
exact Wasm hash, assets, oracle, admission sequence, and resulting state. No `canary_passed` or
`routing_eligible` on-chain flag is added by this ADR. Once the pool leaves setup it is callable:
the installed manager can restrict participants, but it cannot make canary completion an on-chain
precondition, and an admitted third party may interact before Templar's routing decision. This is an
explicit product/security acceptance for v1. Off-chain completion gates only Templar-operated
routing; a requirement to prevent every premature direct interaction selects Alternative G or a
separately reviewed coordinator.

### Upgrade, recovery, and sealing

Interface compatibility is necessary but does not prove the economic semantics above. Before an
ADR 0002 maintenance upgrade or current-pool-implementation rotation can serve an isolated pool,
the candidate's exact optimized Wasm hash must carry fresh state, flow, accounting, size,
authorization, canary, and monitoring evidence for this ADR. Missing or mismatched evidence blocks
execution or deployment even when interface versions are compatible.

Until ADR 0002's one-way upgrade seal executes, the no-rehypothecation guarantee is enforced by the
attested current Wasm and governed upgrade process. After sealing, deployed code becomes immutable
under that governance path. Sealing does not make token contracts, oracle behavior, external
copies, or off-chain routing immutable, and it does not retroactively certify an untested Wasm.

The final isolated-pool code ceremony uses ADR 0002's exact root-first partial order. Governance
first completes `Seal(RegistryCode)`, then verifies `is_sealed(RegistryCode) == true`, the committed
scoped-seal action and terminal events, the registry's executable hash, and rejection of
`UpgradeRegistry`. Only after that completed root transition does governance make and record ADR
0004's explicit `HookAdmission` disposition:

- if hook enable/re-enable authority must remain available during maintenance, record
  `is_sealed(HookAdmission) == false`, the continuing authority, and the fact that a later enabled
  hook may still be attached by the pool admin; or
- if the reviewed hook set is final, propose and execute `Seal(HookAdmission)`, verify the live flag,
  exact scoped-seal action and terminal events, rejection of first enable/re-enable including an
  older proposal at execution, and the continuing ability to disable or attach an already-enabled
  hook.

Only after one of those dispositions is evidenced may governance propose and execute
`Seal(PoolCode(pool))`. It then verifies `is_sealed(PoolCode(pool)) == true`, the pool's unchanged
executable hash, the pool-scoped action and terminal events, and rejection of `UpgradePool` for that
address. The root seal alone fixes registry code but does **not** freeze pool code. The pool-specific
seal fixes pool code but, when `HookAdmission` remains open, does **not** freeze the set of hooks that
registry governance may enable or the active hook that the pool admin may attach. Product and
security claims state the recorded disposition; calling pool code or admission policy immutable
before the applicable terminal state is a release failure.

Recovery is intentionally asymmetric:

- Before deployment or setup release, reject the candidate or deploy a corrected pool.
- After release, freeze a misconfigured pool at status 4, preserve repay and withdrawal, deploy the
  correct pair, and coordinate voluntary migration. Do not mutate a listed role.
- If monitoring observes collateral `d_supply != 0` or a committed collateral-asset borrow or
  flash-loan event, freeze every affected Templar pool, stop Templar-operated routing,
  deployments, and upgrades using that Wasm hash, cancel pending uses where possible, and preserve
  trace and ledger evidence. Do not raise `l_factor` to hide the violation.
- During the unsealed maintenance phase, a compatible repair uses ADR 0002's timelocked upgrade
  only after the replacement satisfies this ADR. After sealing, recovery uses a new address.
- Registry rotation can stop future registry-mediated use of a bad implementation. It cannot
  revoke independently deployed copies or undo transactions that already committed.

### Events and observability

No role event is added. Existing `queue_set_reserve` and `set_reserve` events record configuration.
Lifecycle monitoring decodes both event shapes whose first topic is `Symbol("set_status")`: the
one-topic permissionless variant and, critically for setup exit, the two-topic admin variant emitted
through `set_status_admin` (`pool/src/events.rs:42-104`). `get_reserve_list`, `get_reserve`, and
`is_isolated` expose current topology. Existing `borrow` and `flash_loan` events remain part of the
flow invariant (`pool/src/events.rs:253-304`). ADR 0004 manager, epoch, permit, and hook events retain
their exact shapes. This ADR adds no event.

Release and live monitoring for each isolated pool must be able to establish, without prescribing
an artifact layout:

- network, pool address, exact pool Wasm hash, deployment transaction, root/pool code-seal state,
  and the explicit open-or-sealed `HookAdmission` disposition;
- immutable isolated identity, current status, and exactly two reserve addresses;
- one enabled `CollateralOnly` and one enabled `LendOnly` reserve at setup exit;
- each reserve's factors, enabled state, debt supply, `supply_cap`, and `max_util`;
- envelope-domain eligibility, exact checked/wider `E_cfg` calculation, current and stressed
  committed lend-asset liability, the distinct temporary `< 100%` flash-utilization boundary,
  declared risk tier, route, capacity method and comparison, or the explicit uninsured disclosure
  for `None`;
- collateral `d_supply == 0` and no committed collateral-asset borrow or flash-loan event;
- the declared and observed manager/hook tuple, epoch changes, canary grants, and cleanup
  transactions;
- oracle reads and successful lend-supply, collateral-supply, lend-borrow, repay, and exit
  transactions against the exact deployed artifact; and
- a routing-eligibility decision timestamp or ledger that follows all required transactions and
  is explicitly identified as off-chain evidence, not inferred contract state.

A role/count mismatch, collateral debt, collateral borrow/flash event, unexpected code hash,
unapproved exposure-pair increase, an ineligible or failed `E_cfg` calculation, capacity/disclosure
drift, a claim that `E_cfg` caps transient flash principal, premature admission cleanup, out-of-
order canary, or final policy mismatch is page-worthy. The operator freezes at status 4 when
necessary, stops affected Templar routing and risk-increasing admission, and reconciles the trace
and current ledger state without blocking repayment, settlement, or exit. Monitoring and user
interfaces must never label registry membership, active status, isolated identity, an admission
tuple, or the root registry seal alone as certification of canary completion, coverage, or pool-
code immutability.

## Alternatives

Options are scored 1 (poor) through 5 (strong) against the ranked drivers.

| Option | Exact exposure boundary | Small public/maintenance surface | Cumulative lifecycle safety | Result |
| --- | ---: | ---: | ---: | --- |
| A. Pool-enforced derived roles plus immutable isolated identity | 5 | 4 | 4 | Chosen |
| B. Factors plus operator convention only | 2 | 5 | 1 | Rejected |
| C. Stored public role enum in addition to factors | 4 | 2 | 3 | Rejected |
| D. Several collateral-only and lend-only reserves in one isolated pool | 2 | 4 | 4 | Rejected |
| E. Separate collateral vault or purpose-built market contract | 5 | 1 | 2 | Rejected |
| F. Registry enforcement instead of pool enforcement | 2 | 2 | 2 | Rejected |
| G. On-chain canary-passed or routing-eligibility state | 4 | 2 | 3 | Rejected |

**A. Pool-enforced derived roles plus immutable isolated identity (chosen).** This makes factors
the single role source, closes ordinary and flash loan admission, distinguishes state from flow,
and gives clients one immutable topology view. It adds one public scalar and one deployment
argument, stricter factor-zero behavior, setup-release conditions, and evidence obligations.

**B. Factors plus operator convention only.** This has the smallest code and interface delta and
can deploy the intended pair. It cannot prevent flash lending, net-zero borrow batches, later role
changes, duplicate roles, or setup release with an incomplete pair. Monitoring would detect a
breach only after the promise had failed.

**C. Stored public role enum.** A named wire type improves discoverability and can make transitions
explicit. It duplicates the public factors, requires synchronization and migration semantics, and
still needs action and topology enforcement. Existing reserve views make that additional public
state unnecessary.

**D. Multi-asset role-separated pool.** It can prevent each collateral asset from being borrowed,
but every collateral asset supports every loan asset through one health account. That is a useful
cross-collateral product, not the requested directed pair, and deserves a different name and ADR.

**E. Separate vault or market contract.** Physical custody separation gives the strongest
architectural boundary. It duplicates or reimplements pool interest, liquidation, oracle,
backstop, fee, permission, upgrade, and accounting behavior. That audit and maintenance surface is
not justified for the requested fungible two-asset market.

**F. Registry enforcement instead of pool enforcement.** Central deployment policy can restrict
which configurations Templar creates, but it cannot govern later pool-local reserve changes or
direct calls and should not become a borrow-path dependency. Registry checks may supplement
deployment; they cannot own no-rehypothecation.

**G. On-chain canary or routing state.** An authenticated flag can prevent operator routing before
someone asserts completion. Unless an on-chain coordinator executes and verifies the entire
oracle, borrow, repayment, cleanup, and final-policy sequence, the flag remains an assertion rather
than proof. It adds authority, public state, events, and migration rules without replacing the
transaction-bound evidence.

The chosen option follows the drivers: A is the smallest public extension that makes the exposure
boundary a pool invariant, preserves one role source of truth, and composes with the predecessor
ADRs without making the registry or a hook the enforcement authority.

## Consequences

What becomes easier:

- Users, integrators, and auditors can identify one directed two-asset market through a stable
  scalar view and existing reserve data.
- No-rehypothecation is stated as both zero collateral debt and zero collateral-loan flow, so a
  fully repaid flash loan cannot hide behind end-state accounting.
- Invalid factor-zero entries fail with existing typed pool errors rather than a later arithmetic
  failure or a worthless position.
- Release review has a bounded topology and a transaction-bound canary that exercises the exact
  oracle, artifact, and admission policy.
- Reserve roles add no second wire type, registry copy, or event stream.

What becomes harder:

- A mistaken isolated asset or role is intentionally not repaired in place; recovery is a new
  pool and voluntary migration.
- Collateral-only assets cannot provide ordinary or flash liquidity, and lend-only assets cannot
  be supplied as collateral.
- Open pools with factor-zero reserves receive stricter entry behavior and need an inventory before
  any live migration.
- Even a final manager-free isolated pool must pass a manager-interlocked, event-ordered launch and
  prove its final admission tuple before Templar routes users.
- Every maintenance upgrade serving isolated pools needs semantic evidence; interface versioning
  alone is insufficient.

### Invariants and preservation

ADR 0003 remains authoritative for supplier and claimant accounting. In its notation:

~~~text
N  = b_supply
P0 = floor(N * b_rate / P)
Ks = supplier_rounding_carry
V  = P0 + Ks
L  = aggregate underlying liabilities
B  = underlying token balance held by the pool
Cb = current backstop_credit
D  = aggregate deferred_backstop_credit across former cohorts
X  = unowned_surplus
Y  = ownerless_accrual
Cp = protocol_credit
H  = B + L - V - Cb - D - X - Y - Cp
E  = Y + Cp + H = B + L - V - Cb - D - X
F  = B - Cb - D - X = V - L + E
W  = V                    when L == 0
W  = max(V - L, 0)        when L > 0
~~~

The cumulative balance sheet remains `B + L = V + Cb + D + X + Y + Cp + H`. Every supplier quote,
health calculation, utilization check, liquidation valuation, mint, burn, and supply-cap check
continues to use ADR 0003's exact `V / N` semantics, including `Ks`; this ADR introduces no alternate
supplier-value formula or separately owned residual. It neither consumes nor relabels current or
deferred backstop credit, quarantined `X`, ownerless `Y`, or protocol credit. Accrual or explicit
default may make signed `H` negative and may leave `V < L` exactly as ADR 0003 permits. The cumulative
invariants remain `E >= 0`, `F >= 0`, and `N > 0 && Y > 0 => H >= 0`; this ADR adds no unconditional
`H >= 0` or `V >= L` gate.

| Required invariant | Disposition |
| --- | --- |
| Collateral reserve has no committed debt | New: `l_factor == 0 => d_supply == 0` at every committed state. |
| Collateral reserve emits no loan principal | New: factor-invalid regular and flash borrowing fail before debt, event, transfer, or callback. |
| Isolated pool contains one immutable directed pair | New: at most one reserve of each role; exactly one enabled pair is required to leave setup; listed roles never change. |
| ADR 0003 fungible supplier ownership and signed coverage | Preserved using exact `V / N`, `P0`, `Ks`, `Cb`, `D`, `X`, `Y`, `Cp`, `H`, `E`, `F`, and `W`; no claimant or exclusion bucket is consumed or reinterpreted. |
| Reserve `b_rate`/`d_rate` monotonicity except explicit loss paths | Role validation changes no rate arithmetic; ADR 0003 remains authoritative. |
| A healthy successful submit is not immediately liquidatable | Existing health validation remains after all new admission rules, using ADR 0003 value semantics. |
| Supplied value covers borrowing at ordinary action boundaries | Collateral has `L == 0`; lend-reserve checks retain ADR 0003 behavior and explicit default exception. |
| Backstop-take and interest arithmetic do not underflow or round against protocol rules | No allocation rule changes; ADR 0003 governs claimant arithmetic. |
| Flash loan leaves every reserve at least as solvent as before | Collateral flash loans reject before observable effects; permitted lend-reserve flash behavior retains existing and ADR 0003 checks. |
| ADR 0004 self-exit remains available | Role checks do not gate repay, withdrawal, or collateral withdrawal. Manager and hook policy cannot override the factor rules or remove the self-exit exception. |
| Backstop and factory/registry invariants | No backstop accounting, deterministic deployment, route authority, or upgrade authority is reassigned. |

### Acceptance criteria

1. For every `c_factor,l_factor <= SCALAR_7`, the role table is total and mutually exclusive;
   `0/0` fails `InvalidReserveMetadata` in open and isolated pools. Isolated `Both`, a duplicate
   role, a third reserve, and every listed-role transition use the same exact code at queue and
   execution. Queue-time failure creates no queued entry; after the execution unlock check,
   validation failure leaves the existing queued entry and all reserve state untouched.
2. The action matrix succeeds and fails exactly as specified. A factor-invalid `Supply`,
   `SupplyCollateral`, regular `Borrow`, same-batch borrow/repay, or flash borrow returns
   `ReserveDisabled`; `Withdraw`, `WithdrawCollateral`, and `Repay` retain their existing checks.
   At 61 or more requests, ADR 0002's shared guard returns `BadRequest` before admission or role
   validation. At permitted lengths in combined mode, a missing, expired, stale, or partial local
   permit returns `NotPermitted` without hook invocation or authorization. Role-error tests use a
   complete current permit plus an allowing hook, when configured, so `NotPermitted` or a hook
   failure cannot masquerade as `ReserveDisabled`.
3. Every production debt-creation and underlying-egress path is classified. A forbidden
   collateral borrow or flash attempt is atomic, leaves `d_supply == 0`, commits no borrow or flash
   event, invokes no receiver, and causes no collateral loan-principal outflow. Liquidation,
   bad-debt transfer/default, exits, repayment refunds, backstop-interest transfers, and protocol
   collection retain their own accounting and authorization semantics.
4. An isolated pool rejects `Both`, a third reserve, a duplicate role, and every listed-role
   transition with `InvalidReserveMetadata`. Queue and execution both run full metadata validation;
   execution adds that net-new check for new and existing reserves against then-current state before
   queued-entry deletion or reserve mutation. Concurrent same-role queued additions cannot both
   commit; the losing execution preserves its queued proposal.
5. An isolated pool cannot leave setup with zero or one reserve, duplicate roles, a disabled member
   of the pair, queued-only configuration, or no manager. Each case fails atomically with
   `StatusNotAllowed`. The exact enabled pair plus a manager can leave setup under every ADR
   0002-supported backstop mode.
6. The release sequence exercises the exact optimized Wasm, assets, oracle, manager, and retained
   hook. Failure freezes with the manager barrier and preserves self-exit. Success cleans
   canary-only admission and proves the declared manager/hook tuple, the automatic epoch advance
   for every manager transition that occurred, stale-permit behavior, exact local-permit-before-
   hook failure order, allowing-hook role failures, and post-cleanup admission before routing.
7. `is_isolated()` is immutable and fail-closed for incompatible cohorts. The cumulative deploy
   and constructor ABI append only `isolated`; the returned view matches the request atomically,
   and ADR 0004's deployment event is unchanged. No origination/flash fee or public role
   type/view/event appears.
8. Active status, registry membership, isolated identity, manager state, and hook state—alone or
   in combination—are never treated as on-chain certification of canary completion or routing
   eligibility. Release evidence is tied to the complete transaction sequence and exact Wasm.
   Product/security sign-off acknowledges that an admitted third party can call the active pool
   before that off-chain decision; the canary gates Templar routing only.
9. Open `Both` behavior and every exit path pass regression tests. Before applying this behavior to
   a live non-Templar cohort, release inventory identifies factor-zero configurations, queued
   changes, and incompatible positions; any such migration requires a separate ADR.
10. Ledger-consistent evidence uses ADR 0003's exact `N`, `P0`, `Ks`, `V`, `L`, `B`, `Cb`, `D`,
    `X`, `Y`, `Cp`, `H`, `E`, `F`, and `W` semantics before and after accrual, borrow/repay,
    withdrawal, fee collection, flash loan, and explicit default. It proves
    `B + L = V + Cb + D + X + Y + Cp + H`, `E >= 0`, `F >= 0`, and
    `N > 0 && Y > 0 => H >= 0`; it does not substitute a `b_rate`-only value, consume a claimant or
    exclusion bucket, or assert unconditional `H >= 0` or `V >= L` after accrual/default.
11. Every candidate Wasm serving isolated pools carries hash-bound invariant, authorization,
    canary, interface, size, and monitoring evidence. Before sealing, code drift or missing
    evidence blocks deployment or upgrade. Final sealing first completes and verifies
    `Seal(RegistryCode)`, then records whether `HookAdmission` remains open or completes and verifies
    its seal, and only then completes and verifies `Seal(PoolCode(pool))`. Evidence includes every
    selected live flag, exact event, unchanged executable hash, and the required negative execution
    tests. The root seal alone does not freeze pool code; the pool-specific seal rejects
    `UpgradePool` for that address but does not freeze hook enablement or attachment while
    `HookAdmission` remains open. Immutability claims disclose that distinction.
12. Every shipping optimized Wasm is at or below
    `min(131072, active_network.contractMaxSizeBytes)`. The implementation PR reports the final ADR
    0004 baseline, ADR 0005 after-size, and byte delta for every changed artifact, including zero
    deltas.
13. Standalone actual-Wasm and public-testnet runs exercise the exact pair, allowed flows,
    forbidden role actions, admission cleanup, failure freeze, event monitoring, and every final
    manager/hook tuple. Missing required testnet, Kani, size, or interface evidence blocks release
    and is never replaced by a lower verification level silently.
14. Release evidence proves `P > 0`, `S >= 0`, and `0 < m < P`; conditional on final `V <= S`,
    the end-of-batch check enforces `L <= E_cfg`. It records current and stressed committed
    liability, risk tier, and route. `Some` routes compare stressed committed exposure with a
    declared same-unit capacity method and buffer; `None` routes disclose it as uninsured. The
    evidence also exercises a fully repaid LendOnly flash whose transient principal exceeds
    `E_cfg` but stays below 100% utilization and whose final `L` satisfies the committed bound.
    Product and release records state that `E_cfg` is neither a perpetual accrued-liability cap nor
    a transient flash-principal cap. Exact checked/wider arithmetic computes the formula without a
    narrow `S * m` intermediate; overflow, narrowing failure, and `m == P` block envelope approval.
    Vectors cover `m == P`, `m == P - 1`, and `S` near `i128::MAX`. An unapproved pair increase stops
    Templar routing and risk-increasing admission but never repayment, liquidation, bad-debt
    settlement, donation, protocol-fee collection, or withdrawal.

### Risks and controls

| Risk | Owner and trigger | Prevention and detection | Recovery |
| --- | --- | --- | --- |
| A new debt or principal path bypasses role enforcement | pool maintainer; new debt mutation, transfer, event, or callback | classify every production debt/egress path; property/model evidence; debt and event monitoring | freeze affected pools; stop use of the hash; preserve traces; reviewed repair or new pool |
| End-state debt hides a fully repaid loan | implementation owner; net-zero batch or flash | separate state and flow invariants; assert no event, callback, or principal outflow | freeze affected pools, stop use of the hash, and preserve traces; never accept zero ending debt as complete proof |
| Concurrent configuration creates two reserves of one role | pool admin; queued same-role additions | authoritative validation against committed state at execution; exact `InvalidReserveMetadata` before queued-entry deletion or reserve mutation | cancel the preserved losing queue; configure the missing role |
| An atomic factor update swaps a listed role | pool admin; both factors cross zero | compare observable old/new roles at queue and execution; exact `InvalidReserveMetadata` before mutation | reject/cancel; if bad code committed, freeze and migrate or use a reviewed repair |
| Setup release or admission cleanup precedes the canary | pool admin or release owner; status/manager/hook mutation | exact-pair and manager setup gate; transaction-order evidence; final tuple check | retain or restore manager, freeze status 4, clean up via self-exit, withhold routing |
| Configured committed exposure exceeds the approved risk tier, the envelope calculation overflows, or transient flash principal is mislabeled as bounded by `E_cfg` | registry governance/release owner; `supply_cap`, `max_util`, route, liability, capacity, arithmetic, or flash-boundary drift | require `0 < m < P`, exact checked/wider calculation, stressed committed liability, same-unit capacity method, `None` disclosure, and the distinct temporary `< 100%` flash boundary; exercise `m == P`, near-`i128` `S`, and a flash above `E_cfg` | block or stop routing/risk-increasing admission, correct the calculation/disclosure, and lower the pair or reapprove with evidence; preserve repayment, settlement, and exit |
| Topology or permission state is marketed as certification | product/indexer owner; UI/API label or routing rule | explicit non-certification language and evidence-bound routing decision | correct claims, stop new routing, re-run or restore evidence |
| The exact pair is oracle-inoperable | release owner; price or normal-borrow canary fails | use exact deployed assets, oracle, and Wasm before routing | keep manager, freeze, repair oracle support or deploy the correct pair |
| An older cohort defaults missing isolated identity to open | implementation/release owner; upgrade of pre-view pool | fail-closed compatibility and deployment inventory | reject upgrade; separately reviewed migration or new address |
| A compatible maintenance upgrade removes semantics | registry governance; candidate with matching ABI | hash-bound semantic evidence and timelock review | cancel; if executed, freeze and roll forward before seal; after seal use new address |
| Pool code is called immutable while hook admission remains mutable | registry governance and pool admin; `PoolCode(pool)` seals with open `HookAdmission` | root-first explicit open-or-seal disposition, live-flag/event replay, and product disclosure of continuing enable/attach authority | correct the claim; if finality is intended, seal `HookAdmission` under ADR 0004 and re-record the active hook tuple |
| No-rehypothecation is read as custody or total immutability | product/security owner; external disclosure | publish the bounded state/flow and upgrade caveats | correct disclosure; choose a vault or sealed deployment if stronger properties are required |
| Wasm exceeds the active limit | implementation owner; integrated build | before/after attribution against effective network limit | delete or narrow implementation choices; do not ship above the gate |

### Pre-mortem

Assume the isolated-market launch failed:

1. **Collateral was flash-lent even though ending debt was zero.** Warning: a collateral
   `flash_loan` event or receiver callback commits. Cause: enforcement ran after debt creation or
   only checked end state. Prevention: prove failure before every observable loan effect for fully
   repaid flash and ordinary net-zero batches. Recovery: freeze, stop use of the hash, preserve the
   trace, and repair or replace only after incident accounting.
2. **The pool contained three assets or two lend reserves.** Warning: post-configuration role
   counts differ. Cause: queue-time validation was treated as authoritative across a race.
   Prevention: execution must satisfy the committed-state topology atomically. Recovery: do not
   leave setup; cancel the losing queue or deploy a new pool if a wrong asset committed.
3. **A reserve silently changed from collateral to loan.** Warning: a queued update crosses both
   factor zeros. Cause: validation checked individual fields rather than the derived role.
   Prevention: preserve the role across every listed-reserve transition. Recovery: reject the
   change; freeze and use a reviewed repair if faulty code applied it.
4. **The deployed pool's isolated identity disagreed with the request.** Warning: the constructor
   view and requested argument differ. Cause: cumulative ABI integration used an obsolete
   predecessor shape. Prevention: generated-spec comparison and actual-Wasm deployment
   postcondition. Recovery: reject the atomic deployment and rebuild from the reconciled ABI.
5. **An active pool was routed before its canary.** Warning: routing precedes oracle/borrow/cleanup
   transactions or relies only on status and scalar views. Cause: observable configuration was
   mistaken for certification. Prevention: require transaction-bound off-chain release evidence.
   Recovery: stop routing, restore the manager barrier, freeze, reconcile direct use, and rerun.
6. **An older pool silently appeared open after upgrade.** Warning: `is_isolated()` cannot be
   served or defaults on absent state. Cause: missing identity was interpreted as `false`.
   Prevention: fail closed and inventory every target cohort. Recovery: reject the upgrade and use
   a reviewed migration or new address.
7. **A maintenance upgrade kept the ABI but removed no-rehypothecation.** Warning: candidate
   evidence is absent or code hash changes unexpectedly. Cause: interface compatibility was
   treated as semantic proof. Prevention: bind complete evidence to each candidate hash until
   sealing. Recovery: cancel or freeze and roll forward; after sealing, deploy a new pool.
8. **One seal was mistaken for complete code and admission finality.** Warning:
   `is_sealed(RegistryCode)` is true while `is_sealed(PoolCode(pool))` is false, or pool code is
   sealed while `HookAdmission` remains open but product claims say admission policy is immutable.
   Cause: the root prerequisite or pool-code terminal state was treated as closing every independent
   scope. Prevention: after the root seal, explicitly record open `HookAdmission` authority or
   complete and replay its seal, then complete the pool-specific code seal and exercise the exact
   negative transitions against unchanged executable hashes. Recovery: correct the claim; complete
   any still-intended seal in the required order and re-record the active hook tuple rather than
   inferring or backfilling finality off chain.
9. **The configured pair was presented as fully insured or flash-capped by `E_cfg`.** Warning:
   accrued or stressed committed liability exceeds the recorded envelope, the capacity comparison
   uses incomparable units, `m == P` is treated as finite, narrow multiplication wraps, or a fully
   repaid flash transfers more than `E_cfg`. Cause: an ineligible or incorrectly evaluated end-of-
   batch committed-liability envelope was mislabeled as a perpetual loss or transaction-wide
   principal bound. Prevention: require the strict formula domain and checked/wider arithmetic;
   record current/stressed committed liability, conversion method, buffer, route-specific
   disclosure, and distinct temporary `< 100%` flash boundary. Recovery: stop new routing/risk-
   increasing admission, correct the calculation/disclosure, and lower or freshly approve the
   configuration while preserving all risk-reducing paths.

### Verification and release

This ADR may be accepted before implementation. Acceptance records obligations; it does not claim
that downstream contract artifacts or live evidence already exist. Raladr produces only this
Proposed decision record and does not implement the registry or pool.

Draft-time checks:

~~~sh
PATH="$PWD/bin:$PATH" adr --robot lint
git diff --check -- doc/adr/0005-reserve-roles-and-isolated-morpho-style-pools-with-no-rehypothecation-of-collateral.md
git hash-object doc/adr/0005-reserve-roles-and-isolated-morpho-style-pools-with-no-rehypothecation-of-collateral.md
make -n -C vendor/blend-contracts-v2 test
~~~

After implementation, evidence proceeds in the repository-required order:

1. **Unit and property tests.** Run `make test`, which builds before testing. Cover the complete
   role/action/error matrix, state and flow invariants, queue races and transitions, setup-release
   states, full metadata revalidation at queue and execution, exact `InvalidReserveMetadata` for
   `0/0`, isolated `Both`, duplicate/third reserve, and listed-role transition, queue preservation on
   execution failure, open-pool regressions, every allowed egress class, ADR 0003 `V / N`
   accounting, every claimant and exclusion bucket, and the `E`, `F`, and conditional-`H`
   reachability invariants, ADR 0002 60/61 ordering, ADR 0004 all-local-permits-before-hook order
   with both denying-permit/no-hook and valid-permit/allowing-hook role-error cases,
   immutable identity, construction/events, both `set_status` topic shapes, upgrade compatibility,
   exposure-pair calculations and strict formula domain, exact checked/wider arithmetic including
   `m == P`, `m == P - 1`, and `S` near `i128::MAX`, end-of-batch committed-liability bounds, the
   distinct temporary flash boundary including principal above `E_cfg`, and rollback.
   Test organization and helper names are implementation-owned.
2. **Bounded model checking.** Prove role classification, immutable role transitions, exact-pair
   setup release, collateral debt exclusion, pre-effect collateral loan rejection, and the finite
   committed-liability implication for `0 < m < P` over stated protocol-enforced bounds. Harness
   decomposition and names are implementation-owned. An unjustified narrowed domain or timeout is
   not evidence.
3. **Optimized Wasm and interface evidence.** Build the exact final ADR 0004 baseline and combined
   candidate with the same pinned toolchain. Record commits, lockfile hashes, optimized artifact
   hashes, public specs, before/after bytes, and deltas. Confirm that only the public ABI and event
   changes named here appear and that every artifact meets the effective network size limit.
4. **Standalone actual-Wasm integration.** Exercise construction, both structural pool modes, the
   action matrix, queue races, allowed egress, forbidden regular and flash borrowing, manager and
   hook combinations, a fully repaid LendOnly flash above `E_cfg` but below 100% temporary
   utilization, canary success/failure, cleanup, monitoring, compatible upgrade, and seal behavior
   using the actual optimized artifacts rather than Rust mocks. The seal run completes
   `RegistryCode` first, records an open `HookAdmission` disposition or completes its seal, and then
   completes `PoolCode(pool)`. It verifies every selected live flag/event/hash, proves that the root
   alone still permits an otherwise valid pool upgrade, proves that the pool-specific seal rejects
   it, and—when hook admission remains open—proves and discloses continuing hook enable/attach
   authority.
5. **Public-testnet smoke.** Exercise real authorization and cross-contract behavior against the
   configured oracle and assets. Record ledger-consistent pre/post state, RPC results, committed
   events, receiver invocation, token balances, ADR 0002 oversize rejection, combined-mode local
   denial without hook invocation, valid-permit allowing-hook role failure, admission changes,
   freeze and self-exit, and the off-chain routing decision. No production or mainnet deployment is
   authorized by this ADR.
6. **Release and rollback.** The release evidence binds the accepted ADR, upstream commit, exact
   Wasm hashes, effective size limit, interface versions, complete canary transaction chain, final
   admission tuple, envelope-domain and exact-arithmetic evidence, `S`/`m`/`E_cfg` and stressed-
   capacity or uninsured disclosure, root-first registry/`HookAdmission`/pool-code disposition, the
   distinct transient flash disclosure, and monitoring readiness. Before deployment, rollback is
   ordinary code reversion. During unsealed maintenance, rollback means a reviewed compatible roll-
   forward under ADR 0002; after the pool-specific seal, recovery is a new-address deployment.

At draft time, the ADR-0005 contracts, optimized Wasm, proofs, standalone flow, and public-testnet
evidence do not exist. `stellar` and `cargo-kani` are unavailable in this shell. Those gaps are
implementation and release blockers at their required stages, not evidence claimed by this ADR.

Follow-ups: a cross-collateral product with several role-separated assets; custody segregation or
stronger immutability claims; migration of any initialized cohort; and any on-chain release
coordinator justified by evidence that transaction-bound operational gating is insufficient.
