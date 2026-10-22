# ADR 0008 / ADR 0011 — final local closeout

## Verdict and custody

**Local implementation and verification complete.** Sequential final security and correctness/maintenance reviews both accepted the same frozen source/evidence pair, with no remaining blocking findings. This is not publication, release, deployment, external-audit, or comprehensive formal-verification approval.

This additive annex supersedes the pre-final-review “pending” status in the frozen implementation evidence report. It does not rewrite that reviewed payload.

| Item | Exact local identity |
| --- | --- |
| Blend branch | `adr/0008-0011-local-security-fork` |
| Production commit | `23abc743035994a5c0f18855c8cb8f54795dbce8` |
| Single test/support head | `273a5131da1f7407887a4a40867c992790809f3d` |
| Preserved documentation ancestor | `6c3bbb46f89653b13bec7c39e4b811161c82c765` |
| Pinned stock baseline | `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` |
| Precept branch | `chore/accept-adr-8-11` |
| Frozen, reviewed evidence payload | `8a0b063eac2daa43c90e94cb2eb47b4f5a1e3d2a` |

The evidence payload also records the user-authorized `Proposed → Accepted` status changes for ADRs 0008 and 0011. Unrelated ADR0009 changes and the untracked duplicate ADR0006 were excluded and remain untouched. No push, PR, protected-ref mutation, vendor-pin update, deployment, routing change, or activation was performed.

## What landed

The dedicated Blend repository owns the combined production fork: zero backstop take; restricted configuration and post-activation disable-only reserve transitions; absorbing frozen status; forbidden flash-loan/backstop-auction paths; stricter price handling; direct supplier-loss accounting with pool-owned orphan collateral custody; and the six ABI-preserving backstop emissions/reward-zone traps. Precept owns decisions, harness source, receipts, logs, specification bytes, and review records. Implementation was not placed in Precept’s vendor checkout.

Production changes precede all test/support changes in local history. The support head contains migrated ordinary scenarios, native and optimized-Wasm regressions, stock-positive differential controls, the full-state comparator and its mutation controls, and generated-sequence replay. The frozen standalone harness is in Precept and consumes an exact local clone of the Blend head without changing Blend manifests.

## Verification results

| Gate | Result |
| --- | --- |
| Exact-source pinned formatting and `make test` | 490 passed, 0 failed, 1 intentionally ignored differential test; 22 suites |
| Explicit exact-source differential invocation | 1 passed, 0 failed; 243.32 seconds; no filtered tests |
| Common-domain observations | 84 normalized base/fork pairs byte-identical |
| Retained differential corpus | 532 artifacts, archived with per-member hashes and byte-verified against originals |
| Exact-source generated replay | 10,000 cases passed; seed `9110008`; 28184.17 seconds |
| Separate development generated replay | 10,000 cases passed; 28354.85 seconds; not substituted for exact-source proof |
| Independent optimized builds | Three clean local producers, byte-identical pool/backstop/factory outputs |
| Public ABI/specification | All three XDR-base64 specifications byte-identical to baseline |
| Factory runtime | Optimized Wasm byte-identical to baseline |
| Build inputs and runtime dependencies | Blend manifests, lockfile, Makefile, toolchain unchanged; normal pool/backstop dependency trees equal after source-root-only normalization |
| Payload custody | All 66 payload-manifest entries matched committed evidence bytes |
| Final sequential reviews | Security accepted, then correctness/maintenance accepted; no remaining blockers or safe Ponytail findings |

The generated replay uses Wasm-registered fixtures and bounds of at most 64 operations, 4 users, 3 reserves, amount selectors 0–5, and time selectors 0–8. It is deterministic bounded testing, not a proof over every reachable protocol state. The earlier Kani receipt proves only the runtime-shared disable-only reserve predicate; it does not prove the full accounting implementation or Soroban host.

| Optimized artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Pool | 53921 | `351257924a5499a6c7c947a7c81e27ffce51ed013650cd52ce188d50cff5c96d` |
| Backstop | 22701 | `6bd13007c020b617c43c718c2ad3d2d13605d4d42fa8f739da2293f8a4e293a0` |
| Factory | 3149 | `80abaf3c68fd15adade9f82165baa9b50d85800e497c99b45809e1fbd7397de2` |

All are below the plan’s 131072-byte gate. The active target network’s actual limit remains a deployment check.

## Did everything go to plan?

**The accepted production design and local verification obligations were delivered, but execution was not smooth.** Verification and evidence custody needed substantial repair. Incorrect intermediate expectations, weak/vacuous assertions, missing retained evidence, failed commands, and lost earlier runtime output are not disguised as successful work.

The [full issue history](adr-0008-blend-fork-implementation-evidence.md) records 19 development/review problems plus two final-runner issues, their causes, and dispositions. The categories are:

1. Wrong-toolchain lockfile contamination and an unauthorized agent validation attempt; restore and prove the pinned inputs, and reject unsupported agent claims.
2. Legacy fixtures incompatible with the intentionally disabled emissions/reward-zone operations and nonzero take; migrate fixtures while retaining ordinary lifecycle coverage.
3. Incorrect factory test inputs and a sealed-reserve control/queue-observation repair.
4. Timestamp-versus-ledger-sequence auction maturity, wrong caller identities, nonexistent-auction trap expectations, and incorrect assumptions about request-type status gating.
5. A wrong emissions discriminator and an overly permissive final-fill oracle; replace them with grounded pre-state conditions, integer accounting, custody, event, and rollback checks.
6. SDK snapshot/event type mistakes, missing committed-event discrimination, and compile errors during those repairs.
7. A suspected stale-Wasm condition that exact rebuilds disproved; dirty-tree artifact provenance still needed honest labeling and committed-source replay.
8. Differential executable-role normalization that could hide TTL swaps, initially ineffective swap fixtures, and missing common-path raw/normalized evidence.
9. An earlier unrecoverable 10,000-case verdict; later supervised runs had durable logs. Wait-call timeouts were not runner failures, and both retained runs ultimately passed.
10. Synthetic return evidence for void reward methods and an auction-deletion assertion against the wrong storage class; verify actual unit returns and deletion through the public getter.
11. Redundant artifact hashing, unused helper arguments/mutability, formatting/mode drift, and duplicate/missing fields introduced during the cleanup; repair and rerun full gates.
12. Standalone Cargo workspace auto-membership; exclude the nested Blend clone without editing Blend manifests.
13. My incorrect differential filter selected zero tests; reject that exit-zero attempt and execute the actual named ignored test. The accepted differential log records one test run and passed.

### Authorized deviations and reasons

- **Local-only workflow:** the user prohibited new worktrees and remote operations. Three independent local clones of the exact source commit provide clean producer separation and reproducibility. They do not claim remote publication custody.
- **Commercial decision timing:** the commercial comparison and explicit owner-approved hard abandonment date were deferred by the user to before publication. They are not silently satisfied or dropped.
- **Combined fork criteria:** ADR0008 incorporates ADR0011. Pool Wasm must change for the combined fork; applying the isolated backstop-only plan’s pool-byte-equality requirement would be incorrect. Factory-byte and public-spec equality were retained and passed.
- **Evidence packaging:** 532 observation files are retained as a 29,369,733-byte archive plus a member-level SHA-256 manifest, instead of loose generated JSON in the checkout. Every archive member was compared against the original before the loose files were moved to the temporary producer area.
- **Test/harness corrections:** the stronger oracles, role-specific normalization, public deletion assertion, and Cargo workspace exclusion repair execution or proof defects; they do not weaken production invariants or widen runtime dependencies.
- **Deliberately retained stock code:** dormant internals and public parameter names remain where deletion would widen the narrow executable/ABI change boundary. No suppression attributes were introduced to hide them.

No remaining local source/evidence deviation requires an additional waiver according to the sequential final reviews. Deferred publication and deployment gates remain binding.

## Next steps

1. **Before publication:** finish the commercial comparison, agree the hard abandonment date, and obtain the required human authorization. Preserve the exact reviewed source/evidence pair and do not absorb unrelated dirty Precept work.
2. **When publication is authorized:** follow the plan’s protected-ref and evidence-before-source publication sequence, verify remote payloads and exact candidate identities, and complete applicable PR/CI and review requirements. These operations have not been performed.
3. **Before deployment or activation:** complete independent external security review and report-to-release coverage, deployed authority-tree and policy/oracle/proxy/shim controls, active-network size/configuration checks, canary and operational/maintenance/migration readiness, exposure limits and sunset conditions. This local fork acceptance is not approval to handle funds.

## Canonical evidence

- [Issue history and development/exact checkpoints](adr-0008-blend-fork-implementation-evidence.md)
- [Exact candidate and command/source bindings](evidence/adr-0008/exact-local-candidate.json)
- [Independent build, size and ABI receipt](evidence/adr-0008/exact-local-artifacts.json)
- [Committed payload manifest](evidence/adr-0008/exact-local-payload-manifest.json)
- [Exact generated replay transcript](evidence/adr-0008/logs/exact-local-property-10000-running.log)
- [Actual one-test differential transcript](evidence/adr-0008/logs/exact-local-differential-executed.log)
- [Differential archive](evidence/adr-0008/exact-local-differential.tar.gz) and [member manifest](evidence/adr-0008/exact-local-differential-manifest.json)
- [Final security acceptance](evidence/adr-0008/final-local-security-review.json)
- [Subsequent correctness/maintenance acceptance](evidence/adr-0008/final-local-maintenance-review.json)

## Superseded security status

The forward-looking local acceptance verdict in this closeout was superseded by the [security gate reopen record](adr-0008-0011-gate-reopen-record.md). This document remains historical evidence for its exact reviewed source/evidence pair; it is not the current publication, release, deployment, activation, or funds-handling verdict.
