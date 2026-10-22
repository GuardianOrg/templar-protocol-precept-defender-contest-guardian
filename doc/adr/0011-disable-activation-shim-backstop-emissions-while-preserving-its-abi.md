# 11. Disable activation-shim backstop emissions while preserving its ABI

Date: 2026-09-09

## Status

Accepted

Amended 2026-09-16: ADR 0008 restriction 7 now traps pool `set_emissions_config`; the sentences below
that described it as retained-but-unreachable are updated accordingly. Same fresh-review requirement.

Amends [8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)

Amends [9. Migrate ADR-0008 legacy pools to registry-native successors](0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md)

This ADR resolves ADR 0008's previously unresolved emissions disposition by forking the backstop.
It does not preserve any review or acceptance result for the changed ADR-0008/0009 bundle.

## Context

ADR 0008 needs a non-economic activation shim, not a reward program. Its pinned Blend v2.0.0
backstop nevertheless exposes seven emissions methods in `backstop/src/contract.rs`:
`reward_zone` is a getter (`258-260`), while `distribute`, `gulp_emissions`, `add_reward`,
`remove_reward`, `claim`, and `drop` are effectful exports (`264-310`). The stock implementation can
extend TTL, authorize callers, mutate reward-zone/distribution/backfill/accrual state, change token
allowances, move BLND through Comet, mint LP-backed shares, emit events, or call the configured
emitter. In particular, permissionless `add_reward` can add an above-threshold activation-shim pool
to the reward zone (`backstop/src/emissions/manager.rs:17-55`). Operationally promising not to call
these public methods cannot make those effects unreachable.

Ordinary backstop operations also invoke an internal hook. `deposit`, `queue_withdrawal`, and
`dequeue_withdrawal` call `emissions::update_emissions`
(`backstop/src/backstop/deposit.rs:7-32`,
`backstop/src/backstop/withdrawal.rs:9-49`). From constructor-complete genesis,
`update_emissions` returns without writing when no pool emissions configuration exists
(`backstop/src/emissions/distributor.rs:19-26`). The stock roots that can create or consume that
configuration all pass through the effectful public emissions exports above.

The production profile is immutable and starts from fresh constructor-complete state; it does not
upgrade or import a prior backstop's storage. Comet remains constructor metadata as the syntactic
emitter and remains the backstop token, but no supported path funds BLND or configures pool reserve
emission shares. ADR 0008's pool fork traps `set_emissions_config` (its restriction 7), so no admin route to pool
emissions configuration exists; this ADR itself changes no pool bytecode.

In scope: the backstop's six effectful emissions exports, the retained internal hooks, ABI/spec and
artifact provenance, non-interference with fund/loss paths, and the corresponding terminal-state
simplification in ADR 0009. Out of scope: ADR 0008's seven pool restrictions, pool-side emissions
bytecode, liquidation semantics, Comet/backstop-token economics, factory behavior, public LP
deposit/share admission, and the ADR-0010 registry-native successor. Public LP deposit therefore
remains supported by the fork and ADR 0009's foreign-share/q4w branches remain required.

Principles:

1. Make prohibited emissions effects impossible in contract code, not dependent on operator
   restraint.
2. Minimize the security delta: reject at the ABI boundary and do not rewrite ordinary fund paths.
3. Preserve wire compatibility and exact, reproducible base-to-fork provenance.
4. Treat a fresh immutable deployment and an already-populated upgraded deployment as different
   systems; this decision authorizes only the former.

Decision drivers, in order:

1. No public caller can create a cohort-retirement or reward-funded-share grief path.
2. Deposit, queue/dequeue, withdrawal, draw, donation, pool status, liquidation, and bad-debt
   behavior stay equivalent to the pinned base except where ADR 0008 already changes the pool.
3. A small team can audit, rebuild, forward-port, and retire the fork without a parallel emissions
   subsystem.

## Decision

ADR-0008 cohorts use a fork of the backstop at pinned Blend source
`6cf6dd1ca712ea3cf17a767fe49b7da01f80b909`. The factory remains byte-identical to that base.

The backstop preserves its public ABI and existing `BackstopError` contract spec. Each of the six
effectful emissions exports below unconditionally raises existing `BackstopError::BadRequest` as
its first operation:

1. `distribute`;
2. `gulp_emissions`;
3. `add_reward`;
4. `remove_reward`;
5. `claim`; and
6. `drop`.

The rejection occurs before instance or persistent TTL extension, caller authorization, storage
read or write, event emission, allowance change, token movement, Comet call, emitter call, or any
other contract call. Arguments are accepted only to preserve the ABI and are not inspected.
`reward_zone` remains an unchanged read-only getter and returns the empty vector in every reachable
state from constructor-complete genesis.

The implementation retains the internal `update_emissions` calls in deposit, queue, and dequeue.
It does not modify `backstop/src/emissions/**`, storage keys, events, constructor arguments,
dependency clients, ordinary backstop methods, or the generated public spec. Tests must prove that
the retained hook is permanently inert from genesis because every root that could initialize
backstop emissions configuration is trapped. Removing the hooks is not authorized: touching
deposit/withdrawal accounting would widen the fund-path audit surface without strengthening the
genesis-reachable invariant.

The invariant is:

> From constructor-complete genesis, no backstop export or call authorized by the backstop can
> create reward-zone membership, distribution, backfill, accrual, or user-emission state; create a
> nonzero reward allowance; move BLND; mint LP or backstop shares from rewards; or call the emitter.
> Every effectful backstop emissions export rejects with `BadRequest` before any observable effect.
> All non-emissions backstop behavior remains equivalent to the pinned base.

An external BLND holder can still transfer tokens directly to the backstop address without calling
or receiving authorization from the backstop. Such unsolicited BLND is inert foreign state: it
creates no reward configuration, accrual, allowance, Comet mint, pool reward value, or claim path.
Monitoring and ADR 0009 archive it as an unrepresented foreign-token balance; they do not classify
it as an emissions-boundary escape or a Templar-represented claim.

The implementation patch is an allowlisted change to the six method bodies in
`backstop/src/contract.rs`, plus tests. Generated files may change only when their bytes are
reproducible outputs; the generated contract spec must remain equal to the base. No feature flag,
admin switch, dormant alternative implementation, new error, new dependency, storage key, event,
or constructor field is permitted.

Pool-side `gulp_emissions` reaches the trapping backstop method and rolls back. Pool `claim`
remains in bytecode and can never pay out. ADR 0008 restriction 7 traps pool `set_emissions_config`
unconditionally; that is a pool-fork decision recorded there, not part of this backstop decision.

This decision does not restrict public LP `deposit`. ADR 0008's launch controls and ADR 0009's
foreign-share and foreign-q4w terminal branches remain unchanged. Custodian-only one-shot admission
would require an immutable binding and new state and is not smuggled into this emissions decision.

ADR 0008's reviewed Markdown deployment record binds the backstop fork with standard Git and
SHA-256 evidence. It names the pinned base and ordered patch commits; source-tree, lockfile,
toolchain, optimizer, diff, generated-spec, optimized-Wasm, focused-review, and equal
base/deployed-factory hashes; and the Wasm byte count. A clean independent rebuild must reproduce
the backstop Wasm hash. The release gate rejects source changes outside the six method bodies and
tests and proves the factory Wasm is byte-identical to the pinned base.

Upstream changes are not merged routinely. The authoritative security-change sources are the
`Templar-Protocol/blend-contracts-v2` default branch, releases, and GitHub security advisories,
plus RustSec advisories for the locked dependency graph. While any cohort is live, the named fork
owner records one reviewed Markdown checkpoint in protected Git history, linked from the deployment
record, at least every seven days. It classifies every intervening commit and advisory as
applicable, irrelevant, or already mitigated with evidence and an independent reviewer, and names
the next deadline. An overdue checkpoint, history gap, or applicable unresolved change disables
official routing, new cohorts, and cap increases until the checkpoint is complete. Required fixes
pass the same diff, spec, differential-test, review, and artifact gates. Existing immutable cohorts
are contained and migrated rather than patched in place. No feature development lands on this
finite-life bridge after a registry-native successor is ready.

Why chosen: this removes the public emissions grief surface in code while leaving the ordinary
backstop accounting and ABI as close to the pinned base as possible. It replaces indefinite
interpretation of economically inert stock emissions state with six deterministic boundary
failures and a reproducible artifact delta.

## Alternatives

1. **Keep stock emissions economically inert behind a byte-identical backstop.** This preserves
   upstream backstop provenance, but public callers can still create reward-zone, backfill,
   accrual, allowance, event, and emitter-call state that operations must classify. It loses on
   driver 1.
2. **Trap only `add_reward` and `distribute`.** This is a smaller textual patch, but leaves four
   effectful exports whose safety depends on root-reachability arguments and unexpected-state
   assumptions. Six uniform boundary failures are easier to audit and test.
3. **Remove `update_emissions` hooks and the emissions modules.** This produces a conceptually
   smaller runtime, but rewrites deposit and withdrawal-adjacent code, changes more generated
   surface/build behavior, and enlarges the fund-path audit. It loses on driver 2.
4. **Remove emissions methods or constructor metadata from the ABI.** This is a cleaner new
   protocol, but breaks compatibility and may require factory/binding changes. The temporary bridge
   gains no safety over unconditional traps sufficient to justify that delta.
5. **Restrict LP share admission in the same ADR.** Custodian-only one-shot deposit could simplify
   ADR 0009, but needs a new immutable binding and persistent transition state. It is separable from
   emissions and would obscure review of both boundaries.
6. **Skip the activation shim and wait for ADR 0010.** This has the lowest permanent maintenance
   burden, but does not meet the accepted temporary delivery objective. ADR 0008's exposure and
   sunset gates bound that tradeoff.

## Consequences

Easier: no caller can use public emissions state as a cohort-retirement or reward-funded-share
switch. ADR 0009 no longer needs an unbacked-emissions tombstone, expired reward-allowance waiting,
or emissions-specific terminal reconciliation. Operators monitor six deterministic rejections
rather than tolerated dynamic emissions state. The public ABI and normal backstop call graph stay
stable.

Harder: Templar owns a second Blend-core fork artifact and cannot inherit upstream fixes without
review. The backstop's whole emissions module and retained ordinary-operation hooks remain in the
audit scope even though only six ABI bodies change. A critical issue in an immutable deployed
cohort still requires containment and migration, not a live patch.

Acceptance criteria:

1. ADR 0008 selects this ADR, identifies the factory as byte-identical and the backstop as forked,
   and contains no unresolved emissions disposition or stock-emissions canary requirement.
2. ADR 0009 contains no terminal tombstone, allowance-expiry, incident, reconciliation, or hostile
   test obligation introduced solely by stock backstop emissions. It retains foreign-share/q4w
   terminal handling, I0/I1 one-way incident containment, and unsolicited-value archival.
3. The source diff against the pinned base changes only the six effectful method bodies and tests.
   Every method's first operation is an unconditional `BadRequest` rejection.
4. Generated backstop specs are byte-equal. Existing clients compile unchanged. Before/after
   optimized Wasm byte counts and the exact fork hash are recorded and stay below 131072 bytes.
5. One contract test per effectful export observes `BadRequest` and no TTL, storage, event,
   authorization, allowance, token, Comet, emitter, or receiver effect.
6. Adversarial call-sequence tests establish the invariant from fresh genesis, including attempts
   before and after deposit, queue, dequeue, withdrawal, draw, donation, activation, freeze,
   liquidation, and bad-debt operations. A direct BLND transfer to the backstop remains inert: it
   creates no reward state, allowance, Comet mint, pool reward value, or claim path and is classified
   only as an unrepresented foreign-token balance.
7. Base-versus-fork differential tests produce equal results, state, events, authorization trees,
   token/allowance deltas, and failures for deposit, queue, dequeue, withdrawal, `pool_data`, draw,
   authenticated donation, status activation/freeze, liquidation, and bad-debt paths.
8. Pool `gulp_emissions` and `claim` tests prove the exact deployed authority tree cannot create
   reward value or bypass the trapped backstop, and pool `set_emissions_config` is shown to reject
   unconditionally with no ledger effect.
9. A focused independent review covers the exact base-to-fork diff, the full emissions root/call
   graph, retained `update_emissions` hooks, ABI/spec comparison, and differential evidence.
10. A disposable Comet/backstop/pool canary uses a trapping emitter and proves ordinary lending and
    loss handling do not require emissions while all six effectful exports reject.
11. A human-readable Markdown review record names the exact final ADR-0008/0009/0011 blob IDs and
    contains a fresh sequential Architect/Critic pair. Only a human may accept the ADRs.
12. ADR 0008's reviewed Markdown deployment record contains every required backstop source,
    build, diff, spec, Wasm, review, and equal-factory hash. Standard Git/SHA-256 checks pass, and a
    clean independent build reproduces the recorded backstop Wasm.
13. Before first activation, a route-control test proves that a complete reviewed upstream
    checkpoint enables official routing and that an overdue checkpoint, history gap, or applicable
    unresolved change disables official routing, new cohorts, and cap increases. The checkpoint is
    reviewed in protected Git history, linked from the deployment record, and covers every
    intervening commit and advisory at intervals no longer than seven days while a cohort is live.

Verification:

- Build before test inside the implementation checkout: `make build && make test`.
- Record optimized Wasm sizes with
  `stat -c '%s %n' target/wasm32-unknown-unknown/optimized/*.wasm`.
- Compare generated spec and factory/backstop artifacts against the pinned base; fail on any
  undeclared difference or a factory byte mismatch.
- Use Soroban contract tests for real authorization, storage/TTL, events, allowances, token deltas,
  cross-contract calls, and rollback. Kani may cover only extracted pure transition predicates;
  it is not evidence for host effects. Use proptest for caller/order/amount/time sequences.
- Run the disposable testnet canary against the exact release Wasm and archive invocations, events,
  state, hashes, and negative results.
- Run `PATH="$PWD/bin:$PATH" adr --robot lint` before review.

Pre-mortem:

1. **A disabled export performs work before rejecting.** Warning: TTL/event/state or mocked-call
   deltas in a negative test. Prevention: unconditional first-operation traps and per-export
   no-effect tests. Recovery: block release; an observed production effect closes routes and starts
   ADR-0008 containment.
2. **A retained hook becomes stateful through an overlooked root, or unsolicited BLND is
   misclassified as an emissions escape.** Warning: a non-empty reward-zone/config/accrual key,
   differential divergence after an ordinary operation, or a foreign balance reported as a claim.
   Prevention: full root/call-graph review, adversarial sequences from fresh genesis, and a direct
   token-transfer negative test. Recovery: archive inert foreign state; any actual reward effect
   freezes and migrates the affected cohort without an operational exception.
3. **An upstream security fix is missed or forward-porting widens the delta.** Warning: an
   overdue checkpoint, history gap, unclassified or unresolved applicable change, diff-allowlist
   failure, spec drift, or non-reproducible Wasm. Prevention: a named fork owner records a reviewed
   checkpoint in protected Git history at least every seven days and links it from the deployment
   record; an independent reviewer covers every intervening commit and advisory. Recovery: disable
   official routing, new cohorts, and cap increases; issue a newly reviewed artifact for
   future cohorts and contain/migrate affected immutable deployments.
4. **Operators infer emissions disablement also restricts public LP deposit.** Warning: external
   share/q4w state or an ADR-0009 terminal mismatch. Prevention: explicit product disclosure,
   foreign-share monitoring, and unchanged ADR-0009 branches. Recovery: quarantine custodian
   claims, preserve the serviced tail, and use the existing foreign-share terminal procedure.

Follow-ups: implementation and focused review in the upstream-source repository only after human
acceptance; exact release records and testnet canary before any activation; registry-native
successor delivery and ADR-0008 cohort sunset. No open design question remains inside the emissions
boundary. Share admission remains explicitly outside this decision.
