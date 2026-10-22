# ADR 0008 / ADR 0011 external security audit brief

## Status

**Source integration is authorized; release, deployment, activation, and funds are not.** Merging records the reviewed source without triggering a release or deployment. The security gate remains open for those later actions until independent findings are resolved and the exact candidate receives fresh final acceptance.

## Exact review target

| Item | Identity |
| --- | --- |
| Repository | [`Templar-Protocol/blend-contracts-v2`](https://github.com/Templar-Protocol/blend-contracts-v2) |
| Pull request | [#3](https://github.com/Templar-Protocol/blend-contracts-v2/pull/3) |
| PR base | `6c3bbb46f89653b13bec7c39e4b811161c82c765` |
| Pinned stock baseline | `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` |
| Implementation commit | `f4b2722086008bfd2b3751309fbaf9e37d230402` |
| Audit target / PR head | `54afdae01bb8e484514940229e06460aa3fa6a1f` |
| Audit-target Git tree | `f1adad95d7d51f533b27f3ec8d302f66ecb14fe6` |
| Base-to-head diff | 50 files, +7,796 / -4,286 |

The PR base is a documentation-only descendant of the pinned stock baseline; their executable contract/build trees are identical. The pinned baseline is executable-tree-equivalent to upstream Blend v2.0.0 release `c19abee5b9be4f49e0cda9057e87d343e5dcc095`: review PR changes from the PR base, and inherited provenance from the pinned/upstream baseline.

Review the exact tree above. Later branches, including the separate Kani/formal-verification PR, are supplemental and are not part of this audit target.

Reproduce the source binding with:

```sh
git fetch origin
git rev-parse origin/adr/0008-0011-local-security-fork
git rev-parse 'origin/adr/0008-0011-local-security-fork^{tree}'
git diff --stat 6c3bbb46f89653b13bec7c39e4b811161c82c765..54afdae01bb8e484514940229e06460aa3fa6a1f
```

## Governing decisions

- [ADR 0008 — narrowly hardened Blend v2 fork](../0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md)
- [ADR 0011 — disable activation-shim backstop emissions while preserving ABI](../0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md)
- [ADR 0009 — migration and terminal-state context](../0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md)
- [Security-gate reopen record](adr-0008-0011-gate-reopen-record.md)

ADR 0008 authorizes a narrowly capped, isolated, immutable pilot rather than a general Blend deployment. ADR 0011 keeps the backstop ABI but traps six effectful emissions exports before any observable effect. ADR 0009 is supplied only for coupled foreign-share/Q4W and terminal-state context; its migrator, registry cutover, and successor implementation are outside PR #3.

Accepted ADRs are immutable. If review shows an accepted obligation was wrong, record the correction or supersession in a new review/decision artifact and repeat the affected review loop; do not rewrite accepted history or block otherwise-sound source integration merely to make old governance prose look correct.

## Security delta

The pool fork:

1. requires zero backstop take, sealed reserve caps, and `max_util < 100%`;
2. permits only delayed enabled-to-disabled reserve transitions after activation, while authenticated status 4 is idempotent, handled before any backstop construction or call, and absorbing;
3. rejects flash loans, bad-debt auctions, and backstop-interest auctions;
4. enforces stricter price handling;
5. sets off a defaulting user's same-reserve ordinary supply before socializing only residual liabilities;
6. moves collateral to pool-owned orphan custody only when residual debt is actually defaulted;
7. makes `gulp(asset)` ignore surplus and backstop credit, allowing only debt-free pool-owned orphan settlement with the stock zero-unit result/event; and
8. traps pool emissions configuration while preserving rollback and explicit accounting events.

The backstop fork makes `distribute`, `gulp_emissions`, `add_reward`, `remove_reward`, `claim`, and `drop` fail immediately with `BadRequest`. Ordinary deposit, queue/dequeue, withdrawal, draw, donation, and threshold logic remain in audit scope because retained internal emissions hooks must stay inert from fresh genesis.

## Priority audit questions

1. **Setoff and loss arithmetic:** Can rounding, zero rates, last-supplier/no-denominator states, multi-reserve positions, or final auction fills create value, erase excess debt, misallocate supplier loss, or desynchronize events from state?
2. **Withdrawal boundary:** Can an insolvent borrower withdraw same-reserve supply that should be recoverable for setoff, directly or through sequencing/reentrancy?
3. **Orphan custody and `gulp`:** Can pool-owned collateral be revived, privately withdrawn, emission-enabled, stranded, double-counted, or settled while debt remains; or can any `gulp` path recognize surplus, transfer tokens, or create backstop credit?
4. **Price and liveness boundary:** Can stale, future, nonpositive, wrong-decimal, or multiplication-floor prices bypass safety checks or permanently block required loss processing?
5. **Configuration and containment:** Can reserve addition, re-enable, mutation, setup queues, stale indices, accrued interest, or admin calls bypass sealed `max_util < 100%`, the per-reserve `supply_cap`, zero take, or the disable-only delay; and can authenticated status 4 depend on the backstop, perform effects first, or ever exit?
6. **Trapped entrypoints:** Do every disabled pool/backstop entrypoint and every inter-contract route reject before authorization, TTL, storage, event, allowance, token, Comet, or emitter effects?
7. **Retained call graph:** Is backstop emissions state unreachable from constructor-complete genesis despite retained internal hooks and unsolicited token transfers?
8. **Compatibility, provenance, and inherited audit coverage:** Are public specifications preserved where claimed, is the factory byte-identical, are build/dependency inputs reproducible, which inherited Blend v2 reports apply to pinned baseline `6cf6dd1…` / upstream release `c19abee…`, and what exact-tree surfaces remain uncovered?

## Fresh exact-head preflight

Run on 2026-09-22 from clean tracked source at `54afdae01bb8e484514940229e06460aa3fa6a1f`:

| Gate | Result |
| --- | --- |
| `devenv shell -- make test` | PASS: 379 tests, 0 failed; the explicit differential test was intentionally ignored |
| `devenv shell -- cargo test -p backstop --lib` | PASS: 115 tests, 0 failed |
| Combined ordinary tests | 494 passed, 0 failed, 1 intentionally ignored differential |
| `devenv shell -- make differential` | PASS: 1 test in 249.32 seconds; all common-domain steps matched and all 11 mutation controls were detected |
| Public specifications | Pool, backstop, and factory XDR were byte-identical to the pinned stock baseline |
| Optimized pool | 53,860 bytes; SHA-256 `2db04d55fa91246f263c7cd98ba9b436a97e5f32218d67a86ea7beca057939fa` |
| Optimized backstop | 22,701 bytes; SHA-256 `6bd13007c020b617c43c718c2ad3d2d13605d4d42fa8f739da2293f8a4e293a0` |
| Optimized factory | 3,149 bytes; byte-identical to stock; SHA-256 `80abaf3c68fd15adade9f82165baa9b50d85800e497c99b45809e1fbd7397de2` |

`make test` intentionally omits the differential; the separate `make differential` result above is the verdict-bearing comparison.

## Fresh public testnet A/B evidence

The source-bound `precept-abtest` tool is published at Precept commit `de96051925762cd615ad1f7189434489f09418b3`. A fresh three-family campaign pinned stock `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` against the exact audit target `54afdae01bb8e484514940229e06460aa3fa6a1f`, exercised 207 steps, and reconciled 204 successful public Stellar transactions across isolated deployments.

- Every requested ADR-0008/0009/0011 ledger row was `PASS` or `NOT REQUESTED`.
- Accounting failures: 0. Timeline failures: 0. Restarts or replay: 0.
- **Overall certification remained FAIL in all three families.** No waiver was applied.
- The retained canonical failure is the stock `add_reward` reward-zone mutation versus the fork's required trap: raw `backstop_rz` entry presence and observable reward-zone state differ.
- Ordinary report SHA-256: `e04a011a2a968c4d0a5e332c7991c5c406a4c64b50bfeb9017e57a0938e5e9ef`.
- Oracle report SHA-256: `7465028fe1b34636ec2b3731f829b077b564cfa4163be384b8ceca6b201fdf1e`.
- Loss report SHA-256: `b8dd9df63036db33d257a9c735089e3c1f5a963947402a56326db6dbea232e9b`.
- Custodied campaign manifest SHA-256: `cbfaf27457bbb0af332882f241a51ec3db656475e260bc2642eb389a69110459`.

This failure is disclosed evidence, not a passing differential verdict and not permission to relax exact comparison. The complete campaign bundle should be supplied to the auditor out of band against the manifest above.

## Known residual risks and boundaries

- **Activation custodian authority:** the share custodian can affect pool availability through Q4W. The accepted design requires M-of-N custody, 30/50/75% alerts, withdrawal-delay response, and rehearsed dequeue/redeposit recovery before activation.
- **Public share admission and tail:** backstop LP deposit remains public; transferable or unsolicited LP can create foreign shares/Q4W and extend availability or retirement state. This fork adds no custodian-only admission; ADR 0009 terminal handling remains required.
- **Policy caller obligation:** post-activation disable calls must use the live stored reserve index.
- **Loss-processing liveness:** stale prices for any residual position can delay default handling until the oracle recovers.
- **Unsettled orphan cash:** other suppliers may withdraw available cash under the utilization limit before permissionless orphan settlement.
- **Synthetic-authority evidence:** direct draw/donate controls using mocked authorization prove body/state/event equivalence, not reachability from production fork callers.
- **Bounded evidence:** tests, generated scenarios, A/B campaigns, and Kani are not proofs of all Soroban host or protocol states.

No local review found a surviving critical/high arbitrary-user drain in its examined scope. That is a bounded internal result, not an external-audit verdict.

## Outside this code audit but still release-blocking

- deployed M-of-N custody and alert evidence;
- exercised incident, withdrawal-delay, recovery, migration, and irreversible-freeze runbooks;
- reviewed policy-admin and proxy-oracle deployments and live configuration;
- commercial comparison and the owner-approved abandonment date;
- a current independently reviewed upstream-commit, GitHub-security-advisory, and RustSec checkpoint with every applicable change resolved or explicitly blocking release/routing;
- final source/evidence freeze, independent finding closure, and fresh security then maintenance acceptance; and
- canaries, exposure limits, routing controls, and deployment authorization.

## Requested auditor deliverables

- severity-ranked findings with exact source references and exploit preconditions;
- fund-at-risk and liveness impact;
- minimal reproductions or regression tests for each confirmed defect;
- explicit treatment of the priority questions and known residual risks above;
- a report-to-release matrix mapping inherited Blend v2 findings/surfaces to the pinned baseline and identifying exact-tree gaps without claiming inherited byte-for-byte coverage; and
- a statement binding the report to commit `54afdae01bb8e484514940229e06460aa3fa6a1f` and tree `f1adad95d7d51f533b27f3ec8d302f66ecb14fe6`.
