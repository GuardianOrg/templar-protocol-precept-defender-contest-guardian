# 3. Protocol fee on borrow interest; defer origination and flash-loan fees

Date: 2026-08-27

## Status

Proposed

Amends [2. Pool registry as deployment, configuration, and optional-backstop routing layer](0002-pool-registry-as-deployment-configuration-and-optional-backstop-routing-layer.md)

Amended by [8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)

## Context

Templar needs protocol revenue from Blend pools after the SDF grant. The durable source in
scope is a share of borrow interest. A future BLND buyback or burn may consume that revenue,
but issuance and treasury execution are separate decisions.

The vendored contracts (`vendor/blend-contracts-v2` @ `6cf6dd1`) have one existing interest
skim, owned by the backstop:

- `Reserve::load` updates `d_rate`, derives accrued aggregate liabilities, and calls
  `Reserve::accrue` (`pool/src/pool/reserve.rs:35-75`). `accrue` floors the configured
  `bstop_rate` into `backstop_credit` and gives the remainder to suppliers through `b_rate`
  (`pool/src/pool/reserve.rs:91-106`; `pool/src/storage.rs:26-32,68-78`). There is no
  protocol claim or collection path.
- `backstop_credit` leaves through the interest auction. A lot snapshots reserve credit; a fill
  decrements it and transfers underlying from the pool
  (`pool/src/auctions/backstop_interest_auction.rs:11-26,41-81,86-118`).
- Public `gulp` loads a reserve, compares the token balance with represented supplier value,
  liabilities, and backstop credit, then assigns positive surplus to `backstop_credit`
  (`pool/src/pool/gulp.rs:18-36`; entrypoint `pool/src/contract.rs:513-518`). ADR 0002 changes
  that destination to suppliers when a pool has no backstop.
- Aggregate liabilities use ceil rounding and aggregate supplier value uses floor rounding.
  Borrow and repay deliberately round their d-token conversions in opposite directions
  (`pool/src/pool/reserve.rs:158-183,206-220`;
  `pool/src/pool/actions.rs:391-439`). Default currently applies another ceil-rounded
  conversion to the supplier rate and clamps at zero
  (`pool/src/pool/user.rs:101-116`).
- `PoolConfig` can be changed by the pool admin. A change to `bstop_rate` first settles every
  reserve under the old config (`pool/src/contract.rs:392-399`;
  `pool/src/pool/config.rs:16-64`). ADR 0002 adds registry policy checks around deployment and
  later transitions.
- ADR 0002 also separates a pool's registry lifecycle from its local risk status. `Contained` and
  `Retired` stop risk/config mutation, while a `Live` pool may be locally frozen at status 4 for
  incident response. A new fee setter must respect that lifecycle boundary without making a frozen
  live pool impossible to repair, and it cannot reuse full-tuple deployment validation in a way that
  re-rejects an unchanged fee/take grandfathered by later policy tightening.

Adding a second skim exposes accounting defects that the one-skim implementation can currently
hide:

- Flooring each protocol allocation independently makes revenue depend on how frequently a
  reserve is settled. An unbounded number of sub-unit intervals can erase an unbounded share of
  protocol revenue.
- Retaining that same per-interval floor for the backstop leaves its economics caller-cadence
  dependent. At a 10% take, one settlement of ten units allocates one unit while ten settlements
  of one unit allocate zero. A remainder must belong to the current credit epoch and must not
  survive a close, route change, or physical-backing cap.
- Recomputing `b_rate` from an already floored represented balance can make `b_rate` fall during
  positive accrual. With `P = 10^12`, `b_supply = 1_800_000_000_000`,
  `b_rate = P + 3`, one unit of accrued interest, a 20% fee, and a carried protocol fraction that
  converts that one unit to protocol credit, the pinned formula would recompute
  `b_rate = P + 2` even though suppliers lost nothing.
- A reserve-wide rounding residual is not safe merely because it remains in the supplier class.
  If holder quotes still use `b_rate` alone, a later supplier can inherit value accrued before
  entry and an earlier supplier can forfeit it on exit. Individual b-token ownership must be
  fungible and pro rata.
- A protocol credit omitted from the accounted-balance equation can be swept by `gulp`. A credit
  collected without reserving existing backstop claims and fundable supplier withdrawals can make
  those claims fail.
- ADR 0002's uninsured additive `b_rate` rule is call-count-dependent. At
  `b_supply = 1_800_000_000_000`, `b_rate = P`, and surplus five, one additive floor represents
  only three units; an unchanged second call represents the remaining two. This ADR replaces that
  rule with exact pooled supplier-value accounting.
- ADR 0002's positive-cash allocator classifies its whole derived balance difference by route and
  supply state. Once protocol credit and liability-created `OwnerlessAccrual` exist, a positive
  difference can contain both physically free cash and non-cash accounting coverage. Assigning the
  whole amount to current backstop credit would create an unfunded claim; assigning the whole amount
  to cash-backed `UnownedSurplus` would make that exclusion exceed the pool's free cash. The final
  allocator must classify only the physically free part as cash while preserving the non-cash part
  in supplier or ownerless accounting.
- ADR 0002's `None -> Some` attachment can change the owner of pre-cutover uninsured surplus.
  Public `gulp` is not a sufficient operational precondition because it is Borrow-gated and valid
  pools at statuses 2 and 4 have Borrow disabled
  (`pool/src/pool/gulp.rs:18-24`; `pool/src/pool/pool.rs:75-81`).
- Aggregate conservation does not prove fair supplier entry. With `N = V = 1`, a direct transfer
  of 50 followed by the current uninsured `gulp` gives `V = 51`; a 100-unit deposit then mints one
  share, and the original one-share holder can withdraw 75. The balance sheet remains exact while
  24 units move from the newcomer to the incumbent. Quarantining unsolicited cash and rejecting
  membership rounding beyond an authorization-bound and code-bounded limit are therefore separate
  requirements. Stellar's draft tokenized-vault standard independently identifies this empty or
  nearly-empty donation/inflation family and treats it as a slippage problem, with virtual decimal
  offsets as one possible structural defence
  ([SEP-0056, pinned evidence](https://github.com/stellar/stellar-protocol/blob/8912a8047931453bb5d6a631e10a9d7125c570f3/ecosystem/sep-0056.md#notes-on-decimals-offset)).
- Permissionless collection has no protocol-wide treasury circuit breaker. Requiring every pool
  admin to reach status 4 is neither complete nor proportional when the fee recipient alone is
  compromised, especially when one pool admin is unavailable. Collection already consults the
  registry, so a registry-level immediate pause adds no ordinary-action dependency.
- Leaving uncollected protocol credit senior to supplier value can make a fully defaulted reserve
  impossible to reopen through ordinary supply. In the reachable state
  `(B,L,V,Y,Cp,H) = (0,0,0,0,2,-2)`, with every omitted bucket and remainder zero, first supply
  rejects before its inflow can cure the shortfall, while a raw transfer can be collected back out.
  Unrealized protocol credit and its fractional remainder must participate in the default waterfall
  before supplier principal.
- The current interest-auction create/fill paths are not named in ADR 0002's `V < L` replacement
  matrix even though they are the exit for current backstop credit. A stale blanket check there can
  strand a physically backed claim. User-liquidation fills transfer auctioned positions and move
  residual user debt to the backstop on a full fill; only the separate backstop-default path writes
  debt down. Bad-debt fills likewise transfer auctioned debt before the inherited threshold check
  may default a remainder (`pool/src/auctions/user_liquidation_auction.rs:206-222`;
  `pool/src/auctions/bad_debt_auction.rs:96-128`; `pool/src/pool/bad_debt.rs:53-77,100-124`).
  All three auction families need explicit shortfall-state rules. Separately, requiring clients to
  assemble projected reserve, carry, ownerless, deferred, quarantine, and token-balance reads at one
  ledger is logically sound but operationally brittle; one cold atomic snapshot is the safer public
  boundary.

`PoolConfig` and `ReserveData` are public wire types, not storage-only details. The pool exports
`get_config()` and `get_reserve()`; `Reserve` embeds `ReserveData`
(`pool/src/contract.rs:80-94,431-446`; `pool/src/pool/reserve.rs:14-20`). Rust fixtures decode
both types (`test-suites/src/test_fixture.rs:13,216-224,251-260`), and the build generates
TypeScript clients from optimized Wasm (`Makefile:32-44`). Any new accounting state that clients
must use is therefore an interface decision.

Borrow, repay, flash-loan, transfer, callback, authorization, and event shapes are existing public
surfaces (`pool/src/pool/actions.rs:11-18,58-63,386-439`;
`pool/src/pool/submit.rs:68-147`; `pool/src/events.rs:253-304`). A flash receiver can require
the exact callback arguments in the user's Soroban authorization tree
(`test-suites/tests/test_flashloan.rs:75-107`). Origination and flash-loan fees therefore need
their own decision about quoting, authorization, funding, and default; they are not incidental
extensions of an interest skim.

Constraints and assumptions:

- ADR 0002 lands first. This ADR extends its registry, typed governance, optional-backstop route,
  maintenance upgrades, interface-version discipline, and genesis manifest; it does not create a
  parallel authority.
- ADRs 0002 and 0003 are one greenfield release. No non-disposable pool may use the original,
  ADR-0002-only, or another interim public layout. Discovery of such a pool blocks release and
  requires a migration ADR.
- `vendor/` remains read-only here. Contract changes are separate upstream work.
- Each optimized contract must remain at or below
  `min(131072, active_network.contractMaxSizeBytes)` on every deployment network. After-build
  sizes and executable evidence are implementation and release gates, not evidence required for
  Raladr consensus.
- The protocol fee is at most 20%, and protocol plus backstop interest skim is at most 30%.
  These are safety ceilings, not an asserted optimum. Beginning from zero `R`, `G`, and `Cq`, every
  cumulative settled prefix allocates at least 70% of its gross accrued interest to residual rather
  than protocol, backstop, or joint fee reserve, while cumulative protocol allocation is at most one
  fifth. With positive b-token supply that residual is supplier-owned; with zero supply it becomes
  durable ownerless accrual that no later supplier, backstop, or protocol recipient can capture. A
  small interval may release claimant remainders or the joint reserve carried from earlier intervals,
  so this ADR asserts no equivalent per-interval percentage guarantee. Registry policy sets the
  actual product rates and may impose tighter bounds, but cannot raise either code-level ceiling
  without a new ADR and compatible pool upgrade. Every production pool within the Precept
  registry, including permissioned and isolated pools, has a positive protocol interest fee. The
  sole documented exception is the out-of-registry ADR-0008 legacy cohort while governed by that
  ADR and migrated or retired under ADR 0009; those legacy pools launch zero-fee by that ADR's
  terms. Any further zero-fee product remains a separate economics decision requiring its own ADR
  rather than an undocumented exception.
- Protocol credit is a reserve-level face claim against fungible liquidity, not a claim attached
  to a particular repayment. It is written down before supplier principal when aggregate debt
  defaults and is not realized revenue until transferred.

This ADR decides observable economics, public interfaces and events, governance bounds,
compatibility, invariants, and release conditions. It does not decide helper or module names,
internal factoring, a Rust prototype, proof-harness names, test-runner phases, or evidence-manifest
layout. Those choices belong to the later implementation. Equivalent implementations are valid
only when they satisfy the equations, atomicity, interfaces, and acceptance criteria below.

Principles:

1. Take protocol revenue only in the existing interest-accrual path. Borrow, repay, and flash-loan
   economics and callback shapes remain unchanged; the cumulative greenfield `Request` wire gains
   one authorization-bound membership limit so supply and withdrawal cannot silently cross a
   caller's quoted share boundary.
2. Preserve exact claimant accounting: protocol fractions and current-epoch backstop fractions
   survive settlement cadence, every b-token owns the same pro-rata share of pooled supplier value,
   and positive non-loss transitions do not reduce that share value. Liability-created ownerless
   value and unsolicited cash never masquerade as supplier-owned yield.
3. Make claimant priority deterministic on-chain. `gulp` cannot reclassify supplier value, and
   only physically free coverage can become a cash-backed backstop or quarantine amount. Protocol
   collection cannot consume a current/deferred backstop claim, quarantined or still-unclassified
   cash, or a fundable supplier withdrawal.
4. Reuse ADR 0002's registry and maintenance authority, while keeping ordinary pool actions free
   of registry and token cross-calls introduced solely for revenue.
5. Keep the public and hot-path surface no larger than the economics require.

Decision drivers, ranked:

1. Durable borrow-interest revenue from every pool, including a pool with no backstop.
2. Preserve supplier ownership, backstop-auction liveness, and frozen-pool safety.
3. Minimize hot-path code, public-interface growth, and one-year maintenance burden.

## Decision

Add one pool-wide **protocol interest fee**. Defer origination and flash-loan fees.

### Interest split and public accounting state

Let `Q = SCALAR_7 = 10^7` and `P = SCALAR_12 = 10^12`. The final genesis public wire
types add:

| Public type | Field | Meaning |
| --- | --- | --- |
| `PoolConfig` | `interest_fee: u32` | protocol share of accrued borrow interest, scaled by `Q` |
| `ReserveData` | `protocol_credit: i128` | whole underlying units owed to the protocol and not yet collected |
| `ReserveData` | `protocol_fee_remainder: u32` | carried numerator units, always less than `Q` |
| `ReserveData` | `backstop_fee_remainder: u32` | carried numerator units owned by the current open backstop credit epoch, always less than `Q` |

All three new reserve fields initialize to zero. Their public names and meanings are normative
because `get_reserve` exposes them. Their internal storage placement and the code used to update
them are not decided here. This ADR reuses ADR 0002's persistent `SupplierRoundingCarry` and
cash-backed `UnownedSurplus` state and views. It adds one distinct persistent `OwnerlessAccrual`
amount per reserve, with the exact view and event below; it does not add a duplicate supplier
residual or reinterpret quarantined cash.

For one settled interval, let `a >= 0` be accrued aggregate borrow interest, `f` the protocol
rate, `g` the **effective** backstop rate, `R0` the old protocol remainder, and `G0` the current
open credit epoch's backstop remainder. The derived whole-token joint fee-rounding reserve is
`Cq = floor((R + G) / Q)`; because each remainder is below `Q`, `Cq` is zero or one and requires no
additional stored scalar. Per ADR 0002, `g` equals the configured take only while the canonical
route is `Some` and that backstop credit epoch is open; it is zero for `None` and for a closed epoch,
where `G0` must also be zero. The observable split is:

~~~text
protocol = floor((a * f + R0) / Q)
R1       = (a * f + R0) mod Q
backstop_numerator = a * g + G0
nominal_backstop = floor(backstop_numerator / Q)
candidate_G1 = backstop_numerator mod Q
headroom = max(V0 - L0, 0)
backstop = min(nominal_backstop, headroom)
if backstop_numerator <= Q * headroom:
    G1 = candidate_G1
    discarded_backstop_numerator = 0
else:
    G1 = 0
    discarded_backstop_numerator = backstop_numerator - Q * headroom
Cq0 = floor((R0 + G0) / Q)
Cq1 = floor((R1 + G1) / Q)
residual = a + Cq0 - protocol - backstop - Cq1
~~~

Every observable result equals the mathematical value above. Invalid rates, an invalid remainder,
or an unrepresentable result fail atomically; the arithmetic decomposition is not part of this
decision. The pool enforces:

~~~text
0 <= f <= 0.20 * Q
0 <= g
f + g <= 0.30 * Q
0 <= R < Q
0 <= G < Q
Cq in {0, 1}
~~~

`V0` and `L0` are pre-interval aggregate supplier value and liabilities. `Cq` withholds the one
whole unit that the two claimant fractions may jointly own before either owns it individually. It
prevents simultaneous maturation of `R` and `G` from making interval `residual` negative; it is not
supplier value, protocol credit, backstop credit, or collectible cash. The backstop term preserves
ADR 0002's non-cash credit cap. The cap binds whenever the full numerator exceeds
`Q * headroom`, including `nominal_backstop == headroom` with a positive fractional remainder. Its
complete excess numerator is recorded as `discarded_backstop_numerator`, `G1` becomes zero, and any
unallocated whole-token amount stays in `residual`; none can become a backstop claim after headroom
later recovers. At positive supply that residual is
supplier-owned; at zero supply it is ownerless accrual. The registry may narrow
configured `f` and `g` further. The existing per-interval floor for an eligible backstop is
replaced by a current-credit-epoch remainder; both skims now carry fractions inside their stated
domains. For any sequence of settled intervals:

~~~text
Q * sum(protocol_i) + Rn + sum(discarded_protocol_numerator_i)
    = R0 + sum(accrued_i * f_i)
0 <= Rn < Q
~~~

`discarded_protocol_numerator` is zero except when a default has `delta_L > Y0`; that transition
has reached the joint fee-rounding tier, records the exact old `R`, and sets it to zero so
fractional credit from defaulted interest cannot reappear against a later supplier cohort.

Within one open backstop credit epoch, include every explicit cap or default disposal in
`discarded_backstop_numerator` and require:

~~~text
Q * sum(backstop_i) + Gn + sum(discarded_backstop_numerator_i)
    = G0 + sum(accrued_i * g_i)
0 <= Gn < Q
~~~

Closing the epoch or replacing/removing its canonical route records the then-current `G` as one
final discarded numerator and resets it to zero before the lifecycle/route event. Recompute `Cq`
after the reset. Any exact `Cq0 - Cq1 == 1` release is non-cash value and enters `V` at positive
supply or `Y` at zero supply before that event; otherwise no whole-token bucket moves. A changed
`bstop_rate` inside the same open epoch preserves `G` without repricing it, just as a protocol-rate
change preserves `R`.

Let `T = R + G - Q * Cq`, so `0 <= T < Q`. Across any trace of settled intervals and explicit
default, close, or route-reset transitions:

~~~text
Q * sum(protocol_i + backstop_i + Cq_i - Cq_(i-1))
    + Tn
    + sum(discarded_protocol_numerator_i + discarded_backstop_numerator_i)
    = T0 + sum(accrued_i * (f_i + g_i))
~~~

For an accrual interval, protocol disposal is zero and any backstop disposal is already in the
formula. Because `f + g <= 0.30 * Q`, the identity proves
`protocol + backstop + Cq1 - Cq0 <= a` for every interval and therefore `residual >= 0`. It also
bounds joint cadence dust below one base unit per open epoch absent an explicit cap disposal.

When `R0 = G0 = Cq0 = 0`, every prefix additionally satisfies:

~~~text
sum(protocol_i) <= sum(accrued_i * f_i) / Q
sum(protocol_i + backstop_i) <= sum(accrued_i * (f_i + g_i)) / Q
sum(residual_i) >= 0.70 * sum(accrued_i)
~~~

The inequalities are cumulative and rate-weighted, with disposed backstop numerator excluded from
the claimant term; they do not apply to an arbitrary sub-window
that begins with a nonzero carried remainder. In particular, a small interval can release a whole
protocol unit attributable partly to earlier accrual.

At `N = 0`, `V0 = 0`, and `L0 > 0`, the backstop allocation is zero because
`headroom = max(V0 - L0, 0) = 0`, even if `g > 0`. Every positive backstop numerator binds the cap,
so `G1 = 0`; only protocol credit accrues immediately. Settlement
increases `OwnerlessAccrual` by `residual`, including all nominal backstop excess and any released
`Cq0`, and leaves derived coverage `H` unchanged. It creates neither cash-backed
`UnownedSurplus` nor a claim for a
later supplier, backstop, or protocol recipient.

This identity applies to the gross accrual each actual load observed. It does not claim that two
different `Reserve::load` schedules produce the same gross interest; utilization, elapsed time,
`ir_mod`, and `d_rate` rounding remain load-path-sensitive
(`pool/src/pool/reserve.rs:55-75`; `pool/src/pool/interest.rs:23-99`).

The identity governs fee allocations, not the current `Cp` balance. Across any complete trace:

~~~text
Cp_final = Cp_initial
         + sum(protocol fee allocations)
         - sum(successful collections)
         - sum(default protocol-credit write-downs)
~~~

Ownerless accrual never enters this ledger; `Cp` contains protocol allocations only. Its own exact
ledger is:

~~~text
Y_final = Y_initial
        + sum(zero-supply interest residuals)
        + sum(zero-supply joint-reserve releases)
        + sum(zero-supply non-cash positive-H classifications)
        - sum(default y_loss)
~~~

A fee-rate change settles every reserve under the old config before the new rate becomes effective.
Both carried remainders and derived `Cq` survive a same-epoch rate change without repricing. At
positive supply, protocol and backstop credits increase by their split amounts, `Cq` changes only by
the formula above, and pooled supplier value increases by nonnegative `residual`.

The pinned baseline returns from `Reserve::load` whenever `b_supply == 0`, before utilization or
interest calculation (`pool/src/pool/reserve.rs:45-75`). This ADR changes that behavior only when
the ledger timestamp advances and liabilities remain. At `N == 0, L > 0`, the load uses capped
100% utilization without dividing by b-token supply, updates `ir_mod`, `d_rate`, protocol credit,
and both fee remainders through the ordinary accrual calculation, and applies the split above.
Actual backstop allocation remains zero and `G` resets through the binding-cap rule. `residual`,
including nominal backstop excess and any `Cq` release, increases
`OwnerlessAccrual` directly and leaves `H` unchanged; it creates no holder, `V`, `Cb`, `D`, or `X`.
At `N == 0, L == 0`, the baseline no-interest short circuit remains. No zero-supply path may divide
by the b-token supply.

### Fungible supplier ownership

For each reserve, reuse ADR 0002's notation and state:

~~~text
N  = b_supply
P0 = floor(N * b_rate / P)
Ks = supplier_rounding_carry(asset)
V  = P0 + Ks
~~~

`V` is the exact pooled supplier value. For `N > 0` its canonical public representation is:

~~~text
b_rate = floor(V * P / N)
P0     = floor(N * b_rate / P)
Ks     = V - P0
~~~

The result must be exact and checked. It is idempotent and satisfies:

~~~text
P0 <= V
0 <= Ks <= ceil((N - 1) / P)
~~~

For `N == 0`, only `V == 0` is valid supplier state; ADR 0002 separately preserves any
`UnownedSurplus`, and this ADR separately preserves `OwnerlessAccrual`. The implementation adds the
new empty reset `b_rate = P` and `Ks = 0`; this is not baseline Blend behavior. It is a non-economic
epoch boundary and the only permitted non-default decrease caused solely by canonical
representation.

Every b-token is a fungible pro-rata claim:

~~~text
claim(n) = floor(n * V / N)
~~~

The cumulative greenfield `Request` wire appends one exact field:

~~~rust
struct Request {
    request_type: u32,
    address: Address,
    amount: i128,
    limit: i128,
}
~~~

`limit` is part of the `requests` vector bound by the existing `spender`/`from` authorization root.
For `Supply` and `SupplyCollateral` it is `min_b_tokens_out`; for `Withdraw` and
`WithdrawCollateral` it is `max_b_tokens_burned`. Those four request types require `limit > 0`.
Every other request requires `limit == 0`; a negative or semantically nonzero limit is
`BadRequest`. Simulation may suggest a limit, but only this signed invocation argument protects
against state movement before inclusion. The entrypoint signatures and flash callback remain
unchanged; the pre-genesis `Request` spec and every generated client change once.

The protocol additionally sets `MAX_MEMBERSHIP_ROUNDING_LOSS = 1` underlying base unit. A caller
may demand a stricter limit, but no caller may authorize a larger silent share-rounding transfer.
All comparisons use checked wide products; an unrepresentable bound is `InternalError`.

No holder owns `Ks` separately, and no single holder or fee recipient owns derived `Cq` separately;
the split either converts it into whole claimant credit or an explicit disposal releases or writes
it down. This ADR deliberately amends ADR 0002's baseline-compatible
holder conversion: `V / N`, not `b_rate` alone, governs holder quotes, effective
collateral, utilization, `total_supply`, supply caps, health, liquidation and auction valuation,
minting, burning, clients, and indexers. Emission accounting remains b-token-share based; `Ks` has
no separate emission beneficiary.

Positive supply has these economic results:

- At `N == V == 0`, let `H_pre` be coverage immediately before the deposit's economic effect,
  including ordinary accrual and any earlier requests in the same transaction, derived from that
  asset's request-order `projected_B`, but excluding the deposit itself. If `H_pre < 0`, the first
  supply is rejected with `BadRequest`; the whole
  transaction rolls back, with no mint, token transfer, or supply event. If `H_pre > 0`, apply the
  zero-supply cash/non-cash classification below before the deposit. Neither part nor existing
  `OwnerlessAccrual` is assigned to the new depositor, current backstop, protocol, or a former
  backstop cohort. A successful deposit then mints `x` shares at unit value, requires
  `x >= min_b_tokens_out`, and leaves `H == 0`.
- At `N > 0` and `V > 0`, a deposit `x` mints
  `q = floor(x * N / V)`. A zero-share result or `q < min_b_tokens_out` is rejected. Let
  `new_claim = floor(q * (V + x) / (N + q))` and
  `old_claim = floor(N * (V + x) / (N + q))`. The transition additionally requires
  `x - new_claim <= MAX_MEMBERSHIP_ROUNDING_LOSS` and
  `old_claim - V <= MAX_MEMBERSHIP_ROUNDING_LOSS`; otherwise it rejects before transfer, mint,
  cap mutation, or event. The post-state is `(N + q, V + x)`, and the supply cap uses the new `V`.
- At `N > 0` and `V == 0`, new supply is rejected until the worthless positions are removed.

`N` and `V` in these branches are the state after the action's ordinary reserve load. Elapsed
accrual can therefore restore positive `V` before eligibility is decided; only a post-load
`N > 0, V == 0` state rejects supply.

The new holder's post-mint claim cannot exceed `x`; its unconsented rounding loss is at most one
base unit, the pre-existing cohort's aggregate claim cannot fall below its old `V`, and its gain
from the membership rounding is at most that same unit. The exact inflation witness
`N=V=B=1 -> direct transfer 50 -> gulp -> deposit 100` never reaches a mint: the transfer is
quarantined under the classifier below, and even a malformed implementation that first produced
`N=1,V=51,q=1,new_claim=75` would fail the 25-unit loss bound.

For a holder with `n > 0` shares, let `h = floor(n * V / N)`:

- A zero `Withdraw` or `WithdrawCollateral` request is rejected with
  `InvalidBTokenBurnAmount`. The whole transaction rolls back and emits no withdrawal event.
- If `h == 0`, any positive request burns all `n`, pays zero, invokes no underlying-token
  transfer, requires `n <= max_b_tokens_burned`, and emits the existing withdrawal event with zero
  tokens out and `n` shares burned.
  This full-position cleanup applies whether aggregate `V` is zero or positive.
- For `0 < x < h`, burn `q = ceil(x * N / V)` and pay exactly `x` only when `q < n`.
  Require `q <= max_b_tokens_burned` and the checked rational rounding donation
  `q * V - x * N <= MAX_MEMBERSHIP_ROUNDING_LOSS * N`. If either fails, or if `q >= n`, reject
  atomically; a partial request cannot destroy a full position or donate more than one underlying
  base unit to the remaining cohort.
- For `h > 0` and `x >= h`, burn all `n`, require `n <= max_b_tokens_burned`, and pay exactly `h`.
- The final holder has `n == N`, requires `N <= max_b_tokens_burned`, and therefore receives all
  remaining `V`.

Every successful nonfinal burn leaves the remaining cohort's share value nondecreasing. A genuine
partial burn's ceil-rounding donation is nonnegative and at most one underlying base unit, as well
as strictly less than one share's pre-burn rational value `V / N`. A nonfinal
full-position burn donates the fractional difference `n * V / N - h`, which is nonnegative and
strictly less than one underlying unit.
Existing authorization, health, and `require_utilization_below_100` checks still apply to every
positive payout using the post-state `V`; this is the baseline withdrawal bound, not the configured
borrow/flash `max_util` check. A zero-claim full-position cleanup is not rejected solely because
utilization exceeds its ordinary withdrawal bound or, for collateral, because the user's health
check would fail: it moves no underlying and removes zero effective collateral. Other requests in
the same transaction retain every check they independently require. The cleanup preserves the
holder's already accrued BLND through the burn and emits no separate emissions event.
Liquidation-auction fills transfer b-token shares and their pro-rata claims without any
intermediate economic change to aggregate `N` or `V`. This ADR requires that atomic outcome, not a
particular internal transfer routine. Emission settlement must likewise treat the fill as a share
transfer: neither party gains or loses accrued BLND merely because the shares moved.

For the defaulted debt-token amount, let `A` be its existing standalone ceil conversion and let
`delta_L = L0 - L1` be the exact decrease in aggregate represented liabilities after those debt
tokens are removed. Valid checked conversions have `delta_L` equal to `A` or `A - 1`. Uncollected
fee reserve and protocol credit are unrealized borrower-interest revenue, so they absorb default
after ownerless accrual and before supplier principal. The exact loss waterfall is:

~~~text
y_loss = min(delta_L, Y0)

if delta_L > Y0:
    fee_rounding_loss = Cq0
    discarded_protocol_numerator = R0
    discarded_backstop_numerator = G0
    R1 = 0
    G1 = 0
else:
    fee_rounding_loss = 0
    discarded_protocol_numerator = 0
    discarded_backstop_numerator = 0
    R1 = R0
    G1 = G0

cp_loss = min(delta_L - y_loss - fee_rounding_loss, Cp0)
v_loss  = min(delta_L - y_loss - fee_rounding_loss - cp_loss, V0)
Y1      = Y0 - y_loss
Cp1     = Cp0 - cp_loss
V1      = V0 - v_loss
Cq1     = floor((R1 + G1) / Q)
~~~

Default therefore writes down ownerless accrual, the derived joint fee-rounding reserve, unrealized
whole protocol credit, then pooled supplier value once, at unchanged `N`; it never charges the
standalone `A` to any bucket. Reaching the fee tier clears both complete fractional remainders, even
when `Cq0 == Cp0 == 0`, so defaulted interest cannot later reappear against a claimant. Current
whole backstop credit, deferred backstop credit, and cash-backed `UnownedSurplus` remain unchanged.
Coverage is unchanged exactly when `Y0 + Cq0 + Cp0 + V0 >= delta_L`; otherwise its exact change is
`Y0 + Cq0 + Cp0 + V0 - delta_L`. There is no second ceil-rounded supplier loss and no ordering
between `P0` and `Ks`.

This makes ownerless accrual the first non-claimant loss buffer, the joint rounding reserve and
uncollected protocol revenue the next loss tiers, and supplier value the final internal tier.
Already transferred revenue and whole current/deferred backstop credit are final claims.
Default with `N == V == 0` still reduces liabilities, `Y`, `Cq`, and then `Cp` before coverage,
without division, and never invents supplier value. The former terminal state
`(B,L,V,Y,Cp,H) = (0,0,0,0,2,-2)`, with omitted state zero, instead ends
`(0,0,0,0,0,0)`, so ordinary first supply can reopen the reserve without an unauthenticated
raw-transfer recapitalization race.

### Coverage, `gulp`, and backstop attachment

For each reserve also define, retaining every claimant/exclusion bucket already introduced by ADR
0002:

~~~text
L  = aggregate underlying liabilities
B  = underlying token balance held by the pool
Cb = backstop_credit
D  = aggregate deferred_backstop_credit across former cohorts
X  = unowned_surplus
Y  = ownerless_accrual
Cp = protocol_credit
Cq = floor((protocol_fee_remainder + backstop_fee_remainder) / Q)
H  = B + L - V - Cb - D - X - Y - Cp - Cq
~~~

`H` is a derived signed whole-token coverage value, not stored state. Its mathematical value must
be evaluated with checked or wider arithmetic; an unrepresentable result fails atomically.
`Cq` is the derived zero-or-one joint fee-rounding reserve defined above. `H > 0` is
still-unclassified coverage from conversion rounding or net token inflow; non-cash
zero-supply interest is already classified in `Y` and cannot contribute to positive `H`. Every
reachable committed state has `H >= 0`; a negative observed value is malformed accounting and is
never treated as gulpable surplus or recapitalized by a new supplier. Individual fractional
remainders affect the whole-token balance sheet only through derived `Cq`; the residual numerator
`T < Q` has no whole-token bucket.

On a cold path that reads the reserve-token balance, `B` is that one authoritative contract balance
at the call's ledger. At a supplier-membership boundary inside `submit`, `submit_with_allowance`, or
`flash_loan`, `B` in the equation is instead the request-order `projected_B` defined below: the one
physical observation for that asset plus only scheduled-but-unreflected prefix cash, advanced for
later synchronous transfers. Both forms describe the same logical balance at the classified point;
neither may mix ledger sequences or replay a transfer already reflected in the physical observation.
Normatively, every supplier boundary evaluates:

~~~text
H = projected_B + L - V - Cb - D - X - Y - Cp - Cq
~~~

`Y = ownerless_accrual(asset)` is a zero-initialized, nonnegative, persistent accounting exclusion,
not a claimant, recipient, cash-reserve promise, or supplier-value component. Reserve
initialization writes the zero key atomically; readers never synthesize a missing key as zero. A
committed zero-supply interest split increases it by exactly `residual`; an explicit close or route
disposal increases it by exactly any zero-supply `Cq` release; a zero-supply classification increases
it only by the non-cash amount defined below; and a default decreases it only through `y_loss`.
Repayment, raw cash inflow, withdrawal, and protocol collection otherwise leave it unchanged. It is
never included in `W` or reserved by the collection formula, and no v1 entrypoint transfers or
redistributes it. Its purpose is to preserve provenance and absorb later aggregate default before a
newer supplier cohort; a future spend, release, or reclassification requires a separate deliberate
ADR.

The key is renewed and restored with the reserve accounting footprint. The canonical reserve list
and exact change event give keepers a complete `(pool, asset)` inventory, including a
never-incremented zero key; a missing or archived entry fails closed rather than defaulting to zero.
The view and event make the amount observable even though it is not added to `ReserveData`.

The cumulative balance sheet is therefore:

~~~text
B + L = V + Cb + D + X + Y + Cp + Cq + H
E       = Y + Cp + Cq + H = B + L - V - Cb - D - X
F       = B - Cb - D - X = V - L + E
~~~

This ADR replaces ADR 0002's `Q` allocator everywhere that predecessor invokes it: public `gulp`,
`open_backstop_epoch`, every supplier-membership boundary, `close_backstop_credit`, every positive
deferred-credit realization, and final attachment. After any required reserve load and before the
containing economic action, derive `H` from the authoritative observed or projected balance. If
`H == 0`, allocate nothing. A negative value fails as specified below; in particular, first positive
supply at `N == V == 0 && H < 0` rejects the whole invocation with `BadRequest` before mint or
transfer and cannot use the deposit to hide a prior deficit.

For `H > 0`, use pre-classification `F` and compute:

~~~text
cash_H     = min(H, F)
noncash_H  = H - cash_H
~~~

Both parts are nonnegative because every reachable pre-state has `F >= 0`. Classify them from the
pre-allocation supply, canonical route, and credit epoch exactly as follows:

| Pre-allocation state | State transition | `delta_E` | `delta_F` |
| --- | --- | --- | --- |
| `N == 0` | `X += cash_H`; `Y += noncash_H` | `-cash_H` | `-cash_H` |
| `N > 0`, canonical `Some(backstop)`, current credit epoch open | `Cb += cash_H`; `V += noncash_H` | `-H` | `-cash_H` |
| `N > 0`, no canonical backstop or current credit epoch closed | `X += cash_H`; `V += noncash_H` | `-H` | `-cash_H` |

Every positive branch then has post-classification `H == 0`. Only `cash_H` may enter cash-backed
`X` or `Cb`; non-cash coverage remains observable in `Y` or `V`. The exact `E/F` effects are part of
the normative table, not a shared approximation. In the zero-supply row, `cash_H <= F` and
`cash_H <= H <= E`, so both remain nonnegative. In the open-epoch row, post-state
`E = Y + Cp + Cq >= 0` and `F` decreases only by `cash_H <= F`. The no/open-disabled row has the same
scalar effects: unsolicited physical cash is quarantined in `X`, while only non-cash coverage enters
`V`. Every row preserves the complete balance sheet. In particular, a direct token transfer at
positive supply can never raise `V`, the supplier exchange rate, or the supply cap.

ADR 0002's `cash_alloc` event is correspondingly narrowed: it emits only when `cash_H > 0`, and
its `amount` is exactly `cash_H`. Its recipient is `backstop` for the open-epoch row and `unowned`
for either quarantine row. Its inherited `total` is the post-classification `Cb` or `X` for that
recipient. Non-cash classification is never encoded as cash.
A positive `noncash_H` at `N == 0` emits `ownerless_accrual`; either positive-supply row emits
`supplier_carry` for exactly positive `noncash_H`, including when the canonical `Ks` value happens
not to change. In the zero-supply row, `ownerless_accrual` precedes `surplus_quarantine` when both
parts are positive. In
the positive no/open-disabled row, `supplier_carry` precedes `surplus_quarantine`; the open-epoch row
has no quarantine event.
Every bucket/carry event caused by classification precedes `cash_alloc`; `cash_alloc`, when present,
precedes the containing public or cold-path event. At a membership boundary, any later
membership-owned carry or final-reclassification event occurs after `cash_alloc` and before the
baseline supply/withdraw event. Zero parts emit no corresponding event.

Supplier membership uses the same classifier without adding a balance read per request. Each
admitted invocation keeps one private, non-persistent `projected_B` and one checked
scheduled-but-unreflected prefix per distinct membership asset:

- before first observation, deferred `Supply`, `SupplyCollateral`, and `Repay` inflows add to the
  prefix, while deferred `Withdraw`, `WithdrawCollateral`, and `Borrow` outflows subtract from it;
- a `flash_loan` amount is that asset's logical first unreflected outflow because its liability is
  created before request construction and its physical transfer occurs afterwards;
- a synchronous interest-auction lot before first observation is absent from the prefix because its
  `Cb` decrement and reserve-token transfer are already reflected in the physical balance; and
- the first membership boundary initializes `projected_B` from one token-balance read plus the
  checked unreflected prefix. After initialization, every newly scheduled ordinary flow advances the
  projection immediately, and a later successful synchronous interest-auction lot decrements it once
  alongside the `Cb` decrement and physical transfer.

Thus `FillInterestAuction -> membership` observes the lot exactly once in physical cash, while
`membership -> FillInterestAuction -> membership` advances the initialized projection exactly once.
The current supply or withdrawal transfer remains excluded while `H` is derived and classified and
while its quote and mint/burn are computed; only after that accounting succeeds does the exact
current `tokens_in - tokens_out` advance `projected_B`. Repeated membership requests reuse the
cached observation, so every invocation performs at most one reserve-token balance read per distinct
membership asset. A checked prefix, projection, or `H` failure is `InternalError` and rolls back the
complete invocation. Deferred transfers later realize the modeled sequence; transfer failure rolls
back its accounting and events.

The combined ADR-0002/0003 implementation retains the exact shared
`MAX_SUBMIT_REQUESTS = 2 * MAX_RESERVES = 60` boundary. `submit`, `submit_with_allowance`, and
`flash_loan` run their inherited public authorization and self-address guards first, then reject 61
or more requests with `BadRequest` before pool/user load, flash-liability creation, reserve or
token-balance reads, cross-contract calls, mutation, or events. Exactly 60 reaches request
construction. This bound covers both a repeated same-asset sequence and the healthy 30-collateral
plus 30-liability footprint; ordinary supply remains excluded from `effective_count()` and does not
justify an unbounded request loop.

`E` is derived accounting headroom, not an ownership class: it combines non-claimant `Y`, derived
joint fee reserve `Cq`, uncollected protocol claim `Cp`, and still-unclassified `H`. `F` is cash
after the current backstop, former-backstop, and quarantined-cash exclusions. Every reachable state
requires `E >= 0`, `F >= 0`, and the stronger `H >= 0`; equivalently, `F >= 0` is ADR 0002's physical
reserve `B >= Cb + D + X`, while `E >= 0` follows from nonnegative `Y`, `Cp`, `Cq`, and `H`. These are
cumulative invariants, not extra stored fields. A path that already observes `B` checks them
directly. A hot path that does not observe the token balance proves them inductively from its exact
transition identity and does not add a token or registry cross-call merely to recompute `H`, `E`,
or `F`.

The backstop cap is safe under these final invariants without a hot-path balance read. Subscripts
zero and one denote the pre- and post-allocation states:

~~~text
backstop <= max(V0 - L0, 0) <= F0
F1 = F0 - backstop >= 0
~~~

When `V0 >= L0`, the second inequality follows from `F0 = V0 - L0 + E0` and `E0 >= 0`; when
`V0 < L0`, the cap is zero and `F0 >= 0` already holds. Allocation increases `Cb` by exactly
`backstop` without changing `B`, `D`, or `X`, so `F1 = F0 - backstop >= 0`; its complete
transition identity separately preserves the balance sheet, `H1 >= 0`, and `E1 >= 0`. Genesis has
`H = E = F = 0`. Positive-supply accrual changes `E` by
`protocol + Cq1 - Cq0`; a negative joint-reserve delta is at most one and starts from `Cq0 = 1`, so
`E` remains nonnegative. Zero-supply accrual increases `E` by the full gross accrual because `Cb`
remains zero and `Y + Cp + Cq` receives that accrual. Exact supply, withdrawal, borrow, repayment,
credit reclassification/realization, route changes, and attachment accounting either preserve `H` or
classify only positive `H`; collection sends no more than `E` because its pre-clamp availability is
`E - cash_H` when `V >= L` and `E + V - L - cash_H <= E` otherwise. It also sends no more than
`F - cash_H`, so post-collection `F >= cash_H` and the still-unclassified `H` remains safely
classifiable.

Default preserves nonnegativity of all three scalars directly. Before default,
`L = V + Y + Cp + Cq + H - F`, so `delta_L <= L <= V + Y + Cp + Cq + H`. If the first four loss
tiers cover `delta_L`, `H` is unchanged. Otherwise they are exhausted and
`H1 = H0 - (delta_L - V0 - Y0 - Cp0 - Cq0) >= F0 >= 0`. Default does not move `B`, `Cb`, `D`, or `X`, so
`F` is unchanged; nonnegative `Y1`, `Cp1`, and `H1` give `E1 >= 0`. Accrual and exact cash/accounting
moves preserve `H`; every positive classifier resets it to zero. Thus the invariant is inductive
without a special positive-`Y` reachability predicate.

This ADR deliberately replaces every ADR 0002 gate that treats `V < L` alone as
`PoolError::InternalError`, including its general reconciliation, repayment, default, epoch,
auction, deferred-credit, invariant, acceptance, and verification language. Exact protocol
allocation may reduce `V - L` below zero before a later default writes down unrealized credit. The
final gates are:

| Transition | Replacement accounting rule | Failure behavior |
| --- | --- | --- |
| Reserve load or reconciliation | `V < L` is permitted. Preserve the complete balance sheet and `H/E/F >= 0` through the transition-local identities above. Only cold paths that already observe `B` evaluate the balance-backed predicates directly. | A negative persistent scalar, negative observed `H/E/F`, invalid `N/V` representation, unrepresentable arithmetic, or failed exact identity is `InternalError`; `V < L` by itself is not an error. |
| Borrow, including flash borrow | Let `delta_L = L1 - L0`; the exact supplier delta remains `delta_L - tokens_out`. Apply it to `V` and retain the baseline action, utilization, position-health, and token-transfer checks. | A negative supplier delta or failed identity is `InternalError`. A risk-increasing result still fails through its existing error, including `InvalidUtilRate` or `InvalidHf`; it is not remapped to an accounting error. |
| Repay | Let `tokens_in` be the exact net cash retained after any refund, `delta_L = L0 - L1`, and `r = tokens_in - delta_L`. Require `r >= 0`. At `N > 0`, increase `V` by `r`; at `N == 0`, increase cash-backed `X` by `r` directly. Relative to the loaded state, leave `Y`, `Cp`, `Cb`, `D`, `R`, `G`, and derived `Cq` unchanged. Repayment may start and finish with `V < L`. | A negative `r`, inconsistent liability delta, or failed exact identity is `InternalError`; there is no blanket post-repayment `V >= L` rejection. |
| Default | Apply the exact `delta_L` waterfall to `Y`, derived `Cq`, `Cp`, then `V`; clear both `R` and `G` whenever `delta_L > Y0`; permit `V < L` while preserving `H/E/F >= 0`. | The malformed-state and arithmetic cases remain `InternalError`; `V < L` alone is not an error. |
| Supplier membership | Before each quote and mint/burn, derive `H` from the asset's request-order `projected_B` and run the classifier above without the current transfer. Then enforce the request limit and one-unit economic bound, perform membership accounting, and advance the projection by the exact current flow. | A checked projection or identity failure is `InternalError`. First positive supply at `N == V == 0 && H < 0` is `BadRequest`; any other negative `H` is `InternalError`. Existing mint, burn, liquidity, and health failures remain. |
| `open_backstop_epoch` | Settle every reserve with effective `g = 0`, classify positive `H` through the positive-supply `X/V` or zero-supply `X/Y` split, require every `Cb == 0` and `G == 0`, and then open a fresh epoch with `G = 0`. A valid `V < L` reserve neither blocks opening nor receives new backstop credit until headroom returns. | Existing route, auth, lifecycle, and epoch errors remain. Invalid accounting is `InternalError`; `V < L` is not. |
| `close_backstop_credit` | Settle in canonical order, run the classifier while the current epoch is still open, record and clear every reserve's remaining `G`, then reclassify the entire resulting nonnegative `Cb` to the same creditor's `D`. Cash allocated by the close therefore follows that creditor; non-cash coverage goes to `V` or `Y`. The move preserves the cumulative invariants and remains valid with `V < L`. | Existing caller/epoch failures remain; malformed state, incomplete enumeration, arithmetic, or identity failure is `InternalError`. |
| `claim_backstop_credit` | Before paying the old creditor, the cold path observes `B` and runs the classifier using the pool's current route and epoch, never the caller's former cohort. It then requires the complete balance sheet, `E >= 0`, `F >= 0`, and full backing for the selected positive claim, and decreases `B` and that creditor's `D` equally. A cash-backed claim remains realizable with `V < L`. | ADR 0002's backing, balance-delta, and malformed-state cases remain `InternalError`; `V < L` alone cannot block payment. |
| Create current interest auction | Project each selected reserve through the ordinary non-storing load, quote only its positive `Cb`, and preserve the existing caller, lot, bid, size, price, and minimum-value checks. `V < L` neither changes the lot nor blocks creation. | Existing auction errors remain. Invalid projected accounting is `InternalError`; `V < L` alone is not. |
| Fill current interest auction | Load each lot reserve, require the scaled lot not to exceed its current `Cb`, then decrease `B` and `Cb` by the same amount through the existing transfer. The move preserves `H`, `E`, and `F` and may complete with `V < L`. | Existing filler, auction, transfer, and lot errors remain. A stale or overlarge lot and malformed accounting fail atomically; `V < L` alone is not an error. |
| Create or fill user-liquidation auction | Creation values b-token collateral through `V / N` and preserves the existing health, percent, lot, bid, and price checks. A fill transfers b-token and d-token positions without changing aggregate `N`, `V`, or `L`; on a full fill, inherited user-bad-debt handling moves any uncollateralized remainder to the backstop without defaulting it. | Existing liquidation errors remain. `V < L` alone is not an accounting error and does not bypass health, auction, or user-bad-debt validation. |
| Create or fill bad-debt auction | Creation values the backstop's debt through current aggregate liability conversions and preserves the existing lot, bid, price, and backstop-token checks. A fill's position transfer leaves aggregate `L` unchanged; only the inherited full-fill threshold check may default a remaining backstop position, in which case the exact waterfall above applies. | Existing bad-debt-auction errors remain. `V < L` alone is not an accounting error and does not relax auction or default validation. |
| Final backstop attachment | After old-route settlement, validate the cumulative invariants using the already-observed balance, classify positive `H` through the positive-supply `X/V` or zero-supply `X/Y` split, require `G == 0`, and attach. A valid negative `V - L` is carried into the new route; the accrual cap remains zero until positive headroom returns. | Existing policy, readiness, lifecycle, and resource failures remain. Invalid accounting is `InternalError`; `V < L` is not. |
| `gulp` | Using its existing balance read, validate the cumulative invariants. `H == 0` remains a zero result. At zero supply use the `X/Y` split; at positive supply, send only `min(H,F)` to an open current backstop and the rest to `V`, or quarantine `min(H,F)` in `X` and send only the remainder to `V` when no credit epoch is open. | Invalid accounting, including `H < 0`, is `InternalError`; `V < L` alone is not. |
| `collect_fees` | Require local status and one enabled atomic registry state before reserve work; using its required balance read, validate the cumulative invariants and apply the protected availability formula below. Collection may proceed with `V < L` only for the amount the formula proves available. | Frozen or globally disabled collection is `StatusNotAllowed`; invalid accounting is `InternalError`; zero collectible amount remains `BadRequest`; `V < L` alone is not an error. |

No path may repair `V >= L` by consuming, relabeling, or writing down `Cb`, `D`, or `X`.

Public `gulp` retains load-before-classification. Let subscript `l` denote its projected state
after the load:

- If `H_l == 0`, it returns and emits zero and does not commit that projection; `H_l < 0` is
  `InternalError` with no event or committed load.
- If `H_l > 0` and `N_l == 0`, it applies the exact `cash_H/noncash_H` split regardless of route or
  epoch and never assigns either part to a later supplier, backstop, or protocol recipient.
- If `H_l > 0` with no canonical backstop or a closed backstop credit epoch, and `N_l > 0`, it
  increases `X_l` by `min(H_l,F_l)` and `V_l` only by the remaining
  `H_l - min(H_l,F_l)`, at unchanged `N`.
- If `H_l > 0` with a canonical backstop whose credit epoch is open, it increases `Cb_l` by exactly
  `min(H_l, F_l)` and increases `V_l` by the remaining `H_l - min(H_l, F_l)`. Only physically free
  cash becomes a backstop claim.

A positive branch stores the projected accrual and routing result, returns and emits `H_l`, and
leaves post-state `H == 0`. This deliberately amends ADR 0002's interim zero return for its
supplier/quarantine branches: the final greenfield `gulp` return means the whole routing allocation,
independent of recipient. Classification emits any required ownerless, supplier-carry, or
surplus-quarantine event first, then `cash_alloc` for exactly `cash_H` when positive, then the
unchanged legacy `gulp(asset, amount)` event. The legacy amount excludes claimant growth produced
earlier by the load. An unchanged same-ledger second call returns and emits zero.

The exact uninsured direct-transfer regression is normative: with
`N = V_l = B = 1_800_000_000_000`, no other bucket, and an unsolicited five-unit transfer,
`H_l = F_l = 5`. The no-backstop result is unchanged `V`, `b_rate`, `P0`, and `Ks`, with `X = 5`.
It returns five, emits `surplus_quarantine`, `cash_alloc(unowned, 5)`, then legacy
`gulp(asset, 5)`; an unchanged second call emits only legacy `gulp(asset, 0)`.

`None -> Pending` and cancellation do not alter reserve claims. Final attachment must atomically,
while the route is still `None`:

1. after ADR 0002's membership, pending-identity, compatibility, generation, and `check_config`
   checks, read the pool's current `interest_fee` and `bstop_rate` and require the pair to pass
   the registry's current `check_fees` before any readiness call, accounting write, or route
   change;
2. settle every reserve under the old zero-backstop config;
3. derive coverage including `V`, current and deferred backstop credit, cash-backed
   `UnownedSurplus`, `OwnerlessAccrual`, settled protocol credit, and derived `Cq` exactly once, and
   require the closed old route's `G == 0`;
4. run the canonical classifier while the route is still `None`: at `N > 0`, `cash_H` enters `X`
   and only `noncash_H` enters `V`, while at `N == 0` `cash_H` enters `X` and `noncash_H` enters `Y`; the
   not-yet-canonical candidate receives no pre-cutover value; and
5. change the canonical route to `Some` only after all reserve transitions succeed.

The transition is allowed at the ADR-0002 statuses 0, 2, and 4 and does not depend on the public
Borrow-gated `gulp`. Any failure rolls back reserve state, route state, activation, pending state,
and events. In particular, a policy tightened while funding is pending makes finalization fail
with the pending record intact and no readiness call or event. The maximum 30-reserve transition
must fit the active network's transaction-resource limits with recorded margin. If it does not,
release is blocked and the atomicity design must be revisited in a new ADR; implementation may not
silently introduce a staged cursor or temporary unfreeze.

### Protocol collection

Add the public pool entrypoint:

~~~rust
fn collect_fees(e: Env, asset: Address) -> i128
~~~

Collection is permissionless and pull-based. The registry stores and returns one atomic public
control-plane value:

~~~rust
struct FeeCollectionState {
    recipient: Address,
    enabled: bool,
    epoch: u64,
}
~~~

`collect_fees` first applies its inherited local status gate, then reads
`fee_collection_state()` exactly once. Pool status 4 or greater, `enabled == false`, a missing or
malformed registry response, or a failed registry call rejects before reserve load, balance read,
credit mutation, token transfer, or event. A permitted call loads the reserve, observes `B`,
requires the complete balance sheet and `H/E/F >= 0`, and derives:

~~~text
W = V                    when L == 0
W = max(V - L, 0)        when L > 0
cash_H = min(H, F)
available = max(B - Cb - D - X - cash_H - W, 0)
sent = min(Cp, available)
~~~

`W` is the supplier-withdrawal reserve. Every observable result equals the mathematical value
above; an unrepresentable calculation fails atomically. A zero `sent` is rejected without
transfer. A positive call decreases `Cp` and `B` by exactly `sent`, transfers that amount to the
recipient from the same atomic registry response, emits the event below, and returns `sent`.
Transfer failure is atomic.

Collection therefore cannot worsen a pre-existing current-backstop, deferred former-cohort,
quarantined-surplus, unclassified-cash, or fundable-withdrawal shortfall. If the pool already holds
the mathematical `Cb + D + X + cash_H + W`, that amount remains after collection; otherwise
collection sends zero. Reserving `cash_H` does not classify it or emit a classifier event; it leaves
the later canonical boundary able to route every direct-cash contribution to `X` or the open current
backstop. `Y` is intentionally absent because it is not a withdrawal claim or cash-reserve promise.
Relative to the loaded state, collection leaves `Y`, `R`, `G`, and derived `Cq` unchanged, changes
`B` and `Cp` equally, and therefore leaves `H` unchanged.
When `L == 0`, every supplier-withdrawal payout is at most `V = W`. When `L > 0`, every positive
supplier-withdrawal payout that passes `require_utilization_below_100` leaves `L < V_after`, so that
payout is less than `V - L = W`; a zero-claim exit pays zero.

Recipient ownership is resolved at collection time. If credit accrues while recipient A is
configured and governance rotates to B before collection, all uncollected whole credit goes to B
only after collection has been separately resumed for B. The protocol fractional remainder follows
the same rule when it later becomes whole credit. Transfers already made to A are final.

Add the immediate registry entrypoint:

~~~rust
fn pause_fee_collection(e: Env, authority: Address)
~~~

`authority` must be and authorize either the current registry admin or recovery authority. Every
successful call checked-increments `epoch`, writes `enabled = false`, and emits the pause event,
even when collection was already disabled. Allowing a pause refresh deliberately invalidates a
matured resume from an earlier incident generation. The authority can delay protocol revenue but
cannot redirect, withdraw, write down, or otherwise move any claimant funds.

Resumption is only the typed security action:

~~~rust
struct FeeCollectionResume {
    expected_epoch: u64,
    expected_recipient: Address,
    canary_hash: BytesN<32>,
}

RegistryAction::ResumeFeeCollection(FeeCollectionResume)
~~~

It may be proposed only while collection is disabled, must capture the exact current epoch and
recipient, requires a nonzero digest of the network/recipient/token capability canary, uses the
full `SecurityDelay`, and is vetoable by the recovery authority. Execution rechecks disabled state,
the exact epoch and recipient, the recipient's category invariant, and the nonzero digest, then
writes `enabled = true` without changing `epoch` or `recipient`. A pause refresh or recipient
rotation therefore makes every earlier resume proposal stale; an executable proposal cannot be
pre-staged before the incident it reverses.

### Pool and registry interface

The pool adds:

~~~rust
struct AccountingSnapshot {
    ledger_sequence: u32,
    reserve: Reserve,
    represented_supplier_value: i128,
    supplier_rounding_carry: i128,
    supplier_value: i128,
    liabilities: i128,
    token_balance: i128,
    deferred_backstop_credit: i128,
    unowned_surplus: i128,
    ownerless_accrual: i128,
    fee_rounding_reserve: i128,
    coverage: i128,
    accounting_headroom: i128,
    free_cash: i128,
    withdrawal_reserve: i128,
    collection_available: i128,
    collectible_protocol_credit: i128,
    fee_collection: FeeCollectionState,
}

fn get_interest_fee(e: Env) -> u32
fn ownerless_accrual(e: Env, asset: Address) -> i128
fn accounting_snapshot(e: Env, asset: Address) -> AccountingSnapshot
fn set_interest_fee(e: Env, interest_fee: u32)
fn collect_fees(e: Env, asset: Address) -> i128
~~~

`AccountingSnapshot.reserve` is the same non-storing timestamp projection returned by
`get_reserve`; its `ReserveData` contains `N`, `Cb`, `Cp`, `R`, and `G`. The remaining fields expose
the exact same-ledger `P0`, `Ks`, `V`, `L`, `B`, aggregate `D`, `X`, `Y`, derived `Cq`, `H`, `E`, `F`,
`W`, pre-`Cp`-clamp availability after reserving `cash_H = min(H,F)`, and the atomic registry
collection state used by the formulas above. `collectible_protocol_credit` is
`min(Cp, available)` only when the local status gate permits collection and that registry state is
enabled; otherwise it is zero. The cold view performs one reserve-token balance read and one
registry-state call, returns the current ledger sequence, writes no state, and emits no event. Any
missing bucket, failed projection, malformed registry response, or failed checked identity fails
closed; it never returns a partial snapshot.

The registry adds:

~~~rust
fn fee_collection_state(e: Env) -> FeeCollectionState
fn pause_fee_collection(e: Env, authority: Address)
fn check_fees(e: Env, interest_fee: u32, bstop_rate: u32)
~~~

Accrual and every ordinary supply, withdrawal, borrow, repayment, flash-loan, liquidation, health,
and component projected-view path use the pool's stored fee, route, epoch, and reserve state; none
calls the registry or token contract merely to choose a fee allocation. Registry fee-policy calls
remain confined to deployment, fee configuration, and final attachment; the fee-recipient lookup
and fee transfer exist only in `collect_fees`, while the explicitly cold `accounting_snapshot`
observes the same atomic collection tuple without transferring. Existing token transfers, oracle
reads, flash callbacks, and
the balance reads required by `gulp`, deferred-credit realization, and ADR 0002's request-order
supplier-membership classifier are not fee-routing calls. Membership reuses exactly one balance read
per distinct asset across first, later, and final supply/withdraw boundaries; fee allocation adds no
second read. Non-membership ordinary actions and component projected views add no token-balance
read; `accounting_snapshot` deliberately adds one balance read and one registry call to eliminate
multi-call reconciliation races.

`set_interest_fee` has one exact transition order:

1. require the pool admin's authorization;
2. require the pool-local registry lifecycle to be `Live`; `Contained` and `Retired` return
   `InvalidPoolStatus` before any registry call, reserve load, write, or event;
3. read the stored fee and reject an unchanged value with `BadRequest` before registry or reserve
   work;
4. call the registry's existing `min_config()` view once, validate the changed fee against its
   complete current `min_interest_fee..=max_interest_fee` range, and validate the proposed fee plus
   unchanged stored `bstop_rate` against the code-level individual and combined ceilings. The
   unchanged take is not independently re-rejected against a newer registry maximum;
5. settle every canonical reserve under the old fee and old take; then
6. store the new fee and emit `set_interest_fee` last.

The setter remains callable at every pool status, including frozen status 4, while lifecycle is
`Live`; operators must be able to lower or repair a fee without reopening risk-taking. Any failed
authorization, lifecycle, no-op, policy, reserve-settlement, storage, or event step rolls back the
whole transition, and no elapsed interval is repriced. `check_fees` remains the registry's full-tuple
deployment/finalization predicate; this live setter does not add another registry export or misuse
that predicate for a grandfathered unchanged field.

Existing `update_pool` applies the same changed-field semantics after its inherited auth and
`Live`-lifecycle gate. It reads the existing `min_config()` surface when policy validation is
needed. A changed `bstop_rate` must satisfy the current `max_backstop_take` and the resulting pair's
code ceiling, while an unchanged grandfathered protocol fee is not re-rejected against the current
fee range. Every other changed numeric field satisfies its current policy bound; unchanged fields
do not deadlock an unrelated update. When the take changes, all reserves settle under the complete
old config before the new config is stored. Its existing event remains last.

ADR 0002's final registry wire types retain every current security, lifecycle, canonical-target,
and scoped-seal member and add only the fee fields/variants shown at the end of their containing
types:

~~~rust
struct MinConfig {
    max_backstop_take: u32,
    max_positions: u32,
    min_collateral_floor: i128,
    min_interest_fee: u32,
    max_interest_fee: u32,
}

enum ActionTarget {
    Backstop(Address), Pool(Address), MinConfig, TimelockDelay, SecurityDelay,
    RecoveryConfig, AdminRecovery, PoolImplementation, RegistryCode,
    BackstopAdmission, Onboarding, FeeCollection,
}

enum RegistryAction {
    EnableBackstop(BackstopEnable),
    DisableBackstop(Address),
    SetMinConfig(MinConfig),
    SetTimelockDelay(u64),
    SetSecurityDelay(u64),
    SetRecoveryConfig(RecoveryConfig),
    RecoverAdmin(Address),
    SetPoolImplementation(Implementation),
    UpgradeRegistry(ImplementationChange),
    UpgradeBackstop(TargetUpgrade),
    UpgradePool(TargetUpgrade),
    ResumeOnboarding,
    ResumePoolInflows(Address),
    RetirePool(Address),
    Seal(SealScope),
    SetFeeRecipient(Address),
    ResumeFeeCollection(FeeCollectionResume),
}
~~~

Every omitted supporting declaration—including `SealScope`, `Implementation`, payload structs,
errors, and current interfaces—remains exactly ADR 0002's current definition. No
`RegistryUpgrade`, `BackstopUpgrade(Address)`, `PoolUpgrade(Address)`, `UpgradeSeal`, or
`SealUpgrades` wire member exists. The existing multi-field actions retain ADR 0002's named payload
structs; enum variants are tuple variants. `SetFeeRecipient` and `ResumeFeeCollection` both map to
`ActionTarget::FeeCollection`, so a recipient rotation and a resume cannot race under separate live
target owners. They use the same proposal, one-owner target lock, raw `target_owner` view,
execution, cancellation, invalidation, cleanup, and event ordering as every other typed action.
Successful execution emits `execute`; only stale cleanup/replacement emits `invalid`.
Existing names, including `RegistryImplementation` and `registry_implementation()`, do not change.

`SetFeeRecipient` and `ResumeFeeCollection` are added explicitly to ADR 0002's security-action
class: each requires the current registry admin, uses `SecurityDelay`, and is vetoable by the
recovery authority under the inherited security-action rules. Rotation changes the destination of
all uncollected `Cp`, so allowing it to use the shorter ordinary delay would let a compromised admin
redirect realizable revenue before the exit-length response window. A successful rotation always
checked-increments the collection epoch, stores the new recipient with `enabled = false`, and
invalidates every old resume capability. The longer two-step rotate-then-resume path can postpone
revenue, but credit remains in the pool; that liveness cost is preferred to an uncanaried
funds-redirection path. Proposing the already-current recipient is an invalid no-op; pause refresh,
not recipient self-replacement, is the explicit capability-invalidation path.

ADR 0002's registry event ABI is extended exactly as follows:

- Proposal `kind` is `set_recipient` for `SetFeeRecipient` and `resume_fees` for
  `ResumeFeeCollection`, in both creation and terminal events.
- Immediate pause or pause refresh stores the new state and then emits topics
  `(Symbol("registry"), Symbol("fee_collection"), Symbol("pause"), authority)` with
  `(previous: FeeCollectionState, current: FeeCollectionState)` as data.
- Successful `SetFeeRecipient` execution stores the disabled, incremented-epoch state, then emits
  topics `(Symbol("config"), Symbol("fee_collection"), proposal_id, Symbol("recipient"))` with
  `(previous: FeeCollectionState, current: FeeCollectionState)` as data.
- Successful `ResumeFeeCollection` execution stores `enabled = true`, then emits topics
  `(Symbol("config"), Symbol("fee_collection"), proposal_id, Symbol("resume"))` with
  `(state: FeeCollectionState, canary_hash: BytesN<32>)` as data.
- Only after either action-specific event does execution delete the proposal and its still-owned
  target and emit the matching generic `proposal/execute` terminal defined by ADR 0002. All writes,
  deletions, and events roll back together on failure.

The final registry policy is well formed only when:

~~~text
max_backstop_take < Q
2 <= max_positions <= 60
min_collateral_floor >= 0
0 < min_interest_fee <= max_interest_fee <= 0.20 * Q
max_interest_fee + max_backstop_take <= 0.30 * Q
~~~

Registry construction and `SetMinConfig` validate the whole policy. Deployment and attachment
finalization require the complete actual tuple to comply. Live transitions use changed-field
grandfathering so a prospective tightening cannot deadlock unrelated configuration or prevent an
ordered repair:

- code-level individual and combined ceilings are checked on every transition;
- an unchanged `interest_fee` or `bstop_rate` may remain outside a newer registry bound;
- when either fee field changes, its proposed value must satisfy its complete current registry
  range, so a transition cannot worsen or merely inch toward a still-noncompliant value; and
- `update_pool` validates every other changed field against current policy without re-rejecting an
  unchanged grandfathered fee field.

Thus governance may first lower a grandfathered `bstop_rate`, then raise a grandfathered
`interest_fee`, while an unrelated `max_positions` or `min_collateral` correction remains possible.
The stored pool is never silently rewritten. `check_fees` remains the full-tuple deployment and
finalization predicate; live setters implement the explicit transition rule above rather than
misusing that full-tuple predicate.

The registry stores one `FeeCollectionState` and exposes it atomically. Its recipient may be an
account or an otherwise-unregistered contract, but it must never be:

- the registry itself;
- a registered pool; or
- any known enabled or disabled backstop.

The state is renewed and restored with the registry's canonical configuration footprint. A missing,
archived, non-canonical, or internally inconsistent tuple fails closed for collection, pause,
rotation, resume, and snapshot reads; no caller synthesizes `enabled = true` or epoch zero.

Construction and `SetFeeRecipient` enforce those categories. Execution of a recipient rotation is
authoritative and rechecks them. Pool deployment rejects a deterministic pool address equal to the
current recipient, and backstop enablement rejects a matching address. Archived membership data
fails closed under ADR 0002's restore-then-recheck rule.

Address category and state/epoch binding are the on-chain invariants. Treasury control,
token-receipt capability, and a
usable egress or distribution path are operational release and rotation gates because the registry
cannot prove them. Before genesis, before proposing a recipient rotation, and before admitting a
new reserve asset, operators bind a successful nonzero inbound and treasury-controlled
egress/distribution canary to the network, recipient, token, and—when applicable—recipient Wasm
hash. The digest of the complete current canary bundle is the nonzero `canary_hash` in the fresh
resume proposal. Drift before execution is a veto/cancel condition; drift after activation triggers
an immediate global pause. The implementation runbook owns canary cadence and artifact layout; this
ADR owns the capability and digest-binding requirements.

A suspected current-recipient or registry-admin compromise uses one concrete collection-stopping
path: either independently controlled registry authority calls `pause_fee_collection`, then reads
back the incremented disabled epoch. Every pool consults that state at collection time, so one
unavailable or compromised pool admin cannot keep collection open. Pool admins may additionally set
status 4 as defence in depth, but broad lending freezes are not required to contain a treasury-only
incident. The recovery authority vetoes any hostile fee-collection proposal, and collection remains
disabled until the current recipient is proven safe or a canary-bound replacement and fresh resume
each complete their full `SecurityDelay`. Already collected transfers are final.

The final public construction shapes are:

~~~text
registry.__constructor(
  admin, recovery_authority, registry_implementation,
  current_pool_implementation, blnd_id, min_config,
  timelock_delay, security_delay, recovery_delay, fee_recipient
)

registry.deploy(
  admin, name, salt, oracle, backstop_take_rate,
  max_positions, min_collateral, backstop: Option<Address>,
  interest_fee
) -> Address

pool.__constructor(
  admin, name, oracle, bstop_rate, max_positions,
  min_collateral, interest_fee, registry,
  backstop: Option<Address>, blnd_id
)
~~~

Invalid registry construction writes no state. `deploy` applies `check_config` and `check_fees`
and passes the rate to the pool constructor. The pool constructor inherits ADR 0002's final
constructor-authorization boundary; this ADR adds data but neither chooses nor weakens that
mechanism.
`RegistryGenesisEvent` retains ADR 0002's `security_delay` and every existing field and appends
`fee_collection: FeeCollectionState`, initialized to the configured recipient with
`enabled = false` and `epoch = 0`. Collection begins only after a fresh canary-bound
`ResumeFeeCollection` completes. These changes are part of the single
greenfield v1 boundary; after genesis, fields and variants are not added in place.

### Failure semantics

The public failure ABI reuses existing errors; this ADR adds no error codes
(`pool/src/errors.rs:8-55`; ADR 0002's `RegistryError` definition):

| Failure | Error |
| --- | --- |
| Registry construction rejects fee policy or the initial fee-recipient category | `RegistryError::InvalidConfig` |
| `SetMinConfig`, `deploy`, `check_fees`, or attachment finalization rejects fee policy | `RegistryError::InvalidConfig` |
| `propose(SetFeeRecipient)` or `propose(EnableBackstop)` supplies an address that already violates the fee-recipient category invariant | `RegistryError::InvalidConfig` |
| `propose(SetFeeRecipient)` supplies the current recipient | `RegistryError::InvalidConfig` |
| `execute(SetFeeRecipient)` or `execute(EnableBackstop)` finds that intervening membership or recipient state now violates the category invariant | `RegistryError::InvalidState` |
| `propose(ResumeFeeCollection)` supplies a zero canary digest | `RegistryError::InvalidConfig` |
| `propose(ResumeFeeCollection)` is called while enabled or does not bind the current epoch and recipient | `RegistryError::InvalidState` |
| `execute(ResumeFeeCollection)` finds collection enabled or its expected epoch, recipient, or canary-bound proposal state stale | `RegistryError::InvalidState` |
| An immediate pause/refresh or recipient rotation cannot checked-increment the collection epoch | `RegistryError::InvalidState` |
| `deploy` derives a deterministic pool address equal to the current fee recipient | `RegistryError::InvalidConfig` |
| Pool construction, `set_interest_fee`, or `update_pool` rejects a pool-local fee or code-ceiling pair | `PoolError::InvalidPoolConfigArgs` |
| `set_interest_fee` is called with the stored fee | `PoolError::BadRequest` |
| `set_interest_fee` is called while local lifecycle is `Contained` or `Retired` | `PoolError::InvalidPoolStatus` |
| `submit`, `submit_with_allowance`, or `flash_loan` receives 61 or more requests after its inherited auth/self-address guards | `PoolError::BadRequest` |
| A non-membership request has `limit != 0`, or a membership request has `limit <= 0` | `PoolError::BadRequest` |
| First positive supply at `N == V == 0` finds negative pre-supply coverage `H_pre < 0` | `PoolError::BadRequest` |
| Positive supply at `N > 0, V = 0`, computes zero or fewer than `min_b_tokens_out`, or breaches either one-unit membership-loss bound | `PoolError::InvalidBTokenMintAmount` |
| A zero or otherwise invalid b-token burn, a burn above `max_b_tokens_burned`, or a partial burn breaches the one-unit membership-loss bound | `PoolError::InvalidBTokenBurnAmount` |
| A borrow or flash borrow fails its unchanged utilization or position-health boundary, including a risk-increasing attempt from `V < L` | Existing `PoolError::InvalidUtilRate` or `PoolError::InvalidHf`, as applicable |
| Borrow or repayment derives a negative rounding remainder, an inconsistent aggregate-liability delta, or a failed transition identity | `PoolError::InternalError` |
| `collect_fees` is called while the pool is frozen or atomic registry state has collection disabled | `PoolError::StatusNotAllowed` |
| `collect_fees` derives zero collectible amount | `PoolError::BadRequest` |
| `accounting_snapshot` cannot obtain one complete projected accounting/registry/balance tuple or its checked identities fail | `PoolError::InternalError` or the unchanged nested call failure, as applicable |
| A restored `OwnerlessAccrual` value is missing or negative, or its checked increase/write-down is unrepresentable | `PoolError::InternalError` |
| A path that already observes `B` finds `H < 0`, `E < 0`, or `F < 0` | `PoolError::InternalError`, except the explicitly mapped first-supply `BadRequest` |
| A request-order cash prefix, projection, or projected-coverage calculation is unrepresentable or inconsistent | `PoolError::InternalError` |
| Checked arithmetic is unrepresentable, or another internal accounting identity or bound is inconsistent | `PoolError::InternalError` |

Each failure is atomic and emits no success event. A recipient-category failure at proposal writes
no proposal or target. An execution-time `InvalidState` retains the existing proposal and its owned
target under ADR 0002's ordinary failed-execution rule, permitting explicit cancel, veto, expiry
cleanup, or a later retry if the conflicting state is removed. Authorization failures remain
host-auth failures, not remapped contract errors. Nested registry failures propagate unchanged.
`V < L` is not itself a failure. Negative observed `H` is malformed except for the first-supply
failure mapping above; all other validity follows the transition-specific gates and cumulative
predicates.

### Events and observability

Existing borrow, repay, flash-loan, and gulp wire shapes remain unchanged. The gulp amount is the
post-load routing allocation described above, not a whole-call claimant delta or coverage view.

New pool events are:

- `set_interest_fee`: topics `("set_interest_fee", admin)`; data
  `(old_interest_fee, new_interest_fee)`. The new fee is stored first; this is the transition's
  final event and nothing follows it.
- `ownerless_accrual`: topics
  `(Symbol("pool"), Symbol("ownerless_accrual"), asset)`; data
  `(change: i128, total: i128)`. `change` is positive for a committed zero-supply interest residual,
  zero-supply `Cq` release, or non-cash positive-`H` classification and negative for the exact
  default write-down; zero never emits. The amount is stored before publication. This event precedes
  any same-transition surplus quarantine and the containing public action's existing success event,
  including `defaulted_debt`; all writes and events roll back together on failure.
- `protocol_credit_write_down`: topics
  `(Symbol("pool"), Symbol("protocol_default"), asset)`; data
  `(amount: i128, remaining_protocol_credit: i128, discarded_fee_remainder: u32)`. It emits when
  default decreases `Cp` or clears a nonzero `R`; `amount` is the nonnegative `cp_loss`. Zero in
  both fields does not emit.
- `backstop_fee_remainder_discard`: topics
  `(Symbol("pool"), Symbol("backstop_remainder"), asset)`; data
  `(epoch: u64, reason: Symbol, discarded_numerator: i128)`, where `reason` is exactly `cap`,
  `close`, `route`, or `default`. It emits only for a positive discarded numerator and after the
  reserve stores `G = 0`.
- `collect_fees`: topics `("collect_fees", asset, recipient)`; data
  `(amount, remaining_protocol_credit, protocol_fee_remainder, fee_collection_epoch)`.

Default-owned scalar events follow the economic waterfall: negative `ownerless_accrual`, any
`backstop_fee_remainder_discard(default)`, `protocol_credit_write_down`, then any supplier-carry
change, then the inherited
`defaulted_debt` event. A close or route transition emits every per-reserve backstop-remainder
discard, then any `Cq`-release ownerless/supplier event, before its credit-epoch or route event. A
committed cap disposal emits after the projected reserve is stored and before the residual's
ownerless/supplier event and containing public action; a non-storing projection emits nothing.

ADR 0002's `cash_alloc` shape is unchanged, but its amount is now exactly `cash_H`, never total
positive `H`. It does not emit when `cash_H == 0`. Replay derives any non-cash supplier portion from
the classification's `supplier_carry` delta and the paired `cash_alloc`, and derives an ownerless
portion directly from `ownerless_accrual`. Classification-owned `ownerless_accrual`,
`surplus_quarantine`, and `supplier_carry` events precede `cash_alloc`; `cash_alloc` precedes the
containing action. The mandatory supplier-classification event is emitted even when the resulting
canonical `Ks` is unchanged, so no non-cash portion is hidden in a rate-only transition.

Within each submit transaction, replay tracks the same per-asset unreflected prefix and lazy
`projected_B`. It treats a pre-observation interest-auction lot as already reflected, applies a
post-initialization lot exactly once, excludes the current membership flow until classification and
mint/burn complete, and then advances by that exact flow. It rejects a bucket/carry event after its
paired `cash_alloc`, a `cash_alloc.amount` different from recomputed `cash_H`, an omitted non-cash
classification event, more than one physical balance observation per membership asset, or any
request-side effect from a 61+-request call.

Component views remain useful but are not the reconciliation boundary. For a captured ledger
sequence, `get_reserve(asset)`, `supplier_rounding_carry(asset)`, and
`ownerless_accrual(asset)` each run the same timestamp-advancing, non-storing `Reserve::load`
projection and return their component from that projection. ADR 0002's `unowned_surplus(asset)` and
`deferred_backstop_credit(backstop, asset)` remain committed reads because neither value changes in
a reserve load. Clients and operators use `accounting_snapshot(asset)` for complete reconciliation;
assembling separate calls or mixing ledgers has no specified meaning. The snapshot returns every
component and the ledger sequence in one response, including `G` and the atomic fee-collection
state.

Projection writes no storage and emits no event. Supplier-carry and ownerless-accrual events
reconstruct committed state only. Immediately after a successful transaction stores a reserve at
the captured ledger, replayed committed `Ks` and `Y` agree with all three projected views because no
additional time has elapsed. At a later ledger with no committing load, the projected views may
legitimately lead event replay; a later successful commit at that same ledger stores the exact
projection and emits the required scalar changes. No new health or coverage event is added. Clients
and indexers that value supplier positions must use `V / N`; `b_rate` alone is no longer a complete
quote.
A positive-`H` empty-epoch classification occurs atomically with the successful first-supply event
and uses the ownerless and/or ADR 0002 quarantine events followed by any positive `cash_alloc` in
the ordering above; pre/post reserve state distinguishes both parts from the deposited amount.
Monitoring alerts on `N == 0, L > 0` and reconciles each committed load's `Cp`, `R`, `G`, `Y`, and
unchanged-`H` deltas. It separately reconciles later positive coverage through the exact `X/Y`
split. Any positive `Cb` allocation in that state, residual or non-cash coverage missing from `Y`,
movement from `Y` to `X`, or default that does not consume `Y`, then `Cq`, then `Cp`, before `V` is an
accounting incident.
One-ledger snapshots also alert on any `H < 0`, `E < 0`, or `F < 0`;
projected-versus-committed drift is informational until a same-ledger snapshot or immediate
post-commit replay disagrees. Classification replay also
requires the exact branch-specific `delta_E/delta_F`; nonnegative post-state alone is insufficient.
Registry raw-XDR replay recognizes immediate pause/refresh plus `set_recipient` and `resume_fees`,
applies each action-specific state transition, and then consumes its matching generic terminal. At
the captured tip it must reproduce the complete `fee_collection_state()` as well as proposal and
target-lock state. Pool collection replay requires its event epoch and recipient to equal the
registry state observed by that transaction.

### Explicitly out of scope

Origination and flash-loan fees; supply, withdrawal, liquidation, or oracle fees; per-reserve fee
overrides; per-position interest attribution; clawback of already transferred protocol revenue or
an alternative proportional default waterfall beyond ownerless accrual, the joint fee reserve,
uncollected protocol credit, then pooled supplier value;
BLND issuance, buyback, or burn; treasury splitter economics; implementation structure; migration
of any non-disposable deployed layout, whose discovery blocks this greenfield release.

## Alternatives

Options are scored against the ranked drivers: (1) durable interest revenue, (2) claimant and
freeze safety, and (3) minimal hot-path and maintenance surface.

| Option | 1 | 2 | 3 | Assessment |
| --- | --- | --- | --- | --- |
| Interest skim in accrual, defaultable credit, globally pausable protected pull collection (chosen) | strong | strong | mixed | Reuses the one accrual path and keeps ordinary requests free of fee-routing cross-calls. It adds public accounting state, pro-rata b-token conversions, collection, and governance. |
| Charge only realized interest during repayment | mixed | strongest | weak | Avoids an uncollected credit through default, but the current model has no per-position principal/interest basis. Exact attribution expands repay, transfer, liquidation, and default state. |
| Add origination and flash-loan fees now | mixed | weak | weak | Adds debt/default exposure and authorization-sensitive callback changes without improving the ranked interest-revenue source. Deferred to a separate decision. |
| Reuse `backstop_credit` and split auction proceeds | weak | mixed | mixed | Avoids a new credit field but cannot serve a `None` pool and obscures claimant ownership. |
| Transfer the fee during every accrual | strong | mixed | weak | Produces immediately realized revenue but adds registry and token liveness to every reserve load and ordinary submit. |
| Let protocol collection race existing backstop claims | strong | weak | strong | Needs no reserve formula, but a permissionless caller can make a stored interest-auction claim unfundable. |
| Share scarce liquidity proportionally with the backstop | strong | mixed | mixed | Avoids strict backstop priority, but changes an existing auction claim after creation or requires another reservation mechanism. |
| Floor every protocol and backstop interval and give dust to suppliers | weak | mixed | strongest | Smallest implementation, but caller-selected settlement cadence can erase an unbounded amount from either claimant. |
| Carry protocol and backstop fractions independently without `Cq` | strong | weak | strong | Each claimant is cadence-independent in isolation, but both carries can mature in one one-unit interval and make total fees exceed current accrual, forcing supplier value down. Derived `Cq` withholds the already-jointly-earned unit without another stored scalar. |
| Keep uncollected protocol credit senior and add authenticated recapitalization/write-down calls | strong | weak | mixed | Preserves face revenue through default but can leave first supply permanently closed and turns unrealized borrower interest into a claim on later supplier cash. A new authority surface repairs symptoms rather than the loss waterfall. |
| Freeze each pool at status 4 to contain recipient compromise | strong | weak | strong | Adds no registry state, but one unavailable pool admin leaves permissionless collection open and a treasury-only incident unnecessarily freezes broader lending activity. |
| Allow raw cash donations to raise `V` and rely only on caller `min_b_tokens_out` | strong | weak | strong | Conserves assets, but a stale or permissive limit can authorize a large newcomer-to-incumbent transfer and direct cash can manipulate the supply cap. Quarantine plus a code-level one-unit bound is required. |
| Add a permanent virtual-share/decimal offset | strong | strong | mixed | Structurally reduces nearly-empty inflation, but creates permanent phantom-share and precision policy that every quote, cap, loss, and migration must carry. Explicit cash quarantine and a one-unit economic bound provide the required protection without a second ownership base. |
| Reuse cash-backed `UnownedSurplus` for zero-supply non-cash residual | strong | weak | strong | Avoids one persistent amount and view, but can make `X > B`, violates ADR 0002's physical-reserve meaning, and makes collection and monitoring unable to distinguish cash from liability-created value. |
| Convert ownerless accrual to `UnownedSurplus` once liabilities reach zero | strong | mixed | mixed | Eventually turns fully repaid ownerless value into a cash-backed exclusion, but partial repayments have no attributable principal/interest basis, default or collection may intervene, and the transition changes the future loss buffer according to call ordering. Preserving `Y` is simpler and deterministic. |
| Keep ADR 0002's whole-`H` cash allocation | strong | weak | strongest | Smallest amendment, but protocol/ownerless state lets positive `H` exceed physically free `F`; an open epoch could receive an unfunded claim and zero supply could overstate cash-backed quarantine. |
| Read physical cash again at every membership request | strong | strong | weak | Avoids a projection cache but reads balances repeatedly, still needs reflected-versus-deferred ordering, and makes the exact 60-request boundary less likely to fit. One lazy read per distinct asset is equivalent and bounded. |
| Use full `check_fees` for every live config change | mixed | weak | strong | Reuses one registry predicate, but a tightened bound on either unchanged fee field can deadlock the other field and unrelated numeric repairs. Changed-field validation preserves policy without rewriting live state. |
| Permit same-value `set_interest_fee` as a settlement trigger | neutral | mixed | weak | Makes an expensive all-reserve no-op externally callable by the admin and gives it ambiguous event semantics. Explicit `BadRequest` keeps settlement attached to a real configuration change. |

The supplier representation has its own decisive tradeoff:

- An additive `b_rate` floor is smallest but makes `gulp` productive across repeated unchanged
  calls.
- A reserve residual excluded from holder quotes preserves the supplier class while transferring
  value between suppliers according to entry and exit timing.
- Per-account basis or a second holder index can preserve ownership, but adds storage, transfer,
  emission, liquidation, and migration complexity.
- Exact pooled `V` with fungible `V / N` ownership is chosen because it preserves individual
  economic ownership with aggregate state. It costs changes to every b-token valuation consumer
  and permits only the explicit one-base-unit floor/ceil loss bound. Authorization-bound mint/burn
  limits may be stricter.

Rejecting a positive withdrawal while a holder's floored claim is zero would strand shares and
their emissions until the ratio recovers. The chosen full-position cleanup lets the holder donate
that sub-unit rational claim to the remaining cohort without weakening liquidity or health; it
keeps zero request amounts invalid so accidental no-op requests do not become state transitions.

An empty supplier epoch needs two ownership rules because cash and non-cash value have different
backing. Giving either to the first depositor rewards entry timing; assigning either to the current
backstop or protocol reclassifies value that may predate that claimant; and an implicit
first-deposit haircut makes an ordinary supply recapitalize an old deficit without consent. The
chosen rule places only physically free positive coverage in ADR 0002's `UnownedSurplus`, records
the zero-supply non-cash remainder in distinct `OwnerlessAccrual`, and rejects first supply when
derived coverage is negative.
`OwnerlessAccrual` is written down before later supplier value on default but has no spend path.
This preserves the cash-backing meaning of `UnownedSurplus`, keeps a new cohort at unit value, and
adds one scalar/view/event instead of corrupting an existing public accounting contract.

The same distinction applies outside an empty epoch. Classifying all positive `H` as cash is valid
only when `F >= H`; protocol credit and ownerless accrual make that premise false in reachable
states. The chosen `cash_H/noncash_H` split is therefore applied at every inherited allocator
boundary. Unsolicited `cash_H` enters `X` unless an open current backstop epoch owns it; it never
raises incumbent supplier value. Only `noncash_H` can enter `V`. This prevents both unfunded claims
and donation-driven supplier-rate manipulation while reusing the existing bucket/carry plus
`cash_alloc` event surface.

For attachment, an operator pre-`gulp` convention loses atomicity and fails at valid Borrow-disabled
statuses. A staged cursor bounds each transaction but introduces partial-progress, cancellation,
TTL, and ownership states. The chosen atomic transition has the smallest state machine and the
strongest ownership boundary, provided the measured maximum-reserve transaction fits the live
network. Failure to fit requires a new decision, not an implementation workaround.

The chosen option follows the ranked drivers: it earns from all pools, preserves existing insurance
priority and fungible supplier ownership, and confines new fee-routing cross-contract work to cold
governance and collection paths. The request-order membership observation inherited from ADR 0002
serves claimant classification, not fee routing, and remains one read per distinct asset.

## Consequences

Every insured and uninsured pool can accrue protocol revenue in each borrowed underlying asset
without changing borrow, repay, or flash-loan economics or the flash callback. The cumulative
greenfield `Request` wire changes once to add authorization-bound membership limits. The registry
can change bounded fee policy and the recipient during the maintenance year without a pool upgrade,
and either independent registry authority can stop collection globally without coordinating pool
admins.

The final pool spec is intentionally incompatible with the original and ADR-0002-only public
types. Direct clients, fixtures, generated bindings, and indexers move once to the combined
greenfield layout. Registry consumers use scalar views where they do not need the full pool type.

Pooled supplier value is exact and fungible, but `b_rate` is no longer a sufficient quote. Raw cash
cannot raise incumbent supplier value, and every mint or burn has both caller slippage protection
and a protocol-wide one-base-unit economic-loss ceiling. Protocol and current-epoch backstop carries
bound fee loss caused by settlement cadence. These guarantees require more
arithmetic and change every consumer of supplier conversion. A protocol fee also lowers supplier
yield relative to a zero-fee pool and can make a marginal position unhealthy or liquidatable
earlier; that is an intended economic effect. `Cp` contains only whole protocol allocations not yet
collected. Cash-backed empty-epoch value remains quarantined in `UnownedSurplus`; non-cash
zero-supply residual, joint-reserve release, and classification remainder remain separately visible
in `OwnerlessAccrual` and absorb default before a later supplier cohort.

Protocol credit is face value until collected or default reaches its tier. Ownerless accrual, the
derived joint fee reserve, and uncollected protocol credit absorb loss before supplier principal;
already transferred revenue is final. High utilization, current or deferred backstop credit,
quarantined surplus, supplier withdrawal reserves, a pool freeze, or the registry collection pause
can delay collection indefinitely. A hostile recovery authority can refresh the pause and deny
revenue, but cannot move or redirect funds; this is the same deliberate liveness-only power as its
veto role. Revenue is not called realized while it remains credit.

`accounting_snapshot` makes one complete ledger-pinned reconciliation available to operators and
indexers. It is intentionally cold: unlike component projections it reads the token balance and
registry state, but ordinary economic paths gain no monitoring-only call.

Atomic attachment preserves ownership across `None -> Some` but places worst-case work in one rare
transition. Live resource evidence is therefore a release condition.

Every submit containing supplier membership carries the predecessor's one balance observation per
distinct membership asset plus checked request-order projection; repeated requests reuse that read.
This is bounded by 30 assets and the shared 60-request cap, but exact-60 actual-Wasm evidence is a
genesis gate. Fee and take changes likewise remain atomic all-reserve operations. Rejecting a
same-value fee avoids turning that expensive settlement into a no-op API, while allowing status-4
`Live` pools to call the real setter preserves an incident-repair path.

### Risks and controls

| Risk | Owner and trigger | Prevention and detection | Recovery |
| --- | --- | --- | --- |
| Excessive fee harms suppliers or pushes marginal positions into liquidation | pool admin and registry governance; fee change | 20% protocol and 30% combined code ceilings, tighter registry policy, settle-before-change, fee-transition health monitoring | lower the prospective fee; halt new borrowing while separately governed risk parameters are assessed |
| Default misorders unrealized revenue and supplier loss or lets fractional credit reappear | pool risk admin; default with nonzero claims | exact `Y -> Cq -> Cp -> V` order, clear `R/G` at the fee tier, `H/E/F >= 0`, exact write-down events, and default/reopen vectors | block release; if deployed, pause collection and risk-taking before a separately reviewed compatible repair |
| An inherited `V >= L` guard blocks repayment or a cash-backed former-backstop exit | implementation owner; fee accrual creates `V < L` | one cumulative replacement-gate table, exact repayment/epoch/claim vectors, and failure-ABI tests | block release; remove the stale guard without weakening ordinary borrow/withdraw utilization or health checks |
| Backstop accrual exceeds physically free cash after the blanket `V >= L` invariant is removed | implementation owner; positive backstop allocation | prove `H/E/F >= 0` across every transition; cap by `V - L`; carry only within the open epoch; record complete cap/close/route numerator disposal | block release; a different cap or hot-path balance read requires a revised decision and resource evidence |
| Supplier value is diluted, inherited, or orphaned | implementation owner; donation, accrual, empty-epoch entry, mint, burn, default, or auction transfer | `V / N` is authoritative; unsolicited cash enters only `X` or an open current `Cb`, signed request limits and one-unit bounds protect membership, and the exact `1/50/100/75` witness plus cohort properties are tested | block release; if deployed, pause inflows and collection and use a separately reviewed compatible accounting repair |
| Liability-created ownerless value is mistaken for cash or fee revenue | treasury operations; zero-supply accrual, first supply, `gulp`, default, or replay | distinct `Y` view/event and TTL inventory; reconcile `Cp`, `Cq`, `X`, and `Y` as separate causes; assert `B >= Cb + D + X` independently | correct reporting; if state crossed buckets, freeze and use a reviewed accounting repair rather than relabeling history |
| Request ordering counts a deferred or synchronous transfer twice, or classifies non-cash `H` as cash | implementation owner; mixed submit with membership, auction fill, repay/borrow, or flash prefix | one-read-per-asset projection, reflected/unreflected order vectors, exact branch-specific `delta_E/delta_F`, `cash_H/noncash_H` event replay, and 60/61 pre-effect boundaries | block release; if deployed, freeze affected pools and use a reviewed accounting repair rather than relabeling buckets |
| Live fee repair is blocked or reprices elapsed accrual | implementation owner and pool admin; fee/take policy tightening or frozen-pool repair | local lifecycle and no-op gates before external/reserve work, changed-field `min_config()` validation, old-config all-reserve settlement, and event-last tests at every pool status | block release; restore the last compatible implementation or use the still-live ordered setter after the defect is repaired |
| Projected reserve and scalar views are compared at different ledgers or against committed event replay | client and indexer owners; time advances without a committing load | canonical `accounting_snapshot` with its ledger sequence, same-load projected `Ks/Y`, committed-only replay semantics, and no-commit/catch-up vectors | discard separately assembled observations and fetch a fresh atomic snapshot; do not rewrite committed history to match a projection |
| Collection makes a current/deferred backstop claim, quarantine, unclassified cash, or withdrawal unfundable | implementation owner; positive collection | on-chain `Cb + D + X + cash_H + W` reserve and pre/post balance checks | block or freeze; repair only through a reviewed compatible upgrade |
| Recipient or registry admin is compromised, the recipient cannot receive, or it becomes a sink | treasury operations and recovery authority; unexpected proposal or key/code/token/control drift | atomic enabled/epoch/recipient state, immediate admin-or-recovery pause/refresh, state-bound delayed resume, recovery veto, category checks, canary digest, and destination/proposal monitoring | pause globally, veto hostile proposals, restore capability or complete delayed rotate-then-resume; pool status 4 is defence in depth and collected funds are not clawed back |
| Recovery authority griefs protocol revenue by refreshing the pause | registry admin and treasury operations; unexpected epoch increments while already disabled | distinct custody, exact pause events, epoch alerts, and ADR 0002's delayed recovery-authority rotation | leave credit in pools, rotate the authority through the inherited security path, then create a fresh canary-bound resume; never bypass the epoch |
| Attachment transfers old uninsured value to the backstop | implementation owner; `None -> Some` | atomic old-route settlement, closed-route `G == 0`, and pre/post `N,V,H,Cb,D,X,Y,Cp,Cq,R` reconciliation | block release; if deployed, freeze and repair through a reviewed upgrade |
| Atomic attachment exceeds network limits | implementation owner; maximum reserve count | simulate the optimized contracts against active limits with margin | block release and return to architecture; do not ship a hidden staged state machine |
| Credit arithmetic reaches its numeric limit | implementation and treasury operations; long-uncollected credit | checked arithmetic, projected-credit alerting, collection before the operational margin | freeze affected risk-taking and use a reviewed repair; never wrap, saturate, or reallocate |
| Registry policy or recipient metadata is archived | registry keeper; TTL expiry | ADR 0002 renewal and fail-closed reads | restore the exact footprint, re-evaluate, and retry |
| Wasm or public-surface growth exceeds the budget | implementation owner; upstream change | measure and attribute the optimized delta; avoid hot-path calls and duplicate APIs | delete or narrow implementation choices; every artifact must meet the effective network limit |

### Pre-mortem

1. **Settlement cadence or an epoch reset changes claimant ownership.** Warning: one settlement of
   ten differs from ten settlements of one for either fee claimant, `G` survives close/route/cap,
   a first/later supplier claims pre-entry value, or liability-created residual appears in
   cash-backed `X`. Mitigation: reconcile both numerator identities and every explicit disposal,
   the `Y` change stream, first-entry `H`, and every holder through `V / N`. Recovery: reject release;
   a deployed reinterpretation requires a reviewed accounting upgrade.
2. **Collection passes unit tests but drains live auction or withdrawal liquidity.** Warning:
   `B < Cb + D + X + cash_H + W` after collection when the inequality held before, a pre-call-
   fundable current/deferred backstop claim or supplier withdrawal fails, or collection makes a
   direct donation unavailable to the later classifier. Mitigation: exercise full, partial, zero,
   high-utilization, direct-donation, and boundary-`H` cases against real token balances. Recovery:
   freeze and repair before collection resumes.
3. **A fee change reprices elapsed interest.** Warning: the interval since `last_time` uses the
   new rate. Mitigation: compare old-rate settlement and post-change accrual at the boundary.
   Recovery: reject release; any deployed accounting correction needs a new ADR.
4. **A hostile or broken recipient drains or strands revenue.** Warning: an unexpected
   `set_recipient` or `resume_fees` proposal, recipient-control drift, a failed egress canary, a
   collection event after global pause, or a resume whose epoch/recipient predates the incident.
   Mitigation: atomic collection state, immediate pause refresh, state-bound `SecurityDelay`,
   recovery veto, proposal/destination alerts, and token/code-bound canary digests. Recovery: pause
   globally, veto hostile proposals, and keep collection disabled until capability is restored or a
   safe rotate-then-resume completes; one unreachable pool admin does not weaken containment.
5. **Attachment succeeds after reclassifying uninsured supplier value.** Warning: pre-cutover
   positive `H` becomes backstop credit for a reserve with holders, or an unchanged post-cutover
   `gulp` is productive. Mitigation: one atomic old-route settlement and claimant reconciliation.
   Recovery: reject release or freeze and use a separately reviewed repair.
6. **Aggregate conservation hides a holder-to-holder transfer.** Warning: the exact
   `1 / direct 50 / supply 100 / withdraw 75` trace mints, a newcomer loses more than one base unit,
   an incumbent gains more than one through mint rounding, a burn donates above its bound, or an
   auction transfer changes `N/V`. Mitigation: quarantine cash, bind mint/burn limits to auth, enforce
   the independent one-unit bounds, and test severe-loss cleanup plus both exit orders. Recovery:
   reject release; off-chain reinterpretation is not a fix.
7. **The default waterfall bricks a reserve or resurrects revenue.** Warning: a default reduces
   `V` while `Y`, `Cq`, or `Cp` remains, leaves `R/G` nonzero after reaching the fee tier, recreates
   the former
   `(0,0,0,0,2,-2)` terminal, or `Y` silently returns as zero after restoration. Mitigation:
   exercise direct accrual, cash/non-cash classification, repayment, attachment, collection,
   partial/full `Y -> Cq -> Cp -> V` write-down, ordinary reopen, TTL restore, and replay vectors.
   Recovery: block release; if deployed, pause collection and freeze affected pools before
   restoring the exact key or using a separately reviewed accounting repair.
8. **Removing `V >= L` silently weakens physical backstop backing or blocks exit-preserving calls.**
   Warning: `H/E/F < 0` after accrual, or a valid `V < L` repayment, current-interest-auction
   create/fill, epoch close, deferred-credit claim, bad-debt fill, or liquidation fill returns
   `InternalError`. Mitigation: prove the `H/E/F` induction and every replacement-gate vector.
   Recovery: reject release; a deployed
   discrepancy requires containment and a separately reviewed accounting repair.
9. **Projected views appear internally inconsistent or event replay appears stale.** Warning:
   `accounting_snapshot` fails an identity, reports a different collection tuple than the registry,
   or operators compare a later projection to the last committed event total. Mitigation: use the
   atomic snapshot and test no-commit projection followed by exact same-ledger catch-up. Recovery:
   discard separately assembled observations, refetch, and investigate only if the atomic result or
   immediate post-commit replay disagrees.
10. **A mixed request sequence double-counts cash or creates an unfunded backstop claim.** Warning:
    `FillInterestAuction -> membership` differs from a balance-equivalent ordering, a later auction
    lot decrements projection twice or not at all, `cash_alloc > F`, or an open epoch receives
    `noncash_H`. Mitigation: one-read-per-asset projected cash, exact reflected/unreflected order
    vectors across all three submit forms, branch-specific `delta_E/delta_F` assertions, and
    bucket/carry plus cash-allocation replay. Recovery:
    reject release; if deployed, freeze and use a reviewed accounting repair.
11. **A fee repair is unavailable precisely when governance needs it.** Warning: frozen status 4
    blocks `set_interest_fee`, a `Contained` call reaches the registry, an unchanged call settles
    reserves, a tightened unchanged take deadlocks the fee setter, or the event appears before all
    old-fee settlement. Mitigation: test the exact auth/lifecycle/no-op/policy/settlement/store/event
    order and changed-field repair sequence. Recovery: reject release or restore the last compatible
    implementation before retrying the atomic repair.

### Invariants

| Required invariant | Disposition |
| --- | --- |
| Reserve `b_rate` and `d_rate` do not decrease except explicit loss paths | Strengthened for nonempty, non-loss supplier transitions: exact `V` growth cannot lower the canonical `b_rate`. Default may lower it; the empty unit reset is a disclosed non-economic boundary. |
| A healthy `submit` is not immediately liquidatable | Preserved using pro-rata `V / N` in both validation and liquidation. A fee can make fewer positions pass than a zero-fee counterfactual. |
| Supplied value covers borrowing at action boundaries | Deliberately narrowed. New-borrow and positive-withdrawal checks still use exact `V`, but elapsed protocol-fee accrual and explicit default may produce `V < L`. Zero-supply residual is excluded in `Y`; the complete balance sheet and `H/E/F >= 0` replace the unsupported blanket solvency claim. No risk-increasing action may exploit `V < L`, while repayment, liquidation, current-interest-auction fill, and cash-backed exits remain live. |
| Backstop-take and interest arithmetic do not underflow or round in the protocol's disfavour | Strengthened by checked arithmetic and exact protocol plus current-epoch backstop numerator identities. Accrual obeys `backstop <= max(V0 - L0, 0) <= F0`; positive-coverage classification credits only `cash_H <= F0` to `Cb`. Cap, close, and route disposal are explicit and leave `G = 0`, so every new backstop credit is physically backed; absent a binding cap, cadence dust is less than one base unit per open epoch. |
| `flash_loan` leaves every reserve at least as solvent as its post-accrual pre-loan state | Preserved; no flash fee, callback, transfer, or authorization-root semantics change, and validation uses exact supplier value. The shared `Request` wire gains `limit`, which must be zero for non-membership requests. A flash outflow is projected once before membership classification, and 61+ requests fail before liability creation or cash effects. |
| Backstop balance, queued shares, and emissions conserve the backstop-token balance | Unchanged; this ADR adds no backstop-token storage or movement. |
| Withdrawal queue lifecycle cannot be skipped | Unchanged. |
| Backstop `draw` and `donate` move funds only for the named pool | Unchanged. |
| Backstop emissions are never over-claimed | Unchanged; `SupplierRoundingCarry` has no separate emission ownership. |
| Factory deployment is deterministic for a salt and uses the pinned pool Wasm | Preserved by ADR 0002; fee input is constructor data, not implementation-hash authority. |

### Acceptance criteria

1. For every valid settled sequence, protocol allocation, current-epoch backstop allocation, both
   remainders, and every explicit discarded numerator satisfy their separate cumulative weighted
   identities; invalid or unrepresentable arithmetic fails atomically. From zero remainders, the
   configured 20% protocol and 30% combined ceilings are never exceeded over cumulative observed
   gross accrual. Backstop credit is capped by pre-interval `V - L`; the complete unallocated
   numerator is recorded and its whole-token value stays in `residual`: supplier-owned at positive
   supply and added to `Y` at zero supply. A numerator above `Q * headroom` binds the cap even when
   its floored whole amount equals `headroom`; the fractional excess is discarded rather than
   carried. The `Cp` ledger separately reconciles its initial balance,
   fee allocations, successful collections, and default write-downs exactly. `Cq` always equals
   `floor((R + G) / Q)` and every change is matched by residual, claimant allocation, epoch
   disposal, or default. The `Y` ledger separately reconciles its initial balance, committed
   zero-supply residual, zero-supply joint-reserve releases, non-cash positive-`H`
   classifications, and default write-downs exactly; neither `X` nor `Cp` receives non-cash
   ownerless value. Every generated valid-state
   sequence preserves the complete balance sheet and `H/E/F >= 0`. Every positive backstop allocation satisfies
   `backstop <= max(V0 - L0, 0) <= F0` against the pre-allocation state; afterward,
   `F1 = F0 - backstop >= 0` and the other cumulative invariants hold. Every positive-`H`
   classification computes `cash_H = min(H,F)` and `noncash_H = H - cash_H`, assigns only
   `cash_H` to `X` or `Cb`, assigns non-cash coverage only to `Y` or `V`, and leaves `H = 0`.
   Its exact `(delta_E, delta_F)` is `(-cash_H,-cash_H)` at zero supply and
   `(-H,-cash_H)` at positive supply, whether the cash recipient is the current backstop or `X`.
   Each branch preserves the balance sheet and leaves all three scalars nonnegative.
   `cash_alloc` reports exactly positive `cash_H`; the preceding bucket/carry events make every
   non-cash part replayable.
2. Starting from `N = V = B = Cb = D = X = Y = 0`, `L = Cp = 10`, `H = 0`, `R = G = 0`, an
   open-credit `Some` route with `f = 20%` and `g = 10%`, and a timestamp-advancing settlement whose
   aggregate accrued interest is ten, the load uses 100% utilization without division by zero. It
   advances `ir_mod` and `d_rate` through the ordinary calculation, produces `L = 20`, adds two to
   `Cp`, adds eight to `Y`, leaves `R = G = 0`, allocates zero to `Cb`, records the exact discarded
   backstop numerator `Q` for the binding cap, leaves `B`, `V`, `D`, and `X` unchanged, and leaves
   `H = 0`. Thus `B >= Cb + D + X` remains `0 >= 0`; a later `gulp`,
   repayment, first supply, or route change never moves the eight into `X` or another claimant.
   Committing the load emits the backstop-remainder discard and one `ownerless_accrual` change of
   eight; a rolled-back or uncommitted projection emits nothing. The `N = 0, L = 0` short circuit
   accrues nothing.
3. At a captured later ledger before any committing load, `accounting_snapshot(asset)` for the
   zero-supply state in criterion 2 returns one sequence-tagged tuple with projected `L = 20`,
   `Cp = 12`, `R = G = 0`, `V = Ks = 0`, `Y = 8`, `H = 0`, the unchanged committed `D/X`, the
   observed `B`, derived `E/F/W/available`, and the exact atomic registry collection state.
   Committed storage and events remain unchanged. The positive-supply cadence state in criterion 4
   likewise produces one internally consistent snapshot. A later successful load committed at the
   same ledger stores
   exactly the projection, emits only the required committed scalar changes, and makes immediate
   post-commit views agree with event replay.
4. The five one-unit cadence case at `N = V = 1_800_000_000_000`, 20% protocol fee, and zero
   backstop rate ends with `Cp = 1`, unchanged `Cb`, `D`, `X`, and `Y`, `V = N + 4`,
   `b_rate = P + 2`, `P0 = N + 3`, and `Ks = 1`, regardless of same-ledger `gulp`
   interleaving after settled intervals. Separately, from `(B,L,V,N,Cb,G) = (10,90,100,100,0,0)`,
   `f = 0`, `g = 10%`, and an open epoch, one settlement with `a = 10` and ten settlements with
   `a = 1` both end `L = 100`, `V = 109`, `Cb = 1`, and `G = 0`. Closing after only nine one-unit
   settlements records and clears `G = 0.9Q`; no later epoch can claim it. Finally, with
   `R0 = 0.8Q`, `G0 = 0.9Q`, `Cq0 = 1`, `a = 1`, `f = 20%`, `g = 10%`, and sufficient backstop
   headroom, the interval allocates one unit to each fee claimant, releases `Cq`, and has
   `residual = 1 + 1 - 1 - 1 = 0`; supplier value never decreases.
5. Every economic supplier quote uses `V / N`. Deposit, partial withdrawal, full withdrawal,
   final-holder, zero-claim, default, liquidation-auction transfer, supply-cap, health, and
   utilization cases satisfy the rules above. Every membership request has the correct signed
   `limit`; non-membership requests require zero. In particular, a newcomer never claims more than
   its deposit or loses more than one underlying base unit, incumbents neither lose value nor gain
   more than one unit through mint rounding, and a successful partial burn donates at most one unit
   while respecting `max_b_tokens_burned`. Exact canonical recomputation satisfies
   `b_rate_after >= b_rate_before` for every successful mint, burn, donation classification,
   accrual, repayment, and other non-loss transition; only explicit default or the empty reset has
   the exceptions named above. The exact trace
   `N=V=B=1; transfer 50; gulp; supply 100; incumbent withdrawal` quarantines 50 in `X`, leaves
   `V=1` through `gulp`, mints 100 shares, and lets the incumbent withdraw only its one-unit claim,
   not 75. If a malformed
   implementation instead reaches `V=51,q=1,new_claim=75`, it rejects before transfer or mint.
6. From `N = V = 0`, any nonnegative `Y0`, `H_pre = 7`, and a first deposit of 100, both `None`
   and `Some` routes compute `c = min(7,F_pre)`, increase `X` by `c`, increase `Y` by `7 - c`, leave
   `Cb`, `D`, and `Cp` unchanged, then end with `N = V = 100`, `H = 0`, `E/F >= 0`, and a depositor
   claim of 100. Thus `F_pre >= 7` quarantines all seven as cash, while `F_pre = 3` places only three
   in `X` and four in `Y`; neither route gives any part to the depositor or backstop. At `H_pre = 0`,
   no exclusion or credit changes. At `H_pre = -1`, both routes reject with `BadRequest`, transfer
   and mint nothing, preserve all pre-state including `Y`, and emit no supply event. Plain and
   allowance executions of `Repay -> first Supply` include the deferred repayment inflow exactly
   once in `projected_B` and exclude the current supply transfer until after classification and
   mint. Their flash-prefixed equivalents first include the logical flash outflow exactly once and
   produce the same classification as the balance-equivalent non-flash trace.
7. Amount-zero `Withdraw` and `WithdrawCollateral` requests both fail with
   `InvalidBTokenBurnAmount`; the complete transaction rolls back with no token, reserve, user,
   or event change. By contrast, for `N = 2`, `V = 1`, and a holder with `n = 1`, a positive
   request of either type burns the one share, pays zero, invokes no underlying-token transfer,
   preserves accrued BLND, and emits the existing event with `(tokens_out, b_tokens_burnt) =
   (0, 1)`. That cleanup succeeds when it is the only action even if ordinary utilization or
   user-health checks would fail; unrelated requests in the same transaction retain their checks.
8. Default computes `delta_L = L0 - L1` from aggregate liabilities, reduces `Y`, derived `Cq`, `Cp`,
   then `V` by the exact bounded amounts above, and clears both `R` and `G` whenever
   `delta_L > Y0`. It leaves `Cb`, `D`, and `X` unchanged and reports the exact coverage result
   without a standalone per-position supplier-loss ceil. In the rounding witness
   `Y0 = Cp0 = R0 = G0 = 0`,
   `d_rate = 1.5P`, aggregate pre-default debt-token supply two, and defaulted debt-token amount one,
   `A = 2` while `delta_L = 1`; when `V >= 1`, default reduces `V` by exactly one and leaves `H`
   unchanged. In the priority/reopen witness `B = V = Cb = D = X = 0`, `L = 10`, `Cp = 2`,
   `Y = 8`, `R = 7`, `G = 0`, and `H = 0`, a default with `delta_L = 5` produces `L = 5`, `Y = 3`,
   unchanged `Cp = 2`, `R = 7`, and `H = 0`; defaulting the remaining five exhausts `Y`, writes down
   `Cp` to zero, clears and reports `R = 7`, and ends `(B,L,V,Y,Cp,H) = (0,0,0,0,0,0)`. A later
   ordinary first supply succeeds at unit value. Before and after every default, the complete
   `B + L = V + Cb + D + X + Y + Cp + Cq + H` equation and `H/E/F >= 0` hold.
9. The ADR 0002 replacement gates remain live in valid shortfall states. With positive `N`,
   `(B,L,V,Cb,D,X,Y,Cp,H) = (4,12,8,0,3,1,0,4,0)` has `E = 4`, `F = 0`, and `V < L`.
   Repayment with `tokens_in = 2` and `delta_L = 1` increases `V` by one and may then open a closed
   canonical backstop epoch at the same ledger; both succeed with `V < L`, preserve `H = 0`, and do
   not return `InternalError`. A positive borrow from the shortfall state still fails its unchanged
   `InvalidUtilRate` gate. Separately, `(4,12,8,2,1,1,0,4,0)` may close the two units of `Cb` into
   `D` and claim two fully backed deferred units, ending at `B = 2`, `Cb = 0`, `D = 1`, `F = 0`,
   and unchanged `V < L`. At zero supply, `(B,L,V,Cb,D,X,Y,Cp,H) =
   (0,10,0,0,0,0,8,2,0)` repaid with `tokens_in = 3` and `delta_L = 2` adds the one-unit cash
   remainder directly to `X`, leaves `Y = 8`, and preserves `H = 0`. Every case preserves the
   balance sheet and `H/E/F` bounds atomically. For current interest auctions, the exact state
   `(B,L,V,Cb,D,X,Y,Cp,H) = (2,12,8,2,0,0,0,4,0)` permits creation and full fill of the two-unit
   lot. Fill ends `(B,Cb,V,L,Cp,H) = (0,0,8,12,4,0)` without `InternalError`. Valid
   user-liquidation create/fill vectors at `V < L` retain every ordinary health, price, lot, and bid
   check; after required loads, the auction transfer leaves aggregate `N`, `V`, and `L` unchanged,
   and a full fill moves residual user debt to the backstop without default. Valid bad-debt
   create/fill vectors likewise retain the ordinary price, lot, bid, backstop-token, and threshold
   checks; the auction transfer leaves aggregate `L` unchanged, and only a threshold-triggered
   remainder default changes `L` through the exact waterfall.
10. At positive supply with no open credit epoch, an unsolicited cash transfer producing
   `H_l = F_l = 5` leaves `V`, `b_rate`, `P0`, and `Ks` unchanged, increases `X` by five, returns
   five, and makes an unchanged second call return zero. At `N == 0`, every route splits positive
   `H_l` into `min(H_l,F_l)` for `X` and the
   remainder for `Y`, without changing `Cb`, `D`, or `Cp`; direct non-cash zero-supply accrual has
   already increased `Y` and cannot make this branch productive. At positive supply, `Some` with an
   open credit epoch gives `min(H_l,F_l)` to `Cb` and the remainder to `V`, while `None` or a closed
   epoch gives `min(H_l,F_l)` to `X` and only the remainder to `V`. For either positive-supply
   branch with `H_l = 5, F_l = 3`, the cash recipient increases by three and `V` by two. `H_l == 0`
   emits zero; `H_l < 0` fails atomically. Every case includes `D`, `X`, and `Y` exactly once in
   coverage and leaves `H/E/F >= 0`.
   `cash_alloc` emits only the cash amount—three in that open-epoch witness—after the supplier event
   for the non-cash two and before `gulp`; a zero-cash classification has no `cash_alloc`.
   Across plain, allowance, and flash-prefixed forms, `FillInterestAuction -> membership` observes
   the synchronous lot once in physical cash, while
   `membership -> FillInterestAuction -> membership` advances the initialized projection once.
   Each ordering reuses one balance read per distinct membership asset and produces the same
   post-state and event-derived classification as its logically equivalent cash sequence.
11. Enabled collection conserves `old Cp = sent + remaining Cp`, leaves `Y`, `R`, `G`, derived `Cq`,
   and `H` unchanged relative to the loaded state,
   transfers only to the recipient and epoch returned in its one atomic registry read, and satisfies
   `sent <= max(B - Cb - D - X - min(H,F) - W, 0)`. The formula deliberately does not reserve `Y`.
   Every current or deferred backstop claim, unclassified physical-cash component, and supplier
   withdrawal fundable before collection remains fundable afterward, and `UnownedSurplus` and
   `OwnerlessAccrual` are never transferred. A direct donation followed by collection and then the
   canonical classifier still assigns the entire donation to `X` or the open current backstop; no
   part can enter `V`. Frozen, globally paused, registry-failure, and
   failed-token cases are atomic, and nested token behavior cannot observe or collect the same
   credit twice. In an incident drill with one unreachable pool admin, a recovery-authority pause
   checked-increments the collection epoch and immediately makes collection fail from every listed
   pool without changing their lending status. A matured resume from the prior epoch fails.
12. Every pool fee pair satisfies the code ceilings and full current registry policy at deployment
   and attachment finalization. Live changes enforce current policy only on changed fields while
   always enforcing code ceilings. A pool at `(fee=1%, take=27%)` under a later
   `(min_fee=5%, max_fee=5%, max_take=25%)` policy can change an unrelated numeric field, lower take
   to 25%, then raise fee to 5%; a one-step fee change to 5% while take remains 27%, a changed field
   still outside its bound, and every worsening transition reject atomically. Rate changes settle
   old intervals first. `set_interest_fee` requires pool-admin auth and local `Live` lifecycle,
   rejects `Contained`/`Retired` with `InvalidPoolStatus` before registry or reserve work, rejects an
   unchanged value with `BadRequest` before settlement, validates the changed fee through the
   existing `min_config()` view, settles every reserve under the old fee, stores the new fee, and
   emits its event last. Frozen status 4 remains repairable while lifecycle is `Live`; `update_pool`
   uses the same changed-field/grandfathering semantics. Recipient category exclusions
   survive proposal delay, deterministic pool deployment, backstop enable/re-enable, and archived
   metadata restoration. `SetFeeRecipient` and `ResumeFeeCollection` use `SecurityDelay` and
   recovery veto. Rotation increments the epoch and leaves collection disabled; only a fresh
   nonzero-canary proposal bound to that epoch and recipient may resume. Raw-XDR replay reconstructs
   the complete atomic state and matching proposal terminals.
13. Attachment at statuses 0, 2, and 4 preserves holder ownership and every `D`/`X`/`Y`/`Cp`/`R`
   exclusion or remainder, requires the closed old route's `G == Cq == 0`, and classifies positive
   old-route coverage through the positive-supply `X/V` or zero-supply `X/Y` split
   before `Some`, gives none of it to the candidate, and
   rolls back completely on late failure. Finalization rechecks the
   current fee pair before readiness or writes; a tightened-policy failure preserves pending state
   and emits nothing. Attachment and the recurring all-reserve settlement paths for
   `set_interest_fee` and a changed-`bstop_rate` `update_pool` fit the maximum-reserve active-network
   limits with recorded margin; a resource failure cannot leave numeric configuration permanently
   unchangeable.
14. Candidate-versus-ADR-0002 invocation traces prove that ordinary supply, withdrawal, borrow,
    repayment, flash loan, liquidation, position-health evaluation, and the three component
    projected reserve views add no registry or token cross-contract call solely for fee routing.
    `accounting_snapshot` is separately proved to make exactly its one registry-state and one
    reserve-token-balance call and no mutation. Existing underlying
    transfers, oracle reads, and the flash receiver callback remain. No listed path performs a
    registry or fee-recipient lookup. Supplier membership performs exactly one token-balance read per
    distinct asset for request-order claimant classification across first, later, and final
    boundaries; fee accounting adds no second read, and non-membership actions/component views add
    none. Cold governance/attachment, `gulp`, deferred-credit claim, snapshot, and collection retain only the
    calls expressly specified in their rules. Exactly 60 requests reach construction for healthy
    30-collateral/30-liability and repeated same-asset cases in plain, allowance, and flash-prefixed
    forms. Each corresponding 61+-request call returns `BadRequest` after inherited auth/self-address
    guards but before pool/user load, flash liability, reserve/balance access, cross-contract call,
    mutation, or event.
15. Borrow, repay, and flash-loan economics, authorization-root semantics, callback, and existing
    event shapes remain unchanged. The intended public data-field deltas are `Request.limit`,
    `PoolConfig.interest_fee`, `ReserveData.protocol_credit`, `ReserveData.protocol_fee_remainder`,
    `ReserveData.backstop_fee_remainder`, `AccountingSnapshot`, `FeeCollectionState`, and
    `FeeCollectionResume`; the other intended public deltas are the methods—including
    `ownerless_accrual` and `accounting_snapshot`—events, registry-policy extensions, and
    constructor/deploy arguments named in this ADR. ADR 0002's carry/deferred/surplus views and
    events remain intact and `UnownedSurplus` retains its cash-backed meaning.
    The cumulative registry ABI retains every ADR 0002 security/lifecycle/seal member and adds only
    the fee policy and collection-state surface named here. Generated clients and indexers decode
    the final optimized Wasm and reject `b_rate`-only valuation or separately assembled accounting.
16. Failure tests observe every exact mapping in Failure semantics, including that `V < L` alone is
    never `InternalError`, with no new error code, partial
    state, or success event. Authorization failures retain their host-auth shape and nested registry
    failures propagate unchanged.
17. No non-disposable pool uses an interim layout. Every shipping pool, backstop, registry, and
    retained pool-factory Wasm is at or below
    `min(131072, active_network.contractMaxSizeBytes)` for its deployment network.

### Verification and release

The ADR can be accepted before implementation. Acceptance records the following obligations; it
does not claim that after-build evidence already exists. Raladr consensus evaluates the decision
document, not a prototype.

1. **Repository checks now.** `bin/adr --robot lint` and `git diff --check` pass. The ADR remains
   `Proposed` until a human accepts it.
2. **Unit and property evidence after implementation.** In the upstream contract repository run
   `make test`, which builds before testing. Cover both fee-numerator identities, every
   cap/close/route disposal—including zero headroom, exact-whole headroom plus a fraction, and a
   zero-supply `Cq` release—the
   `1x10 == 10x1` backstop witness, and rate boundaries; pro-rata
   mint, burn, final-holder, zero-claim cleanup at both zero and positive aggregate `V`, exact
   authorization-bound min-mint/max-burn behavior, every one-unit economic bound, and the complete
   `1/50/100/75` attack witness,
   empty-epoch first supply, the exact `N = 0, L > 0` ownerless-accrual vector, cash/non-cash
   positive-`H` classification in all three recipient branches, exact bucket/carry -> cash-only
   `cash_alloc` -> containing-event order, `Y -> Cq -> Cp -> V` default, `R/G` clearing, ordinary reopen,
   exact `Y` behavior across repayment/route/attachment/collection, all three `H/E/F` invariants,
   positive- and zero-supply repayment remainders, every `V < L` replacement gate including current
   interest-auction create/fill plus bad-debt and liquidation paths, atomic snapshot and commit
   catch-up, amount-zero withdrawal, auction-transfer,
   route/credit-epoch `gulp`, complete `D`/`X`/`cash_H` collection reservation, request-order
   `Repay -> first Supply`, flash-prefixed equivalent, `FillInterestAuction -> membership`, and
   `membership -> FillInterestAuction -> membership` across plain/allowance/flash forms with one
   balance read per distinct asset, exact 60 success and 61+ pre-effect rejection for both repeated
   same-asset and 30-collateral/30-liability sequences, changed-field config transition, the exact
   `set_interest_fee` auth/lifecycle/no-op/policy/old-settlement/store/event order at statuses
   0/2/4 plus `Contained`/`Retired`, immediate collection pause/refresh, stale-epoch resume
   rejection, delayed canary-bound resume, recipient rotation/category behavior, one-unavailable-
   pool-admin containment, and atomic
   attachment criteria, including every exact failure-ABI and no-hot-cross-call case. Randomized
   valid-state sequences assume the full balance sheet and `H/E/F >= 0`, then assert that each
   successful transition preserves them; they do
   not generate arbitrary algebraic tuples and call them reachable. Classifier properties also
   assert the table's exact branch-specific `delta_E/delta_F`, not merely post-state nonnegativity.
   Include the exact numerical
   regressions above. Authorization tests use exact `mock_auths` trees for `Request.limit` and
   registry authorities; `mock_all_auths_allowing_non_root_auth` is not acceptance evidence for
   contract-to-contract authorization. Test organization and helper names are implementation-owned.
3. **Bounded model checking after implementation.** Add Kani proofs for the checked fee splits and
   both cumulative numerator identities, including complete cap/close/route disposal,
   canonical `N/V` representation, supplier lifecycle including empty epochs,
   timestamp-advancing `N = 0, L > 0` accrual into `Y`, exact `Y -> Cq -> Cp -> V` default,
   `R/G` clearing,
   `H/E/F` preservation and reopen,
   separation of `X` and `Y`, the `cash_H/noncash_H` classifier, its exact three-row
   `delta_E/delta_F`, donation quarantine, min/max membership limits, one-unit mint/burn loss
   bounds, and event-derived recipient/epoch,
   request-order projected cash with reflected/unreflected auction and flash cases, the 60/61
   pre-effect boundary, `gulp` routing, repayment at both supply states, collection reserve including
   unclassified cash, and the backstop cap. A bounded fee-transition state machine proves that
   invalid lifecycle and no-op
   branches reach no policy/reserve effect, every successful reserve settles under the old fee, and
   the new fee becomes visible only after complete settlement. A separate collection-state machine
   proves pause refresh invalidates every older resume, rotation disables and increments, and only
   a delayed proposal bound to the exact epoch/recipient can enable collection. The valid-state preconditions
   explicitly encode the complete balance sheet,
   `H/E/F >= 0` and `N == 0 => V == 0`; transition harnesses
   prove these predicates inductive across accrual, default, repayment, epoch, attachment, `gulp`,
   and collection rather than assuming their post-state. Harness decomposition and names are
   implementation-owned. Each proof states its protocol-enforced bounds; a timeout or an unjustified
   narrowed domain is not evidence.
4. **Built-Wasm and interface evidence after implementation.** Build the ADR-0002 baseline and
   combined candidate with the same pinned toolchain. Record commits, lockfile hashes, optimized
   artifact hashes and sizes, and attributable deltas. Regenerate Rust/TypeScript clients from the
   actual optimized Wasm; compare the contract spec and confirm that only the public deltas named
   here and in ADR 0002 appear. Compare invocation traces for every ordinary action and projected
   component view named in criterion 14 and fail the release on any added registry or token call
   solely for fee routing or accounting. Separately prove the cold snapshot's exact one registry
   and one token-balance call. On the actual optimized contracts, record one balance read per
   membership asset and successful resource usage at exactly 60 requests for both repeated
   same-asset and healthy 30-reserve effective-position cases in all three submit forms; require
   the matching 61-request calls to reach none of the prohibited effects. Failure of an exactly-60
   case to fit the captured active-network limits blocks genesis rather than silently lowering the
   bound.
5. **Standalone and testnet evidence after implementation.** Exercise real authorization and
   cross-contract behavior: fee deployment and transition, insured and uninsured accrual,
   both cadence/remainder lifecycles, actual-Wasm `N = 0, L > 0` accrual into ownerless state,
   atomic accounting snapshot, no-commit event lag and exact commit catch-up, TTL restore,
   donation quarantine and the rejected exploit trace, exact signed membership authorization,
   empty-epoch first supply, zero requests, zero-claim supply and collateral cleanup, `V < L`
   repayment/open/close/claim and all three auction families,
   current/deferred/quarantine/unclassified-cash reservation, `Y -> Cq -> Cp -> V` default and
   ordinary reopen, partial/full collection, recipient
   rotation and category rejection, immediate pause/refresh from each authority, one-unavailable-
   pool-admin global containment, stale resume rejection, canary-bound resume, `SecurityDelay`, recovery veto,
   frozen-pool behavior, attachment at Borrow-disabled status, late-failure rollback, the concrete
   prospective-policy repair sequence and exact fee-setter ordering above, request-order repay,
   auction, membership, and flash permutations through all three submit forms, exact 60/61 behavior,
   raw classification-event replay, and maximum-reserve resource use for attachment,
   `set_interest_fee`, and a changed-`bstop_rate` `update_pool`. Record
   ledger-consistent pre/post claimant state and actual RPC results. Public testnet is required for
   auth, upgrade, recipient, and cross-contract paths; if a layer cannot run, the release records
   why and remains blocked where that layer is required.
6. **Release and rollback.** The genesis evidence binds the accepted ADR, upstream commit, final
   Wasm hashes, effective network limits, public interface versions, fee bounds, initially disabled
   collection state, recipient canary digest, fresh resume epoch,
   and successful checks. A missing size, interface, invariant, or required testnet result blocks
   genesis. Before deployment, rollback is ordinary code reversion. After deployment, retries are
   atomic and code rollback uses ADR 0002's compatible upgrade path. Reinterpreting `H`, `V/N`,
   `SupplierRoundingCarry`, deferred credit, cash-backed `UnownedSurplus`, `OwnerlessAccrual`,
   request-order `projected_B`, cash/non-cash classification, membership-loss bound, remainder
   ownership, collection epochs, claimant priority, or an initialized public layout requires a new
   ADR and migration evidence.

Follow-ups: BLND issuance, buyback, or burn; permissioned pools; an origination/flash-fee ADR only
after it defines an authorization-stable quote or bound, receiver funding, and default treatment;
an alternative proportional protocol-credit/supplier loss waterfall only if evidence justifies
changing claimant priority; any spend, release, or reclassification of `OwnerlessAccrual` only through a
separate deliberate decision that preserves existing claims.
