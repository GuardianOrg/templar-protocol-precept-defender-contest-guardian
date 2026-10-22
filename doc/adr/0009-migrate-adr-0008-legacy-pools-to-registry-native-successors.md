# 9. Migrate ADR-0008 legacy pools to registry-native successors

Date: 2026-09-07

## Status

Accepted

This revision invalidates every Raladr result for an earlier blob. It requires a fresh Architect and
Critic review and remains Proposed until a human accepts it.

Amended 2026-09-16: terminal classification now also names ADR 0008's `debt_setoff` event.

Builds on
[8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md).

The exact ADR-0008 backstop behavior must be the profile selected by accepted
[11. Disable activation-shim backstop emissions while preserving its ABI](0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md).

This ADR selects no permanent pool profile. It governs migration into whichever one registry-native
profile is later accepted and implemented.

## Context

ADR 0008 is temporary legacy infrastructure outside the Precept production registry. Its pool,
backstop, oracle, and user positions cannot be upgraded, repointed, or imported into a successor.
The registry cannot truthfully claim to have deployed an old address. Migration therefore means
new contracts and real user-authorized value movement, not an in-place conversion.

Moving suppliers first can remove the cash borrowers need to repay. The safer order is to seed the
successor, refinance borrowers, use the resulting legacy repayment cash for withdrawals, and then
supply realized cash to the successor.

Direct participation remains permissionless. Templar can close official routes and move its own
vault liquidity, but cannot force an independent user to migrate. Tail support therefore continues
until objective accounting terminality and sink ownership, not a date, percentage target, or raw
token-balance zero.

Status 2 is not a sufficient intake brake because it still permits supply and collateral supply.
The preferred brake is ADR 0008's absorbing status 4. If a non-getter pool failure prevents status 4
while the policy authority and transaction inclusion remain healthy, all reserves disabled with no
unresolved policy queue is the only fallback. Authority loss, policy TTL failure, failed inclusion,
or unknown queue state leaves cutover pending behind closed routes and the oracle/product brake.

Definitions:

- **legacy pool:** an ADR-0008 pool outside the production registry;
- **successor pool:** the one current replacement deployed by the selected production registry;
- **lineage key:** the literal `(network passphrase, legacy pool address)` pair;
- **F4:** the actual ledger/close-time of the first confirmed successful entry into absorbing status
  4, with its transaction and exact no-downgrade-code evidence. Later idempotent status-4 calls do
  not replace it; F4 is absent until a freeze succeeds;
- **T0:** the first evidenced ledger/close-time whose confirmed boundary state satisfies the complete
  chosen intake-brake predicate: preferred status 4 under the verified no-downgrade code with no
  pending or unresolved policy queue, or the authorized fallback with every reserve disabled and no
  pending or unresolved policy queue. An expected pending disable also blocks T0; resolved queue
  history is retained, not treated as an outstanding queue. On the preferred path `F4 <= T0`, with
  equality only if the complete predicate already holds in the freeze ledger. The fallback may
  establish T0 before any F4; never fabricate F4 or rewrite T0 when a later freeze succeeds;
- **I0:** an incident before the complete T0 boundary, including while frozen but awaiting cleanup;
- **I1:** an incident from the complete T0 boundary until retirement-sink ownership; and
- **drained:** objective accounting terminality—zero liabilities, auctions, pool b-token emissions
  configuration/data, and withdrawable collateral or represented claims; zero-raw collateral owned
  by a defaulted user first orphaned to the pool and then settled after reserve debt reaches zero;
  any other residual b-token balance allowed only when its live conversion yields zero underlying
  asset units and archived as a rounding-loss record—one valid ADR-0008 backstop terminal class, a
  durable intake brake, drained migration authority, one successor route, and immutable
  retirement-sink ownership.
  Fully explained unrepresented physical residue is abandoned in place under ADR 0008, even above
  its operating rounding tripwire; it is not an outstanding represented claim.

In scope: successor mapping; inventory; official routes; borrower refinance; vault movement; bridge
liquidity; incidents; direct-user tail support; and terminal retirement.

Out of scope: selecting the permanent profile; registry adoption of legacy addresses; storage copy;
asset swaps; making users whole; forcing migration; reactivating a frozen pool; or incentives.

Principles:

1. Registry membership remains truthful deployment provenance.
2. Every lineage has one current successor and one official route.
3. Establish a durable legacy intake brake before opening the successor route.
4. Refinance borrowers before draining most supplier cash.
5. Move value only with exact native authorization, fixed bounds, conservation, and atomic rollback.
6. Keep automation single-purpose and keep complex positions on explicit manual paths.
7. Completion means verified state, not UI delisting or elapsed time.

Decision drivers, ranked:

1. Preserve value and authorization atomically while moving leveraged positions between unrelated pools.
2. Establish one truthful, irreversible cutover order without inventing registry or position portability.
3. Keep bridge, vault, retry, and direct-user tail operations bounded and maintainable.

This companion is coupled to ADR 0008 because its T0, policy-queue, backstop-terminal, and sink
predicates are defined by that exact legacy fork. A migration review against a different launch
profile would approve the wrong containment and completion conditions.

## Decision

Migrate each ADR-0008 market into a newly registry-deployed successor through a borrower-first
conveyor. Use one small migrator for simple one-liability positions and a checked vault batch path
only when a vault route is independently qualified. The legacy and successor coexist temporarily
but never share registry identity or position state.

Once the first lineage reaches T0, no new ADR-0008 pool may launch. After T0, official activity never
returns to that legacy pool.

### 1. Require a production-ready successor first

No planned cutover starts until:

- exactly one permanent production profile is human-accepted and implemented;
- its registry, pool, fee, oracle, backstop disposition, issuance, isolation, and governance
  decisions are coherent;
- exact successor, registry, oracle, adapter, and route artifacts pass their own build, test, audit,
  public-testnet, and deployment/readback gates; the funded mainnet canary occurs only through the
  lifecycle in section 4;
- the successor has enough cash, caps, and utilization headroom for the planned first tranche; and
- the permanent-profile ADRs explicitly classify ADR-0008 addresses as out-of-registry legacy
  governed by this ADR.

A focused-reviewed migrator build targets the selected successor's exact interface and semantics.
The bridge signer and submission worker live-read registry membership, executable identity, admin
and upgrade authority, oracle, reserve configuration, caps, utilization, cash, and their current lane
phase. A mismatch stops their authorization/submission. Before ordinary bridge signing is enabled,
only the one exact transaction in `CANARY_AUTHORIZED` may receive bridge signatures; submission
then requires its matching `CANARY_PENDING` phase, transaction hash, and reservation. These are
signer/worker checks, not contract observations of an off-chain record.
Replacement code or a replacement successor creates a fresh reviewed and canaried lane inside the
same immutable lineage and retains that lineage's original T0. The predecessor remains the sole
current successor until its route, reservations, and authorization horizons drain; only then may a
protected record name the replacement as the lineage's sole current successor and open its route.

### 2. Keep one readable lineage record

Each lineage has one ordinary Markdown migration plan and evidence record reviewed in a protected
pull request. The literal network passphrase and legacy pool address are the authority key. The
record names exactly one current successor and contains:

- legacy and successor contract IDs, reserve map, assets, indices, decimals, and exact behavior,
  oracle, fee, and risk differences;
- source commits, generated interfaces, lockfiles, toolchains, build commands, standard SHA-256
  artifact checksums, byte sizes, and focused review scopes;
- deployment transactions, ledger sequences, registry membership, live executable/admin/upgrade/
  oracle/reserve/cap/queue/policy/vault/adapter/route readbacks, and the current lane phase;
- inventory coverage and eligibility totals; bridge, vault, and successor capacity; distinct F4 and
  T0/I0/I1 evidence, chosen boundary branch, exact cancellation outcomes and queue/state readbacks;
  batch reconciliation; and terminal state;
- testnet rehearsal, audits, transactions, and named protocol, security, operations, treasury, and
  vault approvals;
- before canary signing: the exact user, lineage, migrator, successor, assets, amount and slippage
  bounds, bridge advance and ceiling, reservation id, source account and sequence, final transaction
  hash constructed as in section 6, inclusion and authorization deadlines, and named approval;
- after inclusion or definitive failure: actual result and network/live-state/reservation
  reconciliation evidence. These outcome fields are absent before the outcome, never placeholders; and
- the protected runtime-configuration change that removes the legacy route and pins only the exact
  successor.

Protected Git review authorizes the human plan and route configuration. Ordinary on-chain multisigs
and Soroban native authorization remain the only authority for contracts and value movement.
Transactions and live readbacks prove execution. If the record, runtime configuration, or live state
disagrees, migration fails closed.
F4 and T0 retain their actual evidenced state ledger/time and transaction provenance, separately from
any later record/reconciliation time. The freeze outcome alone cannot assert queue absence or T0.
Use retained event/state evidence to identify the first complete boundary, not a later point-read
or record timestamp substituted for it; missing or contradictory boundary evidence blocks progress.

No derived market identifier, private serialization, custom signing envelope, vector corpus, or
ADR-embedded checker is normative. The lane phases and transition predicates below are normative.
Any release or signer schema must represent them distinctly and reject unknown or skipped phases;
it cannot collapse candidate-canary permission, ordinary signing, and route opening into one
`canonical` or `enabled` boolean. Field names and storage format remain implementation detail and do
not override the readable record or live facts.

### 3. Build the inventory before launch and refresh it at T0

Before ADR-0008 status-0 activation, start gap-detected ingestion at the legacy pool's deployment
ledger. Archive raw events and ledger entries needed to restore every position, auction, reserve,
backstop, policy, vault, and route observation. Reconcile indexed b-token and d-token totals against
each reserve's aggregate accounting. Missing ranges or unexplained differences block routing until
recovered with point reads and reconciled.

Before cutover, classify every account as:

- **atomic eligible:** at most one nonzero liability reserve and compatible assets/configuration;
- **manual/top-up:** safe only with a reviewed explicit user plan;
- **auction tail:** active or freeze-sensitive auction state requires resolution; or
- **supplier only:** no liability and ordinary withdrawal is sufficient.

Classification predicts support; it never hides an account or changes user rights. Publish counts,
supply, liability, and collateral value by class and reserve. Replay and point-read the complete
inventory again at T0 before the successor route opens.

### 4. Cut over through one fail-closed lane lifecycle

The protected lineage record and every signer/release consumer use this lifecycle:

`INTENT -> DEPLOYED -> CANARY_AUTHORIZED -> CANARY_PENDING -> CANARY_PASSED -> CANONICAL -> SIGNING_ENABLED -> OPEN`

`INTENT` publishes and approves the production deployment plan but authorizes no bridge signature or
successor route. A protected record revision establishes `DEPLOYED` after the exact production
contracts pass deployment, code, authority, configuration, funding, and live readback checks while
the successor route and bridge signing remain disabled. Simulation and readback may repeat in these
phases but move no user value.

`CANARY_AUTHORIZED` is the only pre-canonical bridge-signing exception. A protected record revision
must name one exact capped canary transaction and every pre-signature field in section 2, not future
outcomes. Before emitting any bridge signature, the signer rechecks that revision, the live
predicates in section 1, the existing canary reservation, and the final transaction hash constructed
under section 6, then atomically marks the reservation `CANARY_PENDING`. Threshold shares for that
same transaction belong to one authorization attempt; no second transaction may be signed.
Within that lane, a different transaction, missing or expired pre-signature field, previous pending
or successful canary, or ordinary migration rejects. An ambiguous outcome remains `CANARY_PENDING`
and blocks new signing. A protected record revision may return a conclusively failed canary to
`DEPLOYED` only after
all issued authorization and transaction bounds expire, network/state evidence proves non-inclusion
or an included atomic rollback with no migration value movement, and the reservation reconciles.
Sequence consumption alone never releases capacity. Every retry requires a fresh reviewed approval.

After one included transaction proves full user-authorized movement, bridge restoration, zero
migrator custody, conservation, and reservation reconciliation, a protected record revision
publishes the actual outcome fields and advances to `CANARY_PASSED`. Only then may a reviewed release
make that exact lane `CANONICAL`; canonicalization alone still authorizes no bridge signature or
successor route. The bridge authority then explicitly advances the exact canonical lane to
`SIGNING_ENABLED`. That phase and its `OPEN` successor permit ordinary approved migration
signatures. The protected runtime change advances to `OPEN` and exposes the one successor route only
after T0.

Execute the release in this order:

1. Publish the lineage intent in `INTENT`.
2. Deploy, register, and fund the successor; freeze the exact migrator and, if used, vault build;
   complete production readbacks and public-testnet rehearsal; enter `DEPLOYED` with its route closed.
3. Reserve only the bounded canary advance, publish its exact approval, and enter
   `CANARY_AUTHORIZED`.
4. Atomically mark that reservation `CANARY_PENDING`, then sign and submit only the exact approved
   transaction and reconcile its included result and reservation.
5. Publish the funded-canary attestation and enter `CANARY_PASSED`.
6. Review and select that exact lane as `CANONICAL`; keep its successor route and ordinary signing off.
7. Reserve bridge and successor capacity for the largest eligible position in every enabled lane,
   then publish at least 72 hours' cutover notice.
8. After the notice period, set every applicable vault legacy target to zero and complete all
   freeze-sensitive work.
9. Close every legacy official route and record the close version/time.
10. Request status 4 and record F4 on confirmed success under the exact no-downgrade code. If exact
    policy-owned pending disables remain, cancel them through ADR 0008's frozen cancellation
    surface and confirm every outcome and live state. These per-asset cancellations may occur in
    later ledgers; no atomic freeze-and-cleanup invocation is assumed. Until every pending or
    unresolved policy queue is absent, keep T0 unestablished and the successor route closed.
    Unknown, failed, or unauthorized cleanup remains fail-closed. Only if the non-getter status call
    fails while policy authorization and inclusion remain healthy may the predeclared
    all-reserves-disabled fallback establish the boundary, also with no pending or unresolved queue.
11. Refresh the complete point-read inventory and reconcile T0 from evidence of the first complete
    chosen boundary. Preserve the earlier F4 when cleanup completed later; neither inventory refresh
    nor a repeated status-4 call resets either historical marker.
12. Advance the canonical lane to `SIGNING_ENABLED`, then merge the protected runtime change and
    enter `OPEN` with only the one pinned successor route.
13. Refinance bounded borrower tranches; use restored legacy cash for authorized withdrawals; move
    only realized underlying to successor supply; reconcile after every transaction.
14. Continue direct-user and auction-tail service until the objective terminal gate passes, then
    transfer admin to the immutable retirement sink.

A replacement lane after T0 follows the same phases without reopening legacy intake. Revalidate the
already-established brake, zero vault target, closed legacy routes, queues, and refreshed inventory
at steps 8–11; the historical T0 is not a substitute for current evidence.

Every migration request is recorded before native signing; its final transaction is recorded before
bridge envelope signing. An included transaction occurs only after its
recording point and no later than its authorization deadline; after inclusion the lineage record
cites the exact transaction and ledger/time. A missing or contradictory phase, permission, or fact
blocks progress.

### 5. Use one narrow borrower migrator

The migrator has no admin, upgrade, arbitrary call, token rescue, general router, standing allowance,
or durable custody. One call handles one user and one lineage. The user and bridge authorize the
exact native invocation with short ledger/time bounds. The bridge signer accepts a new request only
in `CANARY_AUTHORIZED` for the exact recorded canary, or in `SIGNING_ENABLED`/`OPEN` for an ordinary
approved migration. The submission worker permits the signed canary only in `CANARY_PENDING` with
the matching transaction and reservation. These off-chain checks do not revoke issued signatures.
The immutable migrator instead verifies native authorization of the complete request by the user
and fixed bridge account; it cannot observe lane phase, route files, or the enclosing transaction
hash. Canary mode requires no pre-T0 condition and remains usable for a replacement lane after T0.

The request fixes:

- literal lineage, legacy/successor pools, fixed bridge account, user, lane mode (`canary` or
  `ordinary`), reservation identity, liability asset, and maximum repay;
- each legacy withdrawal asset and exact/minimum amount;
- each successor supply asset and maximum amount;
- successor borrow asset and maximum amount;
- minimum residual returned to the user; and
- deadline and nonce/idempotency key.

The call:

1. verifies live legacy, registry, successor, oracle, authority, reserve, cap, and cash state;
   `ordinary` additionally requires the on-chain intake brake (status 4 under the verified
   no-downgrade code or every reserve disabled). Full T0, route closure, and lane phase remain
   signer/release-worker predicates, not contract reads;
2. advances only the permitted liability amount from the bridge;
3. repays the entire selected legacy liability and withdraws all selected legacy supply/collateral;
4. supplies only the request's mapped assets to the successor and borrows only the bounded liability;
5. repays the bridge from the successor borrow;
6. sends every residual to the user; and
7. verifies final legacy zero for the selected position, successor bounds/health, zero migrator
   balance and allowance, restored bridge balance, and exact token-flow conservation.

Requested legacy asset, residual asset, successor supply asset, and reserve-map asset must correspond
exactly. Every minimum and maximum applies to that exact asset; a residual in another asset cannot
satisfy it. Unsupported, duplicate, missing, or extra assets reject. Any failed postcondition rolls
back the complete Soroban transaction.

Complex positions use ordinary user-authorized manual transactions under a reviewed plan. The
migrator does not add a loop, callback, adapter framework, or arbitrary route to support them.

### 6. Bound bridge capital and retries

A dedicated bridge account advances only the old liability needed by one approved invocation. The
bridge balance, per-call advance, pending reservations, included-but-unreconciled deficits, and
authorization lifetime are capped per lineage and globally in the reviewed plan. Available
capacity covers the largest atomic-eligible position in each enabled lane with measured interest,
rounding, resource, and transaction-time margin.

Use standard source-account authorization for this bridge: its dedicated classic G-account is both
the transaction and operation source, the bridge credential is
`SOROBAN_CREDENTIALS_SOURCE_ACCOUNT`, and the bridge multisig signs the outer V1 transaction envelope.
Signer policy and transaction construction reject bridge address credentials, including every manual
signing path. At first activation use a fresh dedicated G-account and fresh signing keys reserved
solely for bridge envelope signatures. Document key custody and every authorized signer's attestation
that those keys have never issued native address-auth signatures in any account context. Unused
signatures are not enumerable on chain: this is trusted issuance/custody evidence plus tooling
refusal, not a network absence proof. Apply the same fresh-key/attestation control to signer additions.
The migrator requires fixed-bridge `require_auth` at its root over every request field; it cannot
inspect the credential discriminant. The one `InvokeHostFunction` operation carries that invocation.

Construct the canary without a signature/hash cycle: reserve capacity and obtain all non-bridge
native address authorizations first, including the user's signatures. Finish simulation/resource
assembly, freeze the complete V1 transaction XDR with those auth entries, and calculate its
network-bound inner transaction hash. The lane does not create fee-bump wrappers; changing the
signed inner fee requires new review. Reconcile by inner hash and exact inner XDR even if a third
party wraps the transaction; a wrapper is not evidence of non-inclusion. The pre-signature record binds the final hash and every
request field. Only then mark the existing reservation pending and collect the bridge's outer
threshold signatures; outer envelope signatures do not change this hash. Persist the exact signed
envelope before submission. Any operation, auth-entry, source, sequence, resource, fee, or bound
change requires new review and cannot reuse the approval. A crash after pending preserves the
reservation until exact resolution; it never grants another transaction permission.

Thus another submitter may relay the identical signed envelope but cannot transplant bridge
authorization into a different transaction or source. The user's address-auth nonce may roll back
on failure; neither that rollback nor an off-chain phase change revokes a native capability.
Failure recovery therefore retains the conservative all-bounds-expired rule in section 4.

A durable implementation-owned ledger serializes reservations before signature. Each reservation
binds a unique idempotency key, exact request, source account/sequence, amount, native authorization
identities/bounds, created time, inclusion deadline, and status. The final transaction hash is added
after complete native-auth assembly and before bridge signing, never as a fabricated early value.
It enforces:

`pending reserved advances + included-but-unreconciled bridge deficits <= approved bridge ceiling`.

Those two classes are disjoint: confirmed inclusion atomically moves an advance from pending
reservation to included deficit; it is never counted twice. Network and live-balance proof that the
transaction restored the bridge removes the deficit and releases capacity. Reconciled settled
volume remains in the cumulative conservation and audit ledger but does not consume current bridge
capacity.

Before retry or release, the worker queries the network by transaction hash and reconciles bridge,
migrator, user, and both pool states. No timeout or worker crash proves failure or frees capacity.
An unknown submission blocks the lane until network and live state prove inclusion or, after every
bound expires, definitive failure. Canary signing atomically marks the existing canary reservation
`CANARY_PENDING` before signature emission; ordinary signing first records its reservation. Included
transactions must fall after recording and within both the transaction and authorization
ledger/time bounds.

The ledger's schema is implementation detail, but it must expose the normative lane phase and reject
skipped, unknown, or phase-incompatible signature requests. The observable requirements are
single-writer reservation, no duplicate value movement, no premature capacity reuse, complete
transaction reconciliation, and exact per-asset conservation across the lane transitions above,
crashes, retries, concurrent workers, and upgrades.

### 7. Qualify the vault path or remain direct-only

A vault-routed lineage uses separate legacy and successor adapters. The vault values them under their
native accounting, sets the legacy target to zero, routes every new allocation only to the successor,
and moves only realized underlying. Existing vault shares never encode or reinterpret a pool
address.

Every legacy-position-decreasing path must use one reviewed batch operation that:

- has exact native governance authorization and live adapter/code/configuration checks;
- binds a unique batch id and source snapshot before submission;
- caps each requested withdrawal by live legacy value and the accounting-derived exit-headroom rule
  below;
- sends realized value only to successor supply, user redemption, explicit approved fees, or small
  bounded migration-owned idle cash;
- records exact transfers and successor receipts; and
- closes per-batch and cumulative conservation equations before more capacity is released.

For each reserve, the checked operation reads the current-ledger `Reserve`, raw pool-token balance,
and vault b-token position inside the same atomic invocation as the withdrawal. Let `Q = 10^12`,
`U = 10^7`, `burn(x) = ceil(x * Q / b_rate)`,
`S(x) = floor((b_supply - burn(x)) * b_rate / Q)`,
`L = ceil(d_supply * d_rate / Q)`, and
`C = S(0) + backstop_credit - L`, all in the stock integer order with checked arithmetic. `C` is
**accounted physical cash**. A candidate is invalid when `b_rate <= 0`, `burn(x) > b_supply`,
`backstop_credit != 0`, `C < 0`, or any checked stock operation fails.

While `d_supply > 0`, the complete point-read inventory defines two per-asset values from the same
set of every non-vault, non-pool-owned position with `b_i > 0`:

- `B = user_exit_btokens = checked_sum(b_i)`; and
- `user_exit_buffer = checked_sum(floor(b_i * b_rate_deadline / Q))`, flooring each claim before
  summing.

The inventory spans atomic-eligible, manual/top-up, auction-tail, and supplier-only classes.
`b_rate_deadline` is the greatest rate the exact pinned accrual can reach from the bound reserve
snapshot through the transaction authorization deadline under the live sealed rate configuration.
Missing positions/classes, an unprovable rate bound, or arithmetic failure blocks the lane. These
values reserve every still-withdrawable user claim without using market value, raw cash, or
aggregate-before-floor rounding.

The reviewed lineage plan's initial and every replacement `user_exit_btokens` equals `B`;
`user_exit_buffer` must be at least its derived minimum. Before each batch the plan refreshes both
from cited current-ledger reconciliation. A higher underlying minimum blocks until the record raises
the buffer. Let `S_B(x) = floor((b_supply - burn(x) - B) * b_rate / Q)`. A vault withdrawal is
permitted only when its exact stock burn fits the vault position, the stock withdrawal itself passes,
`B <= b_supply - burn(x)`, and user-preserving utilization is zero when `L == 0`, `U` when
`L >= S_B(x)`, or `ceil(L * U / S_B(x))` otherwise and is strictly below `U`. If no nonnegative `x`
satisfies those conditions, user-preserving accounted exit headroom does not exist and every
official vault withdrawal rejects; otherwise the **accounted exit headroom** is the largest such
`x`. Subtracting `B` models the exact reserve denominator after every inventoried user burns its full
b-token balance; the check never converts the aggregate underlying buffer back into b-tokens. When
`d_supply == 0`, both values become zero for the final reserve tranche; the exact stock and
raw-balance checks remain.

Let `R` be the same-invocation raw balance. `R < C` rejects. Otherwise `D = R - C` is live
unrepresented surplus: the batch records before/after `R`, `C`, and `D`, but does not compare `D`
with a pre-bound expected value. The raw balance must fund the realized transfer and, while
`d_supply > 0`, leave at least the current underlying `user_exit_buffer` after transfer. Under the
enabled stock paths `D`
cannot decrease; any observed decrease closes the lane, and any increase not explained by archived
token-transfer or stock-rounding evidence blocks the next batch pending reconciliation. Raw balance
is a fail-closed physical-availability and reconciliation check only: an uncredited donation or
rounding surplus never enters `burn`, `S`, `L`, `C`, accounted exit headroom, or the
accounting-derived withdrawal cap.

The batch does not retry after ambiguous submission until network and live state reconcile. Terminal
migration-owned idle and unexplained value are zero. If any exported vault path can bypass these
rules, or the exact vault/adapter source and authority cannot be reviewed, the lineage is direct-only.
Adapter repointing is forbidden.

### 8. Treat incidents as one-way events

Before T0, an incident is I0. Close routes, request status 4, record the actual result, and either
restore the exact planned prerequisites or abandon the cutover. Do not declare T0 from a failed
status call or an unavailable authority.
A successful I0 freeze records F4 even if queue cleanup still prevents T0. Preserve the absorbing
freeze and closed routes while completing only authorized cleanup and other prerequisites; a later
successful cancellation may establish T0 but cannot backdate it to F4 or retroactively claim that
the freeze transaction proved queue absence. Abandoning cutover never undoes a successful freeze.

After T0 and before sink ownership, an incident is I1. Preserve the established intake brake, stop
affected migration lanes, reconcile every ambiguous transaction and reservation, and replace a
faulty migrator, adapter, or successor only through a fresh review, canary, drained authorization
horizon, and protected route change. Never resume legacy intake.

A direct or uncooperative user may extend the tail indefinitely. Templar continues funded indexing,
oracle/TTL support needed for safe exits, repay, withdraw, liquidation, backstop withdrawal, and
honest public reporting. No percentage migrated or target date shortens those duties.
Fully explained unrepresented residue alone does not extend service after the complete section 9
terminal proof and sink handoff. A tripwire breach remains a disclosed, investigated incident; it
cannot be erased by relabelling rounding as donations or by later valuation changes.

### 9. Finish only at objective terminal state and sink ownership

The immutable `retirement_sink` exposes only `accept_pool_admin(pool)`. It has no storage, admin,
upgrade, arbitrary call, token, rescue, or pool-configuration method. It accepts admin for any pool
that has proposed it. ADR 0008's policy calls the intended pool's `propose_admin(sink)` only after
its on-chain predicates pass; the sink then authorizes only the nested `pool.accept_admin()` call.

A lineage is drained only when one reconciled ledger proves:

- every reserve has zero d-token supply; every exact aggregate b-token supply reconciles to the sum
  of positions; and each residual user balance converts through live `b_rate` to zero underlying
  asset units;
- every indexed user position has zero liability and no active auction; zero-raw collateral owned
  by a defaulted user moved to the pool-owned noncollateral position with an exact
  `collateral_orphaned`, and that position was removed only after reserve debt reached zero with
  exact `orphan_settled(asset, b_tokens)` plus the unchanged stock `gulp(asset, 0)` event; any other
  zero-asset-unit supply or collateral is archived by owner, position kind, b-token amount, live
  conversion inputs, and zero result as a rounding-loss claim;
- the pool-emissions map, every reserve b-token emissions record, and every pool-address b-token
  user-emissions record are absent;
- pool token balances reconcile exactly to accounting and source-attributed unrepresented residue.
  Physical value from settled pool-owned orphans, unsolicited transfers, and model-attributed
  rounding is archived separately by original source and exact base units as abandoned in place;
  no withdrawable user, vault, treasury, or protocol claim remains. Rounding above ADR 0008's
  25-USD operating tripwire is admissible only with the investigated incident and exact
  reference-model reconciliation retained. No USD ceiling, current-price requirement, new claim,
  donation relabelling, or sweep is part of terminal admissibility; unexplained residue still blocks;
- when vault-routed, legacy allocation, legacy adapter position, migration-owned idle, and
  cumulative conservation residual are zero after classifying each `debt_setoff` as the borrower's
  own same-reserve claim repaying its liability with no supplier loss; `defaulted_debt` as realized
  supplier loss; `collateral_orphaned` as borrower-claim extinction with unchanged denominator;
  later orphan accrual as pool-owned value; and `orphan_settled` plus stock `gulp(asset, 0)` as
  b-token removal and physical-value abandonment, not an underlying transfer or migration residual;
- backstop liability and backstop credit are zero, as structurally required by ADR 0008
  restrictions 2, 4, and 7 and confirmed by live reads, and no auction of any type is active as
  confirmed separately by live reads;
- the backstop is in ADR 0008's exact normal-empty or foreign-state terminal class, with real
  remaining owner/amount/unlock state archived;
- status 4 remains confirmed or all reserves remain disabled, and every policy-owned queue is
  resolved;
- bridge and vault reservations, authorization horizons, migrator instances, and submitted
  transactions are drained and reconciled;
- runtime configuration exposes the successor as the only current route; and
- the sink has accepted pool admin and live code/admin readbacks match.

The Markdown record cites exact transactions, ledger sequences, close times, point reads, and named
approvals for that terminal proof. UI delisting, vault target zero, elapsed time, or a percentage
migrated is insufficient. A later unsolicited transfer is archived as unrepresented abandoned
value; it creates no claim, does not block the completed terminal class, and does not reactivate the
legacy product.
These rules also cover rounding generated by repay/withdraw and other retirement actions: update
the incident and exact source reconciliation, rather than demanding that the remaining exit
sequence fit an unproven lifetime budget. Missing valuation is recorded honestly using the last
valid inputs and the investigated monitoring incident; it is not a terminal amount test.

### 10. Do not add migration incentives

This ADR authorizes no reward, rebate, per-address bonus, claim contract, or compensation promise.
Bridge capital and manual support exist only to execute bounded migration. Any incentive requires a
separate ADR and authority.

Before T0, publish old/new addresses, registry proof, behavior and risk differences, eligibility,
bridge limits, and manual support. After T0, report at least weekly supply, debt, vault allocation,
remaining classes, bridge headroom, status/disable state, and retirement progress until drained.

## Alternatives

1. **Wait and use manual exit/re-entry only.** Smallest code and strongest default. Rejected only for
   leveraged users who cannot source legacy repayment before collateral release; remains the path
   for complex positions.
2. **Adopt legacy addresses into the registry.** Falsifies deployment provenance and retains
   immutable dependencies. Rejected.
3. **Copy or import position storage.** Creates a large privileged state-migration surface and cannot
   preserve native user authorization semantics. Rejected.
4. **Move suppliers first.** Can strand borrowers and slower suppliers by draining legacy cash.
   Rejected.
5. **Keep passive status-0 runoff.** New public positions can replace exits and convergence is not
   guaranteed. Rejected.
6. **Build a universal router.** Broadens asset, topology, callback, and custody risk beyond the
   temporary bridge. Rejected.
7. **Never launch ADR 0008.** Simplest lifecycle. Preferred if the registry-native successor is ready
   before the bridge or ADR 0008 misses its abandonment gate.
8. **Use raw token balance minus buffers as the vault exit gate.** An uncredited donation can satisfy
   the check without increasing represented claims or stock withdrawal capacity. Rejected.
9. **Recognize donations through `gulp` before vault exit.** This would reopen the surplus/backstop
   path that ADR 0008 deliberately removes and change sealed economics merely to rescue an
   operational cap. Rejected.
10. **Make the pre-canonical canary readback-only, then enable ordinary signing for the first funded
    migration while routing remains closed.** This removes the candidate exception but grants the
    general bridge-signing capability before funded acceptance. The exact one-transaction permission
    is narrower and exposes the same real migration path, so the broader alternative is rejected.

## Consequences

The chosen order gives simple leveraged borrowers an atomic refinance path and lets repayment
replenish legacy cash before most supplier withdrawals. Templar-controlled vault liquidity can move
behind unchanged shares only when its exact migration path is independently qualified. Registry
membership remains truthful.

The cost is a period with two pools, split liquidity, bridge capital, a security-critical migrator,
possibly two vault adapters, a durable reservation ledger, continuous reconciliation, a retirement
sink, and an indefinite direct-user tail. The plan cannot promise completion on a date.

Removing the former custom publication protocol reduces code and review burden without discarding
its real goals. Protected Git review gives change approval, standard hashes bind artifacts, native
multisigs authorize actions, Soroban nonces and expiries constrain calls, the implementation ledger
prevents duplicate movement, and transactions plus live readbacks prove execution.

### Acceptance criteria

1. **Truthful lineage.** ADR-0008 addresses never become registry members. Each literal lineage has
   exactly one current successor and one official route.
2. **Readable evidence.** One protected-PR Markdown record contains exact identities, source,
   artifacts, transactions, live readbacks, inventory, approvals, capacity, incidents, and terminal
   state. No custom serialization is required to review it.
3. **Complete inventory.** Deployment-to-current ingestion plus point reads reconcile every reserve,
   position, auction, backstop, vault, and route observation. A gap or unexplained amount blocks T0.
4. **Ordered cutover.** The initial lane progresses only through
   `INTENT -> DEPLOYED -> CANARY_AUTHORIZED -> CANARY_PENDING -> CANARY_PASSED -> CANONICAL ->
   SIGNING_ENABLED -> OPEN` and establishes the lineage's single T0. A replacement uses a fresh lane
   inside that same immutable lineage and retains T0. Deployment checks move no user value. Only one
   exact reviewed, capped transaction may receive a pre-canonical bridge signature; its successful
   reconciliation precedes canonicalization. Canonicalization precedes ordinary bridge signing.
   For the initial lane, legacy routes close and the successor opens only after the intake brake and
   T0 reconciliation pass. For a replacement, the predecessor remains current until its route,
   reservations, and authorization horizons drain; only then may the record switch its sole current
   successor and open the replacement route. Authority/TTL/inclusion/queue failure cannot establish
   T0, replace it, or skip a phase.
5. **Bounded bridge.** Balance, per-call advance, pending reservations,
   included-but-unreconciled deficits, and authorization lifetime remain within reviewed limits;
   capacity covers the largest eligible position and replenishes only after confirmed bridge
   restoration and reconciliation. Reconciled settled volume remains in conservation history but
   not current exposure. Candidate phases permit only the one exact reviewed canary authorization
   attempt; pending or ambiguous outcomes block every new attempt, not the threshold shares needed
   to complete the same pending transaction.
6. **Atomic migrator.** Exact-Wasm tests and canaries prove strict asset correspondence, fixed
   min/max values, full selected legacy exit, bounded healthy successor state, user residuals, bridge
   restoration, zero migrator balance/allowance, conservation, replay rejection, and full rollback.
   Exact-Wasm checks reject `ordinary` without the on-chain brake and enforce full-request user and
   fixed-bridge authorization in both modes. Signer/worker tests, not Wasm tests, enforce phase,
   route, and final-hash checks. Release/signer schemas require pre-signature fields in
   `CANARY_AUTHORIZED`, pending reservation before bridge signatures, and actual outcome fields only
   for reconciliation; unknown/skipped phases and future-result placeholders reject.
   The production regression begins with a genuinely new lane and no prior authorization, proves
   deployment checks cannot sign, executes the one funded canary under its bounded permission, and
   reaches ordinary signing and first routed migration without fabricated canonical state or signer
   bypass.
7. **Crash-safe operations.** Recording precedes signature and inclusion. Before canary signature
   emission, one single-writer transaction marks its existing reservation `CANARY_PENDING`.
   Inclusion precedes deadline, transaction and authorization bounds agree, unknown outcomes stop
   the lane, and no reservation, phase, or capacity is reused before exact network/live-state
   reconciliation.
8. **Vault conservation and redeemable liquidity.** When applicable, every legacy-decrease path uses
   the authorized checked batch or rejects. While debt exists, complete point-read inventory derives
   the exact retained user b-token denominator and, using per-position floor conversion at the
   greatest pinned `b_rate` reachable through authorization expiry, the minimum underlying
   `user_exit_buffer`; the b-token record equals the former and the underlying record is never below
   the latter. Exact stock rounding and the strict utilization boundary subtract the b-token record
   from the post-vault reserve, while raw balance retains the underlying buffer. Every protected
   record refresh uses cited reconciliation; a required underlying increase blocks, and debt-zero
   makes both values zero for the final tranche.
   Raw balance below accounted cash, transfer need, or the live underlying buffer rejects. Positive
   unrepresented surplus is recorded without increasing or griefing the official cap; a decrease
   closes the lane, and an unclassified increase blocks the next batch. Per-batch and cumulative
   equations close, and terminal idle/unexplained value is zero. Otherwise the lineage is explicitly
   direct-only.
9. **Incident containment.** I0 never manufactures T0. I1 never reopens legacy intake. Every
   replacement is a fresh lane within the existing immutable lineage, never an alias or repointing;
   it retains the lineage's original T0, and the predecessor remains the sole current successor
   until its route, reservations, and authorization horizons drain before the protected record
   switches to the replacement.
10. **Objective retirement.** One reconciled ledger proves zero liabilities, auctions, pool
    b-token emissions configuration/data, withdrawable collateral, and withdrawable represented
    claims; proves exact pool-owned orphan transfer and post-debt settlement with b-token event plus
    zero-underlying stock return/event for every defaulted user's zero-raw collateral; proves every
    other residual b-token balance converts to zero underlying asset units and archives its exact
    inputs/result as a rounding-loss record; archives physical surplus after orphan settlement as
    abandoned value; and proves an ADR-0008 backstop terminal class, drained migration authority,
    durable intake brake, one successor route, and sink admin ownership.
11. **No incentives.** No migration code, configuration, approval, or authority can pay or imply a
    reward or compensation.
12. **Current review.** `adr --robot lint` and `git diff --check` pass. A fresh sequential Raladr
    Architect and distinct Critic approve the same candidate commit and ADR blob IDs. Only a human
    accepts it.

### Verification strategy

This ADR adds no implementation. The expanded implementation test plan is:

- **Unit/property:** migrator eligibility, strict asset matching, native authorization, min/max
  values, conservation, final state, replay behavior, reservation chronology, exact stock
  withdrawal conversion/utilization rounding, zero-raw and zero-asset-unit rounding boundaries,
  orphan lifecycle, and rollback. Complete generated position inventories prove
  `user_exit_btokens == checked_sum(b_i)` and, over deadline-bounded rate states,
  `user_exit_buffer >= checked_sum(floor(b_i * b_rate_deadline / Q))`; any b-token inequality or an
  underlying record one unit below rejects, as do missing/class-duplicate positions or an
  unprovable rate bound. For the same reserve, vault
  position, and retained pair, uncredited positive transfers cannot change the accounting-permitted
  maximum withdrawal; raw deficit rejects, while `D` is nondecreasing across every enabled stock
  deposit, withdrawal, borrow, repay, default, accrual, and denominator-safe settlement transition.
  In each concrete vector, `b_rate_deadline = b_rate`.
  With `b_supply = 1,000`, `b_rate = Q`, `d_supply = 900`, `d_rate = Q`, a 900-b-token
  vault position, user positions summing 100 b-tokens, and both retained values `100`, requested
  withdrawal `90` rejects at raw balances `100`, `101`, `200`, and `250`. With a 995-b-token vault
  position, five one-b-token user positions, and both retained values `5`, maximum withdrawal is
  `94` in every raw variant; raw balance `99` rejects as a deficit. With
  `b_rate = 1,234,567,890,123`, `d_rate = 1,100,000,000,000`, the other quantities unchanged, and
  raw balances `244`, `245`, and `344`, maximum withdrawal is `237` in every case and `238` rejects;
  raw balance `243` rejects as a deficit. With `b_supply = 5`, `b_rate = 1.7Q`, `d_supply = 1`,
  `d_rate = Q`, a two-b-token vault, three one-b-token user positions, `B = 3`,
  `user_exit_buffer = 3`, and raw balance `7`, a one-unit vault withdrawal rejects because the
  post-vault/post-user reserve supply equals its liability. The implementation property
  differentially compares its permission result with real stock `submit` outcomes on the exact
  release Wasm for generated rounding-boundary states;
- **Integration:** exact-Wasm borrower and vault flows, live identity/configuration checks,
  same-invocation reserve/raw reads, separate accounted-headroom and raw-balance gates, before/after
  `R`/`C`/`D` receipts, positive-transfer classification without cap increase or exact-match
  griefing, deficit and surplus-decrease closure, unexplained-increase pause/reconciliation,
  complete user-position inventory, b-token-denominator retention, deadline-rate underlying buffer
  derivation, record reduction/increase, debt-zero final-tranche liveness, batch idempotency,
  crash/retry/unknown-outcome behavior, cross-reserve and cyclic default liveness, stock
  `PositionData.collateral_raw == 0` boundaries including positive-rate conversion/oracle rounding,
  invariant aggregate `b_supply` while debt remains, rollback, absent emissions configuration/data,
  later rate/price revival under pool ownership, post-debt orphan settlement, b-token versus
  underlying event/return units, zero-asset-unit terminal classification, and per-asset cumulative
  conservation of supplier loss, borrower-claim extinction, pool-owned revived value, abandoned
  physical surplus, and zero-underlying stock `gulp` delta;
- **Freeze/cleanup chronology regression:** freeze with an exact legitimate policy-owned pending
  disable at ledger L, deliberately include its authorized frozen cancellation at L+1, and retain
  both transaction results and state snapshots. At L require F4=L, T0 absent, the outstanding queue
  honestly recorded, and no successor-route opening or ordinary-signing permission. At L+1, if
  every chosen boundary predicate first holds, require T0=L+1 while F4 remains L. A failed
  cancellation, another pending queue, or unresolved queue evidence must keep T0 absent. Cover an
  already-clear freeze with F4=T0, a fallback with no F4, and later idempotent freezes or delayed
  inventory reconciliation without timestamp replacement. Exact-Wasm/testnet coverage must use
  separate confirmed ledgers for the main case, not mock atomic cleanup; same-ledger evidence must
  retain transaction/state provenance rather than infer queue absence from ledger equality.
- **End-to-end/testnet:** public-testnet rehearsal and a fresh production-lane regression beginning
  with no prior authorization. It proves `INTENT` and `DEPLOYED` cannot sign; an unrecorded,
  mismatched, expired, second, or ordinary transaction cannot use `CANARY_AUTHORIZED`; the exact
  capped borrower canary records pending before signing and completes one funded migration; an
  ambiguous result blocks retry and all other signing; a separately exercised included failure
  preserves its reservation despite source-sequence consumption and user nonce rollback, and resets
  only after every issued authority/transaction bound expires plus exact failure reconciliation;
  retry requires fresh review. Signer/construction tests reject bridge address credentials and verify
  the recorded fresh-key custody/attestations without claiming a host credential-discriminant check.
  Pinned native XDR/host tests verify fixed-bridge root/source authorization, stable hash after outer
  signatures, changed hash after address-auth insertion, and replay rejection under another
  source/sequence/hash, including reconciliation of third-party wrappers by exact inner identity.
  Successful reconciliation alone permits `CANARY_PASSED`; only the subsequently reviewed `CANONICAL`,
  `SIGNING_ENABLED`, T0, and `OPEN` transitions permit ordinary signing and broad routing. Also run
  one vault batch when vault-routed,
  plus full status-4/all-disabled T0, I0/I1, successor replacement, terminal-class, and sink-handoff
  drills; and
- **Terminal-residue regression:** initialize retirement with no represented economic obligations,
  all pre-handoff terminal prerequisites satisfied, and exactly 25.01 USD of fully model-attributed
  rounding surplus. Require investigated-incident archival and successful sink handoff with the
  same base units still in the pool, source still rounding, and no created claim or transfer.
  Repeat after tail actions cross the tripwire and after price-only revaluation or unavailable
  current prices. An unexplained base unit, any withdrawable claim, unsettled orphan, live debt,
  auction, queue, authorization, or missing sink ownership must still prevent `drained`.
  Exact-Wasm tests must exercise the policy handoff too: raw positive balance or tripwire excess
  alone cannot reject it, and no live-price check may be required solely to abandon this residue.
- **Observability:** exercised alerts for unknown/skipped lane phase, candidate-phase signature
  mismatch or duplication, stuck `CANARY_PENDING`, route/phase disagreement, code, authority,
  configuration, queue, inventory, bridge reservations/capacity, successor cash/caps/oracle, vault
  accounted-headroom/raw-balance gates, buffer-record changes, `D` decrease/unclassified increase,
  tail TTL, and retirement.

### Risks and mitigations

| Risk | Prevention / detection | Recovery |
| --- | --- | --- |
| Pre-canonical canary permission becomes a general signing bypass | one exact reviewed transaction hash and bounds; distinct fail-closed phases; pending-before-signature transition; reject second, mismatched, expired, skipped, or unknown phase | keep routes closed, preserve the reservation, reconcile inclusion/non-inclusion, return only to `DEPLOYED` after definitive failure, and require a new reviewed authorization |
| Migrator steals or partially moves value | immutable narrow surface; exact native auth; fixed assets/bounds; atomic postconditions; focused review | stop signing, reconcile, drain authority, replace through fresh review |
| Successor cannot accept a position | per-position live health, oracle, cash, cap, utilization preflight | retain serviced tail; use manual top-up/partial repay or wait |
| Bridge liquidity is insufficient | largest-position gate; one-tranche reservations; per-cycle reconciliation | pause, add approved cash, or reduce tranche size |
| Submission outcome is ambiguous | record before signing; bind transaction hash; network/live-state reconciliation before retry | stop the lane; never free capacity on assumption |
| Suppliers race ahead of borrower repayment, or surplus masquerades as exit liquidity/griefs batches | borrower-first batches; accounting-derived exit headroom with exact stock rounding; `R < C` fail-closed gate; recorded `D` without cap effect; decrease/unclassified-increase alerts; donation-invariance regression | pause supplier batches; reconcile `D`; refinance or repay borrowers; reduce tranche size or retain the serviced tail; never send uncredited cash to the legacy pool |
| Retained b-token denominator differs from the exact current claim sum, or the underlying buffer undercounts claims or accrual through authorization expiry | complete point-read inventory; exact b-token equality; per-position floor conversion; conservative deadline `b_rate`; exact record and sequential-exit properties | close the vault lane, correct and re-review the lineage record, or remain direct-only |
| Vault accounting transcription drifts from pinned stock rounding | release-Wasm differential property and exact rounding vectors on every pinned build | close the vault lane, correct and re-review the adapter, or remain direct-only |
| Vault path bypasses conservation | enumerate exports; separate adapters; checked native batch; exact transfer receipts | disable vault migration and remain direct-only |
| Successor or route changes underfoot | one protected route; live executable/admin/config checks | close route, drain old authority, review and canary one replacement |
| Status 4 fails | exact canary; narrowly defined all-disabled fallback | keep routes closed; establish a real durable brake or remain pending |
| Direct user extends tail | complete inventory; funded exit dependencies; no date promise | maintain service and reporting until objective terminal state |
| Residual collateral is mistaken for auctionable or migratable value, revived for a borrower after debt default, orphaned without preserving the reserve denominator, accrues an unreachable emissions claim, or a withdrawable dust claim is marked terminal | ADR-0008 stock zero-raw predicate, pool-owned orphan, emissions-absence guards, post-debt settlement with zero-unit stock compatibility, and per-position live conversion proof; exact reserve/position/event archive; rounding, cycle, revival, rollback, and terminal tests | correct the record, keep routes closed, complete orphan settlement only after reserve debt and emissions claims are zero, and migrate every withdrawable unit |
| Admin could reactivate an empty pool | sink transfer after full terminal proof | no in-place recovery; future market is a new deployment |

### Pre-mortem

1. **A canary or later retry duplicates value movement or frees bridge capacity early.** Early
   warning: a candidate-phase signature does not match the exact canary, two distinct transaction
   hashes reference one approval, ambiguous transaction status, sequence mismatch, or reservation/live-balance
   disagreement. Mitigation: distinct lane phases, one exact pre-canonical permission,
   pending-before-signature recording, and transaction-hash/live-state reconciliation. Recovery:
   halt the lane, preserve the reservation, and resume only after exact resolution; a definitively
   failed canary returns to `DEPLOYED` and requires a new reviewed approval.
2. **Cutover drains legacy cash before borrowers can refinance.** Early warning: accounted exit
   headroom falls, either retained claim record violates its derivation, supplier withdrawals outrun
   repayment, raw balance fails reconciliation, or largest-position capacity fails. Mitigation:
   borrower-first order, complete-inventory b-token and deadline-rate underlying derivation, exact
   per-tranche gates, and donation-invariance checks. Recovery: pause supplier batching, complete
   borrower-authorized refinance/repayment through stock `submit`, correct and re-review an invalid
   claim record, reduce tranche size, or retain the legacy tail; never inject uncredited cash to
   raise the official cap.
3. **The team declares completion while a claim, queue, or authority remains.** Early warning:
   inventory/aggregate mismatch, any residual balance converting to an underlying asset unit, any
   pool b-token emissions state, missing `collateral_orphaned`/`orphan_settled`/stock-`gulp(0)`
   evidence, unresolved policy phase, live authorization horizon, or nonzero vault/bridge residual.
   Mitigation: one objective terminal checklist that distinguishes pool-owned denominator positions,
   revived pool-owned value, abandoned physical surplus, and zero-asset-unit rounding records from
   withdrawable user claims, with point reads and sink handoff last. Recovery: retract the completion
   claim, keep routing closed, and continue funded reconciliation and tail service.

### Invariants

- The literal network passphrase and legacy pool address identify one lineage with one current
  registry-native successor.
- Cutover follows the exact lifecycle and order in section 4; no phase may be inferred or skipped.
- Every supported value movement has exact native authorization, fixed per-asset bounds,
  conservation, and atomic rollback. Before ordinary signing, the only permitted movement is the
  exact `CANARY_AUTHORIZED` transaction, recorded pending before signature emission.
- No crash, retry, worker, or upgrade can duplicate movement or reuse bridge capacity before the
  original transaction is resolved.
- Every official vault batch uses complete point-read inventory to derive
  `user_exit_btokens == checked_sum(b_i)` for all remaining user-owned positions and, at the
  conservative deadline rate, an underlying `user_exit_buffer` no smaller than the separately
  floored user claims. Exact stock accounting retains the former in the reserve denominator while
  debt exists; raw balance retains the latter. Every record refresh uses cited current-ledger
  reconciliation, a required underlying increase blocks until recorded, and debt-zero sets both
  values to zero so the final tranche can exit.
  Raw balance below accounted cash rejects; positive `D` neither enlarges the withdrawal cap nor
  rejects by itself, while a decrease closes the lane and an unclassified increase pauses the next
  batch. Realized legacy value is conserved into successor supply, user redemption, approved fees,
  or bounded temporary idle; terminal idle and unexplained value are zero.
- `debt_setoff` is an explained borrower self-repayment with no supplier loss. `defaulted_debt` is
  an explained supplier loss. `collateral_orphaned` extinguishes borrower
  ownership without changing the denominator; any later underlying accrual is pool-owned; and
  `orphan_settled` plus stock `gulp(asset, 0)` removes b-tokens and abandons physical value without
  reporting an underlying transfer. None is an unexplained migration conservation residual.
- Pool b-token emissions configuration/data is absent throughout the cohort lifetime.
- Zero-raw collateral owned by a defaulted user moves atomically to pool-owned noncollateral supply
  without changing reserve `b_supply`, then is removed only after reserve `d_supply == 0`. Every
  other residual user b-token balance is terminal only when its live conversion yields zero
  underlying asset units; it is an archived rounding-loss record, not migratable value.
- After T0, no official route or operation increases legacy exposure.
- Completion requires objective accounting terminality as defined above, a valid backstop terminal
  class, drained migration authority, one successor route, and irreversible sink ownership.
- No migration incentive exists.

### Follow-ups

- Amend the accepted permanent-profile ADRs to exclude ADR-0008 addresses from registry membership.
- Implement and independently review the migrator, bridge reservation logic, retirement sink, and
  fail-closed lane-phase policy consumed by signing and release tooling.
- Implement and review vault migration mode only for lineages that need it; otherwise stay direct-only.
- Create and approve the Markdown lineage record and protected runtime-route change per market.
- Exercise the full fresh-lane canary, cutover, and terminal path before any production T0.
