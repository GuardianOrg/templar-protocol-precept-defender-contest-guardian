# 2. Pool registry as deployment, configuration, and optional-backstop routing layer

Date: 2026-08-27

## Status

Proposed

Amended by [8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)

## Context

Templar's Blend fork needs pools that can run **without** a backstop, and a single place that
enforces protocol-level minimums when a pool is created. The vendored contracts
(`vendor/blend-contracts-v2` @ `6cf6dd1`) hard-wire exactly one backstop into every pool and
gate pool creation on nothing but argument bounds. Blend v2 already runs one backstop for
all pools with **per-pool segregated balances** (`backstop/src/storage.rs:78,255-279`;
`draw`/`donate` only touch the named pool, `backstop/src/backstop/fund_management.rs:10-42`),
so "many pools, one backstop" is the existing baseline; what is missing is "a pool with no
backstop" and "a pool routed to a different backstop".

Current coupling, all paths cited:

- **One backstop, baked in at deploy.** `PoolInitMeta { pool_hash, backstop, blnd_id }` is
  set once in the factory constructor (`pool-factory/src/storage.rs:20-26`,
  `pool_factory.rs:54-56`) and passed to every pool's `__constructor`
  (`pool_factory.rs:98-110`, `pool/src/contract.rs:340-364`), which stores it under a
  non-optional instance key (`pool/src/pool/config.rs:38`,
  `pool/src/storage.rs:104,262-283`). No contract has an `upgrade` entrypoint
  (`cd vendor/blend-contracts-v2 && rg -n 'update_current_contract_wasm|fn upgrade' pool
  backstop pool-factory` returns no matches);
  a deployed pool's backstop address is immutable for its lifetime.
- **The pool cannot function without it.** `storage::get_backstop` is read at 11 non-test sites in 7
  files. Those reads feed cross-contract calls, address comparisons, or accounting branches:
  - status machine: `execute_update_pool_status` / `execute_set_pool_status` read
    `backstop.pool_data()` and the BLND⁴·USDC threshold
    (`pool/src/pool/status.rs:12-63,72-115,129-148`); permissionless update rejects status 6 and
    only admin `set_status` can leave setup, but that setter still requires backstop data, so **a
    pool with no funded backstop can never activate**;
  - bad debt entry and user bad debt: `bad_debt.rs:12` (entry), `bad_debt.rs:53-78`
    (`check_and_handle_user_bad_debt`, moves a liquidated user's remaining liabilities onto
    the backstop's `User` position, `:63-74`), `bad_debt.rs:100-123`
    (`check_and_handle_backstop_bad_debt`, defaults them below ~5 % of threshold, `:113-119`);
  - **liquidation hot path**: `submit(FillUserLiquidationAuction)` → `actions.rs:207` →
    `user_liquidation_auction.rs:219` → `check_and_handle_user_bad_debt`; and
    `new_auction(UserLiquidation)` compares the target to the backstop at
    `user_liquidation_auction.rs:19`;
  - interest auction: creation is permissionless but the auction key/`user` must be the current
    backstop; the backstop itself is forbidden from filling, and another filler settles via
    `backstop.donate()` (`pool/src/auctions/backstop_interest_auction.rs:18-25,86-105`);
  - bad-debt auction: creation is likewise permissionless and keyed to the current backstop, which
    is forbidden from filling (`pool/src/auctions/bad_debt_auction.rs:21-29,96-126`);
  - `gulp` (`pool/src/pool/gulp.rs:18-37`, entrypoint `contract.rs:513`, permissionless) writes any
    token surplus into `reserve.data.backstop_credit` regardless of `bstop_rate`;
  - emissions: `gulp_emissions` (`pool/src/emissions/manager.rs:62-64`) and `claim`, which
    pays BLND via `transfer_from(backstop, …)` (`pool/src/emissions/distributor.rs:53-57`).
- **Backstop take has no floor and there is no protocol-fee concept.** `PoolConfig.bstop_rate`
  (`pool/src/storage.rs:29`) is the only interest skim (`pool/src/pool/reserve.rs:75,91-101`);
  the factory checks only `< SCALAR_7` and positional bounds (`pool_factory.rs:70-90`), and
  `update_pool` (`pool/src/pool/config.rs:44-63`) lets the pool admin reset take rate,
  `max_positions`, and `min_collateral` afterwards subject only to `require_valid_pool_config`
  (`:198-213`) — creation-time checks alone are bypassable on the next authorized update.
- **The backstop trusts the factory, not a routing table.** `require_is_from_pool_factory`
  (`backstop/src/backstop/pool.rs:93-99`) is called from `deposit` (`deposit.rs:14`) and
  `donate` (`fund_management.rs:30`) and only when the pool's share balance is 0; `draw`
  (`backstop/src/contract.rs:315-322`) has no check. The backstop `contractimport!`s the
  factory wasm (`backstop/src/dependencies/pool_factory.rs:3`) and the pool imports the
  backstop wasm (`pool/src/dependencies/backstop.rs:3`), fixing build order
  factory → backstop → pool (`Makefile`); a registry that both deploys pools and is called by
  the backstop cannot `contractimport!` either of them.
- **A backstop share epoch can exhaust its LP tokens while the pool still owes reserve-denominated
  credit.** The last-share conversion returns all accounted backstop tokens, including zero
  (`backstop/src/backstop/pool.rs:151-165`), while `withdraw` rejects a zero-token result before
  burning matured shares (`backstop/src/backstop/withdrawal.rs:61-73`). Bad-debt `draw` can reduce
  tokens without shares or q4w (`backstop/src/backstop/fund_management.rs:10-19`). Separately,
  interest accrual books `reserve.data.backstop_credit` without a separate escrow
  (`pool/src/pool/reserve.rs:55-75,91-106`), and its later auction fill calls `backstop.donate`
  (`pool/src/auctions/backstop_interest_auction.rs:42-56,86-118`). A final-share transfer therefore
  either deadlocks on zero LP tokens or couples an LP-token exit to a multi-asset payout, while
  leaving historical credit auctionable into a later epoch gives new depositors the old cohort's
  value. The selected accounting therefore proves aggregate cash backing but segregates old-cohort
  ownership and settlement from the legacy withdrawal wire.
- **The reserve universe is bounded and non-shrinking, and rate rounding leaves real balance
  residuals.** `ResList` is append-only and rejects a 31st reserve
  (`pool/src/storage.rs:458-485`, `pool/src/constants.rs:16`); disabled reserves remain in that
  list. A new reserve starts with `b_supply = 0` (`pool/src/pool/config.rs:135-145`), accrual skips
  zero supply (`pool/src/pool/reserve.rs:50-53`), and `gulp` detects a positive physical-balance
  delta (`pool/src/pool/gulp.rs:24-33`). With positive supply, both rate update and represented
  supply use floor arithmetic (`pool/src/pool/reserve.rs:91-106,163-183`), so one observed unit can
  increase `b_rate` without increasing represented claims and be rediscovered by the next `gulp`.
  The submit builder mutates `b_supply` while processing its request vector. Ordinary supply,
  withdrawal, borrow, and repayment token movements are accumulated and execute only after request
  construction (`pool/src/pool/actions.rs:138-225,288-433`; `pool/src/pool/submit.rs:44-63,198-245`).
  Interest-auction fill is different: dispatch occurs inside that loop and immediately decrements
  current credit and transfers each reserve-token lot before returning
  (`pool/src/pool/actions.rs:248-265`; `pool/src/auctions/auction.rs:140-169`;
  `pool/src/auctions/backstop_interest_auction.rs:108-117`). The public `flash_loan` export also
  creates its liability before the request list but sends the flash asset only afterwards
  (`pool/src/contract.rs:159-164,484-493`; `pool/src/pool/submit.rs:84-106,123-143`). A balance read
  at a supplier boundary must therefore use request-order projected cash: physical cash plus only
  prior scheduled-but-unreflected same-submit flows, including the flash outflow. A synchronous
  auction lot already present in the observation is not replayed, and the current mint/burn
  transfer remains excluded until after allocation.
  The new accounting must therefore preserve the bounded enumeration, carry positive-supply floor
  residual exactly once, and quarantine no-owner cash rather than gifting either value to a later
  supplier/backstop depositor or letting it block epoch opening.
- **Emissions enter through the backstop** from the un-vendored v1 emitter
  (`backstop/src/emissions/manager.rs:96-144`, `emitter/README.md`). The backstop constructor stores
  its `drop_list` and emitter address (`backstop/src/contract.rs:175-196`), and public `drop()` reads
  both before calling the emitter (`:305-310`). Positive backstop `claim` converts BLND to LP and
  then mints/stores per-pool shares with `deposit` events
  (`backstop/src/emissions/claim.rs:19-35,38-88`).

Constraints and assumptions:

- `vendor/` is read-only here; every change is an upstream PR to
  `Templar-Protocol/blend-contracts-v2` (`AGENTS.md`).
- The project budget for each optimized Wasm is 128 KiB, but deployability is also bounded by
  the active network. The effective release limit is
  `min(131072, active_network.contractMaxSizeBytes)` and is read and recorded before upload or
  deployment (`AGENTS.md`). On 2026-08-29 Mainnet (Protocol 27) and Testnet (Protocol 28) each
  reported `contractMaxSizeBytes = 131072`; that observation is release evidence, not a permanent
  constant. The old Protocol-22 `65536` standalone fixture and its 8,289-byte pool-headroom claim
  are not a production limit. No current-toolchain implementation exists yet, so this ADR does not
  preserve obsolete before sizes or invent after sizes. The upstream implementation PR rebuilds a
  clean baseline and candidate, reports both, and fails before review if any artifact exceeds the
  live effective limit.
- Toolchain versions are release evidence, not a protocol interface. The current supported stable
  Mainnet baseline is Protocol 27 with
  [`soroban-sdk 27.0.6`](https://github.com/stellar/rs-soroban-sdk/releases/tag/v27.0.6), its
  [declared Rust MSRV 1.91.0](https://github.com/stellar/rs-soroban-sdk/blob/v27.0.6/Cargo.toml),
  `wasm32v1-none`, `stellar contract build`, and the official Protocol-27 CLI/Core line 27.1.0.
  `wasm32-unknown-unknown` is prohibited. Testnet is already Protocol 28, but the official
  Protocol-28 SDK matrix is still unpublished and the available SDK 28 is a release candidate;
  exercising SDK 27 against Testnet is forward-compatibility evidence, not permission to adopt
  Protocol-28 APIs. Each release re-reads the official
  [software-version matrix](https://developers.stellar.org/docs/networks/software-versions), pins
  exact SDK/Rust/CLI/Core or immutable Quickstart provenance, and records the target network's
  live protocol and settings. The
  [SDK support policy](https://github.com/stellar/rs-soroban-sdk#support) provides critical fixes
  only for the newest two SDK majors, so an out-of-window line is a release blocker rather than a
  comfortable long-term pin. A stable protocol-major SDK change, migration guide, regenerated
  ABI/auth/XDR evidence, and any storage consequences require a separately reviewed change.
- **Assumption: greenfield deployment.** Templar has no deployed pools or backstop; this
  ADR plans a fresh registry + backstop + pools. Migration of Blend-mainnet pools is out of
  scope; if Templar ever must adopt legacy pools, that needs its own ADR because legacy pools
  cannot change their backstop address.
- **Assumption: an operator-maintained stabilization phase.** Templar will maintain the fork
  for roughly one year. During that phase, reproducible upgrades and interface evolution are
  preferable to replacing every live pool for each audited fix. Upgrade authority is temporary,
  explicit, timelocked, and sealable; the target end state is a reviewed release whose upgrade
  surface is irreversibly sealed or handed to the final governance system.
- Product requirements: optional/configurable backstop per pool; a registry that deploys
  pools and enforces minimum fees and valid configurations; borrow-interest fee hooks; BLND
  issuance moved from backstop to registry; permissioned pools. This ADR covers the registry,
  the optional backstop, and one-way attachment. Fees, issuance, permissioning, and
  re-routing/detach are follow-up ADRs that depend on this one.

This file is the base decision in a cumulative greenfield stack. Fee, permissioning, reserve-role,
and issuance/emissions decisions are being reviewed in separate branches, so this branch cannot add
reciprocal links without creating dangling ADR references and failing `adr lint`. Before production
genesis, the merged repository must contain a cumulative manifest naming the actual Accepted ADRs
in dependency order and materializing their final ABI, storage, and verification obligations. That
manifest is the **production-v1 freeze point**: before it exists, an expressly amending pre-genesis
ADR may change an ADR-0002-local type, signature, event, or accounting rule in place because every
such build and client is disposable; after it exists, the append-only/versioned compatibility rules
below apply to the cumulative shapes and no Accepted ADR may be omitted. A later ADR changes this
base only where it expressly says it amends or supersedes a rule here.
The protocol-fee/fungible-supplier-accounting decision and an issuance/emissions disposition are
mandatory production-genesis inputs and must be Accepted before that manifest freezes. The former
must replace the disposable rate-only holder treatment below with a complete fungible claim over
aggregate supplier value; the latter may explicitly retain the disabled profile below or replace it
with fully specified positive economics. Until both exist, only ADR-0002-only local tests,
property/Kani models, actual-Wasm standalone fixtures, and disposable Testnet canaries may use the
local accounting and disabled profiles. Those artifacts are not production-authorized and every
created pool address is recorded as disposable. Templar will not deploy legacy
emitter/drop/backfill/reward state solely to migrate it during the first maintenance year.

Principles:

1. The pool is the sole current-route source of truth; the registry is the only authority that
   may change it. Liquidation never reads routing from another contract.
2. A pool with no backstop is a first-class state whose bad debt and positive supplier value are
   accounted exactly and observably, including rate-rounding carry.
3. A live pool gains a backstop only after that backstop is funded and the risk cutover is
   explicit; a pending backstop never changes pool accounting or liquidation behavior.
4. Registry policy, upgrade power, and admin-loss recovery are typed, delayed, cancellable,
   observable, and bounded. Admin **loss** and admin **compromise** are different incidents:
   recovery can restore a lost admin, while compromise is contained by independent veto,
   liveness-only pause controls, and an exit-length security delay rather than by pretending the
   hostile admin will cancel its own proposal.
5. The one-year maintenance surface is versioned and testable, then can be irreversibly sealed;
   maintenance convenience never weakens funds-flow or authorization checks.

Decision drivers, ranked:

1. Unblock institutional pools that cannot hold BLND:USDC LP (no forced backstop).
2. Preserve liquidation and bad-debt safety across both steady state and `None -> Some` cutover.
3. Controlled fixes and policy evolution during the one-year stabilization phase, without a
   new liquidation-path dependency and with a credible path to seal maintenance authority.

## Decision

Introduce a **`registry`** contract that replaces `pool-factory` as deployer, policy authority,
pool/backstop registry, pending-attachment coordinator, and maintenance-upgrade authority.
Make the pool's current backstop **optional and canonical in the pool**. The registry is the
only contract authorized to write that value, but it does not keep a second current-route copy.

### Normative boundary and equivalent implementations

This ADR decides observable protocol behavior: public exports and wire types, authorization and
delay classes, one-route semantics, state-transition preconditions and terminal states, atomicity,
events/replay, invariant changes, and release evidence. It does **not** choose Rust module paths,
private helper names, a particular storage-key enum, build-target names, scripts, container command
lines, evidence filenames, runner phases, or journal representation. Code sketches and source paths
explain the decision against the pinned baseline; they are not a mandate to copy that layout.

An implementation is conforming when it produces the same public ABI and events, accepts and
rejects the same authorized transitions, preserves the stated atomicity/replay/crash-safety and
economic invariants, and satisfies the verification gates. Internals and evidence tooling may be
smaller or different. Where this ADR says a declaration or event shape is “exact,” that wire shape
is normative; otherwise the implementation may use any equivalent representation.

### Issuance boundary and emissions-disabled profile

ADR 0002 does **not** authorize positive BLND issuance, emission accrual, reward-zone membership,
or a claim-originated backstop-share mint. Those economics and every positive emission `claim` or
`gulp_emissions` transition belong to the cumulative issuance/emissions ADR. Before that decision
is Accepted, a conforming ADR-0002-only development or canary artifact uses this exact profile:

- deploys no legacy emitter, reward-zone allocation, drop/backfill configuration, or positive
  emission-entitlement state. The backstop constructor retains its inherited `emitter` and
  `drop_list` argument slots for flat ABI compatibility, requires `drop_list.is_empty()` with
  `BackstopError::BadRequest`, and stores neither value; `emitter` is reserved and never called;
- keeps the inherited flat emission exports for cumulative interface compatibility. After any
  inherited caller authorization, `set_emissions_config`, `distribute`, `add_reward`,
  `remove_reward`, and `drop` reject the inherited `BadRequest` atomically without an event;
- makes pool `gulp_emissions` return `0` locally without a backstop call; backstop
  `gulp_emissions(pool)` retains `pool.require_auth()` and returns `0`;
- after `from.require_auth()`, pool `claim` preserves the inherited reserve-token-id validation,
  including its accepted empty/duplicate inputs, returns `0`, and rejects an out-of-range id.
  Backstop `claim` requires a non-empty, duplicate-free list in which every pool has a local
  `Active`, `Contained`, or `Retired` relationship, then returns `0`; and
- causes no **emission-originated** BLND/LP-token transfer, backstop-share mint, `deposit` event,
  share-epoch open, or emission-key write. Ordinary pool/backstop balance actions retain their
  independently specified transfer, mint/burn, event, and epoch effects, but their emission-update
  hooks are no-ops. Pool `get_reserve_emissions` and `get_user_emissions` return `None`, and
  backstop `reward_zone()` returns an empty vector.

Each successful zero-result value path publishes exactly one event, after the stated auth and input
checks and before returning; a failed call publishes none. The topics and data types/order are
exact and remain raw-XDR compatibility requirements (`pool/src/events.rs:121-145`,
`backstop/src/events.rs:89-106,134-145`):

| Entrypoint | Preconditions before publication | Exact topics | Exact data | Return |
| --- | --- | --- | --- | --- |
| pool `gulp_emissions()` | No caller auth or input; local disabled path succeeds. | `(Symbol("gulp_emissions"),)` | `0_i128` | `0_i128` |
| pool `claim(from, reserve_token_ids, to)` | `from.require_auth()`; every id names an existing reserve token. | `(Symbol("claim"), from)` | `(reserve_token_ids: Vec<u32>, 0_i128)` | `0_i128` |
| backstop `gulp_emissions(pool)` | `pool.require_auth()`. | `(Symbol("gulp_emissions"), pool)` | `(0_i128, 0_i128)` | `0_i128` |
| backstop `claim(from, pools, min_lp_tokens_out)` | `from.require_auth()`; `pools` is non-empty, duplicate-free, and every relationship is `Active`, `Contained`, or `Retired`; the zero path accepts any `min_lp_tokens_out`. | `(Symbol("claim"), from)` | `0_i128` | `0_i128` |

The no-state rule covers the inherited balance-update hooks, not only the public emission exports.
Backstop `deposit`, `queue_withdrawal`, and `dequeue_withdrawal` currently invoke depositor-emission
updates (`backstop/src/backstop/deposit.rs:13-18`,
`backstop/src/backstop/withdrawal.rs:18-22,37-41`); ordinary `withdraw` does not (`:51-78`). Pool
liability, collateral, and supply add/remove helpers invoke reserve/user emission updates
(`pool/src/pool/user.rs:67-99,128-197,252-271`). In the disabled profile those hooks are absent or
true no-ops: no emission key or emission event is created. That obligation follows every caller,
including all `submit` actions, flash-loan liability creation, liquidation position transfers, and
user/backstop bad-debt movement/default.

This ADR-only profile is implementation and canary evidence, not a production economics decision.
Production genesis requires the Accepted issuance disposition in the cumulative manifest. That
decision may explicitly retain this exact disabled profile; if it retains any positive legacy
claim, it must instead decide direct payout versus share minting and, for every share-mint source,
specify lifecycle eligibility, pre-mint holder checkpointing, zero-share epoch opening, dust
normalization, multi-pool atomicity, failure codes, event order, containment/retirement and resume
behavior, TTL/replay, and unit/Kani/actual-Wasm evidence. ADR 0002 authorizes shares only through
the deposit/activation protocol below; an implementation cannot infer a second opening path from
the retained flat export.

### Registry contract (supersedes `pool-factory`)

Registry membership, pending attachments, proposals, and target ownership are durable keyed state;
the private key layout is implementation-owned. Each successful write extends its affected live
persistent entries to the network's then-current
[`Storage::max_ttl()`](https://docs.rs/soroban-sdk/27.0.6/soroban_sdk/storage/struct.Storage.html#method.max_ttl);
each corresponding getter and `touch_pool` renews a live entry when it falls below half of that
maximum.
`max_ttl` and `extend_ttl` are ledger counts, not seconds. Persistent entries remain restorable
after archival, but contract code cannot read or renew an archived entry: transaction-level
restoration must run first, and archival is an availability incident rather than normal control
flow.

The following declarations are exact public Soroban wire types. Every non-error struct/enum is a
Soroban `#[contracttype]`; ordinary Rust derive lists are implementation-owned because they do not
change XDR. Private state may use a different layout, but the public views and transitions below
must preserve these values:

```rust
struct MinConfig { max_backstop_take: u32, max_positions: u32, min_collateral_floor: i128 }
struct ProposedAdmin { candidate: Address, epoch: u64 }
struct InterfaceRequirements { registry: u32, pool: u32, backstop: u32 } // 0 when unused
struct Implementation {
    wasm_hash: BytesN<32>,
    interface_version: u32, // cumulative flat-export compatibility level
    storage_version: u32,
    requires: InterfaceRequirements,
}
enum PoolLifecycle { Live, Contained, Retired }
enum BackstopRelationship { Funding(u64), Active(u64), Contained(u64), Retired(u64) }
enum SealScope {
    RegistryCode,
    PoolCode(Address),
    BackstopCode(Address),
    FuturePoolImplementation,
    BackstopAdmission,
}
struct PoolMeta { implementation: Implementation }
struct BackstopMeta {
    enabled: bool,
    generation: u64,
    exit_delay: u64,
    implementation: Implementation,
}
struct PendingAttachment { backstop: Address, generation: u64 }
struct Proposal { action: RegistryAction, ready_at: u64, expires_at: u64, epoch: u64 }
struct BackstopEnable { backstop: Address, exit_delay: u64, implementation: Implementation }
struct RecoveryConfig { authority: Address, delay: u64 }
struct ImplementationChange { expected: Implementation, replacement: Implementation }
struct TargetUpgrade {
    target: Address,
    expected: Implementation,
    replacement: Implementation,
}
struct PoolBackstopData {
    tokens: i128,
    shares: i128,
    q4w_pct: i128,
    blnd: i128,
    usdc: i128,
    token_spot_price: i128,
}
struct BackstopEpochState { epoch: u64, open: bool }
enum ActionTarget {
    Backstop(Address), Pool(Address), MinConfig, TimelockDelay, SecurityDelay,
    RecoveryConfig, AdminRecovery, PoolImplementation, RegistryCode,
    BackstopAdmission, Onboarding,
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
}
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
enum RegistryError {
    OverflowError = 12,
    InvalidConfig = 1300,
    NotPool = 1301,
    NotBackstop = 1302,
    InvalidState = 1303,
    InvalidProposal = 1304,
    ProposalNotReady = 1305,
    ProposalExpired = 1306,
    TargetLocked = 1307,
    Incompatible = 1308,
    Sealed = 1309,
    Paused = 1310,
    InvalidImplementation = 1311,
}
```

Multi-field actions use one named payload struct and one tuple payload; no action is represented by
separate discriminator and payload values that can disagree. Exported identifiers must remain
within the active host's contract-spec limit. `RegistryImplementation` /
`registry_implementation` deliberately avoids the overlong
`current_registry_implementation` spelling.

`min_protocol_fee` is **not** in the ADR-0002-local `MinConfig`: the pool has no fee field yet. The
mandatory pre-genesis protocol-fee ADR may append that field and its `deploy` check in place before
the cumulative production-v1 freeze; `MinConfig` + `deploy` are the enforcement hook it plugs into.
That decision must use the typed proposal/action model above before acceptance and the cumulative
manifest must materialize the resulting final shapes; it must not reintroduce `(action, payload)`
pairs that can disagree at runtime.

Interface:

```rust
fn __constructor(e: Env, admin: Address, recovery_authority: Address,
                 registry_implementation: Implementation,
                 current_pool_implementation: Implementation, blnd_id: Address,
                 min_config: MinConfig, timelock_delay: u64, security_delay: u64,
                 recovery_delay: u64) -> ()
fn deploy(e: Env, admin: Address, name: String, salt: BytesN<32>, oracle: Address,
          backstop_take_rate: u32, max_positions: u32, min_collateral: i128,
          backstop: Option<Address>) -> Address
fn admin(e: Env) -> Address
fn proposed_admin(e: Env) -> Option<ProposedAdmin>
fn governance_epoch(e: Env) -> u64
fn next_proposal_id(e: Env) -> u64
fn blnd_id(e: Env) -> Address
fn min_config(e: Env) -> MinConfig
fn timelock_delay(e: Env) -> u64
fn security_delay(e: Env) -> u64
fn registry_implementation(e: Env) -> Implementation
fn is_pool(e: Env, pool: Address) -> bool
fn pool_lifecycle(e: Env, pool: Address) -> PoolLifecycle // membership + pool.lifecycle()
fn is_backstop(e: Env, backstop: Address) -> bool // enabled for new deployment/attachment
fn backstop_metadata(e: Env, backstop: Address) -> Option<BackstopMeta>
fn backstop_of(e: Env, pool: Address) -> Option<Address> // membership + pool.get_backstop()
fn pending_admission(e: Env, pool: Address) -> Option<PendingAttachment> // normalized
fn pending_attachment(e: Env, pool: Address) -> Option<PendingAttachment> // raw replay view
fn current_pool_implementation(e: Env) -> Implementation
fn pool_implementation(e: Env, pool: Address) -> Implementation
fn backstop_implementation(e: Env, backstop: Address) -> Implementation
fn check_config(e: Env, take_rate: u32, max_positions: u32,
                min_collateral: i128, has_backstop: bool) -> () // read-only, panics InvalidConfig
fn begin_attach(e: Env, pool: Address, backstop: Address) -> ()
fn cancel_attach(e: Env, pool: Address) -> ()
fn finalize_attach(e: Env, pool: Address) -> ()
fn pause_onboarding(e: Env, authority: Address) -> ()
fn contain_pool(e: Env, pool: Address, authority: Address) -> ()
fn propose(e: Env, action: RegistryAction) -> u64
fn proposal(e: Env, proposal_id: u64) -> Option<Proposal>
fn target_owner(e: Env, target: ActionTarget) -> Option<u64>
fn execute(e: Env, proposal_id: u64) -> ()
fn cancel(e: Env, proposal_id: u64) -> ()
fn cancel_recovery(e: Env, proposal_id: u64) -> ()
fn veto(e: Env, proposal_id: u64) -> ()
fn cleanup(e: Env, proposal_id: u64) -> ()
fn touch_pool(e: Env, pool: Address) -> bool
fn interface_version(e: Env) -> u32
fn storage_version(e: Env) -> u32
fn onboarding_paused(e: Env) -> bool
fn is_sealed(e: Env, scope: SealScope) -> bool
fn recovery_config(e: Env) -> (Address, u64)
fn propose_admin(e: Env, candidate: Address) -> ()
fn accept_admin(e: Env) -> ()
```

These signatures, the types above, and the numeric `RegistryError` discriminants are the complete
**ADR-0002-local pre-genesis baseline**, not a separately deployable production v1. Expressly
amending mandatory ADRs may change that disposable baseline in place until the cumulative manifest
freezes the complete production-v1 wire ABI. Authorization failures remain Soroban host-auth
failures. Checked arithmetic
uses `OverflowError`; invalid bounds/policy use `InvalidConfig`; unknown membership uses `NotPool`
or `NotBackstop`; failed transition preconditions use `InvalidState`; absent, wrong-epoch, or
wrong-owner proposals use `InvalidProposal`; time-window failures use `ProposalNotReady` or
`ProposalExpired`; a current target owner uses `TargetLocked`; directed-edge failures use
`Incompatible`; closed scopes use `Sealed`; onboarding-gated calls use `Paused`; and invalid,
unavailable, or mismatched code metadata uses `InvalidImplementation`. After the cumulative freeze,
a later version may add new codes but never renumber or reinterpret the frozen set.

The registry's version views return the corresponding fields of
`RegistryImplementation`; they are not separately writable copies.

The constructor is the only initialization path; there is no callable `initialize`. It first
requires both `admin` and the distinct `recovery_authority` to authorize, before writing any
state. It then:

- validates `48 hours <= timelock_delay <= 30 days`,
  `Q4W_LOCK_TIME + 24 hours <= security_delay <= 30 days` (18 days with the pinned 17-day
  withdrawal queue), and `7 days <= recovery_delay <= 30 days`, all with checked timestamp
  arithmetic;
- applies one pure minimum-config predicate requiring
  `max_backstop_take < SCALAR_7`, `2 <= max_positions <= 60`, and
  `min_collateral_floor >= 0`;
- rejects a zero Wasm hash or zero interface/storage version in either implementation, requires
  the registry implementation's versions and requirements to equal the constants compiled into
  this Wasm, and checks both directed `R ↔ CurrentPoolImplementation` edges;
- stores exactly the supplied admin, recovery configuration, implementations, `BlndId`, validated
  policy, and three delays; initializes `GovernanceEpoch = 1`, `NextProposalId = 1`,
  `OnboardingPaused = false`, and every `SealScope = false`; and leaves `ProposedAdmin`, every
  proposal/target, membership entry, and pending attachment absent.

After all writes, the constructor emits the exact registry-genesis event defined below. Any failed
authorization, validation, write, or event publication rolls the constructor back atomically.
Requiring both authorities prevents a third party from front-running initialization with
attacker-selected governance. The host cannot introspect the executing Wasm hash, so the asserted
registry hash is still bound and verified by the deployment manifest; the constructor freezes every
field it can verify on-chain rather than pretending the hash is self-verifiable. `SetMinConfig`
applies the same minimum-config predicate before writing; `SetSecurityDelay` and
`SetRecoveryConfig` apply the same delay/address predicates used at genesis. Thus proof
preconditions are established at every state transition, not supplied only to a harness. The scalar/config
views above renew the registry instance and expose exactly the state needed to verify genesis and
event replay; none is a second writable copy.

`deploy` explicitly calls `admin.require_auth()` as the current factory does
(`pool_factory.rs:61-72`); hashing the admin into the salt is namespacing, not authentication.
It keeps the factory's salt derivation (`:92-96`) and bounds (`:70-90`), and additionally
rejects: `backstop_take_rate > MinConfig.max_backstop_take`;
`max_positions > MinConfig.max_positions`; `min_collateral < MinConfig.min_collateral_floor`;
`backstop = Some(b)` unless `Backstop(b)` is enabled, reports this registry, and all six
registry/pool/backstop edges in the compatibility table pass for the implementation selected by
this transaction;
`backstop = None ∧ backstop_take_rate > 0`. `backstop_of` and
`pending_admission` panic `NotPool` for an unregistered address, so `None` unambiguously means
an uninsured registered pool. `deploy`, `begin_attach`, and `finalize_attach` reject while
onboarding is paused; `pending_admission` returns `None` while paused or when its candidate is
disabled, its pool lifecycle is not `Live`, or its recorded generation is stale, immediately
stopping candidate deposits. The raw `pending_attachment` view does not normalize any of those
conditions and exists only for reconciliation/replay. `cancel_attach` remains available while
contained so funded users can exit and the pending record can be cleared. Backstop
`retire_funding` uses this raw view plus raw backstop generation metadata; it never interprets
`pending_admission == None`, for any normalized reason, as terminal by itself.

The internal pure validator, called `validate_config` in this ADR to distinguish it from the
public export, has conceptual signature `(&MinConfig, u32, u32, i128, bool) ->
Result<(), RegistryError>`. It first enforces the factory's absolute bounds
`take_rate < SCALAR_7`, `2 <= max_positions <= 60`, and `min_collateral >= 0`, then the stored
policy bounds and `None ⇒ take_rate == 0` rule. `deploy` calls it and separately validates the
selected backstop. The exact public export
`check_config(Env, u32, u32, i128, bool) -> ()` calls it and returns unit on success or panics
`RegistryError::InvalidConfig` on any failure. Thus deployed pools call one unambiguous flat ABI
from `update_pool`; no internal `Result`-returning function shares that export name.
With `backstop = Some(b)` the pool constructor writes the initial value from the
registry-supplied argument inside `deploy`. Pool constructor args become
`(admin, name, oracle, bstop_rate, max_positions, min_collateral, registry, backstop:
Option<Address>, blnd_id)`. Before storing any pool state, that constructor requires both
`admin.require_auth()` and `registry.require_auth()`; the registry's direct deployment invocation
satisfies the latter, while an independent deployment cannot make `registry()` impersonate the
official registry. The pool initializes `BackstopEpochState { epoch: 0, open: false }` regardless
of route; neither construction nor zero-balance activation creates a share owner. After successful
construction the registry records
`PoolMeta { implementation: CurrentPoolImplementation }` only after the pool reports the expected
registry/interface/storage versions and canonical local `lifecycle() == Live`; when the initial
value is `Some(b)`, it also calls `b.activate_pool(pool, generation)` before `deploy` returns.
Either all construction, metadata checks,
membership, and activation writes commit, or none do. The current route exists only in the pool;
the local backstop relationship is a capability of that particular backstop, not another
address-valued route.

Every registry public method renews the registry instance/code below half of the current maximum.
`touch_pool(pool)` renews the live `Pool(pool)`, `PendingAttach` when present, current/pending
`Backstop` metadata, and pool instance/code; it then asks each current or pending backstop to renew
its pool-local entries through the backstop's permissionless, local-only `touch_pool`. That method
never calls the registry or pool. A cancelled candidate is no longer discoverable from registry
state, so the keeper also calls its backstop directly while residual balance/q4w exists. If any
required entry is archived, the transaction fails until operations restore the exact footprint and
retry; no getter converts archival into `false` or `None`.

### Funded two-phase attachment

Status 4 is not a safe attachment barrier. It blocks `Supply` and `SupplyCollateral`; `Borrow`
is already blocked for every status above 1. Its request-type-9 restriction is
`DeleteLiquidationAuction`, not auction creation or fill, so liquidation fills remain allowed
(`pool/src/pool/pool.rs:75-81`, `pool/src/pool/actions.rs:18-34`).
`FillUserLiquidationAuction` is request type 6
(`pool/src/pool/actions.rs:23-33`) and its dispatch remains reachable (`:207-225`). A full fill
reads the current backstop when
it handles residual debt (`user_liquidation_auction.rs:206-220`, `bad_debt.rs:53-74`). The old
freeze-then-fund design could therefore move an in-flight liquidation from uninsured semantics
onto an empty backstop.

Attachment is instead `None -> Pending(b) -> Some(b)`:

1. `begin_attach(pool, b)` requires the pool admin's auth (read through `pool.get_admin()`), an
   unpaused registry, a registered `Live` pool, an enabled compatible backstop,
   `pool.get_backstop() == None`, and no pending candidate. It records
   `PendingAttachment { backstop: b, generation }`, tells `b` to enter
   `Funding(generation)`, and emits the exact begin event below atomically. Pool status,
   liquidation, bad debt, and interest remain exactly in `None` mode. Emissions follow only the
   Accepted issuance disposition; ADR-0002-only evidence remains in the disabled profile.
2. Backstop `deposit` first reads its local `BackstopRelationship`. `Active(generation)` accepts
   an already current route without a registry call. `Funding(generation)` requires the
   pause-aware `registry.pending_admission(pool)` to name this backstop and the same current
   generation on every attempt. An absent or archived relationship fails closed: only the
   registry's atomic `begin_attach` transition may create `Funding`.
   `Contained` and `Retired` reject deposits. Thus pause, disable, cancellation, or generation
   change stops pending deposits, while an attached active pool retains deposit liveness if the
   registry is unavailable. `donate`, `draw`, and any future issuance-defined reward-zone
   admission never accept a merely funding route. Depositors fund a segregated candidate balance
   without changing who bears current pool losses.
3. `finalize_attach(pool)` requires the pool admin's auth again and rechecks the onboarding pause,
   `Live` lifecycle, membership, allow-list, registry address, cumulative interface
   compatibility/generation, `None`, pending identity, and current `MinConfig`. It reads the pool's
   scalar policy view declared by the applicable local or cumulative manifest and calls the
   registry's current `check_config(..., true)` before any readiness call or route write. Thus a
   policy tightened while funding is pending fails with the pending record intact. It then calls
   the pool's cold `backstop_ready(b)` view, which applies the same BLND⁴·USDC threshold
   as `status.rs:129-148` and requires `q4w_pct < 50%`. Only then does it call
   `pool.set_backstop(b)`, `backstop.activate_pool(pool, generation)`, deletes `PendingAttach`, and emits the
   exact final event below atomically.
   Any liquidation filled before this transaction uses uninsured semantics; any filled
   afterwards sees a funded, locally activated backstop.
4. `cancel_attach(pool)` requires the pool admin, reads the full pending record, deletes it even
   when capital exists, and emits the exact cancel event below in the same transaction. New
   deposits stop immediately because every `Funding` deposit must revalidate the now-absent
   pending record. Existing depositors retain queue/dequeue/withdraw until their segregated
   balance is empty. A separate permissionless reconciliation call may move local
   `Funding(generation)` to `Retired(generation)` only when the raw `pending_attachment(pool)` is
   absent or names a different `(backstop, generation)`, or when the registry's raw current
   `BackstopMeta.generation` for this address differs. Missing/archived raw state fails closed.
   Onboarding pause, pool containment, health, or any other reversible admission filter is
   insufficient while the raw tuple and generation still match. Disable and upgrade remain
   sufficient because each permanently increments the generation.
   Cancellation itself does not depend on a call into a possibly broken candidate.
   This avoids letting an unsolicited dust depositor or unavailable candidate veto cancellation.
   Disabling a backstop has the same deposit/finalization effect and never traps withdrawals.

The following attachment event wire shapes are the ADR-0002-local pre-genesis baseline. An expressly
amending mandatory ADR may extend the cumulative event schema before the production-v1 freeze; the
manifest then freezes the resulting shapes for the event-derived keeper. Every address is a topic,
the pending generation is the data value, and an event is published only after all writes and
cross-contract calls for that transition have succeeded:

| Transition | Topics, in order | Data |
| --- | --- | --- |
| begin | `(Symbol("attach"), Symbol("begin"), pool, backstop)` | `generation: u64` |
| finalize | `(Symbol("attach"), Symbol("final"), pool, backstop)` | `generation: u64` |
| cancel | `(Symbol("attach"), Symbol("cancel"), pool, backstop)` | `generation: u64` |

Every backstop-local relationship change also has one exact event: topics
`(Symbol("backstop"), Symbol("relationship"), pool, state)`, where `state` is one of
`Symbol("funding")`, `Symbol("active")`, `Symbol("contained")`, or `Symbol("retired")`, and data
`(previous: Option<BackstopRelationship>, replacement: BackstopRelationship)`. It emits after the
local write. Registry-triggered transitions order the backstop event before the registry's later
attach/contain/resume event; a permissionless stale→`Retired` transition has only the local event.
Replay therefore observes local capability history without inventing a second route.

`finalize_attach` and `cancel_attach` retain the record in memory until publication, so deletion
cannot erase the address or generation needed by the event. The lifecycle symbol is the result;
indexers do not infer begin/finalize/cancel from storage disappearance or the pool's separate
`backstop_set` event. In the local baseline that pool-emitted event has topics
`(Symbol("backstop"), Symbol("set"), registry, backstop)` and unit `()` data, published by
`pool.set_backstop` only after its one-shot `None -> Some(backstop)` storage write. During
finalization it therefore precedes the registry's `attach/final` event in transaction event order;
the latter remains the authoritative completed-lifecycle record because it follows activation and
pending-record deletion.

The registry's calls to `pool.set_backstop` and `backstop.activate_pool` are direct invocations,
so their `registry.require_auth()` checks use Soroban's normal invoker-contract authorization
([SDK 27.0.6](https://docs.rs/soroban-sdk/27.0.6/soroban_sdk/struct.Env.html#method.authorize_as_current_contract)).
`authorize_as_current_contract` is unnecessary for these direct calls; any future deeper call
that requires registry auth must pre-authorize its exact argument-bound invocation subtree.
Liability ownership changes at finalization: an auction created while `Pending` but filled after
finalization may use the now-funded backstop. It can never use an empty or merely pending one,
and the pool admin explicitly authorizes that cutover against the pool's then-current state.

**`Some -> None` and `Some(A) → Some(B)` are not supported by this ADR.** The bad-debt
`User` position is address-keyed (`bad_debt.rs:64`, `bad_debt_auction.rs:107`), while
reserve-local `backstop_credit` (`reserve.rs:97-100`) and pool-local emission accrual settle
through whichever address is current (`distributor.rs:53-57`). Re-routing must assign those
outstanding entitlements explicitly; the issuance ADR does so after emissions stop flowing
through the backstop.

### Liveness-only containment and terminal pool lifecycle

`pause_onboarding(authority)` is an immediate circuit breaker. `authority` must be and authorize
either the current registry admin or recovery authority. The first `false -> true` transition emits
the exact pause event; an already-paused call fails without an event. While paused, new pool
deployment, `begin_attach`, pending deposits, and `finalize_attach` fail. Current-route deposits,
repayment, liquidation, loss settlement, q4w, and withdrawal do not read this flag and remain
available. A `ResumeOnboarding` proposal may be created only while this flag is already true; only
that delayed, vetoable action can clear it. This gives a cold
guardian a liveness-reducing response without authority to deploy, attach, configure, or move
funds.

`contain_pool(pool, authority)` similarly moves a registered pool's canonical local lifecycle from
`Live` to `Contained`.
`authority` must be and authorize the current registry admin, recovery authority, or that pool's
current admin. The registry invokes the pool's narrow lifecycle transition; for a `Some(b)` pool
the same transaction also moves `b`'s local relationship from `Active(generation)` to
`Contained(generation)`. A contained relationship rejects deposits and
issuance-defined new reward admission but still permits `donate`, `draw`,
queue/dequeue/withdraw, and the issuance-defined settlement paths, if any, for the named pool. For
a `None` pool, containment changes only the pool-local lifecycle and makes no backstop call. In
either route branch, contained pool code rejects new risk/reserve configuration and any
issuance-defined new-accrual operation but preserves risk-reducing cancellation and every
repayment, liquidation, settlement, and exit path. The registry cannot freeze lending by
impersonating the pool admin: the pool admin separately executes `set_status(4)` when the incident
requires Supply,
SupplyCollateral, Borrow, and auction cancellation to stop. A backstop whose code cannot execute
the containment call causes the entire containment transaction to roll back, leaving the pool
`Live` and the relationship `Active`. Registry metadata alone is not falsely treated as a hot-path
deposit block. If a compatible upgrade is unavailable or the backstop code scope is sealed, the
registry has no degraded authority that can manufacture `Contained` or stop current-route deposits;
the bounded fallback is the pool admin's direct status-4 freeze plus off-chain routing suppression,
drain where the still-callable contracts permit it, and a replacement pool. That failure mode cannot
reach on-chain `Retired` under this ADR.

Returning a contained pool to `Live` is the `ResumePoolInflows(pool)` security action. Execution
requires the pool admin in addition to the registry proposal, and the recovery authority may veto
it. Both route branches require local lifecycle `Contained`, pool status 4, and registry↔pool
compatibility. For `Some(b)`, no raw pending attachment may exist; the backstop must additionally
remain enabled at its current registry generation, pool↔backstop compatible, and healthy before
its relationship returns to `Active`; that backstop event precedes the pool's `Live` event. The
stored `Contained(cohort_generation)` need not equal the registry's current generation: disable and
same-address upgrade deliberately advance registry metadata without detaching an existing cohort.
Execution passes the current generation to `resume_pool`, which requires any stored `Contained`
relationship for the same canonical address and atomically replaces its cohort generation with the
current one. For `None`, the raw
pending record may be absent or may be the exact still-current `PendingAttachment { backstop: b,
generation: g }`. The latter branch revalidates raw enabled metadata at generation `g` and both
directed `registry↔pool`, `registry↔b`, and `pool↔b` edges. It does not invoke or mutate `b`: the
atomic begin transition already established `Funding(g)`. Returning only the pool to `Live` removes
the lifecycle filter, but a separately paused global onboarding state continues to normalize
`pending_admission` to `None`; only later `ResumeOnboarding` makes it visible and permits deposits.
If onboarding was already unpaused, admission becomes visible immediately. Every deposit still
checks the unchanged local capability. A missing, replaced, disabled, or generation-stale raw
candidate fails resume until cancellation or the specified retirement reconciliation resolves it.
This delayed, vetoable, pool-admin-authorized branch prevents immediate containment by a recovery
authority from permanently destroying an uninsured pool's later attachment liveness.

`ResumePoolInflows(pool)` and `RetirePool(pool)` proposals may be created only while the pool's
canonical local lifecycle is already `Contained`; `ResumeOnboarding` may be proposed only while
onboarding is already paused. Proposal creation rechecks those incident states before allocating an
id or target. No separate incident revision is needed: the only transitions out of paused or
`Contained` are actions that own the same canonical `Onboarding` or `Pool(pool)` target, and every
execute/terminal path requires that exact proposal id still own it. A resume or retirement proposal
therefore cannot be prepared while healthy, mature silently, and erase or terminalize a later
guardian response without serving its full `SecurityDelay` from the actual incident state.

`RetirePool(pool)` uses the same delay, veto, and pool-admin co-authorization; it requires the
pool's local lifecycle `Contained`, pool status 4, and no pending attachment, then calls the pool's
registry-and-pool-admin-authenticated `retire()` and records local lifecycle `Retired` atomically.
The pool's lifecycle is the one source of pool-terminal state; the registry stores no duplicate.
`Retired` fixes the pool's
effective status at 4, rejects later status revival and risk/configuration intake, and continues to
allow repayment, liquidation fills, collateral/supply withdrawal, and a compatible exit-preserving
code repair. Retirement cannot be resumed or attached, remains a registry member, and
`backstop_of` continues to expose its unchanged canonical route. Its contained backstop
relationship remains able to settle existing losses and exits.

The terminal no-reroute incident sequence therefore has two honest branches. If the current
backstop still implements the compatible containment/settlement interface, pause new protocol
onboarding if the fault is systemic; contain the affected pool; have its admin set status 4;
preserve repayment, liquidation, donation/draw settlement, and user/backstop exits; mark the
drained pool `Retired`; and deploy a separately identified replacement pool. If the backstop cannot
execute containment and cannot be compatibly upgraded, pause onboarding, have the pool admin set
status 4 directly, stop recommended routing/deposits off chain, drain pool and backstop positions
only where their existing entrypoints remain callable, and deploy the replacement without claiming
that the old pool is `Contained` or `Retired`. Current-route deposits remain possible on chain while
the unchangeable backstop still reports `Active`; a stronger terminalization path must be decided
before sealing if governance is unwilling to accept that limit. A stable-address route rewrite,
forced detach, accelerated code change, emergency drain, or registry-forged lifecycle state is not
smuggled into either branch and needs a separate ADR.

Lifecycle event order is exact. Containment emits pool `Contained`; for `Some`, ADR 0002's disabled
profile emits no reward event, then backstop `Contained`, then registry `pool/contain`. If an
Accepted issuance decision defines terminal reward removal, its exact local terminal event instead
occurs after pool `Contained` and before backstop `Contained`; the relationship and registry events
retain their order. Resume emits a `Some` route's backstop `Active` at the current generation when
applicable, then pool `Live`, then the registry action event. A `None` resume with exact pending
state emits no relationship event because `Funding(g)` is unchanged; pool `Live` is followed by the
registry action event. Retirement emits pool `Retired`, then the registry action event. For proposed
resume/retirement, proposal cleanup and
`proposal/execute` follow as specified below. All
writes/events roll back together, so replay never accepts a durable partial lifecycle.

### Shared interfaces and compatibility

Maintain one shared no-std interface boundary containing traits and boundary types for the exact
**flat Soroban exports** of registry, pool, and backstop. Before the cumulative genesis freeze it
materializes the complete current ADR stack and all generated clients together; afterwards those
production-v1 declarations are frozen. Its crate/module layout is implementation-owned. In
cumulative production v1, each implementation and caller is generated or checked from the same
declaration. This replaces manually duplicated client traits such as
`backstop/src/dependencies/pool.rs:4-22`, which type-check the caller but do not require the callee
to conform. It also avoids `contractimport!` cycles and preserves an acyclic
interface → registry → backstop → pool build dependency.

After the cumulative production-v1 freeze, in-place compatibility is append-only.
`interface_version = N` means the Wasm retains every export and wire shape from lower versions and
may add only uniquely named symbols; existing symbol signatures are never changed or removed. A
later extension trait therefore contains only new
exports, and the contract continues implementing every older trait it serves. A breaking
replacement uses a distinct exported name such as `check_config_v2` while the old symbol remains,
or deploys at a new address when both cannot coexist. Rust module names do not namespace Soroban
exports, and a candidate that cannot remain compatible with older callers is not eligible for a
same-address upgrade.

Compatibility is directional. For implementations `X` and `Y`, where `Y` is a registry, pool, or
backstop, the exact predicate is
`X.requires.<Y kind> <= Y.interface_version`; `0` means `X` makes no call to that contract kind.
`X ↔ Y` below means that both directed inequalities pass. Checks use the implementations
re-read in the executing transaction and fail before any state or code change. Metadata is still a
governance assertion, so every accepted edge is also exercised by the real-Wasm canary.
Let `R`, `P`, and `B` denote the implementations at the concrete registry, pool, and backstop
addresses. The six edges are exactly:
`R.requires.pool <= P.interface_version`,
`P.requires.registry <= R.interface_version`,
`R.requires.backstop <= B.interface_version`,
`B.requires.registry <= R.interface_version`,
`P.requires.backstop <= B.interface_version`, and
`B.requires.pool <= P.interface_version`.

| Transition | Compatibility checks before commit |
| --- | --- |
| `EnableBackstop(B)` | `R ↔ CurrentPoolImplementation`, `R ↔ B`, and `CurrentPoolImplementation ↔ B`. |
| `deploy(None)` | `R ↔ P`; the backstop edge is dormant. |
| `deploy(Some(B))` | `R ↔ P`, `R ↔ B`, and `P ↔ B`. |
| `begin_attach(P, B)` / `finalize_attach(P)` | Both calls check `R ↔ P`, `R ↔ B`, and `P ↔ B`; finalization re-reads the same generation and all six directed edges. |
| `contain_pool(P)` / `ResumePoolInflows(P)` / `RetirePool(P)` | Check `R ↔ P`; for `Some(B)`, also check `R ↔ B` and `P ↔ B` before the relationship or local-retirement call. Risk-reducing containment may proceed against a disabled but interface-compatible current backstop; `Some` resume additionally requires enabled/current metadata and health. A `None` resume makes no backstop call; if raw pending `(B,g)` remains, it rechecks enabled generation `g` and all six directed `R↔P`, `R↔B`, and `P↔B` edges before restoring only the pool to `Live`. A global onboarding pause continues to hide normalized admission until separate `ResumeOnboarding`. |
| `SetPoolImplementation(P2)` | Check `R ↔ P2`; also require `P2.interface_version >= P1.interface_version` and `P2.requires.backstop <= P1.requires.backstop`. Because every enabled `B` passed the enable check, these inequalities preserve both `P2 ↔ B` directions without enumerating the set. A release needing a higher backstop requirement first needs a separately tracked compatibility cohort or a new-address design. |
| `UpgradePool(P2)` | Require `P2.interface_version >= P1.interface_version`, check `R ↔ P2`, and check `P2 ↔ B` plus `R ↔ B` for each current or pending `B`; only a `None` pool with no pending candidate has no backstop edge. |
| `UpgradeBackstop(B2)` | Require `B2.interface_version >= B1.interface_version`, `B2.requires.registry <= B1.requires.registry`, and `B2.requires.pool <= B1.requires.pool`; check `R ↔ B2` and `CurrentPoolImplementation ↔ B2`. Interface monotonicity plus non-increasing requirements preserves both directions for the non-enumerable set of already attached/pending pools. |
| `UpgradeRegistry(R2)` | Require `R2.interface_version >= R1.interface_version`, `R2.requires.pool <= R1.requires.pool`, and `R2.requires.backstop <= R1.requires.backstop`; recheck `R2 ↔ CurrentPoolImplementation`. Those componentwise rules preserve both directions for non-enumerable deployed pools/backstops. |

The release inventory derived from the cumulative production-v1 registry event ABI replays all six
directed inequalities for every recorded live or pending relationship before any upgrade proposal. Its
genesis-to-tip raw-XDR reconstruction must match the on-chain views described below. This is defence
in depth for metadata errors, not a substitute for the on-chain checks above. New outbound calls
remain dormant until the counterparty's reported cumulative version is sufficient; existing
operations continue working with the oldest counterparty version previously supported.

Boundary methods use scalar/stable views (`get_admin`, `get_status`, `get_backstop`, `lifecycle`,
`policy_config`, `backstop_ready`, `registry`, `interface_version`, `storage_version`) rather than
decoding the pool's full `PoolConfig`. In the ADR-0002-local baseline, `policy_config` is the ordered
tuple
`(bstop_rate: u32, max_positions: u32, min_collateral: i128)`; it exposes only fields already
needed by registry policy and is the sole local finalization-policy input. An expressly amending
pre-genesis ADR may extend or replace that disposable view before the manifest freezes the
cumulative shape. Built-Wasm integration tests instantiate the actual artifacts and exercise every cross-
contract method with the frozen production-v1 clients, then exercise every claimed extension with
its new client. An allow-listed backstop must
report `registry() == current registry` and a cumulative
interface version at least as high as the pool release requires; governance metadata is still an
assertion, so the real-Wasm spec/call suite is the release gate. A human review of method names is
not sufficient.

Governance and policy types follow the same freeze boundary. Before production genesis, every
mandatory amending ADR is compiled into one cumulative `RegistryAction`, `ActionTarget`, `Proposal`,
and policy ABI and no partial predecessor client is retained. After the manifest freezes those
production-v1 shapes, they are never extended or reinterpreted in place. A new
action family uses uniquely named versioned types and exports such as `RegistryActionV2` and
`propose_v2`; an existing policy dimension keeps its old check, while a new dimension gets a
separate named check rather than changing production-v1 `MinConfig` or `check_config`. All versions
allocate
from the one monotonic `next_proposal_id`, bind the same `GovernanceEpoch`, and map semantically
identical resources into one canonical target-exclusion namespace. A v2 action therefore cannot
coexist with a production-v1 action merely by spelling the same pool/backstop target differently.

Each extension defines versioned create/terminal events without changing production-v1 event XDR.
Replay merges all versioned streams in ledger/transaction/event order, rejects duplicate ids and
cross-version target ownership, and compares every surviving proposal with its versioned view.
Before `RegistryCode` is sealed, an extension cannot add an unseal path, weaken the root-first seal
rule below, shorten an action's applicable delay below the current class floor, or give old
proposal payloads new meaning. No subordinate scope can yet be proposed. After `RegistryCode` is
sealed, no registry extension can be installed and the fixed registry enforces every later
subordinate seal. Before that root seal, the final governance ADR must inventory every policy
question operators still need; code sealing also freezes the ability to add a new on-chain policy
dimension.

### Timelocked registry governance

`TimelockDelay` is constrained to **48 hours through 30 days**. Changing it may increase or
decrease within those bounds, but the change itself waits under the old delay. `SecurityDelay` is
constrained to **`Q4W_LOCK_TIME + 24 hours` through 30 days**; with the pinned 17-day queue its
minimum is 18 days, so a depositor who reacts to a proposal can complete the ordinary exit before
execution with a one-day operational margin. It may only increase: every enabled backstop records
an immutable `exit_delay`, and admission requires `SecurityDelay >= exit_delay + 24 hours`, so a
decrease could violate a non-enumerable existing cohort. Same-address backstop upgrades preserve
that recorded delay; changing it requires a newly admitted address. `SetSecurityDelay` therefore
requires `replacement > current` both when proposed and executed; equality and decrease fail.
`RecoveryDelay` is constrained
to **7 through 30 days**. All timestamp arithmetic is checked.

`propose(action)` embeds its payload in the typed action, allocates one checked monotonically
increasing id, captures `GovernanceEpoch`, and selects authorization/delay from the action:

- `RecoverAdmin(candidate)` requires the current `RecoveryAuthority`, rejects the current admin,
  recovery authority, or registry address as candidate, and uses `RecoveryDelay`.
- `UpgradeRegistry`, `UpgradeBackstop`, `UpgradePool`, `SetPoolImplementation`,
  `SetSecurityDelay`, `SetRecoveryConfig`, `ResumeOnboarding`, `ResumePoolInflows`, `RetirePool`,
  and every `Seal(scope)` require the current `Admin` and use `SecurityDelay`.
- Every other action requires the current `Admin` and uses `TimelockDelay`.

Before id allocation, `ResumeOnboarding` additionally requires `onboarding_paused == true`, while
`ResumePoolInflows(pool)` and `RetirePool(pool)` require that registered pool's live lifecycle view
to equal `Contained`. Failure is `InvalidState` with no proposal, target, or event. These are
proposal-time as well as execution-time requirements; elapsed proposal time from a healthy state is
never credited toward incident recovery or terminalization.

After authorization and any applicable pool/backstop membership validation, seal-state validation
has one exact order at both proposal and execution. `Seal(RegistryCode)` requires that scope to be
unsealed; otherwise it returns `RegistryError::Sealed`. For every subordinate scope, the registry
first requires `is_sealed(RegistryCode) == true`, returning
`RegistryError::InvalidState` while the root remains mutable, and then requires the named scope to
be unsealed, returning `RegistryError::Sealed` if it is already closed. A rejected proposal writes
no proposal/target and emits no event. A rejected execution leaves its existing proposal/target
unchanged and emits neither the scoped-seal action event nor a terminal event.

`SetMinConfig` and `SetTimelockDelay` deliberately remain ordinary-delay actions. A minimum-config
change is prospective: it cannot rewrite an existing pool, and any later `update_pool` still needs
that pool admin while the recovery authority can immediately pause new deploy/attach intake.
Changing the ordinary delay itself waits under the old ordinary delay and cannot reduce or bypass
the independent monotonic `SecurityDelay` used by code, seal, resume, implementation-selection,
and authority-rotation actions.

The security-delay set is deliberate: code, future implementation selection, authority rotation,
resuming risk intake, terminal lifecycle, and irreversible sealing cannot outrun the existing
backstop exit. `SetRecoveryConfig` is the one security action the recovery authority cannot veto;
otherwise a lost or hostile old guardian could entrench itself forever. It still waits the full
old `SecurityDelay`, requires the replacement authority's acceptance, and invalidates all
old-epoch proposals when executed. Every other security action is vetoable.

Each proposal stores `ready_at = checked(now + selected_delay)` and
`expires_at = checked(ready_at + selected_delay)`. A canonical target mapping prevents two live
actions over the same resource: enable/disable/backstop-upgrade and
`Seal(BackstopCode(b))` map to `Backstop(b)`; pool-upgrade/resume/retire and
`Seal(PoolCode(p))` map to `Pool(p)`; registry upgrade and `Seal(RegistryCode)` map to
`RegistryCode`; future-pool rotation and its seal map to `PoolImplementation`; backstop-admission
seal maps to `BackstopAdmission`; other actions use their named singleton targets. Thus an
enable/disable proposal cannot race an upgrade for the same address, and sealing cannot race the
capability it closes.

For proposal `id` and derived target `T`, ownership means exactly `target_owner(T) == Some(id)`.
`execute`, `cancel`, `cancel_recovery`, `veto`, `cleanup`, stale replacement, and renewal compare
that id before mutating the target. They may remove a stale proposal record but may remove or renew
the target only while it still names that proposal. A new proposal may replace an expired or
epoch-invalid owner only by terminally removing the old proposal before writing the new proposal
and target. No later operation on the old id can alter the new lock. The raw `target_owner` view
does not normalize expiry, epoch, or missing-proposal corruption into `None`; replay must detect an
orphan, missing, or wrong-id relationship. Restoration preserves the original time window and
epoch. Timestamp windows are never converted heuristically into ledger counts; the release proves
the network TTL can carry the maximum window and rehearses restore-then-execute/cleanup.

`execute(id)` is permissionless inside the half-open interval
`ready_at <= now < expires_at` and captured epoch after all action-specific authorization and
state checks pass. `UpgradePool`, `ResumePoolInflows`, and `RetirePool` also require the pool's
current admin; `SetRecoveryConfig` requires the replacement authority; `RecoverAdmin` requires
the recorded candidate. `cancel(id)` lets the current admin cancel any live proposal.
`cancel_recovery(id)` lets the current recovery authority cancel only its own `RecoverAdmin`.
`veto(id)` lets that authority terminally reject any current-epoch, unexpired security proposal
except `SetRecoveryConfig`; it cannot create, modify, execute, or accelerate a proposal.
`cleanup(id)` permissionlessly removes expired or epoch-invalid proposals. All terminal paths use
the id-owned target rule and exact event classification below.

A successful `RecoverAdmin` writes the candidate as `Admin`, clears `ProposedAdmin`, checked-
increments `GovernanceEpoch`, and invalidates every proposal from the lost-admin epoch. A
successful `SetRecoveryConfig` atomically writes both replacement fields after candidate auth,
clears `ProposedAdmin`, and checked-increments the epoch. Cooperative
`propose_admin(candidate)`/`accept_admin()` remains the deliberate non-timelocked exception: the
current admin and exact candidate both authorize, no code/funds/policy changes, acceptance consumes
the epoch-bound record atomically, and the incumbent could already cancel ordinary proposals.
Adding a delay would add denial-of-service power without slowing a hostile incumbent's existing
governance capabilities. Acceptance revalidates role distinctness, increments the epoch, and emits
only after all writes; a second acceptance cannot increment again or invalidate later proposals.

Admin-loss recovery is not mislabeled as compromise recovery. A live hostile admin can cancel a
`RecoverAdmin` proposal, so the recovery authority cannot seize ordinary governance. Its
compromise response is to pause onboarding, contain affected pool inflows, veto high-risk
proposals, and give users the full exit-length warning while governance coordinates a safe
handoff. Conversely, a hostile recovery authority can delay vetoable maintenance but cannot move
funds, execute actions, or block its own delayed replacement forever. Loss of both authorities
remains deliberately unrecoverable.

Every upgrade compares `expected` with current metadata, validates the uploaded replacement and
all directed compatibility/seal rules, and changes code plus metadata atomically. Pool/backstop
upgrades invoke the target's narrow registry-authenticated upgrade export and then record the
replacement; the callee never calls back into the active registry frame. Registry self-upgrade is
fully specified: old code (1) re-reads `RegistryImplementation == expected`, (2) validates the
replacement metadata, uploaded live code entry, compatibility, and unsealed `RegistryCode`, (3)
writes `RegistryImplementation = replacement`, (4) invokes
`e.deployer().update_current_contract_wasm(replacement.wasm_hash)`, (5) emits the registry-upgrade
action event, (6) removes the still-owned proposal/target, and (7) emits `proposal/execute`.
Failure at any point rolls back metadata, scheduled code replacement, deletions, and events. The
executing invocation finishes under old code; the next registry invocation runs the replacement
and reads replacement metadata. General action/terminal ordering below applies unchanged.

Registry and shared-backstop replacements may not raise dependency requirements because their
existing counterparties are not enumerable on-chain. A pool replacement may raise a registry
requirement only when the current registry satisfies it and may raise its backstop requirement only
when its current/pending backstop satisfies it. A `None` pool may record a higher dormant backstop
requirement, but finalization must satisfy it. Rotating the future pool implementation affects only
new deployments. `execute` deletes the proposal and owned target only after the complete action
succeeds.

### Registry event ABI and replay order

Registry membership and proposal state are keyed, not enumerable. Their events are therefore a
wire contract, not descriptive logging. All topic tuples below contain at most four values and use
only `Symbol`, `Address`, or integer topics; typed actions, implementations, hashes, vectors, and
options occur only in data. This section specifies the ADR-0002-local pre-genesis event baseline.
Expressly amending mandatory ADRs may extend its types, variants, and rows in place only before the
cumulative manifest freezes production v1; no partial predecessor decoder is production-supported.
After that freeze, these compatibility rules apply to the complete cumulative event ABI. The local
proposal-kind symbols are:

| `RegistryAction` variant | `kind` symbol |
| --- | --- |
| `EnableBackstop` | `enable_bstop` |
| `DisableBackstop` | `disable_bstop` |
| `SetMinConfig` | `set_min` |
| `SetTimelockDelay` | `set_delay` |
| `SetSecurityDelay` | `set_security` |
| `SetRecoveryConfig` | `set_recovery` |
| `RecoverAdmin` | `recover_admin` |
| `SetPoolImplementation` | `set_pool_impl` |
| `UpgradeRegistry` | `upgrade_registry` |
| `UpgradeBackstop` | `upgrade_bstop` |
| `UpgradePool` | `upgrade_pool` |
| `ResumeOnboarding` | `resume_onboard` |
| `ResumePoolInflows` | `resume_pool` |
| `RetirePool` | `retire_pool` |
| `Seal` | `seal` |

`ProposalEventData` is the exact contract type
`{ target: ActionTarget, action: RegistryAction, epoch: u64, ready_at: u64, expires_at: u64 }`.
Creation publishes topics `(Symbol("proposal"), Symbol("create"), proposal_id, kind)` with
`ProposalEventData` as data. A terminal event uses the same first, third, and fourth topics and one
of `execute`, `cancel`, `veto`, `expire`, or `invalid` as its second topic; its data is the ordered tuple
`(ProposalEventData, validation_epoch: u64)`. The result symbol is selected before any
action-specific write: an in-window, current-epoch `execute` records the pre-action epoch and its
terminal is unconditionally `execute`, even when that action then increments governance epoch;
an authorized explicit cancellation is `cancel`, and a recovery-authority rejection is `veto`.
Only stale removal by `cleanup` or replacement
classifies between `invalid` and `expire`: it snapshots the current epoch, chooses `invalid` first
when the proposal's captured epoch differs (even if time also passed `expires_at`), and otherwise
chooses `expire` when `now >= expires_at`. A proposal has exactly one create event and at most one
terminal event; it has exactly one terminal if and only if it has left storage. If `propose`
replaces an expired or epoch-invalid target owner, it deletes that old proposal and emits its
classified terminal before writing and emitting the new proposal; it never leaves an
event-terminal proposal in storage.

`RegistryGenesisEvent` is the exact contract type
`{ registry_implementation: Implementation, current_pool_implementation: Implementation,
blnd_id: Address, min_config: MinConfig, timelock_delay: u64, security_delay: u64,
recovery_delay: u64, governance_epoch: u64, next_proposal_id: u64,
onboarding_paused: bool, sealed_scopes: Vec<SealScope> }`.
`PoolDeployedEvent` is the exact contract type
`{ caller_salt: BytesN<32>, derived_salt: BytesN<32>, implementation: Implementation,
initial_backstop: Option<Address>, initial_backstop_generation: Option<u64> }`. `derived_salt` is
the actual deployer salt: Keccak-256 of the 32 caller-salt bytes followed by the 56-byte
`admin.to_string()` bytes, preserving `pool_factory.rs:92-96`; it is not an alias for caller input.

Successful local action and lifecycle events are as follows. The cumulative manifest freezes their
final production-v1 forms. `generation` is the post-transition
generation; `expected` and `replacement` are complete `Implementation` values.

| Transition | Topics, in order | Data, exact order |
| --- | --- | --- |
| registry construction | `(Symbol("registry"), Symbol("init"), admin, recovery_authority)` | `RegistryGenesisEvent` |
| pool deploy | `(Symbol("pool"), Symbol("deploy"), pool, admin)` | `PoolDeployedEvent` |
| first backstop enable | `(Symbol("backstop"), Symbol("enable"), backstop, generation)` | `(proposal_id: u64, exit_delay: u64, implementation: Implementation)` |
| backstop re-enable | `(Symbol("backstop"), Symbol("reenable"), backstop, generation)` | `(proposal_id: u64, exit_delay: u64, implementation: Implementation)` |
| backstop disable | `(Symbol("backstop"), Symbol("disable"), backstop, generation)` | `(proposal_id: u64, exit_delay: u64, implementation: Implementation)` |
| minimum-config change | `(Symbol("config"), Symbol("minimum"), proposal_id)` | `(previous: MinConfig, replacement: MinConfig)` |
| timelock change | `(Symbol("config"), Symbol("timelock"), proposal_id)` | `(previous: u64, replacement: u64)` |
| security-delay change | `(Symbol("config"), Symbol("security"), proposal_id)` | `(previous: u64, replacement: u64)` |
| recovery-config change | `(Symbol("config"), Symbol("recovery"), proposal_id, replacement_authority)` | `(previous_authority: Address, previous_delay: u64, replacement_delay: u64, new_governance_epoch: u64)` |
| current pool implementation | `(Symbol("pool_impl"), Symbol("rotate"), proposal_id)` | `(expected: Implementation, replacement: Implementation)` |
| pool upgrade | `(Symbol("pool"), Symbol("upgrade"), pool, proposal_id)` | `(expected: Implementation, replacement: Implementation)` |
| backstop upgrade | `(Symbol("backstop"), Symbol("upgrade"), backstop, generation)` | `(proposal_id: u64, exit_delay: u64, expected: Implementation, replacement: Implementation)` |
| registry upgrade | `(Symbol("registry"), Symbol("upgrade"), proposal_id)` | `(expected: Implementation, replacement: Implementation)` |
| immediate onboarding pause | `(Symbol("registry"), Symbol("pause"), authority)` | `governance_epoch: u64` |
| onboarding resume | `(Symbol("registry"), Symbol("resume"), proposal_id)` | `governance_epoch: u64` |
| immediate pool containment | `(Symbol("pool"), Symbol("contain"), pool, authority)` | `(previous: PoolLifecycle, backstop: Option<Address>, generation: Option<u64>)` |
| pool-inflow resume | `(Symbol("pool"), Symbol("resume"), pool, proposal_id)` | `(backstop: Option<Address>, generation: Option<u64>)` |
| pool retirement | `(Symbol("pool"), Symbol("retire"), pool, proposal_id)` | `backstop: Option<Address>` |
| scoped seal | `(Symbol("registry"), Symbol("seal"), proposal_id)` | `(scope: SealScope, governance_epoch: u64)` |
| delayed admin recovery | `(Symbol("admin"), Symbol("recover"), old_admin, candidate)` | `(proposal_id: u64, recovery_authority: Address, new_governance_epoch: u64)` |

The cooperative admin events remain exact: proposal topics are
`(Symbol("admin"), Symbol("propose"), current_admin, candidate)` with the captured epoch as data;
acceptance topics are `(Symbol("admin"), Symbol("accept"), old_admin, candidate)` with the new
epoch as data. Attachment events retain their separate exact table above.

Ordering is normative. Registry construction emits only after every specified instance value is
written. Pool deploy emits only after construction, compatibility checks, `PoolMeta`, and any
initial activation succeed. Proposal creation emits only after both proposal and target-lock writes.
Successful execution first completes every action-specific write and cross-contract call, emits the
action event above, deletes the proposal and its still-owned target, then emits
`proposal/execute`; all roll back together on failure. `cancel`, `cancel_recovery`, `veto`, and `cleanup`
delete the proposal and only its still-owned target before emitting the appropriate terminal event.
Immediate pause/contain, cooperative admin, and attachment events emit after their complete state
transition. The `RegistryCode` scoped-seal event and its `proposal/execute` terminal must therefore
both precede every subordinate seal's `proposal/create`; each subordinate scoped-seal event then
precedes its own `proposal/execute`. Replay rejects a subordinate create or scoped-seal action before
that completed root-seal pair. No failed call emits a durable event.

A raw-XDR replay test starts at the registry creation ledger, requires exactly one first
`registry/init`, and consumes events in ledger, transaction, and event order. It reconstructs every
genesis scalar, deployed pool and implementation, backstop enabled flag/generation/implementation,
current route, raw pending attachment, normalized pending admission, every backstop-local
relationship, pool lifecycle and onboarding pause, current pool and registry implementation,
per-pool/backstop/registry upgrade, each scoped seal, authority
epoch/handoff, and proposal
create/terminal state. At the captured tip it compares every field and discovered address/id with
`admin`, `proposed_admin`, `governance_epoch`, `next_proposal_id`, `blnd_id`, `min_config`,
`timelock_delay`, `security_delay`, `onboarding_paused`, `registry_implementation`,
`current_pool_implementation`, `is_pool`, `pool_lifecycle`, `pool_implementation`, `is_backstop`,
`backstop_metadata`, `backstop_of`, `pending_attachment`, `pending_admission`,
`proposal`, `target_owner`, `recovery_config`, the registry version views, and
`is_sealed` for every discovered scope; the registry's proxied lifecycle must equal the pool's
local view. For every discovered `(backstop, pool)` pair, replay also calls that backstop's exact
`relationship(pool)` view and compares `Some(state)` or `None` with the local relationship-event
history. A relationship event emitted by initial activation precedes `pool/deploy`; replay holds it
as a provisional pair within that transaction and requires the later deploy event to bind the same
backstop, pool, and generation. Registry-created `Funding`, stale `Funding -> Retired`, and
contain/resume ordering are compared the same way rather than inferred from registry state. For
every `ActionTarget` discovered from creation events, replay compares its
derived expected owner with the raw view. A live stored proposal must have no terminal, match its
view, and own its target; an absent proposal must have exactly one terminal and must not own a
target. At each `ResumeOnboarding`, `ResumePoolInflows`, or `RetirePool` create event, replay requires
the preceding reconstructed state to be paused or the named pool to be `Contained`, respectively;
an event whose proposal clock predates its incident state fails release even if execution later
rechecked the state. An orphaned target, a missing live-proposal lock, or a lock naming another id
fails replay.
Missing, duplicate, out-of-order, or non-decodable events fail the release; indexers never infer a
deleted record from silence.

The recovery authority is a narrow cold path, not a second protocol admin: it can propose/cancel
only `RecoverAdmin`, veto the defined high-risk set, and reduce liveness through onboarding/pool
containment. It cannot configure, upgrade, resume, attach, deploy, retire, seal, move funds, or
execute recovery without the candidate's authorization and full delay. The ordinary admin can
cancel a malicious recovery proposal or rotate the recovery configuration through the
non-vetoable old `SecurityDelay`. Deployment requires distinct operational signer sets and an
actual-Wasm rehearsal of loss, compromise containment, veto/rotation races, both admin/recovery
interleaving orders, authority rotation, and successful recovery. Every valid constructor and
governance transition preserves `Admin != RecoveryAuthority`: proposing C before recovery rotates
to C is invalidated when the
rotation clears the epoch-bound proposal, while rotating recovery to C before an admin proposal
makes that proposal fail its current-role distinctness check. The contract cannot prove signer
independence, so the release manifest records it. Deliberate absence of a usable recovery authority
is a deployment failure, not an undocumented irrecoverability mode.

### Authority accounts and exact-XDR co-signing

For this release the ordinary admin, recovery authority, recovery candidate, and pool admins are
classic Stellar account addresses. Production admin and recovery accounts have disjoint signer sets
and nontrivial medium thresholds; a contract-account wallet needs its own audited `__check_auth`
decision and is not silently accepted by the release tooling. The transaction source/fee payer is
distinct from both registry authorities during genesis, so source-account implicit authorization
cannot masquerade as either required constructor approval.

CLI 27.1.0 can
[parse, simulate, and sign address-valued constructor arguments](https://github.com/stellar/stellar-cli/blob/v27.1.0/cmd/soroban-cli/src/commands/contract/deploy/wasm.rs#L405-L455)
when a non-source address is controlled by one matching key. It still cannot
assemble a classic-account multisig Soroban authorization entry: the CLI chooses one signer whose
public key equals the account address and writes one signature map, while account authorization
must aggregate the live signers whose weights meet the medium threshold
([current signer selection](https://github.com/stellar/stellar-cli/blob/v28.0.0/cmd/soroban-cli/src/signer/mod.rs#L156-L185),
[single-signature write](https://github.com/stellar/stellar-cli/blob/v28.0.0/cmd/soroban-cli/src/signer/mod.rs#L293-L313),
[Soroban auth-entry signing](https://developers.stellar.org/docs/build/guides/transactions/signing-soroban-invocations#method-2-auth-entry-signing)).
Updating the CLI therefore does not by itself make non-source multisig constructor or governance
authorization safe.

Those calls use a versioned, independently tested auth-assembly component; its binary name and
implementation language are not part of this decision. It must recording-simulate one bounded
transaction; bind every returned authorization address, invocation tree, nonce, network id,
signature expiry, source sequence/time bounds, transaction data, and fee; read the authorizer's
live classic-account signer weights and medium threshold; collect distinct valid signatures with
sufficient weight; build the complete auth-entry signature vector; enforcing-simulate that exact
transaction; and only then collect the independent source-envelope signature and submit. Any bound
field change discards the signatures and starts a new attempt. Release tests prove insufficient,
duplicate, unknown, stale, expired, and divergent signatures fail and the minimum valid weighted
set succeeds. A future CLI may replace this component only after an evidence-backed check shows it
implements signer discovery, weighted aggregation, and enforcing validation for non-source
classic-account auth entries.

Production signers receive non-secret signing material out of band and keep seeds offline. Durable
evidence records addresses, live signer weights/thresholds, bound payload and envelope hashes, and
terminal transaction outcomes, never seeds. Test identities survive the multi-day timelock run in
a restricted private location and are fingerprinted on restart; lost or changed private authority
state fails the disposable deployment instead of silently generating replacement keys against old
proposals. The same authorization semantics cover constructor, recovery, veto, recovery rotation,
and pool-admin-coauthorized actions regardless of the surrounding runner.

Pool-admin custody is independently load-bearing. The registry is not a universal pool super-admin
and cannot recover a lost or compromised pool admin. Every production pool therefore has a
documented threshold-custody model, signer independence from registry/recovery, backup and rotation
procedure, and successful loss/compromise rehearsal before deployment. The release disclosure names
which actions become unavailable after threshold loss (`set_status`, configuration, attachment,
containment resume/retirement co-auth, and pool upgrade) and which a hostile key can attempt.
Missing or untested custody blocks that pool's genesis; it is not papered over with an undocumented
registry override.

`DisableBackstop` means **disable new deployments, pending deposits, and finalizations**. Pools
already attached remain attached, may receive current-route deposits, and retain withdrawals.
Registry admin and recovery authority are expected to be distinct multisigs; the contract proves
only that their addresses differ, while the release manifest and signer drill establish operational
independence.
`EnableBackstop` creates new metadata as `enabled = true, generation = 1`, or re-enables an address
only with its unchanged recorded implementation, `exit_delay`, and current generation. First
enable requires the live `exit_delay()` view to equal the payload/record and requires
`SecurityDelay >= exit_delay + 24 hours`; changing code/version uses
`UpgradeBackstop` instead. `Seal(BackstopAdmission)` permanently rejects first enable and
re-enable without preventing risk-reducing disable or settlement for existing routes. Enable,
disable, upgrade, and per-address code seal share the one `Backstop(address)` target. Disable and
upgrade each increment the checked backstop generation, so
pending records from before either transition stay withdraw-only and cannot silently revive after
re-enable; the pool admin must cancel/re-begin against the new generation. The registry can query the
backstop's registry and interface-version views but cannot introspect its storage schema or prove
economic behavior. Initial Wasm/storage metadata is therefore a governance-supplied assertion
bound to the reviewed release manifest and real-Wasm canary evidence. An allow-listed backstop
must expose no code-upgrade authority other than this registry; otherwise its recorded
implementation can drift and it is ineligible.
Same-address backstop upgrade must preserve the recorded `exit_delay`; a changed q4w lag is a new
address/economic migration decision, not an implementation-detail upgrade.
`SetMinConfig` is prospective: it gates new deployments, later pool updates, and attachment
finalization, but does not silently rewrite or halt an existing pool.

### Pool changes (upstream PR to `pool/`)

The exact new or behavior-changed ADR-0002-local pre-genesis pool exports are below. An expressly
amending mandatory ADR may change this disposable list before the cumulative manifest freezes the
production-v1 pool ABI; after that point, changes follow the versioned compatibility rules above.

```rust
fn __constructor(e: Env, admin: Address, name: String, oracle: Address,
                 bstop_rate: u32, max_positions: u32, min_collateral: i128,
                 registry: Address, backstop: Option<Address>, blnd_id: Address) -> ()
fn get_admin(e: Env) -> Address
fn get_backstop(e: Env) -> Option<Address>
fn get_status(e: Env) -> u32
fn lifecycle(e: Env) -> PoolLifecycle
fn policy_config(e: Env) -> (u32, u32, i128)
fn registry(e: Env) -> Address
fn interface_version(e: Env) -> u32
fn storage_version(e: Env) -> u32
fn update_pool(e: Env, backstop_take_rate: u32, max_positions: u32,
               min_collateral: i128) -> ()
fn queue_set_reserve(e: Env, asset: Address, metadata: ReserveConfig) -> ()
fn cancel_set_reserve(e: Env, asset: Address) -> ()
fn set_reserve(e: Env, asset: Address) -> u32
fn submit(e: Env, from: Address, spender: Address, to: Address,
          requests: Vec<Request>) -> Positions
fn submit_with_allowance(e: Env, from: Address, spender: Address, to: Address,
                         requests: Vec<Request>) -> Positions
fn flash_loan(e: Env, from: Address, flash_loan: FlashLoan,
              requests: Vec<Request>) -> Positions
fn update_status(e: Env) -> u32
fn set_status(e: Env, pool_status: u32) -> ()
fn set_backstop(e: Env, backstop: Address) -> ()
fn backstop_ready(e: Env, backstop: Address) -> bool
fn contain(e: Env) -> ()
fn resume(e: Env) -> ()
fn retire(e: Env) -> ()
fn backstop_epoch(e: Env) -> BackstopEpochState
fn open_backstop_epoch(e: Env, share_epoch: u64) -> ()
fn close_backstop_credit(e: Env, share_epoch: u64,
                         close_epoch: bool) -> Vec<(Address, i128)>
fn claim_backstop_credit(e: Env, backstop: Address, asset: Address,
                         max_amount: i128) -> i128
fn deferred_backstop_credit(e: Env, backstop: Address, asset: Address) -> i128
fn supplier_rounding_carry(e: Env, asset: Address) -> i128
fn unowned_surplus(e: Env, asset: Address) -> i128
fn gulp(e: Env, asset: Address) -> i128
fn gulp_emissions(e: Env) -> i128
fn set_emissions_config(e: Env,
                        res_emission_metadata: Vec<ReserveEmissionMetadata>) -> ()
fn claim(e: Env, from: Address, reserve_token_ids: Vec<u32>, to: Address) -> i128
fn new_auction(e: Env, auction_type: u32, user: Address, bid: Vec<Address>,
               lot: Vec<Address>, percent: u32) -> AuctionData
fn bad_debt(e: Env, user: Address) -> ()
fn upgrade(e: Env, wasm_hash: BytesN<32>) -> ()
```

Every other pre-existing pool export and boundary type retains its baseline wire signature. No new
pool error code is needed. Wrong route/lifecycle/epoch transitions, a non-current backstop attempting
epoch open or credit closure, an address without deferred credit attempting realization, and `None`-mode
interest/bad-debt auction creation use existing `PoolError::BadRequest = 1200`. A submit request
vector longer than `MAX_SUBMIT_REQUESTS = 2 * MAX_RESERVES = 60` uses the same `BadRequest` before
pool/user load, flash-liability creation, reserve or token-balance reads, cross-contract calls,
mutation, or events. Negative credit,
a duplicate/incomplete canonical reserve traversal, missing reserve state, checked-accounting
failure, a supplier-carry equation/bound mismatch, negative pre-reconciliation `Q`, a
post-reconciliation `cash + L != V + C + D + U`, `V < L`, a deferred-credit backing/reconciliation
mismatch, or a positive claim transfer shortfall uses `PoolError::InternalError = 1` and clears or
transfers nothing. An initial positive identity difference is unaccounted cash handled by the
canonical allocation below, not an error. A negative `max_amount`
retains `PoolError::NegativeAmountError = 8`.
Invalid pool configuration retains
`PoolError::InvalidPoolConfigArgs = 1201`; an unsupported live/contained `set_status` target or
retirement outside stored status 4 uses `PoolError::StatusNotAllowed = 1204`; and every action or
risk/config mutation rejected solely because lifecycle is `Contained` or `Retired` uses
`PoolError::InvalidPoolStatus = 1206`. This includes the exact blocked calls below; risk-reducing
`cancel_set_reserve`, admin handoff, repayment/liquidation/withdrawal paths, settlement, and the
narrow exit-repair upgrade remain available. Registry policy failures propagate the exact
`RegistryError`, nested pool failures from credit closure or realization propagate unchanged through
backstop `withdraw`, and authorization failures remain host-auth failures. These mappings are
public failure behavior, not permission to collapse distinct registry error codes internally.

Observable route/lifecycle behavior is exhaustive for the changed export set. Lifecycle/status
rejections are evaluated before any backstop or registry call, so an unavailable dependency cannot
mask the applicable local-baseline or cumulative-manifest numeric failure:

| Export group | `Live(None)` | `Contained` | `Retired` |
| --- | --- | --- | --- |
| `submit`, `submit_with_allowance` | Baseline request/status rules; a full liquidation socializes residual user debt to suppliers. | Baseline request/status rules until the pool admin freezes; no registry lookup. | Effective status 4: Supply, SupplyCollateral, Borrow, and DeleteLiquidationAuction reject `InvalidPoolStatus`; repayment, collateral/supply withdrawal, and liquidation fill remain available. |
| `flash_loan` | Baseline Borrow/status rule; no backstop or registry call is added. | Same until admin freeze. | Always `InvalidPoolStatus`. |
| `new_auction`, `bad_debt` | User-liquidation creation remains available; interest/bad-debt creation rejects `BadRequest`; user bad debt defaults directly to suppliers. | Current-route auction/loss settlement remains available. | Current-route auction/loss settlement remains available for drain; no new risk position is created. |
| `update_status`, `set_status` | `update_status` rejects `StatusNotAllowed`; admin `set_status` accepts only `{0,2,4}`. | `update_status` rejects `InvalidPoolStatus`; `set_status(4)` remains available and every other target rejects `StatusNotAllowed`. | Both reject `InvalidPoolStatus`, including an idempotent status-4 write. |
| `update_pool`, `queue_set_reserve`, `set_reserve` | Allowed subject to existing auth/config plus registry policy where stated. | Reject `InvalidPoolStatus`; `cancel_set_reserve` remains available. | Reject `InvalidPoolStatus`; `cancel_set_reserve` remains available. |
| `set_emissions_config`, `gulp_emissions`, emission `claim` | The emissions-disabled profile makes configuration reject and permits only the authenticated/validated zero-result `gulp_emissions`/`claim` paths above. An Accepted issuance decision owns every enabled behavior. | Same disabled behavior; an enabled issuance decision must stop new accrual and define non-confiscating settlement without inventing a share-epoch transition. | Same disabled behavior; an enabled issuance decision owns terminal settlement and may not infer a share mint from ADR 0002. |
| `gulp` | Exact `None` rules below. | Keeps its Borrow/status gate and current-route accounting. | Rejects through effective status 4. |
| `backstop_epoch`, `open_backstop_epoch`, `close_backstop_credit`, `claim_backstop_credit`, `deferred_backstop_credit`, `supplier_rounding_carry`, `unowned_surplus`, `upgrade` | Open/close require the current `Some` backstop, so `None` rejects `BadRequest`; epoch/deferred/carry/surplus views and realization remain available; upgrade follows its dual auth/seal gate. | Epoch/deferred/carry/surplus views, close, realization, and upgrade remain available; opening requires local `Live` and therefore rejects. | Epoch/deferred/carry/surplus views, close and realization remain available for exit preservation; opening rejects permanently. |

- Storage: `Backstop` becomes `Option<Address>` (`storage.rs:262-283`); `Registry: Address`,
  `Lifecycle: PoolLifecycle = Live`, and `StorageVersion` are added at construction
  (`config.rs:38`). `get_backstop() -> Option<Address>` is the canonical route. Scalar
  `get_status()` / `lifecycle()` and
  `registry()` / `interface_version()` / `storage_version()` views avoid exporting config layout
  to registry clients. The ADR-0002-local `policy_config() -> (u32, u32, i128)` view returns only
  `(bstop_rate, max_positions, min_collateral)` in that order; it exists so attachment
  finalization can reapply current registry policy without importing the full `PoolConfig` wire
  layout.
- New entrypoint `set_backstop(e, backstop: Address)`: `registry.require_auth()`, panics
  `BadRequest` unless current value is `None` and lifecycle is `Live`, then stores
  `Some(backstop)` and emits
  `backstop_set(backstop)` so pool-only indexers see it. The registry is
  the only authority that can write it; the pool never makes a cross-contract call to learn
  its current route.
- New narrow lifecycle exports `contain()`, `resume()`, and `retire()`; there is no generic setter.
  All require the stored registry as immediate invoker. `contain` enforces `Live -> Contained`;
  `resume` additionally requires the current pool admin and enforces `Contained -> Live`;
  `retire` additionally requires that admin, stored status 4, and enforces
  `Contained -> Retired`. Each writes before emitting topics
  `(Symbol("pool"), Symbol("lifecycle"), registry, state)` with the previous `PoolLifecycle` as
  data, where `state` is exactly `Symbol("contained")`, `Symbol("live")`, or
  `Symbol("retired")`. Afterwards `Retired` effective action gating is permanently at least status 4: Supply,
  SupplyCollateral, Borrow, flash borrowing, and auction deletion remain blocked; repayment,
  liquidation fills, and user withdrawals remain available. `set_status` cannot write another
  status; `update_pool`, reserve queue/execution, emissions configuration, attachment, and any
  later cumulative risk-intake configuration fail. The narrow registry/pool-admin upgrade export
  remains available while its code scope is unsealed so an exit bug can be repaired. Each local
  lifecycle event precedes the registry's corresponding contain/resume/retire event.
- The pool stores `BackstopEpochState { epoch: 0, open: false }` at genesis. Reserve accrual uses
  the configured backstop take only while the route is `Some` and this state is open; with `None`
  or a closed epoch, effective take is zero and the corresponding interest enters aggregate
  supplier value under the local accounting profile.
  Every supplier-side delta uses the carry rule below; `gulp` uses the same route/epoch branch. New
  `open_backstop_epoch(share_epoch)` requires the canonical
  backstop's auth and local `Live` lifecycle, rejects an already-open state, and requires a positive
  epoch greater than the stored value (exactly `stored + 1` after that backstop has previously
  closed an epoch). Before opening, it walks the complete canonical reserve list, accrues with
  effective take zero, and invokes the canonical positive-cash allocator below while the epoch is
  still closed. Consequently positive unaccounted cash enters aggregate supplier value when supply
  is positive and `UnownedSurplus` when `b_supply == 0`; it cannot become current backstop credit
  merely because the current backstop is opening a new epoch. The allocation and any
  bucket-specific and `cash_alloc` events occur before the epoch write. Only then does it store the
  supplied epoch and emit
  `(Symbol("backstop"), Symbol("credit_epoch"), backstop, Symbol("open"))` with data
  `share_epoch: u64`. A failed later first deposit or activation rolls this nested call back, so no
  empty interval's interest or rebasing surplus can become a first-depositor windfall.
- `SupplierRoundingCarry` is the exact positive-supply residual that the 12-decimal `b_rate` cannot
  yet represent; it is neither physical cash a later `gulp` may rediscover nor an independently
  spendable recipient. Let `R = SCALAR_12 = 10^12`, `S = b_supply > 0`, current rate `r`,
  `P = floor(S * r / R)`, nonnegative carry `K`, and aggregate supplier value `V = P + K`.
  A signed supplier delta `A` changes that value at fixed supply. Positive interest or surplus gives
  `A > 0`; an explicit supplier loss gives `A < 0`. Every such transition computes with checked
  `U256` intermediates:

  ```text
  V' = V + A                         # require 0 <= V' and i128-representable V' and r'
  r' = floor(V' * R / S)
  P' = floor(S * r' / R)
  K' = V' - P'
  ```

  It stores `r'` and `K'` atomically only when the containing reserve transition commits.
  Therefore `P' + K' = P + K + A` exactly and
  `0 <= K' <= ceil((S - 1) / R)`; a nonnegative delta cannot lower `b_rate`, while only the
  explicit supplier-loss paths may use a negative delta. Overflow, a negative target, a
  missing/archived carry, or any equation/bound mismatch fails `PoolError::InternalError = 1`
  without a rate, carry, transfer, or event change.

  This ADR-0002-local profile deliberately stops short of a complete fungible holder claim.
  Per-holder b-token conversion, withdrawal conversion, and position-health calculations continue
  to use `r` and their baseline floor/ceil rules, while aggregate accrual, physical reconciliation,
  utilization, supply-cap enforcement, and any aggregate `total_supply` semantic use `V = P + K`.
  Thus a final rate-quoted holder can leave part of supplier-owned `K` behind for the zero-supply
  reclassification below. That value cannot be captured by a later cohort, but calling the local
  profile complete holder ownership would be false. It is permitted only in disposable tests and
  canaries. The mandatory pre-genesis protocol-fee/fungible-supplier-accounting ADR must define
  every holder's pro-rata claim over `V`, mint/burn rounding, and final-holder realization before
  any non-disposable pool or client exists; the cumulative manifest rejects this local profile.
  Current backstop credit, deferred backstop credit, and `UnownedSurplus` remain separate exclusions
  and never enter aggregate supplier value. Tests exercise nonzero `K` exactly at utilization and
  supply-cap boundaries so no aggregate consumer silently falls back to `P`, and separately expose
  rather than conceal the disposable last-holder residual.

  Membership changes target value rather than reinterpreting an old rate under a new supply. Every
  `Supply`, `SupplyCollateral`, `Withdraw`, and `WithdrawCollateral` request admitted by its
  existing action/status gates invokes the canonical positive-cash allocator below after the reserve
  load/accrual and before quoting, minting, or burning b-tokens. Let `S0`, `r0`, and `K0` be that
  post-allocation, pre-membership state;
  `tokens_in` and `tokens_out` are the exact nonnegative underlying amounts entering and leaving
  aggregate supplier accounting in the current request—not a claim that the local rate-only quote
  realizes every unit for each holder—and `S1` is supply after the mint or burn:

  ```text
  V0 = floor(S0 * r0 / R) + K0
  V1 = V0 + tokens_in - tokens_out   # require 0 <= V1

  if S1 > 0:
      r1 = floor(V1 * R / S1)
      P1 = floor(S1 * r1 / R)
      K1 = V1 - P1
  else:
      K1 = 0
      UnownedSurplus += V1
  ```

  The positive-supply branch stores `r1` and `K1` atomically and satisfies `P1 + K1 = V1` with
  the same carry bound. The zero-supply branch reclassifies the entire residual `V1`, not merely
  the old `K0`; under this disposable local profile a final burn can withdraw less than all aggregate
  value and leave whole underlying units, so no prior-carry-only bound is claimed. The residual is
  disclosed and cannot be captured by a later supplier, but it is also not misrepresented as value
  realized by the departing holder. Exact regressions include: `S0 = 2`, `r0 = 1.5R`, burn
  one b-token for `tokens_out = 1` gives `S1 = 1`, `V1 = 2`, `r1 = 2R`, `K1 = 0`; `S0 = 1`,
  `r0 = 2R`, deposit `tokens_in = 3` and mint one b-token gives `S1 = 2`, `V1 = 5`,
  `r1 = 2.5R`, `K1 = 0`; and a partial-value final burn moves all positive `V1` to
  `UnownedSurplus`.

  `supplier_rounding_carry(asset)` returns the exact current `K`. A committed transition emits
  topics `(Symbol("pool"), Symbol("supplier_carry"), asset)` and data
  `(supplier_delta: i128, previous: i128, current: i128, b_rate: i128)` only when `K` changes or a
  final reclassification requires the zero-current marker. `supplier_delta` is `A` for a
  fixed-supply transition and `V1 - V0` for a membership transition. A rate-only transition with
  unchanged `K`, an idempotent reconciliation, and a non-stored `Reserve::load` calculation emit
  no carry event. Final reclassification emits the carry marker after its writes and before the
  corresponding `surplus_quarantine` event and baseline supply/withdraw event. When the same
  membership request first allocates positive `Q`, that allocation's bucket-specific event occurs
  before `cash_alloc`; any membership carry or final reclassification then occurs after
  `cash_alloc` and before the baseline request event. The zero-initialized
  persistent key is renewed and restored with the reserve accounting footprint; missing or
  archived state never defaults to zero.
- `UnownedSurplus` is an explicit accounting exclusion, not a recipient. `unowned_surplus(asset)`
  returns its exact nonnegative amount. Let `L` be aggregate represented liabilities from the
  reserve's existing ceiling conversion, `C` current backstop credit, `D` aggregate deferred
  backstop credit, and `U` unowned surplus. Every reserve reconciliation and `gulp` balance delta
  includes each term exactly once, and every committed reserve transition preserves both
  `cash + L = V + C + D + U` and `V >= L`. Therefore `cash >= C + D + U`: current and deferred
  backstop credit remain cash-backed without netting either claim against supplier loss. For any
  fixed-supply transition, with signed `delta_cash` positive into the pool, the exact supplier
  delta is the balance-sheet remainder
  `A = delta_cash + (L1 - L0) - (C1 - C0) - (D1 - D0) - (U1 - U0)`; specialized paths below must
  reduce to that equation. Borrow therefore allocates aggregate-liability rounding as
  `A = (L1 - L0) - tokens_out`, while repayment allocates
  `A = tokens_in - (L0 - L1)`. Both retain the baseline user conversion directions and reject a
  negative remainder or a post-state with `V < L`. Whenever positive unaccounted cash is observed with
  `b_supply == 0`, regardless of route or epoch state, the entire delta increments this bucket and
  emits exact topics `(Symbol("pool"), Symbol("surplus_quarantine"), asset)` with data
  `(amount: i128, total: i128)`. It never increases `b_rate`, `backstop_credit`, a deferred
  obligation, or backstop shares; first supply and first backstop deposit therefore cannot capture
  it, and its presence cannot reject epoch opening or funded activation. Final-supply membership
  reclassification emits the same event with `amount = V1`, after its required carry marker. This
  ADR adds no spend,
  recovery, or redistribution path and never decrements the bucket. Any future disposition is a
  separate deliberate ADR that preserves existing supplier and backstop claims. The key is
  persistent, renewed with the reserve accounting footprint, inventoried from the exact event,
  and restored before reserve reconciliation; archival fails closed rather than treating it as
  zero.
- Every path authorized to observe a reserve-token balance uses one private positive-cash allocation
  rule after any required reserve accrual and before its containing accounting action. This is
  private factoring, not a new export and not a way to bypass the public `gulp` status gate. From one
  observed or request-order projected balance and the exact state after accrual but before
  allocation or the containing action,
  compute
  `Q = observed_cash + L - V - C - D - U`. `Q < 0` is `InternalError` and changes nothing; `Q == 0`
  is a no-op. For `Q > 0`, the recipient depends only on the pre-allocation state:

  1. `b_supply == 0`: checked-add `Q` to `UnownedSurplus`;
  2. `b_supply > 0`, canonical route `Some(backstop)`, and the current `BackstopEpochState` is open:
     checked-add `Q` to current cash-backed credit `C` without the non-cash headroom cap; or
  3. otherwise: apply the fixed-supply supplier delta `A = Q` to aggregate `V`.

  Every positive allocation emits exact topics
  `(Symbol("pool"), Symbol("cash_alloc"), asset, recipient)` where `recipient` is exactly
  `Symbol("unowned")`, `Symbol("backstop")`, or `Symbol("suppliers")`, and exact data
  `(backstop: Option<Address>, share_epoch: Option<u64>, amount: i128, total: i128)`. Only the
  backstop branch carries `Some(current_backstop)` and `Some(current_share_epoch)`; the other two
  carry `None, None`. `total` is the post-allocation `U`, `C`, or `V` named by `recipient`.
  Any `supplier_carry` or `surplus_quarantine` event caused by the allocation itself precedes
  `cash_alloc`; `cash_alloc` then precedes the containing `gulp`, `credit_epoch/open`,
  `credit_close`, or `credit_realize` event. At a supplier-membership boundary it instead precedes
  that request's membership carry/final-reclassification events and baseline `supply`,
  `supply_collateral`, `withdraw`, or `withdraw_collateral` event.

  The public `submit`, `submit_with_allowance`, and `flash_loan` vector shapes remain unchanged, but
  all three admit at most `MAX_SUBMIT_REQUESTS = 2 * MAX_RESERVES = 60` requests. Their existing
  public authorization and self-address guards run first; the shared length check then returns
  `BadRequest` before pool/user load, flash-liability creation, reserve or token-balance reads,
  cross-contract calls, mutation, or events. Sixty is the existing maximum configurable effective-
  position footprint: `Positions::effective_count()` counts collateral plus liabilities and
  excludes ordinary supply (`pool/src/pool/user.rs:31-36`), while pool configuration permits
  `max_positions <= 2 * MAX_RESERVES` (`pool/src/pool/config.rs:204-206`). One atomic batch can
  therefore address every effective position. Repeated
  actions on the same position do not justify an unbounded loop and may be consolidated or split.
  Raising this cumulative production-v1 behavioral bound after genesis requires a reviewed pool
  upgrade; lowering it is a compatibility break. The cap bounds request-loop/event work, while
  `MAX_RESERVES = 30` separately bounds distinct projected-cash entries.

  Each admitted invocation maintains one private, non-persistent projected-cash value per distinct
  reserve asset whose `b_supply` changes and a checked prefix of scheduled-but-unreflected reserve
  cash deltas. Before an asset's first membership boundary:

  - deferred `Supply`/`SupplyCollateral`/`Repay` inflows and
    `Withdraw`/`WithdrawCollateral`/`Borrow` outflows update that unreflected prefix;
  - the `flash_loan` amount is its asset's logical first unreflected outflow because its physical
    transfer follows request construction; and
  - a `FillInterestAuction` lot is excluded from the prefix because its `C` decrement and physical
    reserve-token transfer have already occurred. User-liquidation and bad-debt fills move
    positions or backstop LP tokens, not pool reserve cash.

  On the first membership boundary, one token-balance observation plus only that unreflected prefix
  initializes projected cash. Once initialized, each newly scheduled inflow/outflow advances the
  projection immediately, and a later synchronous interest-auction lot also decrements it exactly
  when the physical transfer and `C` decrement succeed. A synchronous lot before observation is
  therefore included by the balance read; the same lot after observation is applied once to the
  projection. Neither ordering double counts it. The current supply or withdrawal transfer is
  excluded while `Q` is allocated from the pre-membership `b_supply`, route, and epoch; only after
  quote and mint/burn accounting does the exact current `tokens_in - tokens_out` advance projected
  cash. The implementation may reuse prefix `Actions` maps or another smaller private layout, but
  it may not replay all prior actions indiscriminately or derive order from the final net maps.
  Thus repeated first/later/final membership changes observe the same logical cash sequence as
  eventual transfers and use one balance read per distinct membership asset. After each modeled request,
  `projected_cash + L = V + C + D + U`; the scheduled transfers realize those modeled deltas, and
  any transfer failure rolls back the complete submit, including allocation and events. An
  unscheduled nested transfer that actually occurs only after the membership boundary is
  post-boundary cash by chronology and remains positive `Q` for the next authorized balance
  observation; it is not retroactively assigned using the earlier membership state.

  Public `gulp` after its inherited gate, `open_backstop_epoch` before it marks the epoch open,
  every supplier-membership boundary, `close_backstop_credit`, and every positive deferred-credit
  realization all use this exact rule.
  Closure therefore allocates positive `Q` to its still-open current `C` and then moves the entire
  resulting `C` into `D`. Realization allocates by the pool's current route/epoch, never by the old
  creditor that invoked it. Thus a direct transfer cannot deny a cold operation or let call ordering
  or supplier-membership order choose among suppliers, a backstop cohort, and unowned surplus.
  With no intervening state change, `transfer -> gulp -> close` and `transfer -> close` produce the
  same economic post-state because close itself performs the required accrual. Realization remains
  deliberately non-accruing for exit reliability. If its reserve is already current for the ledger,
  `transfer -> gulp -> realization` and `transfer -> realization` likewise have the same economic
  post-state apart from the required legacy public-`gulp` event. If accrual is pending, both
  orderings must still allocate the same `Q` amount to the same recipient and pay the old creditor
  the same amount, but the public-`gulp` ordering may additionally advance `L`, `V`, current `C`,
  and `last_time` and emit the corresponding accrual/carry effects. Where the inherited Borrow gate
  rejects public `gulp`, there is no public/cold equivalence premise: rejection occurs before a
  reserve load, balance read, mutation, or event, while the private close/realization path remains
  available under its own rules. Lifecycle containment, retirement, and code sealing do not alter
  the recipient function on the cold paths. The routine cannot repair a negative backing deficit
  silently; after allocation the full identity and `V >= L` must hold.
- Non-cash interest accrual preserves the solvency bound explicitly. With pre/post aggregate
  liabilities `L0` and `L1`, compute `delta_L = L1 - L0`. The nominal configured backstop take is
  zero unless the canonical route exists and its share epoch is open; otherwise it retains the
  baseline floor calculation. The actual new current credit is
  `credit = min(nominal_credit, V0 - L0)`, and the fixed-supply supplier delta is
  `A = delta_L - credit`. Thus `V1 - L1 = V0 - L0 - credit >= 0`. Cash-backed allocation is different
  from non-cash interest but identical across every balance-observing path: the state-dependent rule
  above may add the exact surplus to `C` without the non-cash headroom cap, supplier value, or `U`.
  Default computes the exact aggregate loss as
  `loss = L0 - L1` after liability removal and applies `A = -loss`; it does not reuse the
  standalone per-position ceiling conversion. Consequently `V1 - L1 = V0 - L0`, and neither
  current credit, deferred credit, nor surplus is haircut. Negative `Q`, a post-allocation identity
  mismatch, `V < L`, negative headroom, or an inconsistent exact liability delta is
  `PoolError::InternalError = 1` and rolls back the complete accrual/default/reconciliation transition.
- The canonical reserve/credit-asset universe remains unique, append-only, in stable order, and
  bounded by baseline `MAX_RESERVES = 30`; disabling a reserve never removes it from enumeration.
  Every close, deposit sync, withdrawal sync, epoch close, and bounded credit claim uses this same
  universe. A future shrink or reorder is a storage/economic migration requiring a separate ADR
  and proof that every affected reserve has zero current credit, zero deferred obligations, zero
  backstop credit outstanding/dust, and zero user claims before the change. An upgrade cannot
  orphan an asset merely because it is disabled.
- New narrow `close_backstop_credit(share_epoch, close_epoch)` is callable only by the pool's
  canonical current `Some(backstop)`, requires the stored epoch to be open and equal the supplied
  epoch, and is invoked by compatible backstop Wasm only inside `withdraw` after validating matured
  q4w but before computing or burning shares. It is an accounting close, not a token draw. The pool
  accrues every reserve in canonical reserve-list order, requires that
  list to be unique and complete, performs the canonical physical-cash allocation above for each
  reserve while the current epoch is still open, requires every resulting `backstop_credit` to be
  nonnegative, and
  deletes any unfilled interest auction for the current backstop before its quoted lot can become
  stale. For every reserve it then moves the entire scalar, with checked arithmetic, from
  `reserve.backstop_credit` into a segregated deferred-credit obligation whose immutable creditor
  is this backstop; sets the reserve scalar to zero; and returns exactly one `(asset, amount)` for
  every canonical reserve in the same order, including zero amounts. The private obligation layout
  is implementation-owned, but `deferred_backstop_credit(backstop, asset)` exposes its exact
  remaining amount. The move preserves, per reserve, `cash + L = V + C + D + U`, `V >= L`, and
  therefore `cash >= C + D + U`; `gulp` includes aggregate supplier value, aggregate deferred
  credit, and unowned surplus in its accounted-balance calculation so it cannot book the same cash
  twice. Closing credit reclassifies cash-backed `C` as cash-backed `D` but deliberately transfers
  no reserve tokens. This keeps the legacy share-withdrawal wire and its LP-token transfer separate
  from a bounded multi-asset credit claim. When
  `close_epoch` is true, after all credit moves it also stores `open = false` and emits
  `(Symbol("backstop"), Symbol("credit_epoch"), backstop, Symbol("close"))` with data
  `share_epoch: u64`; false leaves the epoch open. Passing true is a cumulative behavioral promise
  that the invoking backstop is about to burn the exact final shares/q4w in this same transaction.
- Malformed closure fails closed. A negative scalar, duplicate or omitted reserve, missing reserve
  state, checked-arithmetic failure, or inconsistent obligation total panics
  `PoolError::InternalError = 1`; nothing is cleared, moved, or auction-invalidated. Existing
  interest-auction deletion follows any bucket-specific and `cash_alloc` events for that reserve;
  it emits `delete_auction`, then each positive credit move emits exact topics
  `(Symbol("backstop"), Symbol("credit_close"), backstop, asset)` with data
  `amount: i128`; zero entries remain in the returned completeness proof but emit no close event.
  Any failure rolls the entire outer withdrawal back. The pool makes no call into the backstop or
  registry and cannot prove the backstop's share allocation. Calling this export outside the
  validated pre-burn withdrawal branch is therefore a compatibility violation that retained-Wasm
  and actual-Wasm upgrade tests must reject; it can reclassify value only for the named creditor,
  never an arbitrary recipient.
- New cold `claim_backstop_credit(backstop, asset, max_amount) -> i128` requires the recorded
  creditor backstop's auth, rejects a negative maximum, and remains callable after containment,
  retirement, route change, or code sealing. It does not accrue or close current-epoch credit. It
  returns zero without a balance read, transfer, write, or event when the maximum or obligation is
  zero. Otherwise it performs the canonical positive-cash allocation against the actual token
  balance before validating the same per-reserve identity and `V >= L` bound. An open current epoch
  receives positive `Q` in its current `C` even when the caller is an older creditor; with no open
  current epoch, positive-supply value enters `V`, and zero-supply value enters `U`. It then pays
  exactly `min(max_amount, deferred obligation)`. A positive amount is fully cash-backed by the proved
  identity; an insufficient observed balance, backing deficit, malformed obligation, or transfer
  delta mismatch fails `PoolError::InternalError = 1` atomically rather than returning a
  liquidity-limited partial result. The caller chooses an intentional partial claim through
  `max_amount`. A positive call transfers exactly that amount to the creditor and decrements the
  same obligation atomically. Its exact positive event topics are
  `(Symbol("backstop"), Symbol("credit_realize"), backstop, asset)` with data `amount: i128`.
  Any bucket-specific and `cash_alloc` events precede that realization event.
  This non-accruing boundary is deliberate: a stale reserve does not make old-cohort exit depend on
  the interest model. Public `gulp` may accrue first when its Borrow gate permits, so only an
  already-current reserve has full public-first/direct economic-state equivalence. With pending
  accrual, the required equivalence is limited to the allocation's `Q` recipient and amount and the
  old creditor's payout; public `gulp` may additionally advance and event its accrual state.
  Thus an old cohort retains its segregated multi-asset ownership without reviving shares, donating
  to a new epoch, choosing an arbitrary recipient, or changing the legacy withdrawal ABI.
  An unsolicited positive balance delta before or after containment, retirement, or code sealing
  therefore cannot block a fully backed claim or pre-burn closure; a deficit still fails closed.
- New cold view `backstop_ready(backstop) -> bool` reads that candidate's `pool_data`, applies
  the existing threshold calculation, and requires q4w below 50%. It does not store the
  candidate or change pool behavior.
- The 11 read sites plus `gulp`, with `None`:
  - `status.rs:12-63,72-115`: permissionless `update_status` panics in setup status 6; only the
    admin `set_status` path can leave setup. In the baseline, status 3 is the permissionless-
    computed on-ice state, although the admin setter also accepts 3 under the same q4w bound as
    status 2. A `None` pool has no backstop signal from which to compute that state, so
    `update_status` continues to panic and the new `set_status` branch deliberately accepts only
    `{0, 2, 4}` (admin active/on-ice/frozen). It rejects 3 as a misleading automatic-health label,
    not because its baseline q4w threshold differs. Leaving setup uninsured is the explicit admin
    call `set_status(0)`; no permissionless path exits status 6. While lifecycle is `Live` or
    `Contained`, `set_status(4)` matches and writes the unconditional freeze before any backstop
    lookup or call. This deliberately
    changes the baseline ordering at `status.rs:74-78,107-110`, where even status 4 first calls
    `backstop.pool_data()`, so a defective current backstop cannot disable the documented direct
    freeze fallback.
  - `bad_debt.rs:12`: the `user == backstop` branch is unreachable; falls to the user path.
  - `bad_debt.rs:53-78`: instead of moving liabilities onto a backstop `User`, call
    the same liability-removal branch for each reserve, snapshotting aggregate `L0` before and
    `L1` after removal, then apply the exact supplier delta `A = -(L0 - L1)` and emit the existing
    `PoolEvents::defaulted_debt`. It does not use the baseline standalone per-position ceiling
    conversion from `User::default_liabilities`, because that amount can differ from the aggregate
    represented-liability reduction. The current-backstop default path adopts the same exact
    aggregate-loss rule. This is the uninsured-pool semantics and the documented risk of running
    without a backstop; the `V >= L` proof preserves solvency without netting current/deferred
    credit or surplus.
  - `user_liquidation_auction.rs:19`: only the `user == current_contract` check applies.
  - `backstop_interest_auction.rs:18-25` and `bad_debt_auction.rs:21`: `create` panics
    `BadRequest` (no backstop to auction with; `bstop_rate` is 0 so interest accrues no
    credit).
  - `gulp.rs:18-37`: preserve the baseline public `Borrow`/pool-status gate **before** either route
    branch. With zero supply, every route/epoch branch quarantines positive surplus as
    `UnownedSurplus` and requires zero supplier carry. Otherwise, with `None` or a closed backstop
    share epoch, positive surplus is credited only to suppliers through the exact rate/carry
    equation; no value is written to `backstop_credit`, `V` increases by exactly the observed
    surplus, and a repeat call without new cash observes zero. With an open current-backstop epoch,
    the observed cash surplus becomes cash-backed current credit under the canonical recipient rule.
    The public return value keeps its baseline meaning—current-backstop credit created—so it
    returns `0` for
    the supplier and quarantine branches even when their state changes.
    Every successful invocation preserves the legacy event contract: after any committed
    `supplier_carry` or `surplus_quarantine` event and any positive allocation's exact `cash_alloc`
    event, it emits exactly one
    `(Symbol("gulp"), asset)` event with the returned `token_delta` as data. A repeated unchanged
    supplier/quarantine call therefore makes no state mutation and emits no new carry, quarantine,
    or `cash_alloc` event, but still emits one legacy `gulp(asset, 0)` event. Failed calls roll back
    every event.
    `open_backstop_epoch` performs the same zero-take canonical allocation internally while the
    epoch remains closed, then emits `cash_alloc` before the epoch-open event;
    its current-backstop auth and `Live`/first-deposit rollback boundary replace the public Borrow
    gate so an empty backstop cannot make opening impossible. A cumulative fee/accounting ADR may
    further divide positive-supply value, but it cannot assign either `UnownedSurplus` or an
    already-accounted supplier carry to a backstop or weaken this zero-supply gate. These are the
    ADR-0002-local recipient/exclusion, rounding, conservation, and failure semantics, not private
    helper layout. An expressly amending pre-genesis ADR may replace them before the cumulative
    manifest freezes the production behavior; a post-freeze change is versioned.
  - `emissions/manager.rs:62-64`: `gulp_emissions` returns 0 without calling out.
  - `distributor.rs:53-57`: unreachable while `None` — `claim` only reads the backstop when
    `to_claim > 0` (`:52`) and a never-attached pool has gulped nothing, so `claim` returns
    0. Guarded with `None ⇒ return 0` for clarity. These are the only emission paths authorized by
    the disabled profile, not a positive-claim specification; the issuance ADR replaces them.
- `update_pool` (`contract.rs:392-399`, admin-only, cold) calls
  `registry.check_config(take_rate, max_positions, min_collateral, backstop.is_some())`
  before `execute_update_pool`; this is the `MinConfig` enforcement on the update path and
  carries the `None ⇒ take_rate == 0` rule. `submit`, `submit_with_allowance`, and `flash_loan`
  make no new registry call and retain their baseline auth and wire behavior. Their only new
  branches are the route-local uninsured bad-debt result after a full liquidation and the exact
  lifecycle/status failures above. `new_auction` continues to permit user liquidation while
  `None`, but interest/bad-debt auction creation rejects `BadRequest`; `bad_debt` uses the explicit
  supplier-default branch while `None`. With `Some`, both retain baseline auction settlement apart
  from credit that a withdrawal has atomically moved into the old cohort's deferred obligation.
- Maintenance entrypoint `upgrade(hash)`: registry invoker auth **and** pool-admin auth,
  then `e.deployer().update_current_contract_wasm(hash)`. It does not call
  `registry.is_sealed(...)` or any other registry view; policy and seal checks already ran in
  the invoking registry transaction, and this ADR forbids a callback into that active governance
  transition regardless of host-version behavior. SDK 27.0.6
  applies the new executable only after the successful invocation; it retains storage without
  conversion, and the next call runs the new code. Every future storage-shape change therefore
  requires its own ADR, an explicit idempotent `StorageVersion` migration state machine,
  entrypoint gating for intermediate states, and migration/rollback tests. Recording an old hash
  alone does not make rollback safe.
- Pool wasm delta: `Option`, the `None` branches, scalar views, supplier-carry accounting,
  attachment/upgrade entrypoints, and shared clients. Report exact before/after bytes; each artifact
  must remain at or below
  `min(131072, active_network.contractMaxSizeBytes)` for every network it will be deployed to.
- Storage wire change: the `"Backstop"` instance key changes type `Address →
  Option<Address>`; `"Registry"`/`"Lifecycle"`/`"BackstopEpoch"` and a
  per-creditor/per-asset deferred-credit obligation namespace plus per-reserve supplier-rounding-
  carry and unowned-surplus accounting are added. Deliberate; greenfield, so no migration (`AGENTS.md` storage-key
  rule acknowledged). Exact private keys remain implementation-owned, while the obligation view,
  identity, events, TTL behavior, and claim transition above are normative. A `None` pool has no
  current backstop; candidate shares may exist only under `PendingAttach`, and the local activation
  gate keeps that capital out of the reward zone. ADR 0002 creates no pool emissions for `None`;
  only the Accepted issuance decision may add them.
- In the ADR-0002-local baseline, each disposable pool's `update_pool` calls this `check_config`
  signature. The mandatory fee ADR adds a separate `check_fees` method rather than changing the
  non-fee predicate; that is a compositional pre-genesis design choice, not an early production-v1
  freeze. Each artifact remains bound to the exact signatures it was built against. The cumulative
  manifest freezes the final production call set, after which a change uses a uniquely named
  extension and deployed pools retain the older symbol until a separately reviewed migration.

### Backstop changes (upstream PR to `backstop/`)

The exact new or behavior-changed ADR-0002-local pre-genesis backstop exports are below. An
expressly amending mandatory ADR may change this disposable list before the cumulative manifest
freezes the production-v1 backstop ABI; later changes are versioned.

```rust
fn __constructor(e: Env, backstop_token: Address, emitter: Address, blnd_token: Address,
                 usdc_token: Address, registry: Address,
                 drop_list: Vec<(Address, i128)>) -> ()
fn registry(e: Env) -> Address
fn interface_version(e: Env) -> u32
fn storage_version(e: Env) -> u32
fn exit_delay(e: Env) -> u64
fn relationship(e: Env, pool: Address) -> Option<BackstopRelationship>
fn fund_pool(e: Env, pool: Address, generation: u64) -> ()
fn activate_pool(e: Env, pool: Address, generation: u64) -> ()
fn contain_pool(e: Env, pool: Address) -> ()
fn resume_pool(e: Env, pool: Address, generation: u64) -> ()
fn retire_funding(e: Env, pool: Address) -> ()
fn touch_pool(e: Env, pool: Address) -> bool
fn deposit(e: Env, from: Address, pool: Address, amount: i128) -> i128
fn queue_withdrawal(e: Env, from: Address, pool: Address, amount: i128) -> Q4W
fn dequeue_withdrawal(e: Env, from: Address, pool: Address, amount: i128) -> ()
fn withdraw(e: Env, from: Address, pool: Address, amount: i128) -> i128
fn share_epoch(e: Env, pool: Address) -> u64
fn credit_state(e: Env, pool: Address, asset: Address) -> BackstopCreditState
fn credit_claim(e: Env, from: Address, pool: Address, asset: Address) -> U256
fn claim_credit(e: Env, from: Address, pool: Address, assets: Vec<Address>,
                to: Address) -> Vec<(Address, i128)>
fn pool_data(e: Env, pool: Address) -> PoolBackstopData
fn reward_zone(e: Env) -> Vec<Address>
fn distribute(e: Env) -> i128
fn gulp_emissions(e: Env, pool: Address) -> i128
fn add_reward(e: Env, to_add: Address, to_remove: Option<Address>) -> ()
fn remove_reward(e: Env, to_remove: Address) -> ()
fn claim(e: Env, from: Address, pools: Vec<Address>, min_lp_tokens_out: i128) -> i128
fn drop(e: Env) -> ()
fn donate(e: Env, from: Address, pool: Address, amount: i128) -> ()
fn draw(e: Env, pool: Address, amount: i128, to: Address) -> ()
fn upgrade(e: Env, wasm_hash: BytesN<32>) -> ()
```

`BackstopCreditState` is the exact public contract type
`{ share_epoch: u64, index: U256, outstanding: U256, dust: U256 }`. Credit amounts are scaled by
the fixed magnitude `M = 2^128`. `index` is an element of the ring `Z/(2^256)` rather than an
ever-growing integer; checkpoints and event replay use the modular-distance rule below. `dust` is
the actor-neutral scaled residual bucket: a close stores its division carry there, and a full exit
also adds that holder's explicitly forfeited fractional residue before normalization. It is never a
close-actor rebate. The other `U256` fields are ordinary nonnegative checked integers. Changing the magnitude,
ring semantics, or any field meaning is an ABI/economic migration, not a private-layout choice.
`credit_claim` returns
the user's exact stored plus currently implicit scaled entitlement after applying the modular index
distance as a read-only calculation; integer tokens presently payable are
`floor(credit_claim / M)`.

`Q4W` and every other unchanged backstop boundary type retain their baseline wire declaration. No
new backstop error code is needed: invalid relationship transitions or admission state use existing
`BackstopError::BadRequest = 1000`; a failed registry-membership check uses
`BackstopError::NotPool = 1004`; zero-share mint retains 1005. A zero withdrawal amount and a
rounding-to-zero LP-token withdrawal with `pool_balance.tokens > 0` each use
`InvalidTokenWithdrawAmount = 1006`; exact zero accounted LP tokens permit a zero-token return only
for a **positive** matured-share burn by a positive holder. Named-pool realized liabilities retain
`BackstopError::BadDebtExists = 1011`. User-supplied duplicate or unknown claim assets, or more
than `MAX_RESERVES = 30` claim assets, use `BadRequest = 1000`. Duplicate/omitted reserve coverage,
a negative or misreconciled local credit amount, a modular index distance outside the proved
half-ring bound, a nonzero pre-mint dust-normalization quotient, post-normalization dust at or above
positive share supply, a carry-equation mismatch, a full-exit residue at or above `M`, a post-burn
normalization mismatch, checkpoint/claim/dust/outstanding overflow or underflow, a pool-return
vector that differs from the pre-call canonical reserve list, or a
realized-token balance delta that differs from the pool result uses
`BackstopError::InternalError = 1`; and authorization failures remain host-auth failures. Nested
pool failures from credit closure or realization propagate unchanged and roll back; the backstop
does not translate them to a generic local code.

Under the emissions-disabled profile, a non-empty constructor `drop_list`, every disabled emission
mutator including `drop`, an empty or duplicate backstop emission-claim pool list, and a claim entry
without a local `Active`, `Contained`, or `Retired` relationship use the same inherited
`BackstopError::BadRequest = 1000`. The constructor never stores or invokes its ABI-reserved
`emitter`; no drop, reward, backfill, distribution, reserve-emission, or user-emission key exists.

- The stored factory authority becomes the registry. For each pool, the backstop maintains the
  exact public `BackstopRelationship` lifecycle and exposes
  `relationship(pool) -> Option<BackstopRelationship>`. Registry-authenticated `fund_pool` enters
  `Funding(generation)` from absent/`Retired`/stale `Funding` (same-state re-begin is idempotent);
  `activate_pool` accepts matching `Funding` or absent initial deployment only after verifying
  `pool.get_backstop() == Some(self)`; when local shares are positive it also requires the pool's
  epoch state closed and atomically calls `pool.open_backstop_epoch(local_share_epoch)`, while a
  zero-balance initial deployment stays closed until its first `Active` deposit. `contain_pool`
  moves stored `Active -> Contained`; and
  `resume_pool` moves `Contained -> Active` while updating to the registry's current generation.
  They reject every other edge. Permissionless `retire_funding` changes
  `Funding(generation)` to `Retired(generation)` only after the raw registry predicate above proves
  the exact `(self, generation)` tuple absent/different or the current raw metadata generation
  permanently different. It fails closed on missing/archived state and cannot retire merely because
  the normalized admission view is temporarily `None`. These are local capabilities, never another
  address-valued route. When activation opens a positive-balance epoch, the pool's
  `credit_epoch/open` event precedes the backstop's `relationship/active` event and the registry's
  later `attach/final` event.
- `deposit` accepts `Active` locally without a registry call, accepts `Funding` only after the
  fresh matching normalized `pending_admission` check, and rejects absent, archived,
  `Contained`, or `Retired` state. `donate` and `draw` require `Active` or `Contained`; existing
  sender/pool authorization still applies. The old `shares > 0 => trusted` shortcut
  (`backstop/src/backstop/pool.rs:83-99`) is deleted because pending candidates can legitimately
  hold positive shares. Missing or archived relationship state fails closed until restored; a
  deposit never manufactures lifecycle state.
- ADR 0002's disabled profile has no reward membership, accrual, or positive claim in any
  relationship state. The relationship lifecycle is nevertheless the mandatory integration
  boundary for an Accepted issuance decision: `Funding` is never reward-eligible; `Active` is the
  only state that may admit new rewards; `Contained` and `Retired` accrue nothing new; resume does
  not silently re-admit a pool; and recorded entitlements, if that later decision creates any, must
  have an explicit non-confiscating settlement that cannot manufacture a share-epoch opening.
  Containment and any issuance-defined reward removal commit in one rollback domain, with the
  issuance-defined local terminal event before the relationship event. A stale `Funding`
  relationship that becomes `Retired` was never reward-eligible.
- The generation on `Funding` must equal current registry metadata. On `Active`/`Contained` it is
  the activation cohort: disable/upgrade invalidates pending funding but does not silently disable
  already attached routes. Containment preserves that possibly older cohort. A delayed successful
  resume accepts that older `Contained` generation, revalidates the same canonical address against
  enabled current code/health, and records the then-current registry generation; requiring the old
  cohort generation to equal current metadata would strand every contained pool after a disable or
  same-address upgrade and is forbidden.
- Share economics remain per-pool and exact across every lifecycle. Each pool balance has a
  checked, monotonically increasing `share_epoch`, initialized to 1 and exposed by
  `share_epoch(pool)`. A fresh epoch is permitted only when `shares == tokens == q4w == 0` and
  the prior epoch's index and dust have closed and reset; `credit_state.outstanding` may remain
  positive for former shareholders and does not block it. Its first positive deposit mints one
  share per token. Any
  positive donation or other accounted LP-token inflow while shares are zero is rejected, so a
  donation cannot become a first-depositor windfall. Later deposits mint
  `floor(amount * total_shares / accounted_tokens)` and reject a zero mint. Cancellation,
  disable, containment, retirement, and re-begin do not reset shares, epoch, or price. For an
  `Active` current route, the first deposit calls `pool.open_backstop_epoch(share_epoch)` before
  mint/transfer; for a `Funding` candidate, the route is still `None`, so the first deposit remains
  segregated and `activate_pool` performs that open atomically with finalization when positive
  shares exist. Either the open and surrounding deposit/activation all commit or all roll back.
  These are the only share-mint/opening paths authorized by ADR 0002. A future positive
  emission-claim share mint is non-conforming unless the Accepted issuance decision supplies the
  complete integration contract required above.
- Closed credit uses a non-enumerating per-asset dividend index patterned after the existing
  emissions index/checkpoint shape, but counts **all** of a user's economic shares: liquid shares
  plus every queued-for-withdrawal share. Let `M = 2^128`, `S > 0` be pre-change total shares,
  `C >= 0` be newly closed base units for one asset, and `D` be that epoch's pre-close dust. Checked
  `soroban_sdk::U256`-width amount arithmetic computes `numerator = C * M + D`,
  `delta = floor(numerator / S)`, and `next_dust = numerator - delta * S`. Because total positive
  shares fit in `i128`, `0 <= next_dust < S < M / 2`. The backstop adds `delta` to the current
  epoch's index **modulo `2^256`**, stores `next_dust`, and checked-adds only `C * M` to aggregate
  outstanding credit. Thus `delta * S + next_dust == C * M + D` exactly: prior dust is moved, not
  re-issued, and no account receives the division remainder directly. The authenticated `from`
  account is the close actor, not a rounding beneficiary. Index wrap is a normal transition, not
  overflow and not an error.
- Each positive `C` emits exact topics
  `(Symbol("backstop"), Symbol("credit_index"), pool, asset)` with data
  `(from: Address, share_epoch: u64, amount: i128, index: U256, remainder: U256)`, where `from` is
  only the authenticated actor, `index` is the post-update ring value, and `remainder` is the
  post-close `next_dust` state rather than an amount credited to `from`. Zero entries remain in the
  pool-returned completeness vector but emit no index event. `credit_state` exposes the same
  epoch/index plus aggregate outstanding and current dust so complete event replay can reconcile
  the carry without enumerating holders.
- User checkpointing is ordered around every ownership change. A user's pending scaled credit is
  synchronized as
  `stored_claim += holder_shares * ((current_index - checkpoint) mod 2^256)` for every canonical
  asset, then the checkpoint is set to the current index. The modular difference is the least
  nonnegative residue. For every positive holder it is uniquely the true unsynchronized distance:
  its product with at least one holder share is a nonnegative component of `outstanding`, while
  `outstanding = deferred * 2^128 <= i128::MAX * 2^128 < 2^255`. Therefore a valid account's
  distance is strictly below half the ring: a positive holder cannot skip a full ring, and the least
  residue cannot be confused with a later wrap. A distance at or above `2^255`, or a product greater
  than outstanding, is invariant corruption and fails atomically with error 1. A zero-holder account
  accrues nothing and simply checkpoints the current ring value. For any positive share supply
  `T`, dust normalization means adding `floor(dust / T)` to the ring index and replacing dust with
  `dust mod T`; it preserves outstanding exactly and leaves `0 <= dust < T`. Before a mint,
  `deposit` invokes the same normalization against the pre-mint supply. This is a checked no-op in
  valid state because the preceding close/burn already established `dust < T`; a nonzero pre-mint
  quotient is corruption and fails with error 1 rather than silently changing replay state. It then
  synchronizes the user's existing holder shares and gives newly minted shares that post-sync
  checkpoint. Every already allocated index unit is therefore excluded from the new shares.
  `withdraw` first closes credit and updates each index,
  then synchronizes the user's full pre-burn holder shares before burning; after every non-final
  burn it normalizes against the remaining positive supply. `claim_credit` synchronizes each requested
  asset before any positive payout; a zero-result path may leave the equivalent entitlement
  implicit rather than writing an otherwise unnecessary checkpoint. Queue/dequeue only moves
  shares between liquid and q4w state, so it changes neither holder shares nor the credit
  checkpoint. A checkpoint is tagged with `share_epoch`; an
  old zero-balance user entering a later epoch starts at that epoch's current index while retaining
  whole claims from earlier epochs.
- An integer per-share index cannot assign a nonzero close carry `dust < S` exactly to the
  pre-change holders without enumerating them. At a mint or non-final burn, that still-unassigned
  carry represents less than `S / M < 1/2` base unit total per asset. It remains conserved and may
  be allocated by a later normalization or close over the then-current shares; it cannot be paid,
  duplicated, or directed to the transaction actor.
- A separate deliberate rounding rule applies when a holder fully exits. Keeping a permanently
  zero-share user's sub-base-unit key would let fractional claims across many departed users strand
  whole realizable tokens. The exiting holder therefore retains `floor(stored_claim / M)` whole
  units but forfeits `stored_claim mod M < M` for that asset. That already allocated residue joins
  dust and is normalized over remaining shares. One account loses less than one base unit per asset,
  and one transition has `pre_dust + added < 3M/2`; nevertheless it can exceed `M` and assign a
  whole unit to the remaining cohort. Residues from multiple full exits accumulate economically
  across transitions and can assign multiple whole units in aggregate. This is not described as a global
  half-unit bound. The exact `credit_dust.added` event makes each forfeiture observable, and no
  account can force another account's authenticated withdrawal.
- The close formula itself contains no address: before the requested burn, two holders with equal
  shares/checkpoints receive equal incremental entitlement even when one is `from`, and close
  writes no actor-specific claim. Equal holders with identical membership histories remain equal;
  a full exit is intentionally a different membership history with the bounded forfeiture above.
  Partitioning a fixed credit amount across closes while supply is unchanged yields the same final
  index and dust as one aggregate close.
- `withdraw` therefore preserves credit ownership without transferring reserve cash. Immediately after
  authenticating `from`, and before a pool call, reserve-list read, credit closure, balance/index
  mutation, or event, it rejects `amount < 0` with the inherited error 8 and `amount == 0` with
  error 1006. It then proves that `amount` is covered by positive matured q4w and that `from` has a
  positive combined liquid-plus-q4w holder balance; ordinary insufficient/unmatured queue failures
  retain errors 10/1001, while an internally inconsistent nonpositive holder balance fails error 1.
  Only after those checks does it reject named-pool realized liabilities, record pre-withdraw
  `S = total_shares`, and read the pool's canonical reserve list and route before changing any
  backstop balance. For `Some(self)`, it calls
  `close_backstop_credit(share_epoch, shares_redeemed == S)` and requires the returned vector to
  have the identical length, addresses, uniqueness, stable order, and nonnegative amounts as that
  pre-call list, then applies the index rule above. A segregated `Funding` or stale `Retired`
  candidate requires route `None` and a zero index/dust for its current share epoch, so it skips
  closure; older whole claims may remain outstanding. `Some(other)` fails `BadRequest`.
- LP-token redemption then retains the baseline floor conversion. If accounted LP tokens are
  positive and the result is zero, it panics `InvalidTokenWithdrawAmount = 1006`; if accounted LP
  tokens are exactly zero, any otherwise-valid matured redemption may return zero and still burn
  the requested shares/q4w. Whenever that burn leaves `from` with zero holder shares, each
  `stored_claim mod M` is removed from the user and checked-added to the current epoch's dust, so
  the former holder keeps every whole base unit, explicitly forfeits less than one base unit, and
  no unreachable fractional key survives. After
  any non-final burn, with remaining supply `S' > 0`, each asset's resulting dust is normalized
  against `S'` before the withdrawal commits; this includes a partial burn where no user residue
  was added. The exact final burn requires `shares_redeemed == S == total_q4w`, returns all
  remaining LP tokens (possibly zero), and synchronizes/consolidates the last holder. At that point
  all current-epoch implicit credit is materialized, so dust must be exactly divisible by `M`; the whole
  `dust / M` is assigned to the final redeemer's claim, dust is zeroed, the index is reset, balances
  reach exact zero, and `share_epoch` checked-increments; every per-asset `credit_state` is retagged
  to that new epoch with zero index/dust while aggregate outstanding persists. This deterministic
  rounding never reallocates a whole unit from any one exiting account: across `N` earlier full
  exits, the total allocated value moved away from those accounts is strictly less than `N` base
  units per asset. Remaining holders may therefore receive whole units in aggregate, as well as less
  than half a base unit of globally unassigned carry; that accepted effect is bounded by the sum of
  the evented forfeitures and carry rather than by a global half-unit claim. A non-divisible or
  unreconciled final dust value fails `InternalError = 1` and rolls the entire withdrawal back.
- Every burn that adds a full-exit residue, changes index/dust through post-burn normalization, or
  closes the epoch emits exact topics
  `(Symbol("backstop"), Symbol("credit_dust"), pool, asset)` with data
  `(from: Address, share_epoch: u64, added: U256, index: U256, dust: U256, assigned: i128)`;
  `share_epoch` is the pre-increment epoch, `added` is the actor's consolidated fractional claim or
  zero, `index` and `dust` are post-normalization state, and `assigned` is zero before the final
  burn and the exact whole amount assigned when the epoch closes. The final branch emits this event
  even when `added` and `assigned` are zero. For a non-final burn, replay verifies
  `normalization_delta * S' + post_dust == pre_dust + added`; for the final burn it verifies
  `assigned * M == pre_dust + added` before the reset. Neither path treats `added` as new
  outstanding. Pool `credit_close` events and its final
  `credit_epoch/close` event precede backstop `credit_index` and `credit_dust` events. The ordinary
  withdrawal event is preserved **unchanged** from baseline and is last: exact topics
  `(Symbol("withdraw"), pool, from)`, exact data `(amount: i128, tokens_out: i128)`. Epoch is
  derived from the corresponding ADR-0002-local `credit_epoch/open|close` and credit events, not
  inserted into the legacy withdrawal wire. Any closure, comparison, arithmetic, checkpoint, dust, storage, or
  transfer failure rolls the entire withdrawal back.
- `claim_credit(from, pool, assets, to)` is a separate exit-preserving path. It requires `from`
  auth and accepts at most 30 unique assets from the pool's canonical reserve universe. Empty
  input returns an empty vector after auth with no pool call or event; duplicate, unknown, or
  over-limit input fails `BadRequest = 1000` before mutation. For each valid asset it first syncs
  the user's current holder shares as a checked calculation and computes
  `whole = floor(scaled_claim / M)`. When `whole` is zero it returns zero for that element without
  a pool call, persistent write, or event. Otherwise it
  calls `claim_backstop_credit(self, asset, whole)` and requires the result and this contract's
  token-balance increase to equal `whole`. The pool-side `cash >= C + D + U` proof makes this
  positive claim fully backed; a zero/partial result, balance-delta mismatch, or backing failure is
  invariant failure and rolls back rather than becoming a liquidity-dependent allocation race.
  Before payout it materializes the synchronized checkpoint/claim, subtracts exactly `whole * M`
  from both the user's scaled claim and aggregate outstanding, transfers exactly `whole` to `to`,
  and emits
  `(Symbol("backstop"), Symbol("credit_payout"), pool, asset)` with data
  `(from: Address, to: Address, amount: i128)`. The returned vector contains one result per input
  asset in input order, including zeros; an all-zero request emits nothing. Pool `credit_realize`
  precedes matching backstop `credit_payout`; failure rolls back both. User-selected input assets
  provide bounded partial collection without making liquidity or call order an economic input.
  Claims remain callable and renewable after relationship containment/retirement, share-epoch
  rollover, route change, or sealing. They are not shares, cannot be donated into a later epoch,
  earn no later credit or emissions, and are not transferable by this ADR.
- Normal interest auctions retain their 120%-priced LP-token conversion for credit not yet closed.
  Once closed, the deferred obligation is no longer auction lot or current-epoch credit. Starting
  a new share epoch neither deletes nor assigns outstanding user credit or any issuance-defined
  reward-emission state to its first depositor; the cumulative issuance decision must preserve
  that separation. Direct
  token transfers that bypass an accounting entry never become an unnamed pool's assets.
- Aggregate LP-token reconciliation includes that direct-transfer case explicitly. Let `T` be the
  backstop contract's physical configured LP-token balance and `A` the event-replayed sum of every
  pool's accounted `BackstopBalance.tokens`. Require `T >= A` and define derived
  `Z = T - A` as unowned token excess. `Z` is not stored, share-priced, withdrawable, donatable,
  drawable, or attributable to any pool under this ADR; a direct transfer increases only `T` and
  therefore `Z`. Every accounted deposit, withdrawal, donation, draw, or loss transition preserves
  the exact `T = A + Z` reconciliation according to its existing physical/accounted deltas, while
  `T < A` is invariant failure. The bounded event inventory plus the live configured-token balance
  derives `Z`; no on-chain pool enumeration or new spend authority is added. Any later disposition
  of that excess requires a deliberate ADR and cannot reduce `A` or outstanding deferred credit.
- Backstop greenfield storage adds checked share-epoch, per-pool/asset scaled
  index/outstanding/dust state, and epoch-tagged per-user/pool/asset checkpoint/scaled-claim state.
  Their exact private keys are implementation-owned; the
  public views, transitions, equality to pool deferred obligations, event replay, and TTL/restore
  behavior are normative. No holder/user enumeration is added on chain; every pool-level sync or
  touch may and must iterate the pool's already-canonical bounded reserve universe of at most 30
  assets, whether the private asset state is co-located or locally indexed.
- Queued-for-withdrawal shares remain in total shares and bear the same gains and realized losses
  until redeemed; queueing is not economic exit. `queue_withdrawal`, `dequeue_withdrawal`, and
  `withdraw` ignore admission/relationship state. Withdrawal is blocked only by the named pool's
  nonzero realized backstop liabilities or a failed accounting reconciliation, not by the mere
  existence of an auction: auction creation is permissionless and a blanket auction lock would be
  griefable. Every successful transition preserves per-pool shares/q4w/token conservation and the
  aggregate `T = A + Z` backstop-token reconciliation rather than falsely requiring direct-transfer
  excess to equal zero. Deferred underlying credit is reconciled separately:
  for each `(pool, asset)`, `pool.deferred_backstop_credit * M == credit_state.outstanding`, and
  outstanding equals epoch dust plus all stored user scaled claims plus all implicit
  `holder_shares * ((index - checkpoint) mod 2^256)` entitlements. Closing increases both sides by
  `C * M`; its carry equation moves prior dust into index without counting it twice.
  Realization/payout decreases both sides by `paid * M`; sync, burn, dust
  normalization/consolidation, and epoch reset only move value within the right-hand decomposition.
  This invariant is checked by aggregate
  state and event replay without on-chain user enumeration. It does not enter LP-token
  reconciliation or change the rule that queued shares remain loss-bearing until actual redemption.
- A cold, permissionless `touch_pool(pool)` makes no external call. It succeeds when any live
  local pool-balance, relationship, share-epoch, or credit-state entry exists, extends every
  present one plus this backstop's instance/code toward the current maximum, and returns `true`.
  If an Accepted issuance decision enables reward state, it must extend this footprint contract to
  every independently archived per-pool emission entry it creates; ADR 0002's disabled profile has
  none. This keeps an
  inactive cancelled pool maintainable while segregated balance/q4w remains, without requiring a
  Registry -> Backstop -> Registry callback. It cannot restore an archived entry; the keeper
  restores the exact transaction footprint first and then retries the local touch.
- Pool-level touch is not user-state maintenance. The current backstop stores
  `UserBalance(PoolUserKey { pool, user })` as non-enumerable persistent state
  (`backstop/src/storage.rs:67-83,208-247`). Operations maintain an event-derived
  inventory of every `(backstop, pool, user)` first seen in a fully decoded `deposit`,
  `credit_index`, `credit_dust`, or `credit_payout` event (topics **and** data) and reconcile it
  with `queue_withdrawal`,
  `dequeue_withdrawal`, and `withdraw`; the baseline share topics already carry pool and user
  (`backstop/src/events.rs:6-75`). The tuple remains in the inventory for the
  backstop's lifetime because zero shares do not prove zero stored credit claims. For
  each tuple, the keeper uses native transaction-level `ExtendFootprintTtl` and
  `RestoreFootprint` operations
  over the exact contract-data keys; it does not add a `touch_user` contract entrypoint. The
  operation-specific footprint is obtained by simulating the intended
  queue/dequeue/withdraw/credit-claim and includes, at minimum, `UserBalance`, every per-user
  scaled claim/checkpoint, the associated pool-balance, share epoch, global credit state,
  relationship, and contract instance/code. An Accepted issuance decision adds its own exact
  per-user/per-pool emission keys and operation footprints; ADR 0002 does not guess them.
  Pool-side deferred-credit, supplier-carry, and unowned-surplus keys are
  likewise inventoried from
  `credit_close|credit_realize|supplier_carry|surplus_quarantine|cash_alloc` events and
  renewed/restored before reconciliation or a credit claim. Replay recomputes each positive `Q`
  from the pre-allocation reserve state, verifies the event's recipient/address/epoch/amount/total,
  and rejects a bucket-specific event after its `cash_alloc` or a containing action before it. For
  membership allocation, replay walks the transaction's requests in order and distinguishes cash
  already reflected in the first physical observation from scheduled-but-unreflected cash. Deferred
  supply/collateral/repay inflows, deferred withdraw/collateral-withdraw/borrow outflows, and the
  logical flash outflow update the unreflected prefix. A synchronous interest-auction lot before the
  first observation is already reflected and is not replayed; the same lot after projection
  initialization decrements that projection exactly once. Replay excludes the current supply or
  withdrawal flow until after allocation and mint/burn, and rejects a membership carry/
  reclassification before `cash_alloc` or a baseline request event before the membership effects.
  Restoration
  completes before the user operation is retried.
- Dependency: the `pool_factory.wasm` `contractimport!` is replaced by the shared registry
  interface, preserving the acyclic interface → registry → backstop → pool dependency. Existing
  fixtures are adapted to construct through the registry; their file and target layout is not
  normative. The shared pool client used by the backstop includes
  `get_reserve_list(Env) -> Vec<Address>`,
  `backstop_epoch(Env) -> BackstopEpochState`,
  `open_backstop_epoch(Env, u64) -> ()`,
  `close_backstop_credit(Env, u64, bool) -> Vec<(Address, i128)>`,
  `claim_backstop_credit(Env, Address, Address, i128) -> i128`, and
  `deferred_backstop_credit(Env, Address, Address) -> i128`; an active/funding relationship can
  exist only after the `B.requires.pool <= P.interface_version` edge has passed. Withdrawal and
  later credit claim use that previously validated local edge and make no registry call.
- Constructor takes `registry` instead of `pool_factory`.
- Stable views `registry()`, `interface_version()`, and `storage_version()` support on-chain
  allow-list/release checks.
  `upgrade(hash)` requires the stored registry as immediate invoker and then calls
  `update_current_contract_wasm`; it never calls the registry back. The registry's typed
  `execute(UpgradeBackstop)` path owns proposal, seal, expected-metadata, and manifest checks.

### Backstop interface the pool depends on

An allow-listed backstop must implement exactly what the pool calls today:
`pool_data(Env, Address) -> PoolBackstopData`, with all six exact fields declared above
(`tokens`, `shares`, `q4w_pct`, `blnd`, `usdc`, `token_spot_price`), `backstop_token()`,
`donate(from, pool, amount)`, `draw(pool, amount, to)`, `gulp_emissions(pool)`. The pool's
status machine interprets `pool_data` through the BLND⁴·USDC threshold
(`status.rs:129-148`), so a backstop with a different collateral model
can be allow-listed only once the threshold semantics are generalized — that is a pool
change and a separate ADR, not a registry entry. The registry does not verify the interface;
the shared interface version and real-Wasm integration suite support it. Human review still
checks economic behavior; it is not the ABI mechanism.

The registry-facing interface additionally exposes `exit_delay() -> u64`, which returns the
immutable seconds a share remains queued before withdrawal (17 days for the baseline). Admission
records it and binds the security-delay inequality; a same-address upgrade returning a different
value is incompatible even if its numeric interface version is higher.

### Maintenance and sealing

`CurrentPoolImplementation` is mutable only through `SetPoolImplementation`, so fixed and
upgraded pool versions can coexist during the maintenance year. `UpgradeRegistry`,
`UpgradeBackstop`, and `UpgradePool` all carry expected Wasm/interface/storage versions and use
the same proposal lifecycle; a pool upgrade additionally requires its pool admin in the
execution authorization tree. Registry/backstop upgrades are protocol-governance actions. Every
same-address upgrade enforces
`replacement.interface_version >= expected.interface_version` after first requiring `expected`
to equal current metadata. It must preserve every previously served flat export and the behavior
of existing outbound calls; new cross-contract capabilities use new symbols and the directed
compatibility checks above. Once an address has served v2, a production-v1-only blob is not rollback
even if it is the former Wasm. A release or recovery artifact that cannot preserve the current cumulative
interface is a new-address migration, not an upgrade.

Every release records old/new Wasm hashes, interface/storage versions, optimized sizes, test
results, code-entry TTLs, and a schema-compatible recovery hash whose interface version is at
least the current one. The replacement Wasm is uploaded and its hash verified before proposal. A
new hash and the recovery hash are each exercised first as disposable canaries with the real
registry/backstop/pool call graph, the oldest still-supported client, and every new extension
client. The canary must also complete registry-triggered pool/backstop upgrades and
registry-triggered/local backstop touches under the pinned current host with an acyclic call graph. Timelock
observation is not a substitute for that canary: an incompatible Wasm may remove the very
entrypoint needed to roll back.

`update_current_contract_wasm` changes only the executable after its successful invocation; it
does not migrate storage or validate ABI compatibility
([SDK 27.0.6](https://docs.rs/soroban-sdk/27.0.6/soroban_sdk/deploy/struct.Deployer.html#method.update_current_contract_wasm)).
New code must understand the old schema and fail closed into an explicit, resumable migration
mode before ordinary state changes. Recovery is allowed only with Wasm that is live or restored,
retains the current cumulative interface, passes every retained client, and can interpret every
post-migration state. An irreversible schema migration needs its own deliberate ADR and cannot
advertise old-hash rollback.

`Seal(scope)` is one-way and component-scoped, with `RegistryCode` as the durability root and the
first seal that may execute. Until it is sealed, proposing or executing each `PoolCode(pool)`,
`BackstopCode(backstop)`, `FuturePoolImplementation`, or `BackstopAdmission` seal fails exactly as
specified above. Once the root flag is written and its action/terminal events commit, the registry
executable can no longer change; only then can its registry-owned subordinate flags honestly be
called irreversible. Each accepted scope changes only unsealed→sealed and has no unseal path. At
**execution time**, including for an older capability proposal, each scope blocks only its named
capability. `BackstopAdmission` blocks first enable/re-enable but not disable;
`FuturePoolImplementation` blocks rotation but not deployment of the already pinned implementation.
This distinguishes immutability promised to existing users from code offered to future opt-in pools
and avoids the old asymmetry where pool rotation closed while new backstop code could still be
admitted.

Normal configuration, risk-reducing pause/contain/disable, terminal pool retirement, cooperative
admin transfer, delayed loss recovery, and recovery-config rotation remain available after code
sealing. None can reopen a sealed scope. The final handoff discloses that recovery can restore
configuration authority but cannot restore code authority. Before sealing `RegistryCode`, complete
the cumulative manifest and every intended policy dimension, soak compatible recovery artifacts on
Testnet, replay the final address inventory, finish all registry upgrades/extensions, and have the
human governance owner select the subordinate scopes. If final governance is a contract account,
its real `__check_auth` invocation trees are rehearsed before handoff and before the root seal;
classic-multisig genesis does not pretend to be that later decision. The approximate one-year
maintenance period is an operational target, not an automatic timestamp trigger.

### Deployment (greenfield)

1. Land known greenfield layout changes, including the mandatory Accepted
   protocol-fee/fungible-supplier-accounting and issuance/emissions dispositions, plus any
   permissioning decision designated as a genesis requirement, before the first mainnet build;
   upgradeability is not a reason to ship an
   intentionally incomplete genesis. The genesis release manifest names only existing required
   Accepted ADRs, plus each implementation commit, Wasm hash, interface/storage version, and
   verification report; a missing required decision blocks deployment without inventing an ADR
   number or status.
2. Build shared interfaces, registry, backstop, and pool; record real-Wasm cross-call tests and
   optimized sizes. Deploy disposable canaries and exercise deploy, pending attachment,
   liquidation, upgrade, cumulative-interface recovery, and TTL touch/restore.
3. Deploy the registry through its exact constructor in maintenance mode with the reviewed
   `RegistryImplementation`, `CurrentPoolImplementation`, `BlndId`, valid `MinConfig`,
   bounded `TimelockDelay`, exit-length `SecurityDelay`, bounded `RecoveryDelay`, ordinary admin
   multisig, and a distinct cold recovery-authority multisig. A distinct source builds the exact
   envelope; both authorities sign their recorded Soroban authorization payloads through the
   verified multisig auth-assembly component, and the source signs only after enforcing simulation
   succeeds. Read back every initialized scalar, decode the
   unique genesis event, and prove the absent/default state before deploying the backstop with that
   registry.
4. Propose/execute `EnableBackstop`, then deploy pools with `Some` or `None`. The production
   artifacts implement the cumulative manifest's Accepted fungible-supplier accounting rather than
   this ADR's disposable rate-only carry profile, plus its Accepted issuance/emissions disposition.
   Positive emissions require its claim/share-epoch integration evidence, while a disposition that
   retains
   the exact disabled profile deploys no legacy emitter, drop/backfill, reward, or entitlement state
   as migration debt. Absence of that disposition never defaults production to either policy.
5. Run a keeper/indexer that decodes and replays the complete cumulative-manifest-frozen registry,
   pool, and backstop event ABIs, calls `touch_pool` and native per-user footprint renewal
   before the half-max threshold, monitors live-until ledgers, and has a tested restore-footprint
   runbook. At each reconciled tip it checks every reserve identity and flags any negative observed-
   cash delta; replays every `cash_alloc` against the pre-allocation supply, canonical route, and
   epoch state; derives `T`, event-accounted `A`, and nonnegative `Z = T - A` for the configured LP
   token, and alerts on `T < A` or any change in `Z`. It also rejects a replay in which an incident-
   exit proposal was created while onboarding was unpaused or its pool was not `Contained`.
6. At the end of maintenance, complete the separately reviewed governance handoff, final inventory,
   retained-client/recovery canaries, and replay reconciliation. Propose and execute
   `Seal(RegistryCode)` first; verify its flag, exact action/terminal events, current code hash, and
   rejection of registry upgrades. Only then propose and execute the selected subordinate
   `Seal(scope)` actions under the now-fixed registry and verify each flag/capability. Never seal
   automatically or infer a global seal from elapsed time.

### Explicitly out of scope, own ADRs

Protocol-fee hooks (future protocol-fee ADR); BLND issuance from the registry and the emissions
schedule (also re-specifies `claim`/`gulp` and enables detach/re-route); permissioned pools via
Predicate/custom hooks; adoption of legacy pools, if ever; and final governance membership after
the maintenance phase. Ordinary and recovery signer composition is an operational deployment
gate here, while the long-term governance handoff remains its own decision. Each links back here.
Every future storage migration remains its own deliberate ADR even though the upgrade mechanism
is defined here.

## Alternatives

Scored against drivers (1 no forced backstop, 2 safe steady-state/cutover liquidation, 3
controlled stabilization fixes without a liquidation-path dependency, followed by a credible
seal).

- **A. Pool-canonical `Option`, registry membership/policy, funded pending attachment, and
  scoped sealable maintenance with exit-length delay, narrow veto, and delayed loss recovery
  (chosen).** 1 ✔ 2 ✔ 3 ✔. It
  adds a registry, shared interfaces, proposal lifecycle, and explicit maintenance/recovery
  controls, but each exists for a current product or one-year operating requirement. Current
  routing remains one value in the pool; the recovery authority cannot exercise ordinary policy
  or stop its own delayed replacement.
- **A′. Duplicate the route in registry and pool, atomically write both.** 1 ✔ 2 partial 3
  partial. It saves one cold registry→pool getter but creates two durable truths whose TTLs can
  diverge. The pool hot path and positive-share backstop shortcut deliberately avoid the reads
  that would renew the registry copy. Rejected: the extra state weakens, rather than strengthens,
  the invariant.
- **A″. Freeze status 4, attach an empty backstop, then fund it.** 1 ✔ 2 ✘ 3 ✔. Status 4 does
  not block liquidation creation/fill, so residual debt can cross the cutover into the empty
  backstop. Rejected on driver 2.
- **B. Allow attachment only in setup status 6.** 1 ✔ 2 ✔ 3 partial. This is the smallest safe
  implementation and avoids pending state, but an active uninsured pool can never add coverage.
  Viable fallback if live attachment is removed as a product requirement; rejected here because
  the requirement is explicit.
- **C. Registry as current-route source on every read.** 1 ✔ 2 partial 3 ✘. User liquidation
  fills and auction creation would depend on a registry call and its TTL. Rejected on driver 3.
- **D. Deployment-time-only `Option` plus registry policy; immutable contracts after genesis.**
  1 ✔ 2 partial (safe steady state by refusing live attachment) 3 partial. This is the strongest
  minimal design: no pending state, local relationship lifecycle, code-upgrade entrypoints, interface generations,
  storage migrations, or upgrade keeper/runbook; smaller Wasm, fewer TTL duties, narrower audit
  scope, and no upgrade-bricking authority. It loses live `None -> Some` coverage and turns each
  audited maintenance fix into a pool migration that fragments liquidity. Rejected for the stated
  one-year operating phase, not because its end-state security is weaker.
- **E. Permanent unsealable protocol-admin upgrades.** 1 ✔ 2 ✔ 3 ✔ during maintenance, but
  violates Principle 5 in the intended end state. Rejected in favor of the one-way seal.
- **F. Optional backstop in the pool while retaining `pool-factory`.** 1 ✔ 2 partial 3 ✘: no
  allow-list, `MinConfig`, pending funding coordinator, pool-implementation rotation, or
  fee-policy hook.
  The factory would become the registry piecemeal. Rejected.
- **G. Put all loss recovery inside the admin wallet.** 1 ✔ 2 ✔ 3 partial. This removes
  `RecoveryAuthority`, `RecoveryDelay`, and `RecoverAdmin` from registry Wasm and can be strong if
  a separately governed wallet has immutable, audited recovery. It makes registry availability
  depend on an otherwise out-of-scope wallet design, gives registry indexers no typed recovery
  window, and leaves a wallet upgrade or signer-policy drift outside the registry's proposal
  evidence. Viable for final governance; rejected for the greenfield maintenance protocol.
- **H. Make admin threshold loss irrecoverable.** 1 ✔ 2 ✔ 3 ✘. This is the smallest contract and
  avoids a second authority, but one lost threshold permanently removes configuration and audited
  repairs during the explicit maintenance year. Rejected on driver 3 rather than left implicit.
- **I. Keep one 48-hour delay and rely on the ordinary admin to cancel a bad upgrade.** 1 ✔ 2
  partial 3 ✔. This is smaller and faster operationally, but a compromised admin is the proposer
  and depositors need 17 days to complete q4w. Rejected because it protects against mistakes, not
  the stated hostile-admin case.
- **J. Let the recovery authority replace the admin immediately or veto its own rotation.** 1 ✔ 2
  partial 3 partial. It can stop compromise faster, but turns the guardian into a second protocol
  admin or lets a lost/hostile guardian entrench itself indefinitely. Rejected in favor of
  liveness-only pause, bounded veto, candidate-authenticated delayed loss recovery, and a
  non-vetoable exit-length rotation.
- **K. Close reserve credit into a scaled per-asset ring index with per-user checkpoints and deferred
  underlying claims (chosen share-epoch subdecision).** 1 ✔ 2 ✔ 3 ✔. It preserves the
  120%-priced interest auction until a redemption closes the then-current credit, freezes ownership
  at close without enumerating holders, checkpoints later deposits after every allocated index
  entitlement, lets
  worthless LP-token shares burn, and separates the legacy LP-token withdrawal from bounded
  multi-asset ownership and payout. It adds a
  bounded 30-asset sync to deposits/withdrawals, modular index/checkpoint arithmetic, scaled
  claim/actor-neutral-carry state, TTL duties, events, and a later claim call. A remainder is kept
  as global dust and folded into later index division rather than paid to the close actor; this
  avoids a repeat-close farming surface. The unavoidable unassigned residual crossing a share
  membership boundary is globally less than half a base unit per asset, remains conserved, and is
  explicitly accepted in place of holder enumeration. Separately, a full exit forfeits that
  account's allocated fraction below one base unit per asset; aggregate evented forfeitures can
  become whole units for remaining holders. This is accepted to avoid permanently stranded claims
  spread across zero-share accounts. Those costs are justified because a mutable
  whole-unit bucket lets later depositors capture material old value. The half-ring proof makes
  index wrap total for the protocol lifetime rather than turning historical turnover into a future
  exit failure.
- **L. Transfer all current reserve credit on every withdrawal and pay it pro rata immediately.**
  1 ✔ 2 partial 3 partial. The `V >= L` invariant makes current credit cash-backed, so this is
  economically viable and avoids persistent deferred claims. Eliminating those claims, however,
  makes each share withdrawal compute and transfer a basket of up to 30 reserve assets, couples the
  legacy LP-token exit to every reserve-token call and worst-case resource bound, and needs a new
  wire for recipient/basket results. Rejected in favor of an independently callable bounded claim
  that preserves the existing withdrawal ABI and keeps a reserve-token failure from redefining the
  share-exit contract.
- **M. Pay the entire credit basket only to the final redeemer, or require the interest auction to
  fill before any exit.** 1 ✔ 2 ✘ 3 partial. Final-only payment lets withdrawal order transfer the
  old cohort's gains to the last account; auction-first preserves the premium but a sub-threshold
  or simply unfilled permissionless auction can block q4w indefinitely. Rejected in
  favor of explicit deferred entitlement and separate realization.
- **N. Block every deposit after credit first closes until the share epoch drains.** 1 ✔ 2 ✔ 3
  partial. This is smaller than index/checkpoint accounting and prevents new-depositor capture, but
  a single matured withdrawal can stop recapitalization for the remaining 17-day cohorts and an
  account that never exits can keep admission closed indefinitely. Rejected because preserving
  ownership should not turn an ordinary exit into a backstop funding freeze.
- **O. Credit each division remainder to the holder that triggers the close.** 1 partial 2 ✘ 3 ✔.
  It makes each individual close conserve exactly without carrying dust, but makes caller identity
  an economic input. With `M = 2^128`, `S = (M + 2) / 3`, and `C = 1`, each close has
  `delta = 2` and initial remainder `S - 2`; the same equal holder can withdraw and replace one
  matured share repeatedly and collect more than `M` of caller-only remainder after four closes.
  Rejected because aggregate conservation does not cure trigger-dependent value extraction.
- **P. Preserve every zero-share user's fractional scaled claim indefinitely.** 1 ✔ 2 ✔ 3 partial.
  It never reallocates an allocated fraction, but underlying tokens are integer-valued: fractions
  below `M` across many departed accounts can sum to whole deferred units that no account can claim.
  Avoiding that deadweight needs a new transferable/mergeable-claim market or permanent user-key
  maintenance after exit. Rejected in favor of the evented per-full-exit forfeiture below one base
  unit per asset; a future aggregation mechanism would be a separate funds-flow/interface decision.
- **Q. Leave positive-supply `b_rate` floor residual in the token balance, or round the rate up.**
  The first option adds no state but lets every later `gulp` rediscover the same unit; the second can
  create represented supplier claims above physical assets. Both violate exact conservation.
  Rejected in favor of one bounded per-reserve carry, the minimum state that makes the existing
  floor policy idempotent without rounding in the protocol's disfavour.
- **R. Permit `V < L` and haircut current backstop credit before applying supplier default loss.**
  1 neutral 2 partial 3 partial. Making unclosed backstop credit junior can preserve more supplier
  value in an overextended reserve, but changes the baseline rule that realized default is borne by
  suppliers, adds a loss-priority/event state machine, and still cannot consume deferred
  former-cohort credit or unowned surplus without violating ownership. Rejected in favor of
  `V >= L`, capped non-cash accrual credit, and exact aggregate-liability default loss. That smaller
  rule proves current/deferred credit cash backing and preserves baseline loss priority without a
  claimant waterfall.
- **S. Retain the flat emission exports but disable every positive path until the issuance ADR
  owns it (chosen issuance-boundary subdecision).** 1 ✔ 2 ✔ 3 ✔. This keeps cumulative interface
  compatibility and zero-result client behavior without importing BLND issuance, reward economics,
  or a second share-mint lifecycle into ADR 0002. It makes an Accepted issuance decision a hard
  gate for positive state and forces that decision to integrate any share mint with lifecycle and
  share epochs. The cost is that ADR 0002 alone cannot launch rewards; that is deliberate greenfield
  sequencing rather than a migration limitation.
- **T. Preserve the vendored positive backstop `claim` in ADR 0002 and define how it reopens a
  drained share epoch.** 1 ✔ 2 partial 3 partial. It could keep legacy rewards usable before the
  issuance redesign, but it makes this registry/optional-routing ADR decide BLND-to-LP conversion,
  multi-pool claim atomicity, claim eligibility in `Contained`/`Retired`, checkpoint/dust ordering,
  and another source of share mints. Templar plans no legacy deployment and already requires the
  issuance decision before enabling emissions. Rejected as avoidable greenfield migration debt and
  a violation of the stated decision boundary.
- **U. Leave `Vec<Request>` length network-limited and publish only a measured resource frontier.**
  This preserves every baseline batch and automatically benefits from future network-limit growth,
  but the largest supported vector changes with action mix, Wasm, host settings, and repeated
  same-asset work. It leaves no deterministic pre-effect failure or stable compatibility promise.
  Rejected in favor of the one-line `2 * MAX_RESERVES = 60` guard plus actual-Wasm evidence at 60;
  a future reviewed upgrade may raise, but never silently lower, that bound.
- **Different insurance assets, tranches, or off-chain attestation.** Orthogonal to this
  decision. Each changes the threshold/economic interface and may become an allow-listed
  implementation under a separate deliberate ADR; none removes the need for optional routing
  and safe cutover.

## Consequences

Easier: institutional pools without BLND exposure; risk tier is derivable on-chain
(`backstop_of(pool) == None` means uninsured); a live pool can pre-fund coverage without an
outage or mid-funding risk handoff; the route has one source of truth; follow-up fee/issuance
work has a versioned policy and release boundary; callers and implementers share interfaces;
known-good fixes can reach live contracts during the maintenance phase.

Harder / risks:

- **Uninsured bad debt is socialized to suppliers immediately.** Owner: pool admin.
  Trigger: liquidation exhausts collateral in a `None` pool. Detection:
  `defaulted_debt` event, `b_rate` drop. Prevention: conservative LTV/oracle policy (pool
  admin's responsibility); front-ends must label `None` and `Pending` pools distinctly.
  Recovery: none on-chain; this is the uninsured product.
- **Registry governance is a temporary upgrade and policy trust root.** Owner: registry
  multisig. Trigger: malicious or mistaken proposal. Prevention: typed action, one live proposal
  per address-scoped target, 18-day minimum for code/seal/resume/rotation actions, independent
  veto, immediate liveness-only pause/containment, pool-admin co-authorization, canary/recovery
  release evidence, and root-first one-way sealing: no subordinate seal is advertised while
  registry code can still replace its enforcing logic. Detection: complete proposal/seal events,
  replayed root-before-subordinate order, and hash/version manifest. Recovery: cancel or veto
  before execution, let users complete the 17-day exit, or
  propose a recorded schema-compatible artifact
  that retains the current cumulative interface and whose code entry is live or restored; an
  incompatible former Wasm is not rollback, so canary success is a release gate.
- **The recovery authority is a second, narrower trust root.** Owner: governance operations.
  Trigger: ordinary admin threshold loss, recovery-key compromise, or correlated signer loss.
  Prevention: distinct signer sets, 7-day recovery delay, candidate authorization, one live
  `AdminRecovery` target, ordinary-admin cancellation, bounded veto, and a non-vetoable
  exit-length recovery-config rotation. Its pause/contain powers only reduce new risk intake.
  Detection: exact proposal/veto/pause/recovery events plus signer-health drills. Recovery: cancel
  a mistaken recovery while the admin remains live, rotate a lost/hostile guardian after the old
  security delay, or let the intended candidate accept after genuine admin loss. Loss of both
  thresholds is deliberately unrecoverable; a hostile admin is contained, not falsely claimed to
  be forcibly recoverable.
- **Pool-admin loss or compromise strands pool-local control.** Owner: each pool's governance.
  Trigger: threshold loss or hostile signer quorum. Prevention: independent threshold custody,
  backups/rotation, rehearsed containment and co-auth, and registry policy bounds; the registry has
  no hidden super-admin. Detection: signer-health and unauthorized-config/status monitoring.
  Recovery: use the documented wallet/account recovery if one exists, otherwise preserve exits and
  terminally drain to a new pool. A missing rehearsal blocks deployment.
- **Multi-party Soroban authorization can sign divergent or stale payloads.** Owner: release
  engineer and authority signers. Trigger: constructor/proposal envelope mutation, stale nonce or
  signer set, CLI signer loss, or restart without private E2E authority state. Prevention: one
  immutable exact-XDR bundle, detached payload-hash signatures, live medium-threshold verification,
  enforcing re-simulation, separate source signing, and write-ahead journaling before raw RPC send.
  Detection: bundle/envelope hash mismatch, underweight/invalid signature, changed account ledger,
  or restart fingerprint mismatch. Recovery: discard the entire unsigned/unsubmitted attempt and
  build a new bounded envelope; never transplant signatures or generate replacement authority keys
  for an already deployed test registry.
- **Pending capital covers the pool only after finalization.** Owner: pool admin and candidate
  depositors. Trigger: pool deteriorates while the backstop is pending. Prevention: UI/indexer
  labels, unchanged `None` accounting, fresh health/threshold checks, second pool-admin auth, and
  raw-tuple/generation retirement that cannot terminalize a reversible pause. Detection: pool
  liquidation/default events during pending and raw/normalized relationship reconciliation.
  Recovery: resume after a temporary pause/containment or cancel immediately and
  let depositors withdraw from the inactive balance, or knowingly finalize once funded. An
  auction created before finalization but filled afterwards belongs to the funded-backstop regime.
- **Backstop share rounding, exhausted or unsolicited LP tokens, and historical reserve credit can
  transfer value or trap exit.** Owners: pool and backstop implementations. Trigger: donation before first deposit,
  zero-share mint, a bad-debt draw leaving `tokens == 0 < shares`, historical reserve-local credit,
  a zero-amount outsider attempting closure, a close actor receiving division remainder, material
  post-close deposit capture, lifetime index turnover, malformed reserve traversal, premature
  credit closure, or a reserve balance-sheet/backing deficit.
  Prevention: positive matured-holder proof before closure; exact epoch state; no accounted LP-token
  inflow at zero shares; floor-rounded pro-rata mint; the positive-share exact-zero-token burn
  exception; pre-burn closure into deferred obligations; a half-ring-bounded modular
  index/checkpoint; caller-neutral carry normalized after membership changes; complete stable
  reserve-list comparison; canonical positive-cash allocation before every supplier-membership
  quote/mint/burn and before cold closure or realization; request-order projected cash that adds
  only scheduled-but-unreflected prefix flows and the logical flash outflow, counts synchronous
  interest-auction lots exactly once, and excludes the current membership flow;
  `cash + L = V + C + D + U` plus `V >= L`; exact physical/accounted LP reconciliation
  `T = A + Z` with nonnegative, unspendable derived `Z`; and a separately callable, fully backed
  authenticated claim. Lifecycle changes never reset price or claims. Detection:
  share/token/q4w/epoch reconciliation; alerts on `T < A`, changes in derived `Z`, or a named-pool
  balance/share quote that changes after only a direct LP-token transfer; per-asset
  `deferred * 2^128 == outstanding == dust + stored + implicit` equality; deposit-between-withdrawal
  permutations including index wrap, the repeated one-share caller-farming vector, and the
  one-large-holder/one-share full-exit accumulation vector; equal-history holder comparisons;
  zero/non-holder call-count assertions; alerts on malformed event order, a
  modular distance at or above `2^255`, dust at or above current positive supply, a claim with no
  matching obligation, or a
  positive claim that does not pay its exact whole-unit amount; first/later/final membership and
  before/after-initialization interest-auction permutations; exact 60/61 request-cap boundaries;
  and worst-case 60-request, 30-distinct-reserve submit plus maximum-reserve deposit/withdraw/claim
  resource simulation. Recovery: share withdrawal performs no
  reserve-token payout; a backing deficit or transfer/balance-delta mismatch rolls the claim back without clearing
  entitlement and is treated as invariant breach, not temporary illiquidity. Malformed closure or
  realization likewise rolls back without clearing, then requires a reviewed compatible pool/
  backstop repair. If maximum-reserve closure or claim cannot fit live transaction limits,
  revise this decision before genesis rather than truncate the basket or trap exit. Unsolicited
  positive reserve cash follows the one pre-allocation-state rule: zero supply receives
  `UnownedSurplus`, a positive-supply open current epoch receives current `C`, and every other
  positive-supply state receives aggregate supplier `V`. Supplier membership first allocates from
  physical cash plus only unreflected prefix deltas, advances an initialized projection for a later
  synchronous lot, then applies only the current exact `tokens_in - tokens_out`; close
  then moves all resulting `C` to the
  current cohort's `D`, while an old creditor's realization never changes the recipient to itself.
  `cash_alloc` replay and the paired `gulp`/cold-path permutations detect any call-order drift.
  Unsolicited LP tokens remain derived `Z` and confer no spend authority. A future
  disposition of `Z` requires a separate deliberate funds-flow decision.
  Tradeoff: the first matured redemption closes all then-current credit for the cohort and gives up
  any unrealized 20% auction premium; q4w makes that exit observable in advance, but governance
  cannot block it merely to preserve speculative upside. An unassigned residual below half a base
  unit per asset may cross a mint/burn membership boundary because exact assignment would require
  holder enumeration; it remains conserved and actor-neutral. Additionally, a full exit forfeits
  only that holder's allocated fraction below one base unit per asset. Those evented forfeitures can
  accumulate into whole units for the remaining cohort; preserving them instead would strand
  integer deferred value across zero-share claims without another aggregation interface.
- **Reserve rounding residuals are explicit state, not rediscovered cash.** Owner: pool
  implementation and protocol governance. Trigger: positive-supply value is smaller than one
  aggregate `b_rate` quantum, membership changes while such value is unrepresented, or unsolicited/
  rebasing cash arrives with `b_supply == 0`. Prevention: include `SupplierRoundingCarry` exactly
  once in aggregate supplier value, derive every post-mint/burn `(r1,K1)` from exact target
  `V1 = V0 + tokens_in - tokens_out`, bound positive-supply `K1` by
  `ceil((S1 - 1) / 10^12)`, and move the entire final-burn `V1` into `UnownedSurplus`; zero-supply
  cash is allocated from physical cash plus only scheduled-but-unreflected prefix deltas before the
  first mint, is quarantined directly, and neither bucket can become backstop credit. A synchronous
  interest-auction lot is either already present in that observation or advances an earlier
  projection, never both. Detection:
  `supplier_carry`/`surplus_quarantine`/`cash_alloc` events, the exact balance identity, the
  preserved legacy zero-`gulp` event, the one-unit/30,000,000-share regression, and the exact
  mint/burn/final-burn vectors. Recovery: an
  equation, bound, overflow, missing-key, or negative observed-cash failure rolls back for a
  compatible repair; a positive observed delta is allocated once by current state before cold
  closure or realization and cannot become a retirement/seal denial of service. Opening and
  funded activation proceed with valid carry/quarantine state intact. This ADR adds no spend path,
  and later redistribution requires a separate funds-flow decision.
- **One-way attachment can strand a pool on a defective current backstop.** Owners: pool admin
  for pool status and protocol governance for backstop eligibility/code. Trigger: a finalized
  backstop later fails health, retained-client, accounting, liveness, or runtime monitoring while
  `get_backstop() == Some(b)`. Prevention: compatible real-Wasm canary plus funding/health checks
  before finalization, and no automatic finalization. Detection: threshold/q4w alerts,
  donate/draw/deposit failures, invariant reconciliation, code-hash drift, or retained-client
  failure for the current route. Recovery in this ADR is bounded: disable `b` for new
  deployments/pending deposits, contain current-route inflows, freeze the affected pool, and use a
  compatible exit-delayed same-address repair when safe. If code is sealed or cannot execute the
  containment interface, the changed pool implementation's unconditional status-4 branch runs
  before any backstop read, so the pool admin can still freeze and users can drain whichever pool/
  backstop exit entrypoints remain callable, but the registry cannot stop active-route deposits or
  move the old pool to `Contained`/`Retired`; deploy a new pool and disclose the old address as
  frozen-but-not-terminal. Moving or removing the route requires the separately specified
  re-routing decision; this ADR provides no emergency detach or forged lifecycle state, and front
  ends must not imply one.
- **Registry membership, pending state, relationship, pool reserve carry/surplus, and per-user
  backstop state still have TTL.**
  Owner: protocol operations.
  Trigger: keeper failure long enough for archival. Prevention: extend toward network max on
  writes/reads, extend pool-balance and relationship separately on every relevant access,
  `touch_pool`, event-derived user/carry/surplus inventory, native footprint renewal,
  live-until monitoring, and tested keeper coverage ahead of the half-max threshold. Detection:
  alert before the touch threshold for every pool/user key class. Recovery: restore the complete
  operation footprint; do not reinterpret archived membership/activation or default an archived
  balance/carry/surplus key. An Accepted issuance decision owns the corresponding duties for any
  emission keys it introduces; the disabled profile creates none.
- **Pending `deposit`, `update_pool`, attachment, fee collection, and upgrades are cold paths**
  that deliberately depend on registry liveness. Current-route deposit, `donate`, and `draw` use
  the fail-closed local relationship capability and add no registry call to ordinary funding or
  bad-debt auction fill.
- **Pool, backstop, and registry Wasm grow.** Owner: each upstream PR author. Trigger: optional
  routing, shared clients, proposal/upgrade code. Prevention: reuse pure helpers/shared types,
  no duplicate route state, and measure each PR. Detection: before/after optimized bytes plus
  `twiggy` attribution for a material jump, together with the active network's decoded
  `contractMaxSizeBytes`. Recovery: narrow/delete code or split the PR; every optimized artifact
  must satisfy `size <= min(131072, active_network.contractMaxSizeBytes)` before upload/deploy.
- **The shared interface crate becomes a compatibility boundary.** Owner: protocol maintainers.
  Trigger: an implementation changes/removes a flat export, changes its wire type, or makes an
  existing operation depend on a newer counterparty. Prevention: implementations and callers
  compile against the same cumulative-manifest-frozen traits, later traits contain unique additive
  exports, and
  same-address upgrades retain old exports/outbound behavior, never lower interface version, and
  enforce both directions of every active dependency edge. Detection: compile/spec failure,
  retained-client real-Wasm canary failure, directed-edge inventory mismatch, or post-upgrade
  call-graph alert. Recovery: reject the candidate, retain compatible current code, use a
  cumulative-interface recovery artifact, or use a separately decided new-address migration; a
  Rust module name or production-v1-only former Wasm is never treated as recovery.

Pre-mortem (deliberate mode):

1. *Silent socialized loss.* A `None` pool with aggressive LTV takes bad debt; suppliers
   discover it via a dropped `b_rate` days later. Warning sign: `defaulted_debt` events with
   no preceding bad-debt auction. Mitigation: indexers alert on `defaulted_debt` where
   `backstop_of == None`; recovery is admin freeze + supplier withdrawal.
2. *Risk changes during pending attachment.* Depositors fund a candidate, then a large borrower
   becomes insolvent before finalization, or a temporary pause is mistaken for permanent
   cancellation. Warning sign: liquidation/default events and falling oracle health while
   `PendingAttach` exists, or `Retired(g)` while the raw matching tuple/generation remains.
   Mitigation: pending never changes loss ownership; finalization needs fresh threshold checks and
   second admin auth; retirement uses only raw tuple/permanent generation mismatch. Recovery:
   resume the pool around the intact `Funding` relationship after rechecking all six directed
   edges, while keeping normalized admission hidden until a separate global `ResumeOnboarding` if
   that pause remains; or cancel and then withdraw; or explicitly finalize knowing the now-current
   pool state.
3. *A maintenance call re-enters the registry.* Registry `execute` calls a pool/backstop which
   calls a registry view, creating a host-version-sensitive cycle or observing half-complete
   governance state. Warning sign: the real-Wasm upgrade/touch canary shows a nested registry frame
   or host rejection. Mitigation: registry owns policy, callees authenticate only
   the immediate stored registry, and local touch performs no external call. Recovery: leave the
   old code/state in place and propose a corrected acyclic candidate.
4. *Bad upgrade bricks a funds contract.* A hash passes unit tests but removes a flat cumulative
   production-v1 export or has an ABI/storage mismatch. Warning sign: oldest-client canary, spec
   diff, migration assertion,
   post-upgrade health call, or rollback smoke fails. Mitigation: no proposal without cumulative
   interface monotonicity/directed-edge checks, real-Wasm canary, version/storage manifest, size
   report, live code-entry checks, and a cumulative-interface recovery artifact. Recovery: cancel
   before execution or use a timelocked recovery only if it retains every served interface and
   understands post-migration storage; otherwise incident response and a new-address migration. A
   recorded hash or `interfaces::v2` module is not recovery.
5. *Keeper misses pool or user entries.* A quiet pool's membership or a cancelled candidate's
   user balance/credit state archives; pending deposit, dequeue, withdrawal, or credit claim fails. Warning
   sign: a tracked pool/user key falls below the live-until threshold. Mitigation: max-TTL
   extension, deploy/deposit-event inventories, native footprint maintenance, and two independent
   keeper alerts. Recovery: simulate the intended operation, restore every archived key in its
   footprint, verify current/pending routing or local residual balance as applicable, then retry
   and renew. Any enabled issuance decision adds its emission keys and claim footprint to the same
   keeper contract.
6. *Governance proposal survives an operator change or conflicts with another action.* Warning
   sign: proposal epoch differs or a target already has a live id. Mitigation: target exclusion,
   id-owned target locks, cancellation, epoch invalidation, bounded checked times, and
   permissionless cleanup. Recovery: compare-and-clean the stale proposal and create a new typed
   proposal; no timestamp-only "expiry" or cleanup of an old id may delete a newer id's lock.
7. *The ordinary admin is compromised and proposes hostile code while users are locked.* Warning
   sign: unexpected high-risk proposal or signer-health failure. Mitigation: its delay is at least
   18 days, the independent recovery authority can pause onboarding/contain inflows and veto, and
   q4w matures after 17 days. Recovery: veto and rotate the ordinary admin cooperatively if
   possible; otherwise users exit and affected pools drain and are replaced. On-chain retirement
   occurs only when the current backstop can complete containment; an uncallable sealed backstop is
   disclosed as frozen-but-not-terminal. `RecoverAdmin` is not claimed to defeat a hostile
   incumbent that can cancel it. A contained uninsured pool can return to `Live` only through the
   same delayed, vetoable, pool-admin-authorized resume action and without a backstop call.
8. *The ordinary admin is lost, or the cold recovery threshold is compromised.* Warning sign:
   signer-health rehearsal fails or an unexpected `RecoverAdmin`/veto appears. Mitigation:
   distinct authorities, candidate auth, target exclusion, role revalidation, epoch invalidation,
   and non-vetoable exit-length guardian rotation. Recovery: after genuine admin loss the intended
   candidate executes; while the admin remains live it rotates a lost/hostile guardian. Both
   “propose C then rotate recovery to C” and the reverse must fail role equality. If both
   authorities are unavailable, configuration and maintenance halt; no bypass exists.
9. *The event-derived inventory silently diverges from registry state.* A lifecycle event is
   omitted, duplicated, reordered, or cannot be decoded after its storage record is deleted.
   Warning sign: genesis-to-tip replay disagrees with any metadata, route, proposal, raw target
   owner, epoch, or seal view. Mitigation: cumulative-manifest-frozen topic/data shapes, pre-action
   terminal
   classification, post-write emission order, non-normalizing `target_owner`, raw-XDR decoder
   corruption fixtures, and replay comparison before every release. Recovery: stop proposals and
   keeper mutation, repair the index from raw ledger history under a separately reviewed additive
   event/version change, and do not infer missing state from silence or silently repair an orphan
   lock.
10. *A Wasm passes the 128 KiB project budget but the target network rejects it.* Warning sign: the
   decoded live `contractMaxSizeBytes` is lower than 131072 or upload simulation reports a size
   error. The 2026-08-29 Mainnet/Testnet value was 131072, but it is not assumed durable.
   Mitigation: record both limits and gate every artifact on their minimum before
   upload. Recovery: attribute and reduce/split the artifact, then rebuild and rerun all evidence;
   changing the fixture or target-network limit to make the build pass is not a protocol repair.
11. *Two authorities sign different constructor or recovery payloads, or a restart loses a signer.*
    Warning sign: payload/envelope hashes differ, a returned key is not in the recorded account,
    signature weight is below the live medium threshold, or private/public fingerprints diverge.
    Mitigation: immutable bound signing material, weighted distinct signatures, enforcing
    simulation, durable restricted test keys, and distinct source/admin/recovery identities.
    Recovery: abandon the attempt before broadcast and reassemble from a fresh simulation; if a
    deployed disposable registry has lost both usable authority states, redeploy rather than
    claiming recovery evidence.
12. *An exhausted share epoch traps exit or assigns material old credit to a later depositor.*
    Warning sign:
    `tokens == 0 < shares`, a zero/non-holder call reaches a pool or closure, a positive close is
    missing one canonical reserve, Alice/Bob own the close but a later Carol deposit changes their
    allocated credit, a repeated one-share close actor accumulates division remainders, repeated
    turnover traps a withdrawal at index wrap, two equal-history holders differ, a full exit loses
    one or more units for one account or reallocates more than evented forfeiture plus carry, share
    burn performs a reserve-token payout, an interest-auction fill donates
    closed credit into a new epoch, `deferred * 2^128 != outstanding`, or a maximum-reserve
    deposit/close/claim exceeds transaction resources. Mitigation: positive matured-holder proof
    before any closure; exact-zero-token burns; complete pre-burn closure; cohort-frozen modular
    index/checkpoint accrual with the half-ring proof; actor-neutral carry and post-burn
    normalization; per-account full-exit forfeiture strictly below one unit with exact
    `credit_dust.added` observability;
    deferred, non-share claims; the reserve identity plus `V >= L`; and a separate user-bounded,
    fully backed realization call.
    The regression fixes `S = M/2 - 1`, A=`S - 1`, B=`1`,
    `C0 = (3S - 1)/4`, then `C1 = 1`: A may forfeit exactly `M - 2`, and B's resulting whole-unit
    gain is accepted only when it reconciles to that evented residue plus prior carry.
    Recovery: share withdrawal never attempts the reserve-token payout; a backing deficit or
    malformed closure/reconciliation rolls back without clearing and requires a compatible repair.
    Normal index wrap never rolls back. Never reset an index before exact epoch closure and
    never pay old claims from new-epoch share accounting. The close carry alone stays below half a
    base unit; aggregate full-exit forfeitures may become whole units for remaining holders, but
    every shift must reconcile exactly to prior carry plus evented `added < M`. Any actor-directed
    close remainder, single-account forfeiture at or above `M`, or larger unexplained shift is
    invariant failure.
13. *Reserve rounding dust or unsolicited cash is double counted or captured across a supply
    boundary.* Warning sign:
    with `R = 10^12`, `b_supply = 30,000,000`, `b_rate = R`, and one new underlying unit, the rate
    becomes `1,000,000,033,333` but represented claims do not increase; a second `gulp` sees the
    same unit, carry exceeds `ceil((S - 1) / R)`, or a zero-supply/final-burn residual increases a
    later supplier or backstop claim; or `transfer -> first supply` assigns cash differently from
    `transfer -> gulp -> first supply`. Mitigation: preserve
    `V = P + SupplierRoundingCarry` with the exact checked fixed-supply equation; allocate physical
    cash plus only scheduled-but-unreflected prefix deltas before every quote/mint/burn; count a
    synchronous interest-auction lot in the observation or later projection update exactly once;
    derive post-membership value from
    `V1 = V0 + tokens_in - tokens_out`; event only committed carry changes; and move the entire
    no-owner `V1` into `UnownedSurplus` exactly once before an epoch can open.
    Recovery: an equation/bound/missing-key mismatch rolls back for a compatible repair; restore the
    exact carry/surplus footprint and retry. Redistribution remains unavailable until a separate
    deliberate ADR names a recipient and conservation proof.
14. *Interest allocation or default creates under-backed credit.* Warning sign: `V < L`,
    `cash + L != V + C + D + U`, a positive deferred claim cannot pay in full, or a default delta
    differs from the aggregate represented-liability reduction. Mitigation: cap non-cash accrual
    credit at `V0 - L0`, give the remainder of `delta_L` to suppliers, and apply exactly
    `A = -(L0 - L1)` on default; cash-backed `gulp` credit remains a separately identified path.
    Recovery: roll back the malformed accrual/default/claim without netting current or deferred
    credit, repair with a compatible reviewed artifact, and replay the balance identity before
    resuming the reserve. A current-credit-first haircut is not an implicit recovery path.
15. *A legacy positive emission claim slips into an ADR-0002-only build.* After a final drain it
    converts BLND to LP, mints the first new shares, and leaves the pool credit epoch closed; under
    containment it instead may strand a recorded entitlement. Warning sign: a nonzero emission
    balance, reward-zone entry, stored emitter/drop/backfill key, or emission-call/hook delta that
    transfers LP, emits `deposit`, returns a positive claim, or changes shares while no Accepted
    issuance disposition authorizes it. Mitigation: the exact emissions-disabled profile,
    production-manifest gating, and negative actual-Wasm delta tests around ordinary deposit/drain
    behavior. Recovery: reject the artifact or roll back the transaction; never infer an
    epoch-opening rule. Enabling positive emissions requires the separate Accepted issuance
    decision and its own lifecycle/share-epoch evidence.
16. *A subordinate seal is advertised while registry code is still mutable.* A later registry
    replacement ignores the registry-owned flag and reopens pool/backstop upgrades, future-pool
    rotation, or backstop admission. Warning sign: a subordinate `proposal/create` or
    `registry/seal` event occurs before the completed `RegistryCode` seal action/terminal pair, or
    replay finds `is_sealed(subordinate) == true` while `is_sealed(RegistryCode) == false`.
    Mitigation: the exact `InvalidState` precondition at both propose and execute, candidate-Wasm
    adversarial tests, root-first ceremony, and raw-XDR replay rejection. Recovery: before the root
    seal, cancel invalid proposals and install a conforming reviewed registry; do not describe any
    subordinate flag as final until the root-first history is verified. A candidate that fails this
    check is never used for the irreversible root seal.
17. *A resume or retirement proposal is prepared before the incident it supposedly waits out.*
    Warning sign: its `proposal/create` event precedes the corresponding pause/`Contained` event or
    is observed while replay says the target is healthy. Mitigation: proposal-time incident-state
    checks plus the one canonical `Onboarding`/`Pool(pool)` target and execution-time revalidation. Recovery:
    reject or permissionlessly clean the invalid proposal; start a fresh `SecurityDelay` only after
    the incident state exists. Never credit healthy-state elapsed time toward recovery or
    terminalization.
18. *An unsolicited reserve-token transfer blocks a deferred-credit close or claim after the public
    `gulp` path is unavailable.* Warning sign: physical cash exceeds the exact reserve identity but a
    retired or sealed pool returns `InternalError` before a fully backed close/realization. Mitigation:
    run the canonical one-balance positive-cash allocation before both operations and exercise it
    after containment, retirement, and sealing. Recovery: retry after a compatible repair only when
    a deficit or malformed accounting—not positive dust—caused the failure; never relabel missing
    backing as surplus.
19. *Permissionless call order chooses who owns unsolicited reserve cash.* Warning sign:
    `transfer -> gulp -> close` and `transfer -> close`, the corresponding realization pair, or
    `transfer -> gulp -> first/later/final membership` and the direct membership ordering produce
    different `V/C/D/U`, creditor entitlements, or `cash_alloc` recipients from the same pre-state.
    Mitigation: derive the recipient only from pre-allocation `b_supply`, the canonical current
    route, and its epoch state; allocate physical cash plus only unreflected prefix deltas before
    every membership quote/mint/burn and every balance-observing action; update an initialized
    projection once for each later synchronous interest-auction lot; close the entire resulting `C`;
    and never use
    the realizing creditor as an input. Unit/property, Kani, actual-Wasm, Testnet, and replay
    evidence cover supplier boundaries and the public/cold orderings. Full realization-state
    equality is asserted only for an already-current reserve; a stale reserve instead requires the
    same `Q` allocation and old-creditor payout while allowing public `gulp`'s accrual effects.
    Status-gated public `gulp`, especially in `Retired`, is tested as exact rejection before any
    reserve/balance effect, followed separately by successful private cold allocation. Recovery:
    stop the affected pool, preserve all buckets and claims, install only a
    compatible reviewed repair while code remains unsealed, and replay from the last agreeing tip;
    never compensate one cohort by silently debiting another.
20. *A direct LP-token transfer is mistaken for a named pool's capital or breaks aggregate replay.*
    Warning sign: physical configured-token balance `T` changes without a matching pool deposit,
    donation, draw, loss, or withdrawal event; `Z = T - A` becomes positive, or worse, `T < A`.
    Mitigation: derive nonnegative `Z` at every reconciled tip, exclude it from every share quote and
    spend path, and test every accounted transition against `T = A + Z`. Recovery: leave positive
    `Z` untouched pending a separate deliberate disposition; treat `T < A` as corruption and stop
    affected mutations rather than haircut a named pool.
21. *An unbounded submit vector exhausts resources or a maintenance release silently reduces batch
    capacity.* Warning sign: repeated same-asset requests grow CPU/events past the 30-reserve cache
    fixture, a formerly valid 60-request batch fails after upgrade, or a 61-request call reaches a
    pool/user load, flash liability, balance read, cross-contract call, mutation, or event.
    Mitigation: one shared `MAX_SUBMIT_REQUESTS = 2 * MAX_RESERVES = 60` pre-effect guard across
    `submit`, `submit_with_allowance`, and `flash_loan`; boundary tests at 60/61; and actual-Wasm
    resource evidence for both 60 repeated same-asset requests and a healthy 30-reserve footprint
    containing 30 collateral and 30 liability positions under captured live limits. Recovery:
    reject the release or revise this ADR before
    genesis; after the cumulative freeze, raise only through a reviewed compatible upgrade and
    never lower the bound for an existing address.

Invariants preserved or changed (`AGENTS.md`):

- Pool preserved: `b_rate`/`d_rate` do not decrease except the explicit bad-debt/`gulp` paths;
  a healthy `submit` cannot make its position immediately liquidatable; supplied underlying is
  at least borrowed underlying per reserve, frozen exactly as `V = P + K >= L`; backstop-take/
  interest math cannot underflow or round in the protocol's disfavour. Non-cash accrual caps new
  current credit at `V0 - L0`; default applies the exact aggregate-liability decrease to suppliers.
  Credit closure reclassifies exactly the nonnegative cash-backed current `C` as equal cash-backed
  deferred `D` without a transfer; realization decreases cash and `D` by the same caller-bounded
  amount and never returns a liquidity-limited partial result. Every reserve preserves
  `cash + L = V + C + D + U`, includes bounded nonnegative `SupplierRoundingCarry` and nonnegative
  `UnownedSurplus` exactly once, and therefore has `cash >= C + D + U`. Every fixed-supply delta
  preserves `P + K` exactly; every membership change preserves
  `V1 = V0 + tokens_in - tokens_out` after allocating all projected pre-membership `Q`; the entire
  no-owner final `V1` becomes `UnownedSurplus`. During submit construction the same identity uses
  request-order `projected_cash = observed physical cash + scheduled-but-unreflected prefix cash`;
  a synchronous interest-auction lot before observation is already physical, while one afterwards
  advances the initialized projection once. Successful scheduled transfers realize the remaining
  modeled deltas, while any genuinely later unscheduled cash remains `Q` for the next observation.
  Every positive observed `Q` is assigned by the same pre-allocation-state function: zero supply to
  `U`, positive supply with an open canonical current epoch to `C`, and every other positive-supply
  state to `V`. Closing subsequently moves all resulting `C` to that epoch's `D`; a realization's
  creditor is never an allocation input. Therefore inserting or omitting a prior permissionless
  `gulp` cannot change the economic post-state of a later close. It also cannot change a later
  realization's `Q` recipient/amount or old-creditor payout; full realization-state equality
  additionally requires the reserve to be already current because the cold claim deliberately does
  not accrue. A public `gulp` rejected by the Borrow/status gate performs no reserve load, balance
  read, mutation, or event and is not used as a cold-path equivalence premise.
  An unchanged `gulp` is state-idempotent and preserves exactly one legacy `gulp(asset, 0)` event.
  No zero-supply surplus becomes supplier or backstop value, and
  `flash_loan` leaves every reserve at least as solvent as before. Every submit form processes at
  most 60 requests; an oversized vector fails before any economic effect.
- Backstop preserved: per-pool balances and q4w shares reconcile, while physical LP-token balance
  is exactly accounted pool tokens plus nonnegative derived unowned excess `Z`; queue→dequeue/
  withdraw cannot be skipped; `draw`/`donate` move funds only for the
  named pool with the existing address auth; and every pool's capital remains segregated. The
  disabled profile has zero emission allocation and claim state. Any Accepted issuance decision
  must separately re-prove that its emissions cannot be over-claimed and that its settlement does
  not violate these share/credit invariants. Queued shares remain gain/loss-bearing until redemption;
  exact-zero LP tokens do not make worthless matured shares unburnable; and every closed credit
  amount enters outstanding at close, with the maximal integral per-share amount assigned through
  the scaled index to the then-current holders, including q4w, while later deposits checkpoint after
  it. Division remainder is actor-neutral dust, is normalized after membership changes, and remains
  below the current positive share supply. At close the triggering holder receives only the same
  per-share allocation as every equal pre-close holder and no actor-specific claim mutation;
  equal-history holders remain equal. Globally unassigned carry below half a base unit may cross a
  membership boundary. Separately, a full exit moves that account's allocated residue strictly
  below one base unit per asset to dust; evented residues can accumulate into whole units for the
  remaining cohort, but no account loses a whole unit and no shift exceeds carry plus the sum of
  those residues. Fractional consolidation obeys that declared per-exit bound and exact final-dust
  reconciliation. A fresh share epoch begins only from exact
  shares/tokens/q4w and reset index/dust; outstanding former-holder claims remain segregated, no
  accounted value enters at zero shares, and lifecycle changes never reset share price or erase
  claims.
- Deployment preserved: a pool address is deterministic for registry + admin-namespaced salt and
  deploys only the `CurrentPoolImplementation` pinned for that transaction. Changing the
  implementation is now an explicit timelocked action rather than an impossible mutation.
- Changed: "every pool has a backstop" becomes "every pool has a canonical
  `Option<backstop>` written only under registry authority".
- New: for a live registered pool, `registry.backstop_of(pool)` returns the pool's canonical
  value rather than an independently stored copy; `PendingAttach(pool)` never affects status,
  liquidation, bad debt, or interest; the disabled profile has no emissions, and any later
  issuance decision must preserve pending non-ownership. `None -> Some(b)` succeeds only while `b` is
  enabled, compatible, at the pending generation, and above the pool's health threshold.
- New: the backstop accepts shares locally for an `Active(generation)` current route or after a
  fresh normalized `pending_admission` check for a matching `Funding(generation)` route.
  `Contained` and `Retired` reject deposits; containment preserves named-pool settlement, and all
  lifecycle states preserve queue/dequeue/withdraw access. The relationship never stores another
  backstop address.
- New: for every pool/asset, the pool's deferred obligation multiplied by `2^128` equals the
  backstop's scaled outstanding credit, decomposed exactly into current-epoch dust, stored claims,
  and implicit index/checkpoint entitlements. Closing current credit increases both sides equally
  without a token transfer: with prior dust `D`, `delta * S + next_dust = C * 2^128 + D`, and only
  `C * 2^128` is added to outstanding. Realization and payout reduce both sides equally. The public
  index and checkpoints are ring elements modulo `2^256`; for every positive holder the least modular
  distance times its shares is below `2^255` and bounded by outstanding, so historical wrap cannot
  block an otherwise valid transition. While an account remains a holder, every allocated index
  entitlement is fixed when credit closes, cannot be diluted by a later deposit, and survives
  lifecycle changes, sealing, and a rolled-back claim attempt. On full exit the account keeps all
  whole units and deliberately forfeits only its `< M` fractional residue; whole retained claims survive
  epoch rollover. The explicitly unassigned carry remains subject to its separate half-unit
  membership-crossing rule.
- New: governance proposals are typed, target-exclusive, time-bounded, cancellable, and bound
  to an admin epoch. A target lock can be renewed or removed only by the proposal id it stores,
  and the raw `target_owner` view makes missing, orphaned, or wrong-id ownership observable.
  Every high-risk action except guardian rotation is recovery-vetoable and waits at least one day
  longer than q4w. `RegistryCode` seals first; only under that fixed executable may each subordinate
  `SealScope` change unsealed→sealed and durably block its capability without reopening another
  scope.
- New: onboarding pause and the `Live -> Contained -> {Live, Retired}` pool lifecycle only reduce
  or deliberately restore new risk intake. They never rewrite the canonical route, accelerate code,
  block repayment/liquidation or user exits, or grant the registry pool-admin authority. Resuming a
  `None` pool is delayed, vetoable, pool-admin-authorized, and backstop-free; resuming `Some`
  additionally restores the current healthy relationship.
- New: genesis is a one-shot, jointly authorized constructor with validated policy/three delays and
  explicit, queryable defaults plus one genesis event. Exact-XDR account authorization proves both
  classic-account medium thresholds on the immutable constructor invocation.
  `Admin != RecoveryAuthority` holds after every valid transition. The recovery
  authority can create/cancel only delayed admin recovery, veto the bounded high-risk set, and
  reduce liveness through pause/containment; it cannot govern or block its own rotation. The
  recovery candidate must authorize execution, and any admin or recovery-config change increments
  the epoch so neither a prior governance proposal nor an epoch-bound cooperative handoff survives
  the authority transition.
- New: a registry-originated callee never calls back into that active registry frame; local
  backstop touch is externally call-free. Every same-address Wasm replacement has a
  non-decreasing cumulative interface version, preserves all previously served flat exports and
  existing outbound behavior, and satisfies both directions of every active dependency edge.
- New: event-derived `(backstop, pool, user)` inventory plus native footprint operations keep
  separately keyed user shares/q4w/credit recoverable even after pending attachment is cancelled
  or a share epoch closes; pool-level touch covers per-pool balance, epoch, global credit state,
  and relationship but is not claimed to cover user state. An Accepted issuance decision extends
  both inventories for its own emission state. Registry lifecycle replay from
  raw XDR is total and ordered: each proposal has exactly one create and at most one terminal;
  absence from storage requires exactly one terminal, while an unterminated stored proposal must
  match its view, current validity, and raw target owner. Successful epoch-changing execution is
  still terminal `execute`; epoch-first `invalid` applies only to stale removal. The reconstructed
  keyed state must match both registry views and every discovered backstop's live relationship
  view, including an expected absent relationship.
- Upgrade obligation: every upgraded Wasm preserves these invariants, its declared storage
  version, and the cumulative interface contract or is rejected by its own ADR/release gate.
  Upgradeability is not evidence that a migration is safe.

Acceptance criteria:

1. A `None` pool leaves setup only through admin `set_status`, accepts exactly statuses `{0,2,4}`,
   and operates without a backstop call. Exhausted user debt defaults immediately to suppliers;
   the supplier loss is exactly the aggregate `L0 - L1` after liability removal and preserves
   `V >= L` without consuming current/deferred credit or surplus;
   interest/bad-debt auction creation fails because there is no backstop key; `gulp` preserves the
   Borrow/status gate, credits positive-supply suppliers through the exact rate/carry equation,
   returns zero current-backstop credit, quarantines zero-supply cash as `UnownedSurplus`, and never
   writes backstop credit. A positive call emits its bucket-specific event, if any, then exact
   supplier/unowned `cash_alloc`, then the legacy `gulp`. The exact reserve identity makes a repeat
   `gulp` without new cash state-idempotent: it emits no carry/quarantine/`cash_alloc` event but
   preserves exactly one legacy `gulp(asset, 0)` event. Liquidation execution makes no registry call.
2. A pending candidate is `Funding(generation)`: it can receive deposits only after a fresh
   matching normalized `pending_admission` check but cannot receive donations/draws, enter rewards,
   or alter pool loss ownership. Cancellation succeeds despite capital or candidate unavailability,
   immediately stops deposits, and preserves exits. Pause or containment makes normalized admission `None` but
   leaves a matching raw tuple in `Funding`; permissionless retirement fails, and resume revalidates
   its enabled generation and all six directed compatibility edges before restoring only the pool
   to `Live`, without a new begin, relationship mutation, or backstop call. Normalized admission
   remains `None` if global onboarding is still paused and becomes visible only after separate
   `ResumeOnboarding`. Cancellation/raw replacement or a disable/upgrade generation
   change permits exactly one `Funding -> Retired` reconciliation with the applicable local or
   cumulative-manifest relationship event. Finalization rechecks pause/lifecycle, generation,
   compatibility, current config and health, then commits exactly one canonical route plus one
   local `Active` relationship atomically. Active deposits remain registry-independent.
3. Immediate onboarding pause blocks only new deployment/attachment/pending intake, including a
   previously funded `Funding` deposit through the normalized `pending_admission` view. When the
   current backstop remains callable, pool containment blocks current-route deposits and any
   issuance-defined new reward admission while preserving repayment, liquidation, settlement and
   exits; resume uses the exit-length
   delay, pool-admin co-auth and recovery veto. Proposal creation while onboarding is unpaused, or
   while the target pool is not already `Contained`, fails `InvalidState` before id allocation,
   target ownership, storage, or event. A failed healthy-state attempt cannot age across a later
   pause/contain event: only a fresh incident-state proposal is allocated and receives `ready_at`, and execution
   still rechecks the incident state after the complete `SecurityDelay`. A contained `Some`
   relationship remains resumable
   after disable/re-enable or a same-address upgrade: its stored activation-cohort generation may be
   older, while execution revalidates current enabled metadata and atomically records the current
   generation. A contained `None` pool resumes through the same
   action with no backstop call: it accepts either no raw pending record or one exact current raw
   candidate whose enabled generation and all six directed registry↔pool↔candidate edges
   revalidate. If global onboarding remains paused, the pool is `Live` but attachment intake stays
   hidden until `ResumeOnboarding`; if already unpaused, intake is reachable immediately. Any other
   raw pending state fails. `Live -> Contained -> Retired`
   is enforced for the terminal branch; retirement
   is monotonic, keeps membership/route observable, atomically sets the pool's local `Retired`
   lifecycle, and prevents admin status/config revival. When the current backstop cannot execute
   containment and cannot be compatibly upgraded, tests instead prove containment rolls back, the
   changed `set_status(4)` executes with zero backstop calls, freeze/drain/replacement remains
   available where existing exit entrypoints work, and neither `Contained` nor `Retired` is falsely
   reported.
4. Backstop share tests cover a fresh exact-zero epoch, first mint 1:1, later floor pro-rata mint,
   zero-mint and zero-share accounted-inflow rejection, q4w gain/loss participation, and
   cancellation/containment/reactivation without price reset. A bad-debt draw can reach
   `tokens == 0 < shares`; every holder with a **positive** matured q4w amount can then burn a
   positive partial or final share amount for zero LP tokens, while `withdraw(..., 0)` and a
   rounding-to-zero result with positive tokens each fail 1006 before a pool call, closure,
   mutation, or event. A non-holder or holder without sufficient matured q4w fails before those
   effects under the exact queue errors. Exact final burn reaches zero and increments the epoch;
   outstanding allocated credit claims do not block a later 1:1 deposit. While the epoch is closed,
   ordinary accrual and any public `gulp` that passes its inherited status gate use zero take.
   At every reconciled tip the configured LP-token balance is exactly `T = A + Z`, where `A` is the
   event-replayed sum of named-pool accounted tokens and `Z = T - A >= 0` is derived unowned excess.
   A direct LP-token transfer changes only `Z`: every pool balance, share quote, withdrawal, donation,
   draw, deferred-credit entitlement, and event-replayed `A` remains unchanged. No path spends or
   assigns `Z`, and `T < A` fails as invariant corruption.
   In the disposable ADR-0002-only profile, positive-supply value is exactly
   `V = P + SupplierRoundingCarry`. Per-holder conversion, withdrawal, and health remain rate-based,
   while aggregate accrual, reconciliation,
   utilization, supply-cap, and total-supply semantics use `V`; nonzero carry is tested at exact
   utilization and cap boundaries. Every fixed-supply transition proves
   `P' + K' = P + K + A` and `K' <= ceil((S - 1) / 10^12)`. The exact
   `S = 30,000,000`, `r = 10^12`, `A = 1` regression stores
   `r' = 1,000,000,033,333`, `P' = P`, and `K' = 1`, emits one `supplier_carry`, then one supplier
   `cash_alloc`, then the legacy `gulp(asset, 0)`, and detects no new balance delta; a second
   unchanged call makes no mutation or new carry/quarantine/`cash_alloc` event but emits exactly one
   legacy `gulp(asset, 0)`.

   Every membership change first allocates `Q` from its request-order projected cash and
   pre-membership supply/route/epoch, then proves `V1 = V0 + tokens_in - tokens_out`, derives
   `(r1,K1)` from `(V1,S1)` when `S1 > 0`, and moves the entire nonnegative `V1` to
   `UnownedSurplus` when `S1 == 0`. The projection starts from one physical observation plus only
   scheduled-but-unreflected prefix cash: deferred supply/collateral/repay inflows, deferred
   withdraw/collateral-withdraw/borrow outflows, and the logical flash outflow. A synchronous
   interest-auction lot before that observation is already reflected and is not added again; one
   after projection initialization decrements the projection exactly once. The current membership
   flow remains excluded until after allocation and mint/burn. The projection uses one balance read
   per distinct membership asset and is never persisted. Thus direct cash
   before first, later, or final supply membership has the same recipient and economic result when
   a Borrow-enabled public `gulp` runs first or allocation occurs inside the membership request.
   When that public gate rejects, it has no effects; an independently allowed withdrawal still
   performs the in-request allocation before burning. Allocation
   bucket/carry events precede `cash_alloc`; membership carry/final-reclassification events follow
   it; and the baseline supply/withdraw event is last. This exposes the local final-holder residual
   and is not accepted as production
   fungible ownership; criterion 12 requires its replacement before genesis. Exact regressions are
   `S0=2,r0=1.5R,K0=0,burn=1,tokens_out=1` →
   `V1=2,r1=2R,K1=0`; `S0=1,r0=2R,K0=0,tokens_in=3,mint=1` →
   `V1=5,r1=2.5R,K1=0`; and a partial-value final burn whose whole residual is quarantined.
   Membership-flow rounding may create a new bounded `K1`; tests forbid capture by a later cohort,
   not every carry increase above `K0` or the disclosed last-holder forfeiture in this disposable
   profile. Zero-supply value enters exact nondecreasing `UnownedSurplus` directly. For each of
   `submit`, `submit_with_allowance`, and `flash_loan`, an exactly 60-request vector remains
   executable within the captured live resource limits, including both 60 repeated same-asset
   requests and a healthy 30-reserve vector containing 30 `SupplyCollateral` plus 30 `Borrow`
   requests. The latter matches `Positions::effective_count()`—collateral plus liabilities, with
   ordinary supply excluded—and the configured 60-position ceiling. A vector of 61 or
   more requests returns exact `BadRequest = 1200` after the inherited auth/self-address guards but
   before pool/user load, flash-liability creation, reserve or balance reads, cross-contract calls,
   mutation, or events.

   Every reserve also proves `cash + L = V + C + D + U` and `V >= L`. Non-cash accrual caps new
   current credit at `V0 - L0`, applies the rest of `delta_L` to suppliers, and distinguishes that
   path from cash-backed `gulp` credit. Default applies exactly `A = -(L0 - L1)`. Current/deferred
   credit and surplus remain untouched, and `cash >= C + D + U` follows after every transition.
   First-deposit/activation opening reconciles every reserve under those rules before minting,
   succeeds in the presence of quarantined dust, and proves no empty-interval value belongs to the
   new depositor. Pre-burn closure and deferred-credit realization perform the same private positive-
   cash allocation without the public `gulp` status gate. For `Q > 0`, zero supply receives `U`; a
   positive-supply open current epoch receives `C`; and every other positive-supply state receives
   `V`. Closure allocates while the epoch is open and then moves all resulting `C` to `D`.
   Realization uses the pool's current route/epoch, never its old creditor. Direct reserve-token
   transfers before or after containment, retirement, and code sealing therefore cannot block a
   fully backed close or claim or acquire an owner through call order. With unchanged intervening
   state, `transfer -> gulp -> close` equals `transfer -> close`. For an already-current reserve,
   `transfer -> gulp -> realization` also equals `transfer -> realization` apart from the expected
   public-`gulp` event. With pending accrual, both realization orderings retain the same `Q`
   recipient/amount and old-creditor payout, while the public-first ordering may additionally
   advance `L/V/C/last_time` and emit accrual/carry effects. In every lifecycle/status state whose
   Borrow gate rejects `gulp`, including `Retired`, rejection is exact
   `PoolError::InvalidPoolStatus = 1206` before reserve load, balance read, mutation, or event; the
   corresponding private cold
   close/realization still allocates successfully when its own preconditions hold. Each positive
   allocation has the exact `cash_alloc` recipient/address/epoch/
   amount/total and required order; an observed deficit fails atomically. The carry/surplus views,
   events, TTL restore, and physical-accounting identity remain exact, and no path in this ADR
   spends or redistributes either exclusion.

   Multi-user permutations use the unique append-only stable-order reserve universe of at most 30
   assets. With prior dust `D`, each close proves
   `delta * S + next_dust == C * 2^128 + D`, increases outstanding by exactly `C * 2^128`, updates
   the ring index modulo `2^256` before syncing the withdrawing holder, and gives no remainder to
   that actor; the `credit_index.remainder` event field equals post-close `next_dust`. After every
   non-final burn, dust is normalized against remaining supply `S'`; before every mint it is
   normalized against pre-mint supply and the depositor is synced before new shares receive the
   current checkpoint. Thus no new share receives an already allocated index entitlement while its
   prior owner remains invested. Conserved unassigned carry `< S / 2^128 < 1/2` base unit total per
   asset may cross a membership boundary. On a full exit, the authenticated account additionally
   keeps every whole claim unit and forfeits only its `stored_claim mod 2^128`; each evented
   forfeiture is below one base unit, although their aggregate can become whole units for remaining
   holders.

   Alice/Bob/Carol cases deposit before, between, and after withdrawals; queue/dequeue preserves the
   combined holder balance, equal holders with identical membership histories remain equal, and
   credit accrued after one holder's full exit belongs only to remaining shares. The adversarial
   vector fixes `M = 2^128`, `S = (M + 2) / 3`, and `C = 1`, starts Alice and Bob with equal shares,
   and has Alice trigger four closes while withdrawing and replacing one matured share after each.
   Starting from zero index/dust, the reference carry oracle advances the index by exactly `11` and
   leaves `dust = S - 8`; it yields the same entitlement for Alice and Bob after every close, never
   stores dust in Alice's claim, and proves Alice cannot accumulate the former caller-only `S - 2`
   remainder. A second vector fixes `S = M/2 - 1`, A=`S - 1`, B=`1`, closes
   `C0 = (3S - 1)/4` and then `C1 = 1`, and fully exits A. It proves pre-exit dust
   `(S + 3)/2`, evented `added = M - 2 < M`, combined normalization amount
   `M + (S - 1)/2`, B's permitted greater-than-one-unit aggregate gain, post-dust zero, and exact
   outstanding conservation. Every full exit moves less than `2^128` scaled units per asset to dust.
   Repeated closes, claims, positive withdrawals, and replacement deposits cross `U256::MAX` in
   both directions without error; every positive holder's least modular index distance remains
   below `2^255`, multiplies to no more than outstanding, and produces the same claim as an
   unbounded-integer oracle. Exact epoch closure proves divisibility, assigns reconciled whole dust
   deterministically, resets index/dust, and never requires outstanding former claims to be zero.
   An unfilled interest quote is invalidated, current reserve credit becomes deferred rather than a
   later donation, and a cancelled/stale funding candidate reaches exact zero without closure. A
   reserve disable preserves enumeration; shrink/reorder fails the stated zero-state migration gate.

   Share withdrawal performs no reserve-token payout; a separate authenticated claim collects the
   selected assets. Empty claim input returns empty after auth, and a zero-whole claim makes no
   pool call, write, or event. A positive whole-unit claim must receive exactly that amount and
   produce matching pool/backstop balance deltas and ordered events; a backing deficit, zero/partial
   pool result, or mismatch fails atomically with error 1 and leaves the entitlement unchanged.
   Duplicate, unknown, or more-than-30 assets
   fail `BadRequest` before mutation; result order matches input. Pool deferred/outstanding equality
   holds after every close/sync/dust-normalization/realization/payout, while LP-token reconciliation
   remains separate and exact. Negative credit, duplicate/omitted/reordered reserve output, missing
   reserve state, a modular distance outside the half-ring proof, dust at or above positive current supply after
   normalization, a full-exit `added >= M`, a post-burn normalization mismatch,
   checkpoint/claim/dust/outstanding overflow, a carry-equation mismatch,
   non-divisible final dust, balance-delta mismatch, or reconciliation corruption fails atomically
   with error 1 and clears nothing; historical ring wrap itself never fails. The ordinary
   withdrawal event remains exact baseline XDR and last. Withdrawal still blocks named-pool
   realized liabilities but not a merely permissionless auction or segregated deferred credit.
   Maximum-reserve deposit sync, close, withdrawal/final burn, and claim each fit the target
   network's simulated CPU, memory, ledger-read/write, footprint, event, and transaction-size
   limits; failure blocks genesis and returns this funds-flow decision for revision.
5. The ADR-0002-local public `check_config(Env,u32,u32,i128,bool)->()` succeeds with unit or panics
   `InvalidConfig`; its internal validator has a distinct name and consistent `Result` semantics.
   Disposable ADR-0002-only built specs match every local registry/pool/backstop signature and
   boundary type declared in this ADR; production built specs instead match the complete cumulative
   manifest after every mandatory amendment. Coverage includes all `submit` variants, `flash_loan`,
   auction/bad-debt, reserve/emission,
   relationship/reward, `SupplierRoundingCarry`, `UnownedSurplus`, and U256-scaled deferred-credit
   state/closure/claim behavior. Every observable failure decodes to the registry or inherited pool/
   backstop numeric discriminant specified by the applicable local baseline or cumulative manifest—
   especially retired
   `InvalidPoolStatus = 1206`—rather than an implementation-chosen replacement.
   Deploy and `update_pool` apply it, while deploy separately verifies the selected backstop. The
   registry stores no route copy; deterministic deployment, duplicate rejection, authenticated
   pool construction, atomic membership/initial activation, and one selected implementation all
   pass against actual Wasm. The retained flat emission exports and views also match the disabled
   profile exactly: the constructor rejects a non-empty drop list and stores no emitter/drop state;
   all five disabled mutators reject atomically; authenticated and structurally valid value paths
   return zero and publish the exact topics/data/order table above at raw XDR; views are exactly
   `None`/empty; every named balance-update hook creates no emission state or event; and no
   emission-originated token/share/deposit/epoch effect occurs beyond each ordinary action's
   independently required transfer, mint/burn, event, and epoch transition.
6. Genesis requires both distinct authorities, validates policy, metadata and all three delays,
   initializes pause/seals/epoch/id exactly, and emits/read-backs one matching genesis state. The
   non-source classic-multisig auth component rejects missing, duplicate, unknown, stale,
   underweight or divergent signatures and succeeds at the live medium threshold. Cooperative
   handoff and delayed recovery preserve role distinctness and epoch invalidation; loss and
   compromise drills exercise their different response paths.
7. Security actions cannot become ready before `Q4W_LOCK_TIME + 24 hours`. Recovery can veto each
   one except `SetRecoveryConfig`, cannot execute it, and cannot veto its own replacement.
   Security delay cannot decrease; backstop admission binds the recorded immutable `exit_delay`
   plus margin, and same-address upgrades cannot change it. Rotation waits the old security delay
   and requires the new authority. Pause/veto/rotation races,
   hostile-admin cancellation of `RecoverAdmin`, genuine admin loss, hostile guardian, and loss of
   both authorities all end in the documented state without a hidden authority path.
8. Address-scoped target exclusion prevents enable/disable/upgrade/per-address-code-seal races;
   the separate global admission seal is revalidated at enable execution. Expired or epoch-invalid
   proposals can be replaced, but cleanup/cancel/veto of old A cannot remove or renew
   B's lock. Raw `target_owner` exposes missing/orphan/wrong-id corruption. Create and exactly one
   terminal event (`execute|cancel|veto|expire|invalid`) reconstruct every proposal across all
   versioned action streams.
9. Pool/backstop upgrades and registry self-upgrade reject stale metadata, sealed scope, missing
   auth, unavailable code, ABI/storage incompatibility, and any failed directed edge. Registry
   self-upgrade proves metadata write + host code update + action event + proposal cleanup +
   terminal event are one rollback domain; its next invocation runs replacement code. Every
   retained client works against replacement and schema-compatible recovery artifacts, with no
   nested registry frame.
10. `RegistryCode` is the first executable seal. Before it commits, proposing or executing any
    subordinate seal returns `InvalidState` with no new write or event; a stored adversarial
    pre-root proposal also fails execution without cleanup. Its scoped-seal action event and
    `proposal/execute` precede every subordinate `proposal/create`. After it commits,
    `UpgradeRegistry` returns `Sealed`, each subordinate scope is monotonic and durably blocks only
    its capability, and risk-reducing disable/contain/retire, policy governance and authority
    recovery remain usable. A pre-root v2 governance/policy extension retains production-v1 XDR,
    shares global
    id/epoch and canonical target exclusion, cannot weaken the root-first rule or a delay, and
    replays with production v1; no extension can be installed after the root seal.
11. TTL tests renew every distinct live membership, pending, proposal/target, relationship,
    balance/share-epoch/global-credit/user-checkpoint/scaled-claim, pool deferred-credit,
    supplier-rounding-carry, unowned-surplus, instance and code entry; archival fails closed;
    restoration preserves proposal
    time/epoch and exact user shares/q4w/credit. Replay reconstructs lifecycle, pause,
    scopes, metadata, generations, routes, raw/normalized pending state, proposals and authorities;
    every discovered backstop/pool relationship, including absent state and pre-deploy activation
    order, equals the live backstop view. An Accepted issuance decision extends these exact TTL,
    restore, and replay gates for every emission key and entitlement it introduces.
12. The cumulative production-genesis manifest names only existing Accepted ADRs, includes both a
    protocol-fee/fungible-supplier-accounting decision that gives every holder a complete pro-rata
    claim over aggregate `V` and an issuance/emissions disposition that explicitly retains the
    disabled profile or defines every positive path, freezes the 60-request submit bound and its
    pre-effect `BadRequest` ordering, records pool-admin custody drills, and pins supported Protocol-27
    SDK/Rust/target plus live network/tool provenance. Absence of either mandatory decision blocks
    production genesis. Each optimized Wasm is freshly measured against
    `min(131072, live contractMaxSizeBytes)` before upload; Testnet being on a newer protocol is
    labeled compatibility coverage, not a silent SDK-major upgrade. Before both decisions exist,
    ADR-0002-only local/standalone/disposable-Testnet actual Wasm proves the explicitly incomplete
    rate-only holder profile and disabled emissions profile from
    construction through final drain: no emitter/drop/backfill/reward/positive-entitlement state,
    no positive emission claim, no emission-originated token transfer/share mint/`deposit`
    event/epoch open beyond the independently asserted ordinary lifecycle, and no path that can
    strand an entitlement.

Verification (report which levels ran; this ADR-only review can run lint/source/toolchain checks,
but functional, Kani, optimized candidate, standalone, and testnet evidence require the upstream
implementation and are release gates rather than claims made by this Proposed record):

1. **Unit and property evidence.** Build the optimized dependencies before testing, as required by
   the vendored workspace. Use the implementation's canonical commands rather than freezing a Make
   target here. Cover at minimum:

   - both `Some` and `None` pool status/action matrices, including setup exit, the preserved
     Borrow-gated `gulp`, positive-supply `SupplierRoundingCarry`, zero-supply `UnownedSurplus`,
     immediate default, auction rules, and a registry fixture that traps if liquidation takes an
     unintended registry dependency; the defective-backstop fixture also traps on every call while
     `set_status(4)` still succeeds;
   - exact public `check_config` success/failure behavior through constructor, deploy,
     `update_pool`, attachment finalization, and policy changes;
   - authenticated construction, deterministic deployment, atomic membership/initial activation,
     wrong-registry and direct-constructor rejection;
   - every `Funding/Active/Contained/Retired` transition; pause→failed retirement→resume and
     containment→failed retirement→resume with the raw tuple intact; cancel/raw replacement and
     disable/upgrade generation change→successful retirement; other pause/cancel/disable/generation
     races, pre-existing liquidation across finalization, and both `Contained(None) -> Live`
     branches without a backstop call—no pending record, and exact raw `(backstop,generation)` after
     enabled-generation/six-edge revalidation. Under a continuing global pause, this restores only
     pool `Live` while normalized admission stays `None`; a separate `ResumeOnboarding` exposes the
     unchanged `Funding` candidate. Missing/replaced/disabled/stale candidates reject;
     `Contained(Some) -> Active`, including contain-before-disable/re-enable, disable-before-contain,
     and same-address-upgrade-before/after-contain sequences whose old cohort generation is replaced
     only on successful current-generation resume; a stale generation alone never strands an
     existing canonical route. Disabled-profile no-accrual behavior, retirement, and failure of
     either cross-contract write have complete rollback;
   - the emissions-disabled profile from construction through final drain: a non-empty constructor
     drop list rejects, the ABI-reserved emitter/drop list are not stored, reward/emission views
     remain exactly `None`/empty, and `set_emissions_config`, `distribute`, `add_reward`,
     `remove_reward`, and `drop` reject `BadRequest` without a write or event. Pool/backstop
     `gulp_emissions` and emission `claim` preserve their exact auth and input rules before returning
     zero; raw event topics/data/order match all four exact rows above. Tests distinguish the pool
     claim's accepted empty/duplicate ids from invalid ids and valid backstop multi-pool claims from
     empty, duplicate, absent, and `Funding` relationships. Backstop deposit, queue, and dequeue
     hooks and each pool liability/collateral/supply add/remove helper create no emission key or
     event; public coverage includes every `submit` action, both submit variants, flash-loan
     liability creation, liquidation position transfers, and user/backstop bad-debt movement and
     default. Ordinary backstop withdrawal is tested for the same empty post-state but is not
     mislabeled as an inherited emission hook. Tests first assert the ordinary deposit,
     queue/dequeue, withdrawal/final-drain, containment, retirement, and resume deltas—including
     required LP-token transfers, share mint/burn, `deposit`, and epoch open/close—then compare the
     additional disabled-export/hook deltas around and after those actions. Those emission deltas
     contain no BLND/LP-token transfer, share mint, `deposit`, epoch-open, emission-key write, or
     emission event. A production manifest/build without both the Accepted
     protocol-fee/fungible-supplier-accounting and issuance dispositions is rejected; an
     ADR-0002-only development/canary fixture rejects every positive emission state and exposes its
     rate-only final-holder residual;
   - first/later/last depositor economics, donation-before-first-deposit rejection, zero-mint,
     q4w gain/loss participation, realized-liability withdrawal blocking, permissionless-auction
     non-blocking, and exhausted `tokens == 0 < shares` partial/final burns versus positive-token
     rounding error 1006; zero-amount, zero-q4w, unmatured-q4w, and unaffiliated callers against
     both positive-token and exact-zero-token pools, asserting the exact error and zero pool calls,
     closures, mutations, and events; plus interest accrual, rebasing surplus, explicit `gulp`, and
     unsolicited dust with both zero and positive supply across a closed empty epoch before atomic
     first-deposit/activation opening, pre-burn closure, and deferred-credit realization; repeat the
     latter two after containment, retirement, and code sealing and require the private allocation
     plus exact event order rather than a public `gulp`. In `Live` and `Contained` fixtures whose
     Borrow/status gate permits public `gulp`, compare `transfer -> gulp -> close` with
     `transfer -> close`; require the same complete economic post-state, with only the expected
     public event-order difference. With a current open epoch plus an old creditor, compare
     `transfer -> gulp -> realization` with `transfer -> realization` twice: an already-current
     reserve requires the same complete economic post-state, while a stale reserve requires only the
     same `Q` recipient/amount and old-creditor payout and separately asserts the public path's
     permitted `L/V/C/last_time` accrual delta and events. For every Borrow/status rejection,
     especially `Retired`, public `gulp` must return exact `InvalidPoolStatus = 1206` before reserve
     load, token-balance read, mutation, or event; then exercise successful private close/
     realization allocation independently. Cover all three `Q > 0` recipients, exact optional
     backstop/epoch fields and bucket-event -> `cash_alloc` -> containing-event order; require close
     to move newly allocated `C` into `D`, realization never to allocate to its caller, and a
     negative balance delta to fail closed. Zero maximum or zero obligation still returns without a
     balance read, write, or event;
   - positive- and negative-supplier-delta rate/carry equations and partition invariance at unchanged
     supply. Fix `R = 10^12`, `S = 30,000,000`, `r = R`, and `A = 1`: require
     `r' = 1,000,000,033,333`, unchanged `P`, `K' = 1`, one exact `supplier_carry` event, one
     supplier `cash_alloc`, public return `0`, and then the legacy `gulp(asset, 0)` event; a second
     unchanged call has no mutation or carry/quarantine/`cash_alloc` event but still one legacy
     zero-`gulp` event. Property cases enforce
     `P' + K' = P + K + A`, `0 <= K' <= ceil((S - 1) / R)`, no rate decrease for nonnegative `A`,
     event emission only for committed carry changes/final reclassification, exact event replay and
     TTL restoration, and atomic failure on target, bound, intermediate, or storage corruption;
   - request-order projected cash before every `Supply`, `SupplyCollateral`, `Withdraw`, and
     `WithdrawCollateral` quote/mint/burn. Instrument exactly one token-balance read per distinct
     membership asset. Track deferred supply/collateral/repay inflows, deferred withdraw/collateral-
     withdraw/borrow outflows, and the logical flash-loan outflow as scheduled-but-unreflected
     prefix cash. Prove that `FillInterestAuction -> membership` observes the synchronous lot once
     in physical cash, while `membership -> FillInterestAuction -> membership` advances an already
     initialized projection once; combine both orderings with prior repay/borrow flows. Exclude the
     current membership transfer until after its accounting, and reject checked projection or `Q`
     failure atomically. In Borrow-enabled fixtures, compare public-`gulp`-first
     and in-request allocation for unsolicited cash before first supply, later supply, partial
     withdraw, and final withdraw under each of the three recipient branches. In status-gated
     fixtures, require public rejection with no effects and then prove an independently allowed
     partial/final withdrawal performs the same canonical in-request allocation. Exercise repeated
     same-asset requests, plain and allowance transfer modes, prior borrow/repay flows, and the
     flash-loan prefix. Run the reflected/unreflected auction permutations through all three public
     submit forms.
     Require exact allocation bucket -> `cash_alloc` -> membership carry/final reclassification ->
     baseline request event order and the same post-state/recipient/amount wherever both orderings
     are executable;
   - the shared request-length guard for `submit`, `submit_with_allowance`, and `flash_loan`: exact
     length 60 succeeds for both a repeated same-asset sequence and a healthy 30-reserve sequence
     with 30 collateral plus 30 liability positions; lengths 61 and above return
     `BadRequest = 1200` after each inherited auth/
     self-address guard but before pool/user load, flash liability, reserve or balance reads,
     cross-contract calls, mutation, or events. Preserve this failure order under invalid-auth,
     self-address, and otherwise-valid oversized inputs;
   - membership target accounting with `V1 = V0 + tokens_in - tokens_out`, including the exact
     burn `S0=2,r0=1.5R,K0=0,burn=1,tokens_out=1 -> V1=2,r1=2R,K1=0`, mint
     `S0=1,r0=2R,K0=0,tokens_in=3,mint=1 -> V1=5,r1=2.5R,K1=0`, partial/non-final
     mint/burn permutations, and a final burn with `S1=0,V1>0` that moves all `V1` to
     `UnownedSurplus`. Assert bounded post-change carry, no capture by a later cohort, and the exact
     disclosed last-holder residual in this disposable profile; do not reject legitimate new carry
     created by membership-flow rounding. Verify rate-based per-holder
     conversion/health and `P+K`-based aggregate accrual, reconciliation, utilization, supply cap,
     and total-supply behavior, including exact nonzero-carry utilization/cap boundaries;
   - the per-reserve `cash + L = V + C + D + U` and `V >= L` invariants across non-cash interest,
     cash-backed `gulp`, mint/burn, repayment, borrow, withdrawal, and default. Interest cases cap
     new credit at `min(nominal_credit,V0-L0)` and apply `delta_L-credit` to suppliers; default cases
     use exact aggregate `L0-L1`, not a standalone per-position ceil amount; borrow/repay rounding
     uses the exact signed `delta_cash + delta_L - delta_C - delta_D - delta_U` remainder. Assert
     `cash >= C+D+U`, full positive deferred-claim backing, and no current/deferred/surplus haircut;
   - at least three-user deposit/withdrawal permutations over credit accrued before/between burns,
     including Alice/Bob followed by Carol after a close; exact `2^128` actor-neutral carry
     arithmetic with prior/post dust, post-burn and pre-mint normalization, the conserved globally
     unassigned `< 1/2`-base-unit carry at a membership boundary, and the exact
     `S = (2^128 + 2) / 3, C = 1` four-close one-share withdrawal/replacement attack that must keep
     equal-history Alice/Bob entitlements equal, assign no remainder to the actor, advance the
     initially-zero index by `11`, and leave `dust = S - 8`; plus the exact
     `S = M/2 - 1`, A=`S - 1`, B=`1`, `C0 = (3S - 1)/4`, `C1 = 1` full-exit vector that must
     event `added = M - 2`, normalize `M + (S - 1)/2` to B, leave zero dust, and preserve
     outstanding while proving A's loss remains below one unit;
     modulo-`2^256` addition/subtraction at zero and both sides of `U256::MAX`, repeated
     close/claim/positive-withdraw/replacement-deposit wrap sequences checked against an unbounded
     integer oracle and the `< 2^255` distance proof, sync-before-mint/burn, combined liquid+q4w
     holder balances, absence of actor-specific claim mutation, equal-pre-close-holder allocation,
     and unchanged-supply close-partition invariance,
     per-full-exit fractional bounds, final-dust divisibility/assignment, complete
     stable canonical reserve-vector comparison, old credit → exact-zero → new-deposit/auction
     isolation, disabled-reserve enumeration and rejected nonzero-state shrink/reorder, and
     funding-candidate exit without close;
   - fully backed deferred claims and lifecycle/epoch/seal survival; empty/all-zero/duplicate/
     unknown/over-limit claim inputs; exact user-selected asset subsets; positive whole-unit claims
     that pay in full; atomic backing-deficit, zero/partial pool-result, and balance-delta failures;
     negative/missing/duplicate/reordered reserve corruption, invalid half-ring distance and
     checkpoint/claim/dust/outstanding overflow (but not ordinary ring wrap), token-balance-delta
     mismatch, exact close/index/dust/realize/payout/baseline-withdraw event
     ordering, and `pool-deferred * 2^128 = outstanding = dust + stored + implicit`
     reconciliation, including maximum-reserve positive-credit deposit/withdraw/claim footprints;
     the largest valid effective-position submit footprint is exactly 60 requests covering all 30
     reserve assets with 30 collateral and 30 liability positions, remains healthy, records one
     balance read per distinct membership asset, and records the complete transaction resources for plain,
     allowance, and flash-prefixed execution. A separate exactly-60 repeated same-asset fixture
     proves request work is bounded independently of cache cardinality, and the corresponding
     61-request calls fail at the shared pre-effect guard;
     direct LP-token transfers prove `T = A + Z` with `Z > 0`, leave every named pool/share quote
     unchanged, and cannot be spent or assigned, while `T < A` fails as corruption;
   - all delay boundaries and checked overflow, canonical address-scoped target collisions,
     stale replacement ownership, cooperative handoff, genuine loss recovery, hostile admin,
     hostile guardian, veto/rotation races, pool-admin co-auth, and incident-state proposal clocks:
     pre-incident `ResumeOnboarding`, `ResumePoolInflows`, and `RetirePool` attempts return
     `InvalidState` before id/target/event allocation; advancing beyond `SecurityDelay` and then
     pausing/containing does not make such an attempt executable; only a fresh post-incident proposal
     becomes ready after its full window. Root-first sealing also proves that pre-root
     subordinate propose/execute return `InvalidState` without mutation or event, the root action
     and terminal precede subordinate creation, a post-root registry upgrade returns `Sealed`, and
     each subordinate capability is then blocked without disabling its stated risk-reducing paths;
   - complete registry self-upgrade order and rollback, retained production-v1 plus extension clients,
     schema-compatible recovery, intermediate migration fail-closed behavior, and prohibited
     registry re-entry; and
   - exact event XDR, action-before-terminal order, versioned replay, corruption fixtures, TTL
     renewal, archival/restore, and restart-safe idempotency properties.

2. **Bounded model checking.** Kani proofs use justified Soroban bounds and implementation-owned
   harness names. Prove: minimum/config validity equivalence; checked normal/security/recovery
   windows including monotonic `security_delay >= recorded exit_delay + 24h`; target-owner
   isolation under arbitrary stale operation order; incident-state proposal creation and execution
   under arbitrary pause/contain/resume/retire ordering, including the impossibility of crediting a
   healthy-state clock toward a later incident; admin/recovery/veto/rotation role separation;
   the root-first seal partial order, including pre-root subordinate `InvalidState`, post-root
   registry-upgrade unreachability, ordered root/subordinate events, and monotonic subordinate
   flags under the fixed registry;
   the route `None/Some` product with relationship `None/Funding/Active/Contained/Retired`
   reachability, including reversible `Contained(None)` with either no raw pending record or one
   exact current `(backstop,generation)`, all-six-edge revalidation, pool-local resume while global
   admission stays paused, later admission visibility only after `ResumeOnboarding`, rejection of
   every stale/mismatched resume candidate, and the raw-tuple/generation retirement predicate that
   rejects reversible normalized `None`;
   exact supplier rate/carry conservation and bound for signed deltas, unchanged-supply partition
   invariance, nondecreasing rate for nonnegative deltas; a bounded ordered-request model with
   separate scheduled-but-unreflected and synchronously reflected cash classes. Prove that lazy
   initialization from one physical observation plus only the unreflected prefix is equivalent to
   a projection initialized before the same request prefix; a synchronous interest-auction lot is
   observed or later-applied exactly once, deferred supply/collateral/repay/withdraw/borrow flows and
   the optional flash prefix are modeled before their physical transfers, and the current
   membership flow is excluded during `Q` then applied as exact `tokens_in-tokens_out`.
   First/later/final membership cannot change the recipient by inserting a prior allocation, and
   repeated requests for one asset never require another observation. Model request lengths around
   the shared bound and prove 60 reaches request construction while 61+ reaches no pool/user load,
   flash liability, reserve/balance access, external call, mutation, or event after the inherited
   auth/self-address guards;
   membership target `V1=V0+tokens_in-tokens_out`, positive-supply `(r1,K1)` derivation,
   final-burn whole-residual
   reclassification, the exact burn/mint/final-burn vectors, repeated-`gulp` state idempotence, and
   the `R=10^12,S=30,000,000,r=R,A=1` regression; the reserve identity
   `cash+L=V+C+D+U`, `V>=L`, non-cash credit cap, exact aggregate default loss, private positive-
   cash allocation before membership/open/close/realization, the `Q < 0`/`Q == 0`/three-recipient
   `Q > 0`
   partition, closure of all post-allocation `C` into `D`, rejection of a negative observed delta,
   and derived `cash>=C+D+U` through every bounded transition, including borrow/repay balance-sheet
   rounding. With unchanged route/epoch/lifecycle inputs, prove economic-state equivalence for
   allocation-before-close versus allocation-inside-close. For realization, prove full-state
   equivalence when `last_time == ledger_timestamp`; when accrual is pending, prove equal `Q`
   recipient/amount and creditor payout while permitting only the modeled public-accrual delta to
   `L/V/C/last_time`. Model every Borrow/status rejection before the reserve-load/balance-read
   transition and prove the private cold branch separately. The realizing creditor is absent from
   the recipient function;
   pool/backstop share-epoch agreement; the emissions-disabled state product from constructor input
   through each named balance-update hook and final drain/containment/retirement/resume, modeling
   ordinary lifecycle deltas separately and proving that no emitter/drop/reward/emission state is
   stored and no **emission-originated** delta can transfer a token, mint a share, emit `deposit`,
   open an epoch, or create an entitlement without the Accepted issuance decision;
   closed-epoch zero-take accounting, positive matured-holder preconditions before closure,
   zero-token burn reachability, and share-epoch conservation; exact actor-neutral carry
   conservation with prior dust, post-burn/pre-mint normalization, absence of actor-specific claim
   mutation, equal-pre-close-holder allocation, unchanged-supply close-partition equivalence, and
   the concrete
   `S = (2^128 + 2) / 3, C = 1` repeated one-share attack with final index delta `11` and
   `dust = S - 8`; the concrete `S = M/2 - 1`, A=`S - 1`, B=`1`,
   `C0 = (3S - 1)/4`, `C1 = 1` full-exit proof with `added = M - 2`, aggregate normalization
   above `M`, per-account loss below `M`, and exact conservation; modulo-`2^256` wrap equivalence to
   an unbounded index under
   `outstanding < 2^255`, including the uniqueness of every positive holder's least modular
   distance; sync-before-share-change, exclusion from already allocated index credit while the
   holder remains invested, the `< 1/2`-base-unit unassigned-carry bound, and the separate
   `< 1`-base-unit per-full-exit forfeiture and `< 3/2`-base-unit per-transition combined-bucket
   bounds with cumulative event reconciliation; final-dust
   divisibility/bounds; deferred-obligation/outstanding conservation and fully backed positive
   realization through arbitrary deposit/withdrawal/claim order; q4w gain/loss participation;
   aggregate-supplier/carry/unowned-surplus conservation for both zero and positive supply; and the
   transition algebra preserving `T = A + Z`, `Z >= 0`, with a direct transfer changing only `Z`
   and every accounted transfer changing `T` and `A` equally.
   Concrete Soroban auth and
   cross-contract invocation remain actual-Wasm tests, not model-checking claims. A timeout or
   unjustified assumption is reported as not proved.

3. **Current-toolchain artifact, ABI, and size evidence.** On the implementation commit, pin the
   official stable Protocol-27 SDK line, its declared Rust MSRV or newer reviewed pin,
   `wasm32v1-none`, CLI/Core fixture provenance, source/lock revisions, and use
   `stellar contract build`. Record baseline/candidate SHA-256, optimized bytes and delta for every
   contract. Compare built specs and generated clients against every exact cumulative production-v1
   wire declaration from the manifest;
   run the oldest retained client plus each extension client against actual optimized Wasm.
   Attribute material growth before changing logic. Immediately before upload, decode the target
   network's live `CONFIG_SETTING_CONTRACT_MAX_SIZE_BYTES` and require every artifact to satisfy
   both it and 131072. Mainnet Protocol 27 is the production authority; Protocol 28 Testnet is
   separately labeled until its stable official SDK matrix is adopted by review.

4. **Actual-Wasm standalone and public-testnet evidence.** Use an immutable, recorded fixture
   compatible with the intended target protocol; no mutable container tag, hard-coded Protocol-22
   limit, or ADR-mandated script/phase name. The evidence runner must:

   - deploy all disposable dependencies itself and exercise actual optimized registry/backstop/pool
     artifacts, both insurance modes, pending funding/finalization/cancellation, raw-versus-normalized
     retirement across pause/contain/resume and cancellation/generation staleness, `Some` resume
     across disable/re-enable and same-address upgrade with an older stored cohort generation, and
     both no-pending/exact-current-pending `None` resume branches plus stale-candidate rejection;
     the exact-pending branch proves all six edges, zero backstop calls/relationship events, pool
     `Live` with admission still hidden under global pause, and visibility only after separate
     `ResumeOnboarding`; before each incident, attempt its resume/retirement proposal, require
     `InvalidState` with unchanged next id, target view, storage, and events, advance beyond the
     nominal delay, trigger the incident, and prove only a newly created proposal can execute after
     its own full `SecurityDelay`;
     the complete emissions-disabled profile, including constructor rejection of a non-empty drop
     list, absence of stored emitter/drop/backfill state, exact empty views, all five rejected
     mutators, raw-XDR equality to all four authenticated/validated zero-result event rows, pool
     claim empty/duplicate/invalid-id inputs, valid and invalid backstop multi-pool inputs, and every
     actual backstop deposit/queue/dequeue and pool position-mutation hook through submit,
     flash-loan, liquidation, and bad-debt calls. The runner asserts ordinary deposit/drain transfer,
     share mint/burn, event, and epoch deltas independently, then proves disabled exports/hooks add
     no emission-originated token transfer, share mint, `deposit`, epoch-open, emission key, or
     emission event through final drain and containment/retirement/resume. A production
     manifest/build without both the Accepted protocol-fee/fungible-supplier-accounting and issuance
     dispositions rejects; an ADR-0002-only development/canary artifact permits only the exact
     disabled emissions profile plus the disclosed incomplete holder profile and rejects every
     positive emission state;
     rejected zero/non-holder withdrawal before any pool call,
     positive zero-token share burn, deferred credit closure/claim, closed-epoch zero-take
     accrual/opening with exact positive-supply supplier carry, quarantined zero-supply cash, and
     matching `cash_alloc` before the epoch-open event,
     the one-unit/30,000,000-share regression plus repeated-`gulp` state idempotence and exact legacy
     zero-event preservation; the burn `2/1.5R/1`, mint `1/2R/3`, and positive-residual final-burn
     membership vectors; nonzero-carry utilization/supply-cap boundaries; and the full
     `cash+L=V+C+D+U`, `V>=L`, capped-accrual, exact-default, and cash-backing checks. Direct
     reserve-token transfers exercise every `Q > 0` recipient and raw `cash_alloc` field/order.
     Before each of first/later/final plain, allowance, collateral, and flash-prefixed supplier
     membership in a Borrow-enabled fixture, compare public-`gulp`-first with in-request allocation.
     Execute `FillInterestAuction -> membership`, `membership -> FillInterestAuction -> membership`,
     and deferred repay/borrow plus interest-auction permutations through plain, allowance, and
     flash-prefixed paths. Assert that a pre-observation synchronous lot is already reflected, a
     post-initialization lot advances the projection exactly once, the current membership transfer
     remains excluded until after allocation, and there is one token-balance read per distinct
     membership asset. Require the exact bucket -> `cash_alloc` -> membership -> baseline event
     order. With positive supply and an open current epoch, compare
     `transfer -> gulp -> close` with `transfer -> close` in Borrow-enabled `Live` and `Contained`
     fixtures. While an old deferred creditor claims under a newer open epoch, compare
     `transfer -> gulp -> realization` with `transfer -> realization` for both current and stale
     reserves: current requires equal economic post-state; stale requires equal allocation
     recipient/amount and old-claim payment while recording the public-only accrual delta. In every
     status-gated fixture, especially `Retired`, require public `gulp` to reject before reserve load,
     balance read, mutation, or event, then require every independently allowed partial/final
     supplier withdrawal and private close/realization allocation to produce its canonical bucket,
     identity, and event order. Repeat applicable cold
     cases after containment, retirement, and pool-code sealing;
     post-close deposits, cohort-frozen modular
     index/checkpoint/carry economics including the four-close caller-farming regression, the
     large-holder full-exit accumulation vector, separate unassigned-carry/per-exit bounds,
     post-burn/pre-mint normalization, exact event post-state, and a forced ring wrap,
     liquidations/bad debt/interest, policy checks, upgrade/recovery, the adversarial root-first seal
     sequence (subordinate propose rejects before the root, a stored pre-root subordinate proposal
     fixture rejects execution, the root action/terminal commits before subordinate creation,
     registry upgrade rejects thereafter, and each later subordinate seal is durable), and every
     retained interface client; maximum-reserve projected-cash submit, deposit sync, close,
     withdrawal, and fully backed claim cases include injected backing-deficit/partial-result
     rollback and record full simulated transaction resources. The submit case spans all 30
     reserves with 30 collateral and 30 liability positions, remains healthy, verifies projected-
     cash cache size/read count stays per distinct membership
     asset under repeated requests, and covers plain, allowance, and flash-prefixed variants. For
     each variant, record successful actual-Wasm resource usage at exactly 60 requests for both the
     30-reserve effective-position case and a repeated same-asset case, then require a 61-request
     call to return `BadRequest = 1200` before every prohibited effect. If either 60-request candidate
     does not fit the captured live limits, genesis is blocked rather than the bound being silently
     reduced. Inject
     a direct configured LP-token transfer, derive positive
     `Z`, prove every named pool/share/claim view is unchanged and no path can spend `Z`, then
     preserve `T = A + Z` across deposit, donation, draw, loss, queue/dequeue, and withdrawal;
   - recording-simulate and then enforcing-simulate classic-multisig address auth, prove the exact
     invocation tree and minimum valid weighted signer set, and reject missing/duplicate/stale/
     divergent auth; source-envelope signing remains a distinct final step;
   - derive each action window from on-chain `ready_at`/`expires_at`, exercise exact-before/at/after
     boundaries, preserve the full 17-day q4w interval, and never substitute nominal sleeps for
     persisted bounds;
   - lower TTL only on a configurable standalone network, archive and restore exact membership,
     proposal/target, relationship, pool/user balance, share-epoch, pool deferred-credit,
     supplier-rounding-carry, global credit/user checkpoint/scaled claim, and unowned-surplus
     footprints; an Accepted issuance decision adds its own emission footprints. On public
     Testnet use its actual TTL or report archival not run rather than claiming a shortened test;
   - durably bind each intended successful mutation to network, source sequence, bounded envelope,
     transaction hash and expected postcondition before broadcast; on restart reconcile that exact
     hash before preparing another mutation, resubmit only an identical still-live envelope, and
     stop on an unexplained consumed sequence. The storage format and filenames are
     implementation-owned; prepared/submitted/terminal semantics and no-duplicate outcome are not;
   - replay raw events from genesis and compare every discoverable view, including pause,
     lifecycles, generations, implementations, proposals, target owners, vetoes, seals, supplier
     carry, unowned surplus, and each cash allocation; recompute `Q` and its recipient from the
     pre-allocation supply/route/epoch state and enforce bucket -> `cash_alloc` -> containing-event
     order. Within each submit transaction, replay deferred supply/collateral/repay/withdraw/borrow
     and logical flash cash as scheduled-but-unreflected deltas. Treat a synchronous interest-
     auction lot before first observation as already reflected and, after projection initialization,
     advance the projection once when that lot succeeds. Exclude the current membership flow until
     after allocation, enforce the 60-request success and 61+-request pre-effect rejection boundary,
     and enforce
     allocation bucket -> `cash_alloc` -> membership carry/reclassification -> baseline request
     order. For stale realization, compare allocation and creditor payout while separately replaying
     the public-only accrual delta; for a status-gated public `gulp`, require no reserve/allocation
     event at all. Require each incident-exit create event to occur while the replayed
     target state is paused/`Contained`, reconcile reserve identities against live token balances,
     and derive nonnegative LP excess `Z` from live `T` and replayed `A`; and
   - repeat the real authorization/cross-contract/upgrade/containment path on public Testnet,
     including `FillInterestAuction -> membership`, `membership -> FillInterestAuction -> membership`,
     and deferred repay/borrow plus auction permutations across plain, allowance, and flash-prefixed
     forms; exact 60-request success and 61-request pre-effect `BadRequest`; first/later/final
     supplier-boundary allocation; a Borrow-enabled close pair; current-
     and stale-reserve realization pairs with their distinct equality claims; and a status-gated
     public-`gulp` rejection followed by private cold allocation. Record the raw allocation events,
     live network protocol/settings, and transaction hashes, and immediately prove that a pre-root
     subordinate-seal proposal returns `InvalidState` with no event. Also prove at least one
     pre-incident resume attempt allocates no id/target/event and cannot use elapsed time after a
     later pause/contain, while its fresh incident-state replacement observes the full delay.
     Rehearse the chosen
     contract-account `__check_auth` tree before any final-governance handoff or relevant seal; the
     live final ceremony observes its real windows and completes `RegistryCode` before any
     subordinate proposal rather than substituting standalone time travel.

Draft/release reports explicitly separate what ran from what remains gated. Missing Stellar/Kani
tools, unavailable current fixtures, absent implementation artifacts, or an ahead-of-Mainnet
Testnet matrix are stated; no lower verification level is silently relabeled as the missing one.

Follow-ups: draft the future protocol-fee decision on the typed proposal/shared-interface
boundary; issuance ADR (re-route/detach and recipient/buyback); permissioning ADR;
final-governance and seal runbook; legacy-adoption ADR if needed; STRIDE-style threat notes for
registry governance before mainnet. Every storage migration or changed liquidation/upgrade
authority gets its own deliberate ADR and links back here.
