# 14. Use a multisig operator account as the ADR-0008 pool admin instead of an immutable policy contract

Date: 2026-10-06

## Status

Proposed

UNREVIEWED: hand-written by an agent at the operator's direction; it has not been through a
`raladr` Architect/Critic pair. It touches admin authority and must be reviewed before human
acceptance. Only a human may accept it.

Amends [8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)

This ADR replaces ADR 0008 section 4 ("Make the policy contract the pool admin from
construction") and the parts of sections 8, 9 and 10 and acceptance criterion 5 that depend on
`v1_policy_admin`. Everything else in ADR 0008, and all of ADR 0009 and ADR 0011, is unchanged.

Amends [8. Launch v1 on a narrowly hardened Blend v2 fork with a Templar-sovereign activation shim](0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)

## Context

ADR 0008 section 4 requires a small immutable `v1_policy_admin` contract to be the pool admin from
deployment. Its purpose was to narrow the admin to a fixed set of buttons: one-time activation,
status 4, the exact `enabled: true -> false` reserve transition before and after sunset, and a
guarded `propose_admin(retirement_sink)` handoff.

The pool fork now on `Templar-Protocol/blend-contracts-v2` `main` (`ffeca26`) already removes most
of what that contract was guarding against, at the pool level where it cannot be bypassed:

- `update_pool` unconditionally rejects (`pool/src/contract.rs:392-394`);
- `set_emissions_config` unconditionally rejects (`pool/src/contract.rs:522-524`);
- after setup, `queue_set_reserve`/`set_reserve` accept only the exact existing configuration with
  `enabled` flipped to false; any other field change reverts with `InvalidReserveMetadata`
  (`pool/src/pool/config.rs:86-88,122-128,209-215`);
- status 4 is absorbing: every request to leave it rejects (`pool/src/pool/status.rs:14-16,77-79`);
- `bstop_rate` must be zero at initialization (`pool/src/pool/config.rs:27-29`).

What remains to a pool admin on this fork is: `set_status` to 0, 2, 3 or 4 (subject to the stock
backstop-threshold and q4w checks for 0/2/3); queue, cancel and execute the disable-only reserve
transition after the stock timelock; and `propose_admin`.

`v1_policy_admin` is not implemented in any Templar repository. The September implementation plan
(`2026-09-11-adr-0008-blend-security-fork-plan.md`) scoped it out as a separate slice, and no slice
has been started. Building, reviewing and canary-testing an immutable admin contract does not fit
the current launch window, and its residual value over the fork's own enforcement is small.

## Decision

The pool admin of every ADR-0008 pool is an **M-of-N multisig Stellar account** (the "operator
multisig"), set as `admin` at factory deployment. No `v1_policy_admin` contract is built or
required.

Requirements on the operator multisig:

1. `M >= 2`, `N >= 3`, with every signer a distinct named person. The roster, thresholds and
   signer public keys are recorded in the pool's Markdown deployment record (ADR 0008 section 7)
   before activation. A signer change is a deployment-record amendment.
2. No signer key is shared with the synthetic-token admins, Comet controller, deployment
   authority or backstop share custodian (ADR 0008 section 5's distinct-authority requirement is
   unchanged).
3. The multisig performs exactly the operations the policy contract would have performed, and
   nothing else, as a documented operating rule rather than a contract invariant:
   - one-time `set_status(0)` after the exact live readback required by ADR 0008 section 7;
   - `set_status(4)` as the primary containment path (ADR 0008 section 8);
   - `queue_set_reserve` with the live `get_reserve` configuration and `enabled: false`, then
     `set_reserve` after the timelock, as `contain(asset)` before sunset and as the sunset
     disable at or after sunset;
   - `cancel_set_reserve` only for a queue the multisig itself created, and only at status 4;
   - `propose_admin(retirement_sink)` only after the terminal predicates of ADR 0008 section 9 and
     ADR 0009 section 9 have been verified off-chain and recorded.
4. The multisig does **not** use `set_status(0)`, `(2)` or `(3)` to change status after activation
   except as part of a reviewed incident, and never to leave on-ice for a pool that has been
   contained. It never calls `propose_admin` to any address other than the recorded
   `retirement_sink`. Any such transaction is a code/admin drift event and closes official routes
   under ADR 0008 section 8.

Obligations that move from contract to runbook, each with a named owner and an exercised runbook
in the deployment record before activation:

- **Sunset.** ADR 0008's permissionless `expire(asset)` no longer exists. The multisig must queue
  the disable for every reserve no later than the sunset date (90 days after activation, ADR 0008
  section 1) and execute it after the timelock. Missing the date is an ADR-0008 route-closure
  incident, not a silent extension.
- **Queue ledger.** The policy's fail-closed queue record becomes an off-chain ledger in the
  deployment record: every `queue_set_reserve` the multisig signs is recorded with its asset,
  configuration and unlock time, and the pool's `get_reserve` is read back after each
  `set_reserve`. Activation and retirement proofs cite this ledger.
- **Retirement handoff.** The immutable `retirement_sink` of ADR 0009 section 9 is unchanged and
  still required. The multisig signs `propose_admin(sink)`; anyone then calls the sink's
  `accept_pool_admin(pool)` before the stock proposal expires.

The deployment tooling must bind the exact multisig address as `admin` in the factory `deploy`
call and read it back with `get_admin` before `set_status(0)`.

## Alternatives

1. **Build `v1_policy_admin` as specified.** Strongest guarantee: sunset and sink handoff are
   enforced on-chain and the admin cannot re-ice or repoint. Lost on schedule: an unstarted
   immutable contract needing its own review and canary does not fit the launch window, and the
   fork already enforces the economic invariants it was protecting.
2. **Single operator key as admin.** Smallest operational footprint. Rejected: one compromised or
   lost key can freeze a healthy pool or lose the freeze button entirely.
3. **Multisig admin plus a tiny permissionless sunset contract.** Restores on-chain sunset
   enforcement with a much smaller contract than the full policy. Deferred: it still needs review
   and canary; can be added for a later cohort without changing this decision.
4. **No admin (burn the admin after activation).** Removes all operator authority. Rejected:
   it also removes status 4, which ADR 0008 names as the primary containment path.

## Consequences

Easier: no new contract to build, review, deploy or canary before launch; the freeze button exists
on day one; signer rotation is a record change, not a redeploy.

Harder, and must be disclosed under ADR 0008 criterion 13:

- The admin can move a pool between status 0, 2 and 3 after activation and can propose any new
  admin. Neither can change economic configuration, but both are powers the policy contract did
  not have. Mitigation is the multisig threshold, transaction alerting on every admin call, and the
  drift rule in Decision item 4.
- Sunset and the retirement handoff are human actions. A missed sunset leaves intake open until
  the multisig acts. Mitigation is the named owner, calendar alert at T-14 and T-7 days, and
  route closure on a missed date.
- The queue ledger is off-chain evidence rather than contract state. Reconciliation must read
  `get_reserve` after every transition; a disagreement blocks activation and retirement as before.

ADR 0008 acceptance criterion 5 is replaced by:

5. **Admin authority.** The deployment record names the multisig roster and thresholds; tests on
   the exact release Wasm prove that the recorded multisig can `set_status(4)`, queue and execute
   the disable-only reserve transition, cancel its own queue at status 4, and `propose_admin(sink)`
   followed by permissionless `accept_pool_admin`; and that `update_pool`,
   `set_emissions_config`, and any reserve change other than disable reject regardless of admin
   authorization. The freeze and sunset runbooks are exercised on testnet against the release Wasm
   before activation.

Invariants (per `AGENTS.md`): unchanged. This ADR changes no contract behaviour, Wasm, storage or
interface; the pool fork's sealed configuration, zero take, absorbing status 4 and disable-only
reserve path are the enforced invariants. It changes an operational assumption: who holds the
admin key and how sunset is enforced.

Verification: `adr --robot lint`; the admin-authority tests above in `test-suites`; testnet
rehearsal of freeze, contain, sunset disable and sink handoff with the exact multisig configuration.
