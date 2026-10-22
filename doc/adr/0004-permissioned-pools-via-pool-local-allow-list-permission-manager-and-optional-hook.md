# 4. Permissioned pools via pool-local allow-list, permission manager, and optional hook

Date: 2026-08-27

## Status

Proposed

Amends [2. Pool registry as deployment, configuration, and optional-backstop routing layer](0002-pool-registry-as-deployment-configuration-and-optional-backstop-routing-layer.md)

Amends [3. Protocol fee on borrow interest; defer origination and flash-loan fees](0003-protocol-fee-on-borrow-interest-with-deferred-origination-and-flash-loan-fees.md)

## Context

Institutional pools need provider-maintained admission for participant supply, borrowing,
user-directed receipt, user-liquidation fills, and BLND rewards. Templar must support Predicate as
well as operator- or institution-specific policy without making one provider's API the pool's
public entrypoint.

The vendored Blend contracts (`vendor/blend-contracts-v2` @ `6cf6dd1`) currently have no such
admission layer:

- `submit` and `submit_with_allowance` authenticate `spender` and, when distinct, `from`; `to` is
  unauthenticated. `flash_loan` authenticates `from`, and `claim` authenticates `from` while paying
  BLND to arbitrary `to` (`pool/src/contract.rs:452-494,539-547`).
- Pool outflows normally go to `to`, while auction lots go to the filler. Repay overpayment is
  refunded to `to`, so even a nominal exit batch can pay a third party
  (`pool/src/pool/submit.rs:198-245`;
  `pool/src/pool/actions.rs:426-431`;
  `pool/src/auctions/backstop_interest_auction.rs:109-118`;
  `pool/src/auctions/bad_debt_auction.rs:117-121`).
- Positions are internal maps rather than Stellar asset-contract balances, so issuer authorization
  flags cannot gate them (`pool/src/pool/user.rs:11-15`).
- The submit path currently builds actions, validates them, performs transfers, and stores state.
  Auction construction can mutate cached state before final validation, so admission must precede
  accounting work (`pool/src/pool/submit.rs:25-65,68-147`;
  `pool/src/pool/actions.rs:131-280`).
- Soroban contract re-entry is prohibited by the pinned host, but a called contract can still invoke
  tokens or unrelated contracts, consume allowances, request authorization, and mutate state later
  read by the pool. A hook is therefore an external trust boundary, not a sandbox.
- `claim` transfers newly issued BLND; it is not repayment or withdrawal of the claimant's reserve
  assets (`pool/src/emissions/distributor.rs:52-60`). Treating claim-to-self as an exit would bypass
  the stated requirement that reward recipients remain eligible.

ADR 0002 supplies the registry, typed timelocked governance, implementation compatibility, pool
upgrade path, and optional-backstop routing. ADR 0003 supplies the final interest-only fee ABI and
exact supplier-value semantics. Both are predecessors of this decision and must land first.

Predicate's Soroban reference at `predicatelabs/predicate-contracts` commit `9206b2d` uses a
single-use, expiring off-chain attestation. Its upgradeable registry authenticates the caller,
checks expiry, UUID/expiration equality, registry-instance-local UUID replay, and attester
registration, then verifies the caller-bound signature and returns `Result<bool, _>`
(`soroban/predicate-registry/src/validation.rs:51-132`). It does **not** compare the statement's
signed `policy` field with the separately advertised `set_policy_id` value. Invalid signatures trap
inside `ed25519_verify` rather than returning a reachable typed `InvalidSignature`. The current
implementation returns only `true` on success, but a compatible client must still reject an
explicit `false`. Its spent-UUID marker is persistent storage extended to the network's maximum
live TTL. Under pinned SDK semantics an expired persistent entry is archived and restorable and
cannot be recreated; archival is therefore a restoration/footprint incident, not a successful
replay path (`soroban-sdk/src/storage.rs:22-25,79-87`). Predicate
has no published mainnet registry id or Soroban audit at draft time, and its client pins Soroban SDK
23.5.3 rather than the cumulative Blend release toolchain. The integration therefore uses a small
boundary client and cannot ship on an inferred encoding or a mock-only compatibility claim.

Constraints and assumptions:

- `vendor/` remains read-only here. Contract changes are separate upstream work.
- This is a greenfield public/storage version. No non-disposable pool may use an interim ADR-0004
  layout; discovery of one blocks release and requires a migration ADR.
- Every optimized contract remains at or below
  `min(131072, active_network.contractMaxSizeBytes)` on its deployment network.
- A revoked or expired participant can repay and withdraw reserve assets to itself without an
  admission read or hook. A live user-liquidation auction must be deleted in the same self-defense
  batch; this ADR permits that exact `Repay`/withdraw/`DeleteLiquidationAuction` batch to bypass the
  baseline status>1 deletion gate while retaining post-batch health and every other validation.
  BLND rewards are not part of that exit guarantee.
- Relative to the reconciled ADR-0002/0003 baseline, open pools add no per-address permit entries,
  permit-TTL work, ADR-0004-specific submit cap, or admission cross-contract call. They still
  inherit ADR 0002's universal `MAX_SUBMIT_REQUESTS = 60` bound. If the Accepted issuance decision
  enables `claim`, ADR 0004's conditional claim-id cap applies to those invocations on every pool,
  including an open pool, without activating a manager or hook that is absent.
- Backstop-depositor permissioning and BLND issuance policy are separate decisions. Pool admission
  does not restrict who may deposit into its backstop, so product language may call only the pool
  position/action surface permissioned. A product requiring permissioned insurance capital is
  blocked until a separate ADR supplies it.
- Interest- and bad-debt-auction fillers are liveness-critical system-settlement actors, not admitted
  participants. They may receive reserve underlying, backstop tokens, or liability positions without
  a manager permit or hook. Product language must disclose this recipient class; a product requiring
  every asset or liability recipient to be admitted cannot ship under this ADR.
- ADR 0002's emissions-disabled zero-value `claim` remains admission/hook-free. Production genesis
  requires an Accepted issuance decision; if it enables the pool `claim` path, every invocation must
  retain the current signature and adopt the recipient and hook contract below, or that decision
  must expressly supersede this ADR before the cumulative ABI freezes.

This ADR decides observable admission semantics, public wire types, interfaces and events,
authority, epoch invalidation, hook trust boundaries, compatibility, and release gates. It does
not select storage keys or internal layouts for the greenfield implementation, TTL-renewal
algorithms, helper or module names, internal factoring, a prototype, proof-harness names, runner
phases, replay architecture, or evidence-file layout. Ordering is normative only where an
externally visible side effect or security boundary depends on it. Equivalent implementations are
valid when they satisfy the public contract, atomicity, invariants, and acceptance criteria below.

Principles:

1. Principal exit to self remains available without eligibility or hook liveness; entry,
   borrowing, third-party participant receipt, user-liquidation fill, and every issuance-enabled
   `claim` invocation require the applicable admission. Protocol-fee collection and interest/bad-
   debt settlement are system flows, not participant admission points.
2. Policy providers write pool-local permits and the pool reads them. Predicate is one manager
   implementation, not the protocol's identity provider.
3. Authority changes cannot resurrect grants or unused attestations. Every manager identity
   transition and every effective adapter configuration change advances one checked epoch
   automatically.
4. Every pool inherits ADR 0002's fixed submit-vector bound. Open pools perform no per-address
   permission read/write or external admission call; ADR 0004 adds only the conditional claim-id
   bound and at most one hook call per admitted user entrypoint.
5. Registry hook membership is a governance record, not on-chain proof of code identity,
   immutability, audit, configuration, or operational fitness.

Decision drivers, ranked:

1. Provider-maintained eligibility at institutional-pool launch without mandatory hot-path trust
   in Predicate's upgradeable registry.
2. Exit safety, deterministic revocation, and liquidator liveness.
3. A small provider-agnostic public surface that remains maintainable through the one-year upgrade
   period.

## Decision

Add two independent, optional seams to each pool:

1. a pool-local expiring permit list written by one permission-manager address; and
2. a registry-admitted hook called before non-exit user actions except the liveness-critical system
   settlements below, and before every issuance-enabled `claim` invocation.

A pool may enable either, both, or neither. With neither, no manager or hook admission behavior is
added; ADR 0002's submit cap and ADR 0004's conditional issuance-enabled claim-id cap apply
independently of those seams.

### Public permission state and interface

The stable boundary type and bit assignments are:

~~~rust
struct Permit {
    flags: u32,
    until: u64,
    epoch: u64,
}

const PERMIT_SUPPLY: u32 = 1;
const PERMIT_BORROW: u32 = 2;
const PERMIT_RECEIVE: u32 = 4;
const PERMIT_LIQUIDATE: u32 = 8;
const KNOWN_PERMIT_MASK: u32 = 15;
const MAX_PERMISSIONED_CLAIM_IDS: u32 = 60;
~~~

The pool's observable state adds optional manager and hook addresses, a logical `u64` permit epoch
initially zero, and one `Permit` per granted address. This ADR freezes those values and public wire
shapes; internal storage is implementation-owned. After deployment, any incompatible storage change
or reinterpretation requires a migration ADR. A pool with no manager creates no per-address permit
state or permit-renewal duty.

The pool adds:

~~~rust
fn set_permission_manager(e: Env, manager: Option<Address>) -> u64
fn set_permit(e: Env, addr: Address, flags: u32, until: u64)
fn revoke_permit(e: Env, addr: Address)
fn revoke_all(e: Env) -> u64
fn get_permit(e: Env, addr: Address) -> Option<Permit>
fn get_permission_manager(e: Env) -> Option<Address>
fn get_permission_epoch(e: Env) -> u64

fn set_hook(e: Env, hook: Option<Address>)
fn get_hook(e: Env) -> Option<Address>
~~~

`PoolError::NotPermitted` is error code `1225`. Malformed masks, invalid times, same-value
manager/hook transitions, and oversized issuance-enabled claim-id vectors fail with `BadRequest`.
Oversized submit request vectors use ADR 0002's existing `BadRequest` contract.

`set_permission_manager` requires pool-admin authorization. Every actual transition—`None` to
`Some`, `Some` to `None`, or one manager address to another—checked-increments the permit epoch and
changes the manager atomically, then returns the new epoch. A same-value call is rejected. Epoch
overflow rolls back without changing either value. This setter makes no external call and invokes no
manager or hook callback. The pool itself is not a valid manager.
Consequently, disabling and later re-enabling the same manager cannot revive a permit from an
earlier tenure. `Some(manager)` makes that address the only manager recognized by the pool and
`get_permission_manager()` returns it. `None` disables manager authorization,
`get_permission_manager()` returns `None`, and no prior manager address remains effective. The
internal representation is implementation-owned.

`set_permit` and `revoke_permit` require authorization from the current manager. A permit must have
`until > ledger_timestamp`, at least one known bit, and no unknown bit. A write records the current
epoch. A zero-mask grant is rejected rather than treated as an implicit revoke. The implementation
owns its renewal strategy, but release evidence must show that an issued permit remains readable
through `until` under the documented operating assumptions.

`revoke_all` requires current-manager authorization and checked-increments the same epoch. It
returns the new value. No epoch transition can wrap, and no stale entry can become current without
a new authorized write at the new epoch.

`get_permit` is epoch-aware. It returns `None` for an earlier-epoch entry and returns an expired
current-epoch entry without treating it as live. `get_permission_epoch` returns the epoch used for
authorization. An unexpectedly archived entry cannot make a permission live; operational renewal
and restoration remain release obligations for long-lived grants.

`set_hook` requires pool-admin authorization. `Some(h)` rejects the pool itself and requires the
registry's current `is_hook(h)` view to return an enabled interface version satisfying the concrete
pool implementation. `None` detaches immediately. Disabling a hook in the registry blocks new
attachments but does not mutate pools already using it. `Some(h)` makes that address the active
hook and `get_hook()` returns it. `None` disables the hook seam, `get_hook()` returns `None`, and no
detached address is invoked. The internal representation is implementation-owned.

Pool-admin threshold loss is an accepted permanent-impairment boundary, not silently delegated to
the registry. ADR 0002's registry/recovery authority may still contain a callable pool to stop new
risk, but it cannot rotate/detach this manager or hook, change status/configuration, resume/retire,
or co-authorize a pool upgrade. If the installed manager or hook is unavailable at the same time,
user-liquidation admission and repair may remain unavailable indefinitely; admission-free
interest/bad-debt settlement and the exact self-defense exit path above remain available where the
pool itself is callable. Production therefore inherits ADR 0002's threshold-custody, signer-
independence, backup, rotation, and loss/compromise rehearsal gate. After actual loss, operators
contain when possible, stop official routing, preserve repayment/settlement/exit, deploy a new pool,
and migrate only through ordinary user exits; the old address may never recover. A delayed registry
super-admin or guardian reset requires a separate deliberate ADR because it would weaken the chosen
dual-authority upgrade boundary.

### Admission matrix and exit boundary

ADR 0002 already caps `submit`, `submit_with_allowance`, and `flash_loan` for **every** pool at
`MAX_SUBMIT_REQUESTS = 2 * MAX_RESERVES = 60`. ADR 0004 neither duplicates nor conditions that
bound. Each entrypoint's inherited caller-authentication and self-address guard runs first; the
shared request-length guard then rejects 61 or more with `BadRequest` before pool/user load, flash-
liability creation, reserve or balance reads, admission lookup or hook call, any other cross-
contract call, mutation, or event. The subsequent admission rules therefore see at most 60
requests on open, manager-only, hook-only, and combined pools.

If the Accepted issuance decision enables `claim` under the retained signature, this ADR separately
adds `MAX_PERMISSIONED_CLAIM_IDS = 60` for every such invocation. After the retained `from`
authentication, a 61st reserve-token id fails with `BadRequest` before permit lookup, hook call,
emission accounting, event, authorization side effect introduced by the hook, or token transfer.
This conditional bound covers two emission token ids for each of the maximum 30 reserves. ADR
0002's emissions-disabled zero-claim path retains its own input rules and does not activate it.

For a permitted-size call, required bits are derived from the public arguments and combined by
address before any permit is checked:

| Entrypoint | Address | Complete required mask |
| --- | --- | --- |
| `submit*` | `from` | `SUPPLY` for any `Supply`/`SupplyCollateral`; `BORROW` for any `Borrow`; `LIQUIDATE` for any `FillUserLiquidationAuction`; no bit for `FillInterestAuction` or `FillBadDebtAuction`; bitwise union when several apply |
| `submit*` | `spender != from` | `SUPPLY` when the batch contains `Supply`/`SupplyCollateral`; no bit merely for funding `Repay` or an admission-exempt interest/bad-debt settlement |
| `submit*` | `to != from` | `RECEIVE` only when the batch contains `Withdraw`, `WithdrawCollateral`, `Borrow`, or `Repay`, whose refund may pay `to`; auction lots pay the filler and do not use this row |
| `flash_loan` | `from` | `BORROW` plus the complete request-derived mask |
| `flash_loan` | receiver contract distinct from `from` | `RECEIVE` |
| every `claim` invocation enabled by the Accepted issuance decision | `to`, including `to == from` | `RECEIVE` |

Unknown request types fail `BadRequest`. When one address occupies several rows, it needs the
bitwise union of every row; satisfying only one bit is insufficient. A live permit authorizes the
address exactly when its epoch is current, its expiry is strictly in the future, and its flags
contain the complete combined mask.

When both seams are configured, the complete local stage runs before the hook: derive and union all
required masks, then validate every required current permit. An absent, expired, stale, or partial
permit returns `NotPermitted` without invoking or authorizing the hook and without consuming hook
resources or touching hook state. Only after all required local permits succeed—or immediately
after mask derivation when no manager is configured—does the pool invoke the one hook callback.
This order applies to `submit`, `submit_with_allowance`, `flash_loan`, and every issuance-enabled
`claim`; a later hook rejection propagates its exact downstream failure and rolls back the call.

The exit exception applies only to pool reserve positions. A `submit*` call is an exit to self
when every request is `Withdraw`, `WithdrawCollateral`, `Repay`, or
`DeleteLiquidationAuction`, and `from == spender == to`. Such a permitted-size call performs no
permit read and no hook call. When a live `UserLiquidation` auction exists, the batch must include
`DeleteLiquidationAuction`; for this exact self-defense shape only, that request bypasses the
baseline status>1 action gate. Auction deletion, post-batch health, utilization, token funding, and
every other ordinary check still apply atomically, so an unhealthy user cannot delete the auction
without repairing its position. Omitting deletion still fails `AuctionInProgress`. The path remains
available when the participant is revoked, the permit is expired or archived, the hook is
unavailable, or the pool is frozen. A larger exit remains available as several calls of at most 60
requests, with the auction deleted in the first successful chunk.

The exception does not cover an issuance-enabled `claim`. The pinned amount is known only after
emissions accounting begins (`pool/src/emissions/distributor.rs:15-26,117-147,223-240`), so the pool
does not attempt a pre-accounting positive-amount preview. When issuance is enabled, every claim
invocation is admitted before that accounting regardless of its eventual amount: a manager requires
`RECEIVE` for `to`, including the authenticated claimant receiving to itself, and a hook runs
`on_claim`. A failed check or callback leaves emission state, token movement, and events unchanged.
A revoked participant may still remove reserve assets but cannot invoke the enabled reward path
until readmitted. A longer claim remains available in chunks of at most 60 reserve-token ids. ADR
0002's distinct emissions-disabled zero-value implementation invokes neither admission branch.

`FillUserLiquidationAuction` requires `LIQUIDATE`; lots already held by a later-revoked filler remain
withdrawable to self. Interest- and bad-debt-auction fills are liveness-critical protocol settlement,
not borrower liquidation: a batch containing only those fills performs no permit lookup and no
hook call. The authenticated filler is deliberately an unpermissioned recipient: an interest fill
can receive reserve underlying, while a bad-debt fill can receive backstop tokens and liability
positions (`pool/src/auctions/backstop_interest_auction.rs:86-118`;
`pool/src/auctions/bad_debt_auction.rs:96-121`). If mixed with an admitted participant action, that
other action contributes its ordinary mask and causes the one normal hook call. `new_auction`,
`gulp`, bad-debt processing, and administrative entrypoints retain their existing permission model
because they are not participant-directed receipt paths covered here.

ADR 0003 `collect_fees` is likewise an admission-exempt system transfer. Its recipient is selected
by timelocked registry governance and its on-chain claimant-reserve formula, not by the submitting
caller. It never requires `RECEIVE` and never invokes the pool hook; a manager or hook therefore
cannot block protocol-credit realization or redefine the registry's fee recipient.

Admission occurs after the entrypoint's existing caller authentication and pool-as-party rejection
but before any pool accounting load or mutation, auction state change, token transfer, or callback
other than the admission hook itself. Its order is required-mask derivation, all configured local
permit checks, then at most one hook call. Any admission-state maintenance performed on that path
rolls back with a denied or failed call.

### Predicate session adapter

A separate adapter contract can serve as the pool's permission manager. Its immutable pool and
owner addresses and absence of any upgrade export make the adapter's code immutable under this
ADR. A code or owner change deploys a new adapter and uses `set_permission_manager`, which
automatically invalidates every old grant; no same-address adapter upgrade exists.
Adapter-local durable blocks do not follow that address change; the replacement procedure below
reconstructs and verifies them before the new adapter becomes the manager.

Its public interface is:

~~~rust
struct Attestation {
    uuid: String,
    expiration: u64,
    attester: BytesN<32>,
    signature: BytesN<64>,
}

fn __constructor(
    pool: Address,
    predicate_registry: Address,
    policy: String,
    flags: u32,
    max_validity: u64,
    owner: Address,
)
fn attest(e: Env, user: Address, attestation: Attestation)
fn register_policy(e: Env)
fn set_config(
    e: Env,
    predicate_registry: Address,
    policy: String,
    flags: u32,
    max_validity: u64,
) -> u64
fn set_permit(e: Env, addr: Address, flags: u32, until: u64)
fn revoke_permit(e: Env, addr: Address)
fn revoke_all(e: Env) -> u64
fn freeze(e: Env)
fn block(e: Env, addr: Address)
fn unblock(e: Env, addr: Address)
fn install(
    e: Env,
    expected_current_manager: Address,
    expected_block_revision: u64,
) -> u64
fn get_pool(e: Env) -> Address
fn get_config(e: Env) -> (Address, String, u32, u64, u64)
fn get_owner(e: Env) -> Address
fn is_frozen(e: Env) -> bool
fn is_blocked(e: Env, addr: Address) -> bool
fn get_block_revision(e: Env) -> u64
~~~

Its exact local public error ABI is:

~~~rust
#[contracterror]
#[repr(u32)]
enum PermissionAdapterError {
    OverflowError = 12,
    InvalidConfig = 1400,
    InvalidAttestation = 1401,
    Blocked = 1402,
    PermitConflict = 1403,
    PredicateRejected = 1404,
    Frozen = 1405,
}
~~~

`InvalidConfig` covers an invalid or unchanged local configuration, a same-value block/unblock, and
an adapter-install expectation mismatch.
`InvalidAttestation` covers an invalid UUID or a signed expiration that is not strictly in the
future. `Blocked` covers either grant path for a denied address. `PermitConflict` covers a
Predicate grant that would replace a different live mask or would not extend the same live mask.
`PredicateRejected` covers an explicit `false`, which the pinned registry never returns but the
declared boolean success type permits. Checked timestamp arithmetic uses `OverflowError`.
`Frozen` covers every later adapter operation that could grant, change configuration or policy
advertisement, mutate the durable deny set, or install that frozen adapter. Repeating `freeze` is
`InvalidConfig`.
After required owner or user authorization, the frozen check precedes every other adapter-local
validation on owner/user operations other than the separately ordered `install` below. Immutable-
owner/user and pool-admin authorization failures remain host-auth failures. Reachable Predicate
typed errors, its invalid-signature crypto trap, other host failures, and downstream `PoolError`
values propagate unchanged; the adapter neither catches nor renumbers them. Every listed failure is
atomic and emits no durable adapter event.

`Attestation` is the exact public wire declaration at Predicate commit `9206b2d`,
`soroban/predicate-registry/src/types.rs:31-39`; its field names and types are part of the adapter
ABI. The unchanged `String` wire type does not imply an unbounded accepted domain. This adapter
accepts a policy or attestation UUID only when its Soroban `String::len()` is in `1..=64`.
Predicate's lockfile resolves the crates.io `soroban-sdk` 23.5.3 package and checksum; the matching
upstream `v23.5.3` tag resolves to `stellar/rs-soroban-sdk` commit `d3e1ab2`. In that source,
`String` is a contiguous byte sequence, `from_str` passes `str::as_bytes()`, `from_bytes`
accepts a byte slice, and `len()` returns the host byte length
(`soroban-sdk/src/string.rs:15-21,251-275`). The limits are byte limits, not Unicode-scalar or
displayed-character limits; ordinary Rust strings therefore count their UTF-8 encoding, while the
wire rule imposes no UUID format, ASCII, encoding-validity, or normalization requirement. Predicate
itself supplies no field limit at the pinned commit. The 64-byte adapter boundary admits its
36-byte golden UUID and its published `x-...` policy examples without turning either fixture
format into a provider guarantee.

Construction and configuration reject with `InvalidConfig` an empty or greater-than-64-byte policy
before local or Predicate-registry storage or event emission, as well as a zero or unknown flag mask
and zero validity. They require `ledger_timestamp + max_validity` to be representable; overflow is
`OverflowError`. The one configured mask is the exact permit shape granted by Predicate; the owner
passthroughs may create separate direct grants such as long-lived liquidator or vault permissions.

Adapter `set_permit`, `revoke_permit`, and `revoke_all` each require immutable-owner authorization.
`set_permit` rejects a frozen adapter or blocked address; neither direct grants nor attestations
can override a durable block before an owner-authorized `unblock`. Risk-reducing
`revoke_permit` and `revoke_all` remain callable after freeze while this adapter is still the
pool's current manager.

Construction authenticates the immutable owner, establishes the immutable pool binding,
`frozen = false`, and `block_revision = 0`, and either publishes the complete validated local
configuration plus Predicate's advertised policy id or rolls back. `register_policy` requires owner
authorization and an unfrozen adapter and rewrites that advertisement without changing pool
eligibility state.
Predicate's `validate` does not read the advertisement, so neither operation is described as a
second on-chain policy check.

`set_config` requires owner authorization, an unfrozen adapter, and an actual change to at least one
of registry, policy, flags, or validity. A successful change advances the pool epoch exactly once
and commits the complete new local configuration and advertised Predicate policy id atomically;
failure commits none of them. There is no caller-selected preservation switch: widening, narrowing,
provider rotation, policy-id change, and validity change all invalidate old Predicate sessions,
direct grants, and unused attestations because the signed statement binds the resulting epoch. The
internal order used to achieve that atomic result is not prescribed here.

Predicate's registry lets a caller advertise a replaceable policy id, but `validate` never reads
that record or compares it with `statement.policy`
(`soroban/predicate-registry/src/lib.rs:147-155`;
`soroban/predicate-registry/src/validation.rs:51-132`). The adapter's own construction of the exact
statement is therefore the only on-chain policy-string binding. Versioned policy identifiers remain
an operational provider/governance requirement, not a registry-enforced property. An official
policy change uses a new id and `set_config`, so the epoch advances with the local change.
Provider-side rule drift under an unchanged id and unchanged reviewed registry code remains live for
existing sessions until permit expiry, mass revocation, or a subject block. `revoke_all` alone does
not contain a provider able to sign a fresh current-epoch attestation. When the adapter owner remains
trusted and available, the exact first on-chain containment transition is owner-authorized
`freeze()` on the current adapter. It atomically records an irreversible frozen flag and, while this
adapter is the current manager, calls pool `revoke_all` to advance the epoch before emitting its
freeze event. Afterwards `attest`, `register_policy`,
`set_config`, `set_permit`, `block`, and `unblock` reject `Frozen`; no provider signature or direct
grant can restore risk intake, while pool self-exit and admission-free settlement remain available.
Suspending official submission and routing is additional incident containment, not a substitute for
that state transition. Recovery deploys a reviewed replacement adapter or deny-all manager,
preserves the frozen adapter's deny set through the handoff below, and changes manager before
resuming. A bounded
known subject may instead be blocked before freeze, but stand-alone `revoke_all` is never presented
as containment. Such a deployment cannot claim immediate or registry-proved policy cutover.
If the adapter owner is unavailable or suspected, the pool admin instead installs a pre-reviewed
immutable deny-all manager, whose manager transition advances the epoch and whose code has no path
that can authorize a pool permit mutation. Predicate registry code drift uses the same contain-and-
replace response. Neither branch resumes risk intake until a trusted deny set and manager are live.

`get_pool` and `get_owner` never change after construction. `get_config` returns the configured
registry, policy, flags, validity, and the pool's current permission epoch; the epoch is read from
the pool rather than duplicated in adapter state. Institutional deployment derives the future
deterministic pool address, constructs the adapter for that address, verifies `get_pool`, and passes
the adapter as the pool's initial manager. The returned pool address must match the prediction
before the pool leaves setup. This prevents a temporary open-pool window.

`attest` first requires the user's authorization, then rejects a frozen adapter with `Frozen`. It
rejects with `InvalidAttestation` an empty or greater-than-64-byte `attestation.uuid`, or an
expiration not strictly greater than the current ledger timestamp, at the adapter boundary before
constructing or hashing the statement, calling
Predicate validation, touching replay storage, writing a permit, or emitting an event. Provider
signing and release-client validation enforce the same byte and time domains so an invalid signed
object is rejected before submission as well. The adapter rejects a blocked user with `Blocked` and
constructs the complete Predicate statement with `uuid = attestation.uuid`,
`expiration = attestation.expiration`,
`msg_sender = user`, the adapter as target, zero value, the stored versioned policy, and
`encoded_sig_and_args` equal to the canonical XDR of the three-value tuple
`(Symbol("attest"), user, current_pool_permission_epoch)`. The fixed selector, WHO address, current
pool epoch, and authenticated sender bind the signature to this adapter method, subject, and manager
tenure. The UUID and expiration passed separately to Predicate validation are the same values in the
statement. `attest` reads the epoch once before the registry call and uses the same value for the
later pool permit write; any mismatch or failed write rolls back the complete invocation. It accepts
only an explicit `true` from the configured registry.

Predicate stores `(Symbol("uuid"), uuid) -> true` in persistent storage inside each registry
contract and the registry address is absent from the signed preimage
(`soroban/predicate-registry/src/validation.rs:21-27,67-71,89-114`). UUIDs are therefore global only
among integrators sharing one registry instance; a different registry has an independent replay
store. The adapter adds no duplicate UUID marker. A live marker rejects; restoring an archived
marker recovers `true` and still rejects, while the persistent key cannot be recreated as a new
unused entry. Binding the pool epoch prevents an unused signature from an old adapter
configuration, registry, manager tenure, or mass-revocation epoch from creating a current permit,
even when the replacement registry knows the same attester and has not seen that UUID. A fresh
attestation issued for the current epoch can still grant after `revoke_all`, so durable exclusion
continues to require `block`. An explicit `false` maps to `PredicateRejected`; a typed error,
same-registry UUID collision/replay, expiry, invalid-signature host trap, or later permit-write
failure propagates and rolls back the entire invocation, including Predicate's spent-identifier
write. The adapter cannot catch or translate an invalid signature into Predicate's declared but
unreachable typed `InvalidSignature` variant.

The resulting expiry is the lesser of the signed attestation expiry and
`ledger_timestamp + max_validity`, using checked arithmetic. Overflow is `OverflowError`. A live
permit with the same configured mask may only have its expiry extended; a non-extension or different
live mask is `PermitConflict`, and the owner must revoke the old permit first. Expired or stale
permits may be replaced at the current epoch. This prevents a Predicate session from silently
narrowing or widening a direct grant.

`freeze()` requires immutable-owner authorization and rejects an already frozen adapter with
`InvalidConfig`. It reads the pool's current manager, epoch, and the current `block_revision`. When
this adapter is current, it calls pool `revoke_all` and requires the returned checked successor
epoch; otherwise it makes no pool write and retains the observed epoch. It then stores
`frozen = true` and emits the final epoch, whether it was current, and the unchanged block revision.
When current, the flag, pool epoch transition, and two events commit or roll back together;
otherwise the flag and freeze event do. There is no unfreeze path. `is_frozen()` returns the durable
flag, and `get_block_revision()` returns the checked monotonic deny-set revision.

`block(addr)` and `unblock(addr)` require immutable-owner authorization and an unfrozen adapter;
repeating the same state is `InvalidConfig`. Every successful block or unblock checked-increments
`block_revision` exactly once; overflow is `OverflowError` and commits neither state nor event.
Blocking atomically records a durable deny and its successor revision. If this adapter is the pool's
current manager, it reads `get_permit` and revokes a returned current-epoch entry in the same
transaction; if it is not current manager, it makes no unauthorized pool write because the later
manager transition invalidates every predecessor permit. Unblocking removes the deny and records
the successor revision. `revoke_all` alone is not a durable user block because a fresh valid
attestation can create a current-epoch permit. Adapter state and pool permits have documented TTL
liveness and restoration obligations.

Replacement uses the old adapter's ordered `blocked`/`unblocked` event history plus live
`is_blocked` views to derive the current deny set. The owner constructs the replacement while the
old manager still gates the pool and stages that set. The old owner then calls `freeze()` while the
old adapter remains manager. Freeze atomically stops later grants and deny-set mutation and advances
the pool epoch once. After the freeze event, operators replay through that ledger, re-read every old
`is_blocked` view, apply any final delta to the still-uninstalled replacement, and prove
`is_frozen() == true` plus exact old/new deny-set agreement. They record the replacement's resulting
`get_block_revision()` and call only its `install(old, expected_revision)` entrypoint for the cutover.

`install` reads the bound pool's current admin and requires that address's authorization before any
local mutation. It requires the pool's current manager to equal `Some(expected_current_manager)` and
that manager's `get_pool()` and `is_frozen()` views to report this pool and `true`; a false or unequal
value is `InvalidConfig`, while a missing interface or downstream contract/host failure propagates
unchanged. Its final pre-write checks reject a frozen candidate with `Frozen` and require the
candidate's current `block_revision` to equal `expected_block_revision`. It then makes no
intervening callback and immediately calls pool
`set_permission_manager(Some(e.current_contract_address()))`, whose normative implementation makes
no external call. The nested setter independently enforces the same current pool-admin
authorization, checked-increments the pool epoch, and returns it.

Under ADR 0002's classic-account-only pool-admin boundary, recording simulation must produce the
pool-admin authorization entry rooted at `candidate.install(old, expected_revision)` with the nested
`pool.set_permission_manager(Some(candidate))` invocation, and enforcing simulation plus weighted
medium-threshold signatures must bind that exact tree before submission (ADR 0002, “Authority
accounts and exact-XDR co-signing”). A contract-account pool admin would need an explicit audited
descendant-authorization decision and is outside this release.
`install` then emits the previous manager, committed candidate revision, and new epoch and returns
that epoch; all reads, writes, authorization, both events, and the manager transition commit or roll
back together. A candidate-owner block followed by unblock still changes the revision twice, so it
cannot restore the expected commitment. Direct pool-admin manager changes remain available for
`None`, a direct manager, or the deny-all incident path, but only this checked adapter-to-adapter
procedure may claim durable-block preservation.

Missing, un-restored archived, non-decodable, disagreeing, or revision-mismatched evidence stops
rotation. The old adapter's frozen flag, configuration, and deny set cannot change; its permitted
risk-reducing revocations do not alter the reconciled block evidence. Any candidate mutation after
evidence collection invalidates the expected revision before the atomic manager write. Adapter code
is immutable under this ADR; every code or owner change uses this address-replacement procedure.

The standby deny-all manager is a separately hash-bound immutable contract with no export or
reachable call path capable of invoking pool permit mutations. It holds no upgrade or owner
authority. Release evidence proves that negative capability on actual optimized Wasm and records
its address before institutional routing; it is an incident-containment manager, not a second
policy engine or a path back to an open pool.

The adapter turns one Predicate attestation into a session bounded by Predicate's own expiration
and `max_validity`. A deployment requiring fresh attestation for every pool action needs a separate
entrypoint/orchestrator decision; this ADR does not claim that sibling Soroban operations can make
pre-attestation atomic.

The exact Predicate statement encoding is a release precondition. The pinned provider commit
already publishes an independent golden digest/signature vector and generator
(`soroban/predicate-registry/src/lib.rs:720-832`, `soroban/scripts/golden-vector.js`). The cumulative
Blend release client must reproduce that vector byte-for-byte under its pinned supported SDK, then
extend it with the actual network, statement UUID and expiration, `msg_sender`, adapter target, zero
value, policy, selector, WHO argument, pool permission epoch, hash, and signature used for release.
The provider command is
`node soroban/scripts/golden-vector.js --check`. A mock proves local rollback and replay behavior
only; it is not provider compatibility evidence.

The configured Predicate registry is an upgradeable external trust, authorization, and resource
boundary. Official use binds the network, registry address, exact optimized code, interface, call
and authorization graph, and measured resource behavior. Code drift or a newly reachable external
effect requires immediate adapter `freeze()`, official submission/routing suspension, and rotation
to a reviewed replacement or deny-all manager after the frozen deny-set handoff. Freeze invalidates
existing grants and stops fresh ones; stand-alone `revoke_all` does not. A passing vector or testnet
call proves compatibility with the tested code, not safety of later upgrades.

### Hook interface and trust boundary

The shared hook interface reuses the pool's existing `Request` and `FlashLoan` XDR shapes:

~~~rust
trait PoolHook {
    fn on_submit(
        e: Env,
        from: Address,
        spender: Address,
        to: Address,
        requests: Vec<Request>,
    );

    fn on_flash_loan(
        e: Env,
        from: Address,
        flash_loan: FlashLoan,
        requests: Vec<Request>,
    );

    fn on_claim(
        e: Env,
        from: Address,
        reserve_token_ids: Vec<u32>,
        to: Address,
    );
}
~~~

When set, `on_submit` runs once for every non-exit `submit` or `submit_with_allowance` except a
batch containing only `FillInterestAuction`/`FillBadDebtAuction`; `on_flash_loan` runs once for every
`flash_loan`; and `on_claim` runs once for every issuance-enabled claim invocation, including
claim-to-self and a call that ultimately accrues zero. Each callback receives the corresponding
public entrypoint arguments unchanged after existing caller authentication and before pool or
emission accounting. Flash calls do not synthesize nonexistent `spender` or `to` arguments. ADR
0002's disabled zero-claim path invokes no callback.

Every hook instance is immutably bound at construction to exactly one reviewed pool address; a new
pool uses a new hook instance even when it reuses the same optimized Wasm. Exact constructor/storage
factoring is implementation-owned, but no setter, upgrade, or second allowed-pool value may change
that binding. An initial hook is constructed against ADR 0002's predicted pool address and its
binding is verified before registry deployment; a later hook is constructed against the existing
pool before attachment. A callback's only pre-auth operation is to load that immutable address; it then calls
`bound_pool.require_auth()` before reading or writing any other hook state, making an external call,
or emitting an event. Soroban automatically authorizes a contract's direct call, so the check proves
the bound pool is the immediate invoker. An account, unrelated contract, or pool to which the hook
was mistakenly attached cannot authorize the bound pool. The pinned SDK states that direct calls by
the current contract are automatically authorized and that deeper calls require an explicit
`authorize_as_current_contract` tree (`soroban-sdk-22.0.7/src/env.rs:417-429`). The existing
pool-to-backstop `gulp_emissions(pool)` path uses the same pattern
(`pool/src/emissions/manager.rs:61-65`; `backstop/src/contract.rs:272-278`). The hook's direct frame
therefore receives the pool authorization needed for its own check. The pool never delegates a
deeper authorization tree for the hook, so descendants do not receive pool authority; any hook
delegation could authorize only the hook itself and is ineligible here. An unauthorized direct call
or call from a deliberately misattached pool fails before other hook effects. A hook rejects the
enclosing action by failing; all nested effects roll back if that action later fails.

The callback cannot read pool state by re-entering the pool, but that does not make it safe. It can
call other contracts, request authorization, consume allowances, or mutate dependencies. Hook
admission therefore requires off-chain review of the exact optimized Wasm and every reachable
dependency. Eligible callback graphs have:

- immutable reviewed code and callback configuration at recorded hashes;
- exactly one immutable allowed-pool address matching the instance's reviewed configuration;
- no reachable self-upgrade, Wasm upload, contract deployment, or asset-contract deployment;
- no dynamic call target, token/allowance movement, participant authorization request, or mutation
  of an oracle, registry, backstop, reserve, or other state the subsequent pool action relies on;
- no caller-sensitive or exhaustible mutable admission state whose consumption by one authenticated
  pool callback can deny another otherwise eligible action;
- bounded request and operational-state work with no recursion or unbounded iteration; and
- permissionless TTL renewal/restoration for every liveness-critical instance, code, and
  persistent entry, with no liveness-critical temporary state.

These are governance and release requirements, not properties enforced by `PoolHook` or the
registry. `is_hook(address) == Some(version)` means only that registry governance currently admits
that address/version. It must never be presented by a contract, indexer, UI, or operator as
on-chain proof that the hook is immutable, audited, correctly configured, safe, or currently
within its resource envelope.

Before first enablement, every attachment, and continued official use after relevant drift, the
release evidence binds the network ledger, hook address, exact transitive code/configuration
hashes, call/auth/allowance review, bounded-state argument, TTL inventory, and measured optimized-
Wasm behavior. Evidence must cover both submit entrypoints, user-liquidation versus admission-free
interest/bad-debt settlement, all flash-auction branches, every issuance-enabled claim alias and
reserve-token-id case including self and eventual-zero accrual, authorized bound-pool invocation,
unauthorized account/contract invocation, the wrong pool calling a deliberately misattached fixture,
rejected descendant pool authorization, the 60-item request and conditional claim limits with
maximum distinct permission parties, hook failure, and exit while the hook is unavailable. Every
successful worst-case total must retain at
least 20% margin under each active transaction resource limit, including signed envelope size.

Missing, stale, incomparable, or failing evidence blocks attachment. Code/configuration drift or an
unbounded resource/liveness observation causes registry disablement for new attachments and pool-
admin detachment for affected pools; membership itself is not a substitute for that response.

### Registry and compatibility

The cumulative ADR 0002/0003 registry types retain every current security, lifecycle,
canonical-target, fee-recipient, and scoped-seal member and add only the trailing hook field/types/
variants below:

~~~rust
struct InterfaceRequirements {
    registry: u32,
    pool: u32,
    backstop: u32,
    hook: u32,
}

struct HookEnable {
    hook: Address,
    implementation: Implementation,
}

struct HookMeta {
    enabled: bool,
    implementation: Implementation,
}

enum SealScope {
    RegistryCode, PoolCode(Address), BackstopCode(Address),
    FuturePoolImplementation, BackstopAdmission, HookAdmission,
}

enum ActionTarget {
    Backstop(Address), Pool(Address), MinConfig, TimelockDelay, SecurityDelay,
    RecoveryConfig, AdminRecovery, PoolImplementation, RegistryCode,
    BackstopAdmission, Onboarding, FeeRecipient, Hook(Address), HookAdmission,
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
    EnableHook(HookEnable),
    DisableHook(Address),
}

fn is_hook(e: Env, hook: Address) -> Option<u32>
fn hook_metadata(e: Env, hook: Address) -> Option<HookMeta>
~~~

Every omitted declaration remains exactly the live predecessor definition. No
`RegistryUpgrade`, `BackstopUpgrade(Address)`, `PoolUpgrade(Address)`, `UpgradeSeal`, or
`SealUpgrades` wire member exists. Backstop enable/disable/upgrade/code-seal continue to collide on
`Backstop(address)`; pool upgrade/resume/retire/code-seal continue to collide on `Pool(address)`.
Hook admission adds no alternative per-address target spelling and cannot weaken ADR 0002's
root-first `RegistryCode` sealing rule. `HookAdmission` is appended to the predecessor `SealScope`
and singleton target order shown above.

`hook_metadata` returns `None` before first enablement and otherwise returns the complete recorded
enabled flag and immutable `Implementation`, including while disabled. `is_hook` returns `None`
before first enablement or while disabled and otherwise returns that metadata's interface version;
it is the narrow attachment check, not the replay view. First enablement permanently binds the
address to its `Implementation`; re-enablement requires that exact value, while a different
implementation uses a new hook address and review. Enabling an already enabled address or disabling
an absent/disabled address rejects atomically. Both actions map to `ActionTarget::Hook(hook)` and
therefore share ADR 0002's one-live-owner target lock.

The proposal-kind symbols are `enable_hook` and `disable_hook`. Successful action events are:

| Transition | Topics | Data |
| --- | --- | --- |
| hook enable or re-enable | `("hook", "enable", hook, proposal_id)` | `implementation: Implementation` |
| hook disable | `("hook", "disable", hook, proposal_id)` | `implementation: Implementation` |

The quoted values denote `Symbol` topics. ADR 0002's generic proposal creation, target ownership,
execution/cancellation/cleanup, action-event-before-terminal ordering, rollback, and raw-XDR replay
rules apply unchanged. Replay reconstructs the full `HookMeta` from these action events, compares it
with `hook_metadata` for every discovered address, and separately verifies that `is_hook` is the
derived enabled interface version or `None`; matching only `is_hook` is insufficient.

`Seal(HookAdmission)` is a security-delay, recovery-vetoable subordinate seal. It may be proposed or
executed only after `RegistryCode` has sealed. At execution time, including for an older
`EnableHook` proposal, the sealed flag permanently rejects first enable and re-enable while
preserving risk-reducing `DisableHook`. It maps to the singleton `HookAdmission` target; per-address
enable/disable actions retain `Hook(hook)`, and every enable execution rechecks the global seal so a
pre-existing proposal cannot bypass it. Like ADR 0002's backstop-admission seal, it neither disables
an enabled hook nor prevents a new pool from attaching one that remains enabled. The approximate
one-year maintenance period may keep this scope open, but the final governance ceremony must
explicitly choose and disclose whether to seal it; no automatic timestamp or inferred root-seal
effect closes hook admission.

`InterfaceRequirements.hook` is appended to ADR 0002's greenfield field order. Registry,
backstop, and hook implementations declare `requires.hook = 0`; only a pool implementation may
declare a positive hook requirement. A hook otherwise declares no protocol-component requirements.
ADR 0002's transition table gains these exact hook edges; every predecessor check still applies:

- `EnableHook(H)`, at proposal and execution, validates the complete immutable `Implementation` and
  requires `CurrentPoolImplementation.requires.hook <= H.interface_version`.
- `SetPoolImplementation(P2)` additionally requires
  `P2.requires.hook <= P1.requires.hook`. The non-increase preserves compatibility with the
  non-enumerable set of enabled hooks; a later cohort needing a higher hook interface requires a
  new registry/address decision.
- `deploy(..., Some(H))` re-reads enabled `hook_metadata(H)` and requires
  `P.requires.hook <= H.interface_version` for the implementation being instantiated.
- `UpgradePool(P2)` re-reads the concrete pool's `get_hook()`. With `Some(H)`, it reads
  `hook_metadata(H)` even when registry governance has disabled `H`, rejects missing metadata,
  requires `P2.requires.hook > 0`, and checks
  `P2.requires.hook <= H.implementation.interface_version` before changing code. With `None`, the
  hook edge is dormant.

A concrete pool independently checks its compiled requirement on `set_hook`; registry deployment
performs the same check for the implementation being instantiated. Registry and backstop upgrade
candidates, and every hook implementation, continue to require `requires.hook = 0`.

A pool implementation declaring hook requirement zero cannot deploy or attach a hook. Upgrading a
concrete pool to a zero-hook implementation therefore requires `get_hook() == None` before code
changes. A hook-only permissioned pool first installs and validates a manager fallback before
detachment if continuous permissioning is a release requirement. ADR 0002 has no composite action,
so those are separate atomic transactions with explicit intermediate state.

Managers are not registry-allow-listed because the pool never invokes them on the user hot path;
they exercise only their authenticated write authority. Hook admission remains the registry's
governance record subject to the non-certification boundary above.

Relative to ADR 0003, registry deployment appends only
`permission_manager: Option<Address>, hook: Option<Address>`:

~~~text
registry.deploy(
  admin, name, salt, oracle, backstop_take_rate,
  max_positions, min_collateral, backstop, interest_fee,
  permission_manager, hook
) -> Address

pool.__constructor(
  admin, name, oracle, bstop_rate, max_positions,
  min_collateral, interest_fee, registry, backstop, blnd_id,
  permission_manager, hook
)
~~~

The registry validates any initial hook and rejects an initial manager or hook equal to the
predicted pool address. The constructor stores the supplied values within ADR 0002's authenticated
registry-deployment boundary. Origination- and flash-loan-fee arguments do not exist.

ADR 0002's `PoolDeployedEvent` appends
`initial_permission_manager: Option<Address>` and `initial_hook: Option<Address>` after
`initial_backstop_generation`. The event therefore remains the canonical discovery record for the
pool's initial permission state as well as its implementation and backstop route.

### Public failure semantics

`PoolError::NotPermitted = 1225` is the only new pool code. A required permit that is absent,
expired, stale, or lacks any combined bit returns `NotPermitted`. The following pool-local failures
use existing codes:

| Failure | Observable result |
| --- | --- |
| Manager-only write while no manager is configured; pool address supplied as manager; same manager/hook transition; invalid mask/time; absent stored entry on `revoke_permit`; oversized issuance-enabled claim-id vector; unknown request type | `PoolError::BadRequest` |
| Present manager or pool admin does not authorize its operation | Soroban host-auth failure |
| Checked permit-epoch transition exhausts `u64` | `PoolError::OverflowError` |
| `revoke_permit` targets a stored expired or earlier-epoch entry | succeeds, deletes that entry, and emits the current epoch; only physical absence is `BadRequest` |
| `set_hook(Some(H))` sees the pool itself, absent/disabled membership, zero local hook requirement, or an interface version below the concrete requirement | `PoolError::BadRequest` |
| Attached hook returns a contract error or traps, or the registry view call fails | exact downstream contract or host failure propagates; pool state and events roll back |

The registry retains ADR 0002's `RegistryError` values and evaluation order. Invalid, unavailable,
zero, executable-mismatched, or re-enable-mismatched hook metadata is `InvalidImplementation`;
hook-version failure, a `SetPoolImplementation` hook-requirement increase, or an `UpgradePool` hook
edge failure is `Incompatible`; enable of an already enabled hook, disable of an absent/disabled
hook, zero-hook upgrade while attached, or another transition-state failure is `InvalidState`.
`EnableHook` after `HookAdmission` seals is `Sealed`; `DisableHook` remains valid. A subordinate seal
before `RegistryCode` is `InvalidState`, and re-sealing is `Sealed`. Authorization, proposal,
target-lock, timing, and rollback behavior remain the exact predecessor contract. Adapter-local
failures use `PermissionAdapterError` above; propagated Predicate, pool, and host failures are never
renumbered. Adapter install on a frozen candidate is `Frozen`; an expected revision, current-manager,
old-pool binding, or old-frozen-state mismatch is `InvalidConfig`; and unauthorized pool-admin use is
a host-auth failure. Block/unblock revision exhaustion returns `OverflowError` before either deny
state or event changes.

### Events and observability

Pool and adapter event shapes are:

| Mutation | Topics | Data |
| --- | --- | --- |
| manager change | `("set_permission_manager", admin)` | `(previous: Option<Address>, replacement: Option<Address>, epoch: u64)` |
| hook change | `("set_hook", admin)` | `(previous: Option<Address>, replacement: Option<Address>)` |
| permit set | `("permit_set", manager, addr)` | `(flags: u32, until: u64, epoch: u64)` |
| permit revoke | `("permit_revoked", manager, addr)` | `epoch: u64` |
| epoch revoke | `("revoke_all", manager)` | `epoch: u64` |
| adapter attestation | `("attested", user)` | `(flags: u32, until: u64, uuid: String)` |
| adapter freeze | `("frozen", owner)` | `(was_current_manager: bool, epoch: u64, block_revision: u64)` |
| adapter block | `("blocked", owner, addr)` | `block_revision: u64` |
| adapter unblock | `("unblocked", owner, addr)` | `block_revision: u64` |
| adapter install | `("installed", pool_admin)` | `(previous_manager: Address, block_revision: u64, epoch: u64)` |
| adapter config | `("config_set", owner)` | `(predicate_registry: Address, policy: String, flags: u32, max_validity: u64, epoch: u64)` |

The quoted names denote `Symbol` topics. Events emit only after their complete transition. A
successful `set_config` produces both the pool epoch event and adapter config event in one
transaction with the same epoch; consumers must not depend on their internal call order. Failed
calls emit no durable event. When a current adapter freezes, the pool's successful `revoke_all`
event precedes the adapter freeze event in the same transaction; a non-current adapter emits only
the freeze event with the unchanged observed epoch. A successful checked install emits the pool
manager-change event before the adapter install event in the same transaction and reports the same
new epoch. The configured policy and
successful-attestation UUID fields in these events are each nonempty and at most 64 bytes.

The pool-deployment event supplies the initial manager and hook. Current state is authoritative via
the public getters; adapter events are mutation records, not a second state store, and this ADR does
not require an event-only reconstruction of constructor state. Freeze and deny-set replay must
agree with `is_frozen`, `get_block_revision`, and every affected `is_blocked` view; install evidence
must bind the checked revision to the manager-change transaction. Registry hook events retain ADR
0002's raw-XDR replay contract because they extend that existing public governance event stream.

Open-pool clients see no change to existing request, flash-loan, claim, reserve, or ordinary pool-
action event wire shapes. Registry and deployment consumers must adopt the cumulative
`InterfaceRequirements.hook` field and the trailing `initial_permission_manager` and `initial_hook`
fields in `PoolDeployedEvent`, including when both deployment values are `None`. Permission-aware
clients must display manager and hook state separately and must label hook membership as governance
admission rather than certification.

### Explicitly out of scope

Backstop-depositor permissioning; enabling BLND claims or selecting their issuance and
emissions economics; transferable positions; per-reserve permits; on-chain KYC policy evaluation;
attestation data in pool requests; a sibling-operation pre-attestation flow; hook implementation
structure; evidence storage format; migration of an already initialized pool. The conditional
issuance-enabled claim contract above binds any later issuance decision that enables the path unless
that Accepted decision expressly supersedes it.

## Alternatives

Options are scored against the ranked drivers: provider-maintained launch eligibility, exit and
revocation safety, and maintainable provider-agnostic surface.

| Option | 1 | 2 | 3 | Assessment |
| --- | --- | --- | --- | --- |
| Pool-local expiring permits plus optional reviewed hook (chosen) | strong | strong | strong | Keeps provider liveness off ordinary permit checks, preserves self-exit, and supports bounded custom logic. Adds permit state, manager authority, hook governance, and release evidence. |
| Predicate attestation on every `submit` | mixed | mixed | weak | Matches Predicate's native model and gives fresh policy decisions, but changes every integrator's ABI and makes an upgradeable unaudited registry a mandatory hot-path dependency. |
| Hook/gate only, no local permits | strong | weak | mixed | Avoids pool permit storage, but provider or hook liveness becomes mandatory for liquidations and every admitted action. |
| Gate only `from` | mixed | weak | strong | Smallest party model, but lets a permitted caller route pool value or rewards to an unpermitted recipient. |
| Gate reserve exits | strong | weak | mixed | Maximizes policy control but can trap repayment and withdrawals after revocation or provider failure. |
| Exempt claim-to-self as an exit | mixed | mixed | strong | Avoids delayed rewards after revocation, but BLND is newly transferred value rather than return of reserve assets and would bypass recipient policy. |
| Stellar asset authorization | weak | weak | strong | Works only when every issuer shares the policy and can freeze the pool or withdrawals; it cannot directly gate internal positions. |
| Optional invalidation flags on authority/config changes | mixed | weak | mixed | Preserves sessions when convenient but permits stale grants to survive manager rotation, disable/re-enable, or policy tightening. Automatic epoch change is simpler and safer. |
| In-place adapter-owner rotation | strong | mixed | weak | Avoids deploying a replacement adapter but creates a second authority handoff and another stale-grant decision. Reusing pool manager rotation gives one invalidation rule. |
| Same-address adapter code upgrade | strong | mixed | weak | Preserves storage, but adds upgrade authority, compatibility, migration, and rollback semantics and can alter grant/block behavior without a manager transition. Immutable adapters plus frozen address replacement keep one invalidation boundary. |
| Permanent hook-admission governance | strong | mixed | mixed | Maximizes later hook onboarding but leaves code-admission authority open after protocol code is otherwise fixed. |

The chosen design does not make hooks safe by naming them in the registry. Its synthesis is local
permits for ordinary provider decisions, an optional hook for bounded self-contained policy, and
explicit off-chain evidence for the trust boundary the chain cannot certify.

Permanent hook-admission governance remains available during the maintenance period but is not the
irreversible end state. The root-first optional seal is preferred because it adds no emergency
enable path, preserves disablement and already-enabled hooks, and lets human governance close this
policy dimension once the hook inventory is mature.

## Consequences

Institutional pools can launch with an installed manager and optional hook. Predicate produces
bounded sessions rather than changing pool entrypoint shapes. Relative to the ADR-0002/0003
baseline, open pools create no per-address permit state or TTL work and add no admission cross-
contract call; every pool retains ADR 0002's universal request bound and guard order, and every
issuance-enabled claim retains ADR 0004's conditional claim-id bound.

Revocation becomes predictable: manager transitions and adapter configuration changes always
invalidate. This deliberately makes rotations and policy changes operationally heavier because
liquidators and other participants must be re-granted, but it removes the one-year maintenance
hazard of remembering whether an optional invalidation flag was safe.

Immutable adapter code makes maintenance a frozen address replacement. Its event-reconstructed deny
set must be staged, frozen, replayed, and verified at the new address, then the candidate's exact
revision must be checked in the same invocation that rotates the manager. Hook
governance likewise has an explicit end state: after the registry-code root is fixed, governance
may irreversibly seal new hook admission while retaining disablement of existing hooks. Keeping
that scope open during the maintenance period is visible continuing authority, not an accidental
consequence of root sealing.

Hook code may be reused, but a hook address may not be shared across pools: each instance binds one
pool immutably and receives its own admission/configuration evidence. This adds deployment and
governance work during the maintenance year, but prevents an arbitrary contract from authenticating
itself as a valid pool and prevents one pool's hook state from affecting another.

Every issuance-enabled `claim` invocation is admitted before emissions accounting, even if the
eventual amount is zero. A revoked holder can still repay and withdraw reserve assets, but cannot
invoke the enabled reward path until the recipient has `RECEIVE` and the hook, when present,
approves. Hook outage can therefore delay rewards and user-liquidation fills, but not self-directed
reserve exit or admission-free interest/bad-debt settlement. ADR 0002's disabled zero claim remains
outside this branch.

Pool permissioning does not permission backstop LPs or interest/bad-debt auction fillers, and
protocol-fee collection is a governance-selected system transfer rather than participant receipt.
Product, UI, and operator language must name all three boundaries; “permissioned pool” alone is not
a claim that every capital provider, settlement recipient, or protocol recipient passed the manager.

Registry hook membership is useful for controlled attachment and interface compatibility, but it
is not an on-chain security attestation. Operational language, UI labels, and incident response must
preserve that distinction.

### Risks and controls

| Risk | Owner and trigger | Prevention and detection | Recovery |
| --- | --- | --- | --- |
| Manager compromise grants entry or liquidation | pool admin; unauthorized permit event | separate manager authority, epoch-bearing events, monitored masks/expiries, pre-reviewed deny-all manager | rotate immediately to deny-all; epoch invalidates old grants; rebuild trusted deny state before a policy manager |
| Rotation or disable/re-enable resurrects stale permits | implementation owner; any manager identity change | mandatory checked epoch increment, same-value rejection, epoch-aware reads and replay | block release; if deployed, freeze new entry and use a reviewed compatible repair |
| Adapter passthrough bypasses its owner or a durable block | adapter owner; unauthorized direct mutation or grant to a blocked address | owner auth on every passthrough, blocked-address rejection, auth/event monitoring | freeze and replay when owner remains trusted; otherwise pool admin rotates to deny-all |
| Adapter config, registry rotation, or policy drift leaves old sessions or unused attestations live | adapter owner; local config change, provider rotation, or same-id policy drift under unchanged reviewed code | every successful config change atomically advances the signed epoch; versioned policy ids, A-to-B replay tests, and policy-drift monitoring | atomically freeze and mass-revoke, suspend submission/routing, then install a reviewed replacement or deny-all manager after frozen deny-set replay |
| Replacement adapter forgets a durable block | adapter owner and pool admin; adapter-address rotation | stage from ordered events/views, freeze/replay the old manager, commit the candidate revision, and check that revision plus old identity/frozen state inside atomic `install` | keep the old adapter frozen and repair or replace a candidate whose revision or views disagree; never use the direct setter to claim block migration |
| Adapter code or owner is changed in place | release owner; candidate assumes an upgrade export or storage-preserving owner rotation | immutable optimized-Wasm hash and exhaustive spec checks; no adapter upgrade or owner-change export | reject the candidate and use frozen address replacement |
| Long-lived liquidator permit archives | operations; TTL approaches expiry | documented TTL policy, monitored renewal, backup liquidators, exact-footprint drills | restore footprint or issue a fresh current-epoch grant; self-exit remains live |
| Issuance-enabled claim or user-liquidation fill is denied by hook failure | pool admin and operations; hook archive/panic/resource exhaustion | immutable reviewed graph, margin and TTL gates, failure/auction-age monitoring; interest/bad-debt fills bypass admission | detach hook; registry disables new attachments; delayed BLND is not reclassified as principal |
| Pool-admin threshold is lost while a manager/hook is unavailable | pool custody owner; failed threshold and liveness incident | ADR 0002 custody drills, independent signers, backup/rotation, admission-free settlement and exact self-defense exit | registry/recovery containment when callable, stop routing, drain through ordinary exits, deploy a new pool; no hidden reset exists |
| Permissioned-pool language is read as covering backstop capital, system-auction recipients, or treasury receipt | product/operator owner; launch disclosure | explicit participant-only scope, admission-exempt settlement/fee collection, and separate backstop-permissioning gate | correct disclosure and stop institutional routing if any required recipient or capital-provider boundary is absent |
| Hook accepts an unauthorized caller or lends pool authority to descendants | registry governance; missing bound-pool auth or pool-delegated descendant tree | load the one immutable reviewed pool and require its auth before other effects; the pool delegates no deeper tree; test direct, misattached-pool, and descendant failures | detach and disable; stop routing until the exact binding/auth graph is repaired and re-reviewed |
| Hook moves value or mutates accounting dependencies | registry governance; hostile or incomplete callback graph | transitive code/call/auth/allowance review and adversarial optimized-Wasm evidence | detach and disable; treat any movement or price/backstop mutation as a security incident |
| Registry membership is marketed as certification | product/indexer owners; UI or API claim | explicit governance-admission labels and evidence linkage; compare current ledger evidence | correct claims, stop routing new users, detach/disable when evidence cannot be restored |
| Predicate returns false, changes encoding/code behavior, or is unavailable | adapter owner and pool admin; provider code/config drift or outage | explicit-true check, exact-code call/auth/resource review, code monitoring, bounded sessions, real-registry testnet gate | freeze the adapter, suspend submission/routing, and rotate after frozen deny-set replay; self-exit remains |
| Oversized policy or UUID makes adapter work input-dependent | adapter owner or participant; configuration or attestation input | nonempty 64-byte adapter bounds before storage, hashing, registry calls, replay writes, or events; exact 64/65-byte and multibyte tests | reject atomically; use a compliant provider identifier or issue a compliant fresh attestation rather than truncating signed data |
| Hook admission remains open after the maintenance period by oversight | registry governance; root-seal ceremony | inventory the live hook set and make an explicit post-root `HookAdmission` seal choice; replay the global flag and older proposals | while open, disclose continuing enable/re-enable authority; when finality is chosen, execute the one-way seal without blocking disablement |
| Admin flips permission state immediately | pool admin; manager/hook transition | admin trust disclosure, exact events, client display of both seams | rotate/restore intended state; an accidental open interval is an incident and cannot be erased |
| Public surface or callback cost exceeds network budget | implementation owner; optimized build/resource run | absolute Wasm gate, before/after attribution, worst-case current-network margin | narrow ABI or callback policy; do not weaken limits or hide staged behavior |

### Pre-mortem

1. **A revoked user cannot repay.** Warning: an exit-to-self batch reads a permit or invokes the
   hook. Prevention: exhaust every exit request combination, aliasing case, missing/expired/
   archived permit, panicking-hook case, and frozen-pool live-auction self-defense deletion.
   Recovery: detach the hook or manager for incident containment and ship only a reviewed
   compatible repair.
2. **A partial mask passes.** Warning: `BORROW` alone authorizes a flash batch that also fills an
   auction. Prevention: combine all rows by address and require the full union; property and bounded
   proofs cover every bit combination. Recovery: freeze affected entry and rotate/invalidate grants.
3. **An issuance-enabled claim bypasses admission.** Warning: any claim invocation reaches emissions
   accounting without recipient `RECEIVE` or `on_claim`, including a call that ultimately accrues
   zero. Prevention: every enabled claim call shares one pre-accounting recipient rule and one
   callback rule. Recovery: disable reward routing until a compatible repair; principal exits stay
   open.
4. **Old grants reappear after a manager cycle.** Warning: a pre-disable permit becomes live after
   re-enabling the same address. Prevention: every identity transition increments the epoch and
   overflow fails closed. Recovery: rotate to a new manager and re-grant from a known epoch.
5. **A configuration change preserves a wider session or revives an unused attestation.** Warning:
   an old grant remains live after `set_config`, or an attestation issued for registry A succeeds
   after rotation to registry B. Prevention: every successful configuration change atomically
   advances the epoch bound into the signed arguments; policy ids are versioned operationally, and
   same-id drift is monitored. Recovery: freeze the current adapter to stop grants and atomically
   invalidate existing sessions, suspend official submission/routing, and install a reviewed
   replacement or deny-all manager after the frozen deny-set handoff.
6. **A blocked user is re-granted through a passthrough or adapter replacement.** Warning: a blocked
   address has a live pool permit, a non-owner causes a direct permit/epoch mutation, or the candidate
   changes after old/new agreement. Prevention: owner auth on every passthrough, one block predicate
   shared by attestation and direct grants, then stage, freeze, replay, record the candidate revision,
   and require atomic `install` to check that revision and the old frozen manager before rotation.
   Recovery: keep the old adapter frozen, repair or replace a mismatched candidate, and repeat the
   checked install; epoch invalidation is not block migration.
7. **A governance-admitted hook is assumed safe on-chain.** Warning: UI says “audited” based only on
   `is_hook`. Prevention: membership is labeled as governance admission and every attachment binds
   external evidence. Recovery: stop official routing, correct the claim, detach/disable if evidence
   is stale or wrong.
8. **Worst-case admitted calls cannot land.** Warning: an inherited 60-request submit or a
   conditional 60-id claim succeeds but the corresponding 61st item reaches admission/accounting,
   or the admitted maximum exceeds 80% of a live limit. Prevention: preserve ADR 0002's universal
   request guard and add the pre-admission claim-id guard, then require optimized-Wasm, signed-
   envelope, state-bound, and TTL evidence. Recovery: reject the release; if the hook causes the
   overage, detach or reject it and narrow the implementation before retrying evidence.
9. **Predicate compatibility or safety was proved only with a mock or stale code.** Warning: the
   published provider vector fails, the registry code changes, or its call/auth/resource graph
   drifts. Prevention: reproduce and extend the byte-exact published vector, exact-code review and
   monitoring, and public-testnet execution. Recovery: freeze the adapter, suspend submission/
   routing, and rotate to a reviewed replacement or deny-all manager after deny-set replay.
10. **Pool-admin loss makes a hook outage permanent.** Warning: threshold recovery fails while
    user-liquidation fills are denied. Prevention: custody rehearsal, signer independence,
    admission-free settlement, and external auction-age monitoring. Recovery: contain when
    callable, stop routing, preserve self-defense exits and settlement, and deploy a new pool; do
    not invent a registry reset during the incident.
11. **“Permissioned pool” is read as “every recipient is permissioned.”** Warning: product or
    compliance evidence omits a backstop depositor, system-auction filler, or protocol fee recipient
    from its exception inventory. Prevention: disclose the participant-only boundary and test the
    exact admission-free transfers. Recovery: correct the claim and stop institutional routing if
    the product requires a boundary this ADR does not provide.
12. **Registry code is sealed while hook admission remains open unintentionally.** Warning: the root
    ceremony has no recorded `HookAdmission` decision, or an old enable proposal succeeds after the
    subordinate seal. Prevention: inventory hooks and proposals, record the explicit open-or-seal
    choice, and test execution-time seal rechecks. Recovery: disclose continuing authority while
    open; if finality is intended, execute and replay the one-way subordinate seal.
13. **An implementation assumes the adapter can change code or owner in place.** Warning: a release
    spec contains an adapter upgrade/owner-change export or a same-address replacement claim.
    Prevention: compare optimized specs and code hash with the immutable adapter ABI. Recovery:
    reject the candidate and use the frozen address-replacement protocol.
14. **A callback accepts an unauthorized caller or a descendant spends pool authority.** Warning:
    a direct account/contract call or a pool other than the immutable binding reaches a hook effect,
    or a nested dependency satisfies `bound_pool.require_auth()`. Prevention: load the immutable pool
    and require its auth before other state/effects, never delegate a descendant pool-auth tree, and
    test all three cases on optimized Wasm.
    Recovery: detach and disable the hook, stop routing, and require a fresh binding/auth-graph review
    before re-enablement.

### Invariants

| Required invariant | Disposition |
| --- | --- |
| Pool reserve, default, collection, liquidation, flash-loan, and factory invariants from ADRs 0002/0003 | Preserved; admission precedes accounting and does not change their arithmetic. |
| A healthy `submit` is not immediately liquidatable | Preserved; admitted actions still run the existing post-action health validation. |
| Self-directed repayment and reserve withdrawal remain live after revocation | Strengthened as an explicit no-permit/no-hook path, subject to ADR 0002's universal 60-request chunk bound and ordinary funding/health/utilization checks. A live user-liquidation auction must be deleted in the same self-defense batch, whose exact shape bypasses only the status>1 deletion gate. |
| Every admitted address holds every required permission bit | New invariant; masks are unioned per address and checked against one current, live permit. |
| Stale permits and unused attestations never resurrect across an invalidation | New invariant; manager changes, mass revocation, and adapter config changes use one monotonic checked epoch, and Predicate signs that epoch in the adapter arguments. |
| A blocked adapter subject cannot regain a permit | New invariant; attestations and owner-authorized direct grants both reject the address until owner-authorized unblocking. |
| Adapter replacement preserves every durable block | New lifecycle invariant; the event/view-derived set is staged, the immutable old adapter freezes, post-freeze old/new views agree, and candidate `install` checks the committed revision and old frozen manager atomically with manager rotation. |
| Hook metadata and admission finality are replayable | New registry invariant; full `HookMeta`, the derived narrow view, and the root-first `HookAdmission` seal agree with ordered events; sealing blocks first enable/re-enable but never disable. |
| BLND never reaches an unadmitted recipient on a manager pool | New conditional invariant; every issuance-enabled claim invocation, including one that eventually accrues zero, requires recipient `RECEIVE` before emissions accounting. ADR 0002's disabled zero claim moves no value and is exempt. |
| Interest/bad-debt settlement and protocol-fee collection remain policy-live | New invariant; system-only settlement batches and `collect_fees` perform no permit read or hook call. Their auction fillers are deliberately unpermissioned recipients; user-liquidation fills remain admitted. |
| Pool permissioning does not imply permissioning of backstop depositors, system-auction fillers, or the fee recipient | Explicit boundary; ADR 0002 backstop entry, liveness-critical settlement, and ADR 0003 collection remain admission-free and product claims disclose them. |
| A hook authenticates its one bound pool and cannot commit side effects when the enclosing pool call fails | Strengthened by stored-pool auth and no descendant pool delegation, then preserved by transaction atomicity. An account, unrelated contract, or different pool fails before other effects. This does not claim a successful malicious hook is harmless. |
| Open pools activate neither admission seam; compatibility changes are limited to the named cumulative boundary delta | Preserved; there is no permit read/write or hook call, while ADR 0002's universal request cap and ADR 0004's conditional issuance-enabled claim-id cap apply independently of the absent seams. Existing entrypoint, request, reserve, and ordinary pool-action event shapes remain unchanged; registry/deployment consumers adopt `InterfaceRequirements.hook` and the two trailing `PoolDeployedEvent` fields. |

### Acceptance criteria

1. The final branch descends the reconciled ADR-0002/0003 public layout. Within that predecessor
   registry layout it adds only the hook requirement, `HookEnable`/`HookMeta`, hook actions,
   events, and views, the root-first `HookAdmission` scope/target, and two trailing deployment/
   constructor arguments stated here. Separately, ADR 0004's complete remaining public delta is the
   `Permit` type and stable bit/bound assignments; pool permission-manager and hook state, setters,
   and getters; `PoolError::NotPermitted`; the Predicate-adapter `Attestation` and
   `PermissionAdapterError` types, frozen and block-revision state, checked-install and remaining
   adapter API; the `PoolHook` callback ABI, immutable one-pool binding, and stored-pool
   authentication rule; the enumerated pool and adapter events; and the
   `PoolDeployedEvent` extension. No other public addition is implied. The result retains every
   predecessor security-delay, canonical-target, lifecycle, fee-recipient, and scoped-seal member.
   No obsolete target spelling and no origination- or flash-fee argument reappears, and the ADR
   remains `Proposed`.
2. Every admission-matrix row is enforced before accounting. Coincident addresses require the
   complete bit union. User-liquidation fills are admitted; system-only interest/bad-debt fills and
   `collect_fees` perform no permit read or hook call. On open, manager-only, hook-only, and combined
   pools, ADR 0002's shared request-length guard permits lengths through 60 and rejects 61 or more
   after inherited auth/self-address checks but before pool/user load, flash liability, balance/
   reserve access, admission, cross-contract calls, mutation, or events. Every issuance-enabled
   claim-id length guard likewise permits lengths through 60 and rejects 61 or more after retained
   `from` authentication but before admission, callback, emission accounting, mutation, transfer,
   or event. Retained per-request, per-id, and downstream checks still determine whole-call success;
   the disabled zero-claim path does not inherit the conditional claim-id bound. In combined mode,
   all required local permits succeed before the hook runs. A missing, expired, stale, or partial
   permit returns `NotPermitted` without hook invocation, hook authorization, hook resource use, or
   hook state access; a valid complete permit followed by a rejecting hook propagates the hook's
   exact downstream failure.
3. Every permitted-size reserve exit to self performs no permit lookup or hook call and cannot fail
   solely because a permit is absent, expired, stale, or archived or because the hook is unavailable.
   It then remains subject to every retained funding, auction, utilization, token, and post-batch
   health check. With a live user-liquidation auction the batch includes deletion and bypasses only
   the status>1 deletion gate; omission or inadequate repair fails atomically. Larger exits remain
   possible in chunks. Third-party payouts and user-liquidation fills remain admitted as specified.
4. Every issuance-enabled claim invocation on a manager pool requires `RECEIVE` for `to`, including
   `to == from`, before emissions accounting can determine the amount; every such invocation on a
   hooked pool invokes `on_claim`, including one that ultimately accrues zero. Denial leaves
   emission state, balances, and events unchanged; principal exit remains available, and longer
   claims remain possible in chunks. ADR 0002's disabled zero claim performs neither admission
   step. The Accepted issuance decision confirms this retained signature/contract or expressly
   supersedes it before production genesis.
5. `None`/`Some`, `Some`/`None`, and manager-address changes each advance the epoch exactly once;
   same-manager calls reject. Disable/re-enable and address reuse cannot make an old permit live.
   The setter makes no external call or callback. `revoke_all` and overflow behavior are exact and
   atomic. The complete new pool failure matrix is exercised, including no-manager writes, host-
   auth, absent versus stored stale/expired revoke, pool-self values, hook membership/compatibility,
   `NotPermitted`, and downstream propagation.
6. Every effective adapter config change advances the pool epoch exactly once and either commits
   the complete new config and provider binding atomically or rolls back everything. There is no
   invalidation parameter or preservation path. A same-config call rejects. Official Predicate
   policy changes use operationally versioned ids; the Predicate registry's policy advertisement is
   not validation. For same-id policy or registry-code drift, owner `freeze()` while the adapter is
   current atomically records `is_frozen() == true` and advances the pool epoch once. Old sessions
   become stale; old and fresh attestations, direct grants, config/policy changes, and deny-set
   mutations reject `Frozen`; self-exit and admission-free settlement remain possible. Official
   submission/routing stops, and risk intake resumes only after frozen deny-set replay and a reviewed
   replacement or deny-all manager transition. If the adapter owner is unavailable or suspect, the
   pool admin's first transition is directly to the pre-reviewed immutable deny-all manager; its
   checked epoch change invalidates all grants, and actual-Wasm evidence proves it has no permit-
   mutation path. The cutover is never presented as registry-proved.
   The signed arguments include the current pool epoch. After registry A rotates to B with the same
   policy and attester, B rejects an unused A-era signature even though its local UUID store is empty;
   a current-epoch B signature can succeed once.
7. The adapter's `Attestation` wire type exactly matches Predicate commit `9206b2d`. Construction
   and `set_config` accept policy byte lengths one and 64 and reject zero and 65 before local or
   registry storage or events; zero validity is `InvalidConfig`, and unrepresentable validity
   arithmetic is `OverflowError`. `attest` accepts UUID byte lengths one and 64 and rejects zero and 65
   before statement hashing, registry validation, replay storage, permit writes, or events; a
   22-scalar-value UTF-8 string encoded as 66 bytes also rejects. An expiration equal to or below the
   current timestamp rejects before Predicate. No path truncates, normalizes, or re-encodes signed
   input. Within those bounds the adapter grants only after an explicit `true`, reproduces and extends
   the published provider vector, respects signed/max validity, never overwrites a different live
   permit shape, and enforces owner-authorized durable blocks. Every local failure maps to the exact
   `PermissionAdapterError`; auth, Predicate typed errors, the invalid-signature crypto trap, host
   failures, and downstream `PoolError` values propagate without renumbering. Live and restored
   persistent UUID markers reject, and no successful path recreates an archived marker as unused.
   Every direct permit passthrough requires the immutable owner, and no path grants a blocked
   address. Adapter code and owner are immutable. Every successful block/unblock checked-increments
   the public revision and emits it; overflow is atomic. Replacement stages the event/view-derived
   live blocks, freezes the old current manager, replays through the freeze ledger, reconciles any
   final delta, and proves `is_frozen` plus exact old/new view agreement. Candidate `install` requires
   current pool-admin auth and, in the same invocation as manager rotation, checks the expected
   candidate revision, expected current manager, old pool binding, and old frozen state. A candidate
   block/unblock after evidence collection changes the revision and aborts installation; mismatch,
   auth, overflow, downstream failure, event order, and full rollback are exact. For ADR 0002's
   classic-account pool admin, recording and enforcing simulation bind one signed authorization tree
   rooted at candidate `install` with the nested pool setter before submission. The statement binds
   attestation UUID/expiration, `msg_sender = user`, adapter target, zero value, stored policy, and
   canonical selector/user/current-epoch arguments. Exact Predicate registry code and its
   call/auth/resource graph are bound and monitored; drift triggers the stated freeze-and-replace
   response rather than stand-alone mass revocation.
8. Institutional deployment installs the adapter as initial manager at the predicted pool address;
   there is no transaction in which the pool is officially live and unintentionally open. Before
   routing, operations also record the hash-bound immutable deny-all manager for emergency rotation.
9. `on_submit`, `on_flash_loan`, and conditional `on_claim` receive their corresponding public
   arguments, including issuance-enabled claim `reserve_token_ids`, and run at the stated pre-
   accounting boundary. Reserve exit-to-self and a system-only interest/bad-debt settlement batch skip
   `on_submit`; ADR 0002's disabled zero claim skips `on_claim`; no issuance-enabled claim invocation
   skips it. Every callback's only pre-auth read loads its one immutable reviewed pool, then calls
   `bound_pool.require_auth()` before other state or effects. The bound pool succeeds; an account,
   unrelated contract, or deliberately misattached pool fails host auth; and the pool delegates no
   descendant authorization. One authenticated callback cannot consume shared admission state needed
   by another otherwise eligible action. Neither a 61st submit request rejected by ADR 0002 nor a
   61st issuance-enabled claim id rejected here reaches a callback, and combined-mode local permit
   failure reaches no callback at all.
10. Hook enablement, attachment, continued-use, and recovery evidence covers immutable transitive
    code/configuration and immutable pool binding, forbidden call/auth/token/accounting edges,
    bounded work/state, all relevant callback branches, TTL, and current-network resource margin.
    Registry membership and pool activity are never asserted to certify those properties on-chain.
11. Registry hook actions, `HookMeta`, interface requirements, concrete attach/deploy/upgrade
    checks, exact public events, and ADR-0002 registry replay agree with optimized-Wasm specs and
    complete live getters; `is_hook` alone is never treated as metadata reconciliation. A root-first
    `HookAdmission` seal rejects first enable/re-enable—including an older proposal at execution—
    while preserving disable and attachment of an already-enabled hook; the final ceremony records
    whether the scope remains open or is sealed. `SetPoolImplementation` enforces the non-increasing
    hook requirement; `UpgradePool` uses complete metadata even for a disabled-but-attached hook and
    requires detachment before a zero-hook implementation; every invalid metadata, compatibility,
    state, seal, auth, and downstream case has the exact failure above. Adapter getters agree with
    its mutation events and no adapter event-only genesis is required. No adapter upgrade or owner-
    change export exists. Disabling blocks new attachment without mutating existing pools.
12. A deployment inventory proves that no non-disposable pool uses an interim ADR-0004 public or
    storage layout. Any discovery blocks release and requires a migration ADR.
13. Every optimized pool, registry, adapter, deny-all manager, hook, backstop, and retained factory
    artifact is at or below the effective network size limit. Worst-case admitted transactions
    retain at least 20% margin under every active transaction resource limit, including ADR 0002's
    universal 60-request boundary, the conditional 60-id issuance-enabled claim, 64-byte policy and
    UUID boundaries, and signed-envelope size.
14. Release evidence states that pool admission does not cover backstop depositors,
    interest/bad-debt auction fillers, or the protocol fee recipient; names the underlying,
    backstop-token, liability-position, and fee transfers those actors may receive; exercises each
    admission-exempt settlement/collection path; and records the permanent-impairment response for
    pool-admin threshold loss. A product requiring permissioned backstop capital, every settlement
    recipient to be admitted, or recoverable pool-admin control cannot ship under this ADR alone.

### Verification and release

The ADR may be accepted before implementation. Acceptance records obligations; it does not claim
that downstream artifacts already exist.

Draft-time evidence on 2026-08-31: `bin/adr --robot lint`, `git diff --check`, and
`make -n -C vendor/blend-contracts-v2 test` pass; the dry run confirms build and optimization
precede the test suite. The ADR-0004 contracts, optimized Wasm, proofs, standalone flows, and
Predicate testnet integration do not exist on this branch. `stellar` and `cargo-kani` are not
available in this shell. Those missing results remain implementation and release gates, not
evidence claimed by this drafting run.

1. **Repository checks now.** Run `bin/adr --robot lint` and `git diff --check`; verify that the
   branch differs from ADR 0003 only by this ADR file and that its status is `Proposed`.
2. **Unit and property evidence after implementation.** In the upstream contract repository run
   `make test`, which builds before testing. Cover every request/party/mask combination, address
   alias, ADR 0002 request and ADR 0004 conditional claim 60/61 boundaries with the exact distinct
   guard orders, combined-mode missing/expired/stale/partial permits returning `NotPermitted`
   without hook invocation or authorization, valid complete permits followed by a rejecting hook,
   exit/claim chunking, all claim aliases, manager
   transitions and observable `Option` semantics, hook attachment/detachment and observable
   `Option` semantics, epochs, TTL, the exact pool/registry/adapter local and propagated failure
   matrix, unauthorized adapter callers, blocked direct grants, adapter sessions/config/blocking/
   irreversible freeze, monotonic block revisions and overflow, staged/frozen/post-freeze block
   replay, candidate mutation after reconciliation, expected-revision/current-manager/old-binding/
   old-frozen mismatches, pool-admin-authorized atomic install and event order, hook metadata
   replay, disabled-but-attached and zero-hook upgrade checks, root-first hook-admission sealing,
   hook order/arguments/failure, stored-pool auth and immutable binding, unauthorized direct calls,
   wrong-pool callbacks after deliberate misattachment, absent descendant authorization, and the
   exact classic-account install authorization tree; user-liquidation admission, frozen-pool
   self-defense auction deletion,
   admission-free interest/bad-debt settlement, deny-all manager rotation, and `collect_fees`,
   exact unpermissioned filler receipts, mixed settlement/participant batches, policy and UUID
   byte lengths zero/one/64/65 plus a multibyte 66-byte case, same-registry UUID
   collision with live and restored persistent markers and failure to recreate an archived marker,
   old-epoch attestation rejection after manager/config/registry changes, invalid-signature host
   traps, registry code/call/auth drift, registry compatibility, and public events.
   Test organization and helper names are implementation-owned.
3. **Bounded model checking after implementation.** Prove the full request-type classification,
   mask-union laws, complete-mask equivalence, coincident-party merging, checked epoch transition,
   and monotonic block-revision transition over protocol-enforced domains. Storage, auth, callback
   order, atomic cross-contract install, and TTL remain concrete integration claims. Harness
   decomposition and names are implementation-owned.
4. **Optimized-Wasm and interface evidence after implementation.** Build the exact ADR-0003
   baseline and combined candidate with the same toolchain. Record artifact hashes, sizes, public
   specs, and attributable deltas, including the deny-all manager. Confirm only the fields,
   functions, errors, actions, arguments, and events named here appear, including the absence of an
   adapter upgrade/owner-change export; every artifact meets the effective size gate. Seed the public
   state, exercise each supported
   protocol-component upgrade path and the root-first `HookAdmission` seal, and prove that complete
   metadata getters, narrow views, hook-aware compatibility, and admission behavior are preserved.
5. **Resource, liveness, and hostile-hook evidence after implementation.** On actual optimized
   Wasm, exercise ADR 0002's universal 60/61 request boundary on open, manager-only, hook-only, and
   combined pools plus ADR 0004's conditional 60/61 claim-id boundary, worst party/flash branches,
   and complete bounded operational state. Compare no-hook and hook totals against current network configuration
   and signed-envelope size. Demonstrate forbidden lifecycle, dynamic-call, token/allowance,
   participant-auth, and accounting-dependency fixtures are ineligible. Archive and restore every
   liveness-critical category with an unprivileged fee payer, proving self-defense exit while the
   hook is unavailable, user-liquidation denial without blocking system-auction settlement or
   `collect_fees`, and full behavior after restoration.
6. **Standalone and public-testnet evidence after implementation.** Exercise direct-manager,
   Predicate-manager, hook-only, combined, and open pools; every manager/config transition and
   rollback; ADR 0002's disabled zero claim and every issuance-enabled claim alias, including an
   eventual-zero result; permitted/unpermitted user liquidations; admission-free interest/bad-debt
   auction fills and `collect_fees`; frozen-pool self-defense deletion; hook detach/disable/upgrade,
   metadata replay,
   and root-first admission seal; authenticated bound-pool callbacks, rejected direct account/
   contract and deliberately misattached-pool calls, rejected descendant pool auth; the recording/
   enforcing-simulated classic-account install tree;
   published Predicate golden-vector reproduction and release-vector extension; same-registry UUID
   collision with marker archive/restore, A-to-B registry rotation with
   an unused old-epoch attestation, current and non-current adapter freeze behavior, freeze/revoke
   event order, owner-unavailable rotation to the actual deny-all manager and proof that it cannot
   mutate permits, same-id drift containment beyond `revoke_all`, invalid-signature-trap rollback, and
   staged/frozen/post-freeze old-to-new adapter block replay, a candidate block/unblock after the
   recorded revision, atomic checked install, mismatch rollback, and paired manager/install events;
   ADR-0002 registry-event replay; predicted-address bootstrap; and a deployment inventory proving
   that no non-disposable pool uses an interim ADR-0004 layout. Public testnet uses Predicate's real
   published registry and provider-issued vector, with the tested registry code and call/auth graph
   recorded. If those do not exist, Predicate-backed institutional release is blocked; a mock is
   not a fallback.
7. **Observability and rollback.** Alert on manager/hook/epoch/config changes, claim denials,
   registry false returns, stale permits, TTL/archive restoration, adapter freeze/error identities,
   deny-all manager hash/address, block-revision/install-event disagreement, post-freeze block-replay
   disagreement, hook-auth failure, incomplete hook metadata, hook-admission seal state, hook
   failures, stalled user-liquidation auctions while interest/bad-debt settlement remains live,
   fee-collection failures, resource margin, Predicate policy/code/
   call/auth drift, evidence drift, and ADR-0002 registry-event disagreement. Release
   records separately disclose that backstop depositors, system-auction fillers, and the fee
   recipient are not admitted and rehearse pool-admin threshold-loss containment, routing cutoff,
   ordinary user exit, and replacement-pool migration.
   Before genesis, rollback is ordinary code reversion. After deployment, a callable pool admin can
   rotate an adapter through the frozen, reconciled, revision-checked install or use the direct
   deny-all incident path, and can detach a hook; registry governance can disable new hook
   attachments and may irreversibly seal future admission after the root seal. A lost pool-admin
   threshold has no reset under this ADR. ABI/storage removal or reinterpretation, weaker
   invalidation, or a claim-admission change requires a new ADR and compatible migration.

Follow-ups: backstop-depositor permissioning and BLND issuance; a separately reviewed single-root
per-transaction Predicate flow if a deployment rejects bounded sessions; migration of any
initialized pool.
