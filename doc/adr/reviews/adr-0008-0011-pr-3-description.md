# Implement the combined ADR 0008 / ADR 0011 Blend security fork

> **SOURCE MERGE AUTHORIZED — NOT APPROVED FOR RELEASE, DEPLOYMENT, ACTIVATION, OR FUNDS.** Merging records the reviewed source without triggering a release or deployment. The security gate remains open for those later actions.

## Exact candidate

| Item | Identity |
| --- | --- |
| Base | `6c3bbb46f89653b13bec7c39e4b811161c82c765` |
| Implementation | `f4b2722086008bfd2b3751309fbaf9e37d230402` |
| PR head / audit target | `54afdae01bb8e484514940229e06460aa3fa6a1f` |
| Git tree | `f1adad95d7d51f533b27f3ec8d302f66ecb14fe6` |
| Pinned stock baseline | `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` |

The PR base is a documentation-only descendant of the pinned stock baseline; their executable contract/build trees are identical. The pinned baseline is executable-tree-equivalent to upstream Blend v2.0.0 release `c19abee5b9be4f49e0cda9057e87d343e5dcc095`.

Review the exact head above. The separate Kani/formal-verification PR is supplemental and is not part of this audit target.

Accepted ADRs are immutable. If later review proves an obligation wrong, correct or supersede it in a new artifact and repeat the affected review loop; do not rewrite accepted history or hold sound code out of source control to make old prose look correct.

## What changes

The pool fork:

- requires zero backstop take, sealed reserve caps, and `max_util < 100%`;
- permits only delayed enabled-to-disabled reserve transitions after activation, while authenticated status 4 is idempotent, backstop-independent, and absorbing;
- rejects flash loans, bad-debt auctions, and backstop-interest auctions;
- applies stricter price handling;
- burns a defaulting user's same-reserve ordinary supply before socializing only residual liabilities;
- creates pool-owned orphan collateral only when residual debt is actually defaulted;
- makes `gulp(asset)` ignore surplus/backstop credit and only settle debt-free pool-owned orphans; and
- traps pool emissions configuration while preserving explicit accounting events and rollback.

The backstop keeps its public ABI but makes `distribute`, `gulp_emissions`, `add_reward`, `remove_reward`, `claim`, and `drop` fail immediately with `BadRequest` before observable effects.

## Governing records

- [ADR 0008](https://github.com/Templar-Protocol/precept/blob/main/doc/adr/0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)
- [ADR 0011](https://github.com/Templar-Protocol/precept/blob/main/doc/adr/0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md)
- [ADR 0009 migration/terminal-state context](https://github.com/Templar-Protocol/precept/blob/main/doc/adr/0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md) — coupled context only; migrator and registry cutover are outside this PR.
- [Security-gate reopen record](https://github.com/Templar-Protocol/precept/blob/main/doc/adr/reviews/adr-0008-0011-gate-reopen-record.md)
- [External audit brief](https://github.com/Templar-Protocol/precept/blob/main/doc/adr/reviews/adr-0008-0011-external-audit-brief.md)

The audit brief defines the priority review questions, residual risks, evidence boundaries, and requested deliverables.

## Fresh exact-head preflight

Run on 2026-09-22 at `54afdae01bb8e484514940229e06460aa3fa6a1f`:

- `devenv shell -- make test`: **379 passed, 0 failed, 1 intentionally ignored differential**.
- `devenv shell -- cargo test -p backstop --lib`: **115 passed, 0 failed**.
- Combined ordinary tests: **494 passed, 0 failed**.
- `devenv shell -- make differential`: **1 passed in 249.32 seconds; all common-domain steps matched and all 11 mutation controls were detected**.
- Pool, backstop, and factory public-spec XDR were byte-identical to stock.
- Optimized pool: 53,860 bytes, SHA-256 `2db04d55fa91246f263c7cd98ba9b436a97e5f32218d67a86ea7beca057939fa`.
- Optimized backstop: 22,701 bytes, SHA-256 `6bd13007c020b617c43c718c2ad3d2d13605d4d42fa8f739da2293f8a4e293a0`.
- Optimized factory: 3,149 bytes, byte-identical to stock, SHA-256 `80abaf3c68fd15adade9f82165baa9b50d85800e497c99b45809e1fbd7397de2`.

`make test` intentionally omits the differential; the separate `make differential` result above is the verdict-bearing comparison.

## Fresh testnet A/B disclosure

The source-bound tool at Precept commit `de96051925762cd615ad1f7189434489f09418b3` pinned stock `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` against exact audit target `54afdae01bb8e484514940229e06460aa3fa6a1f`, ran 207 steps, and reconciled 204 successful public Stellar transactions across isolated ordinary, oracle, and loss deployments.

- Every requested ADR-0008/0009/0011 ledger row was `PASS` or `NOT REQUESTED`.
- Accounting failures: 0. Timeline failures: 0. Restarts/replay: 0.
- **Overall certification remained FAIL in all three families; no waiver was applied.**
- The retained canonical failure is the stock `add_reward` reward-zone mutation versus the fork's required trap: raw `backstop_rz` presence and observable reward-zone state differ.
- Custodied campaign manifest SHA-256: `cbfaf27457bbb0af332882f241a51ec3db656475e260bc2642eb389a69110459`.

This is disclosed evidence, not a passing differential verdict and not permission to weaken exact comparison.

## Known open risks

- The activation-share custodian can affect availability through Q4W; M-of-N custody, 30/50/75% alerts, withdrawal-delay response, and rehearsed dequeue/redeposit recovery remain deployment gates.
- Backstop LP deposit remains public; transferable or unsolicited LP can create foreign shares/Q4W and extend availability or retirement state. The fork adds no custodian-only admission.
- Post-activation disable calls must use the live stored reserve index.
- Stale prices for residual positions can delay loss processing until oracle recovery.
- Available orphan cash can be withdrawn by other suppliers under the utilization limit before permissionless settlement.
- Direct draw/donate mocked-auth observations are body/state/event controls, not proof of production-route reachability.
- Local tests, A/B campaigns, bounded generated cases, and Kani do not replace an independent external audit.

No local review found a surviving critical/high arbitrary-user drain in its examined scope. That bounded result is not an external-audit verdict.

## Source merge basis

- Exact head `54afdae01bb8e484514940229e06460aa3fa6a1f` passed 494 ordinary tests plus the explicit differential gate and retained every disclosed FAIL verdict.
- A merge commit preserves that exact commit as an audit target and does not rewrite either accepted ADR.
- A push to `main` runs CI only. Contract release automation requires a separate `v*` tag; merge does not deploy, activate, route funds, or authorize funds exposure.

## Required before release or deployment

- [ ] Complete an independent external audit against exact commit `54afdae01bb8e484514940229e06460aa3fa6a1f`.
- [ ] Repair or explicitly disposition every finding and rerun affected gates.
- [ ] Complete an independently reviewed upstream-commit, GitHub-security-advisory, and RustSec checkpoint; resolve every applicable item.
- [ ] Freeze the final source/evidence pair and obtain fresh security then maintenance acceptance.
- [ ] Preserve the A/B FAIL verdicts until their exact divergences are dispositioned; do not waive them.

Operational custody, alerts, response/recovery exercises, policy/oracle review, commercial comparison, and the abandonment date still block final acceptance and release. Canaries, exposure limits, routing controls, and deployment authorization additionally block deployment, activation, and funds exposure.
