# ADR 0008 / ADR 0011 — security gate reopen record

## Current verdict

**SECURITY GATE OPEN — publication, release, deployment, activation, and handling funds remain blocked.** A fresh adversarial review of the locally accepted fork found two medium findings, one low finding, and one evidence-scope gap. It found no critical or high-severity arbitrary-user drain in the exact greenfield composition.

This additive record supersedes only the forward-looking acceptance status in the final local closeout and sequential review verdicts. Those records remain immutable historical evidence for the exact pair they reviewed; they are not the current security verdict.

| Item | Historically reviewed identity |
| --- | --- |
| Blend branch | `adr/0008-0011-local-security-fork` |
| Production commit | `23abc743035994a5c0f18855c8cb8f54795dbce8` |
| Test/support head | `273a5131da1f7407887a4a40867c992790809f3d` |
| Preserved documentation ancestor | `6c3bbb46f89653b13bec7c39e4b811161c82c765` |
| Pinned stock baseline | `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` |
| Precept branch | `chore/accept-adr-8-11` |
| Frozen evidence payload | `8a0b063eac2daa43c90e94cb2eb47b4f5a1e3d2a` |

## Reopened findings

### ECON-001 — medium — recoverable borrower supply is ignored during default

`pool/src/pool/actions.rs::apply_withdraw`, `pool/src/pool/bad_debt.rs::check_and_handle_user_bad_debt`, and `pool/src/pool/user.rs::default_liabilities` allow an insolvent borrower to withdraw ordinary same-reserve supply and then socialize the gross liability. The reproduced honest-supplier rate movement was `1000005210900 -> 995005210800`.

Closure requires both parts of the invariant: an insolvent borrower cannot withdraw the same-reserve claim after it becomes recoverable for setoff, and direct default burns that claim before applying supplier loss to only the uncovered liability. The operation must remain atomic and preserve existing collateral-custody and rollback guarantees.

### CUSTODY-001 — medium — activation-share custodian has pool-availability authority

`backstop/src/backstop/withdrawal.rs` immediately includes queued shares in pool Q4W. The public status machine in `pool/src/pool/status.rs` consequently permits the sole activation-share custodian to force `0 -> 3 -> 5`; after the stock withdrawal lock expires, the custodian can remove the backing. This requires custodian compromise or misuse, not an arbitrary caller.

The selected disposition for this narrowly frozen Blend profile is operational rather than a new backstop fork surface: a code-level custodian identity or withdrawal lock would add constructor/storage/ABI policy, alter ordinary withdrawal and foreign-Q4W behavior, and violate ADR 0011's six-trap boundary. Re-acceptance therefore requires explicit human residual-risk acceptance plus evidence that the activation custodian is M-of-N, Q4W alerts cover 30%, 50%, and 75%, the stock withdrawal delay is exercised as the response window, dequeue/redeposit recovery is rehearsed, and irreversible status 4 is reserved for loss of confidence rather than routine Q4W degradation. Until those controls and acceptance are recorded, this finding remains blocking.

On 2026-09-14, the human owner explicitly accepted this residual authority under the operational disposition above. That closes the code-design choice only. Publication, release, deployment, activation, and handling funds remain blocked until the named custody, alerting, response-window, and recovery evidence is recorded.

### QUEUE-001 — low — setup reserve queue bypasses the post-activation delay

`pool/src/pool/config.rs::execute_queue_set_reserve` gives setup queues immediate unlock, while `execute_set_reserve` only rechecks that a post-setup update disables an enabled reserve. A setup-origin disable queue can therefore survive activation and execute without a full post-activation delay.

Closure requires a contract-enforced lifecycle boundary: setup may configure reserves but cannot author an enabled-to-disabled retirement transition. Once live, the exact disable-only transition must be queued anew and retain the full stock delay.

### EVIDENCE-001 — informational — draw/donate observations are synthetic-authority controls

Donation remains a live public backstop ABI requiring both donor and pool authorization. Draw remains a live pool-authorized ABI. The fork's trapped auction paths remove its production pool callers, while `test-suites/src/differential/artifact.rs` uses fixture-wide mocked authorization for the direct `draw` and `donate` observations in `differential/scenarios.rs`.

Those observations are useful stock-body, state, event, and ABI-equivalence controls. They are not evidence that the fork can reach the calls through a production pool path. Successor evidence and review reports must state that distinction explicitly.

## What remains valid

The fresh review found no arbitrary-user fund-loss path in the six backstop emissions traps, fresh-state reward handling, `gulp_emissions` rollback, cross-pool accounting keys, direct BLND/LP transfers, or the immutable greenfield path with zero backstop take. Those bounded conclusions do not close the findings above and do not replace an independent external audit.

## Re-acceptance requirements

1. Land ECON-001 and QUEUE-001 repairs as successor commits without rewriting the historically reviewed pair.
2. Record the explicit CUSTODY-001 residual-risk decision and its deployed operational-control evidence.
3. Relabel draw/donate differential and property observations as synthetic-authority body-equivalence controls.
4. Re-run the targeted native and optimized-Wasm regressions, full suite, executed differential comparison and mutation controls, 10,000-case generated property run, independent producer rebuilds, optimized artifact/spec/factory comparisons, and input/dependency custody checks.
5. Freeze a new source/evidence pair and obtain fresh sequential security then correctness/maintenance reviews. The old review JSON files remain historical and must not be edited.
6. Complete the already-required independent external security review before funds are exposed.

## Post-fix verification checkpoint — 2026-09-14

The first remediation workspace build/test passed, but the explicit differential gate failed because its class-7 default oracle still expected unchanged ordinary supply and gross supplier loss. This is not a passing differential result. The 10,000-case successor property run was cancelled after review identified a similarly stale direct-default oracle; it is not accepted evidence.

The post-fix source review additionally identified two settlement boundaries requiring correction before any new local acceptance:

- After setoff burns the last supplier claim, residual debt must be cleared without dividing by zero. With no outstanding supplier claims, preserve the reserve rate rather than applying a supplier haircut.
- If same-user setoff clears all liabilities without any residual default, retain the user's collateral. Orphan custody and its emissions prohibition apply only when residual debt is actually defaulted.

The same-reserve withdrawal-health boundary remains intentionally scoped to same-reserve liabilities; no cross-reserve setoff or broader withdrawal restriction is authorized here. Full successor verification, exact-source custody, and sequential acceptance reviews remain pending. Historical passing runs do not certify these subsequent edits.

Both settlement-boundary repairs and their independent oracle updates are now implemented. The subsequent rebuilt full workspace gate passed, including the no-supplier-rate and debt-free-collateral regressions. The explicit differential passed with all 11 mutation controls detected; its 532 observation files are retained in `evidence/adr-0008/post-reopen-differential-observations.tar.gz` with member hashes in `post-reopen-checkpoint.json`. The 100-case generated sanity gate passed at seed `9110008`. Raw Wasm public-spec payloads remain equal to baseline and the optimized factory bytes remain identical. These are working-tree checks, not yet independent exact-successor-commit build receipts.

The retained token-share rounding policy is unchanged: supplied claims convert with conservative floors/ceilings, repayment is measured in d-tokens, and only residual d-token debt is defaulted. The split ceiling can differ from a single gross-debt ceiling by one smallest underlying unit; fractional share-conversion residuals remain bounded by the d-token conversion quantum. No asset-denominated replacement or cross-reserve netting is introduced.

The current source fingerprint is `c844218d02197a3b99ec63e0ab7003c6a2a08414ae6df3d7d3075fea9d9816a7` under the method declared in `post-reopen-checkpoint.json`. The frozen-source security reviewer reported no surviving blocking code finding; this is not security-gate acceptance. The sequential correctness/maintenance review and the 10,000-case terminal verdict remain pending. The new generated run is owned by user-systemd unit `blend-adr8-postreopen-10000-20260914.service`; output is retained at `evidence/adr-0008/logs/post-reopen-property-10000.log`. An active process or exit-code field before process termination does not establish success.

The subsequent correctness review found a generated-coverage gap: the prepared liquidation borrower never held ordinary supply, and generated supply to that borrower was rejected while its auction existed. Consequently the previous generated final-fill cases did not exercise partial or full setoff, despite the oracle containing those branches. The incomplete 10,000-case unit was stopped without a verdict. The fixture is being corrected to seed ordinary supply before auction creation and explicitly reach zero/partial/full setoff. The earlier standalone direct-default and final-fill tests remain valid within their narrower exercised scope; they do not substitute for combined-path coverage. No production defect was established by this coverage finding.

The fixture repair is complete and the correctness reviewer closed that coverage finding after inspecting the delta. Only `test-suites/src/adr8_properties.rs` changed relative to the preceding source fingerprint. The repaired fingerprint is `19e7326ef00b8e349f86177034c273846686e49f713320a626e6a15f43ad9e73`. All 12 registered-Wasm composition smoke cases passed, covering zero/partial/full initial claims with independent and exposed fillers; partial/full seeded-emission variants are included. Selector 3 remains unseeded, and the first tuple must use an odd user selector to choose the liquidation lifecycle. The corrected 100-case generated run passed, formatting passed, and the rebuilt full workspace suite passed again. The replacement 10,000-case run is active under `blend-adr8-composition-10000-20260914.service`, with retained output at `evidence/adr-0008/logs/post-reopen-composition-10000.log`; its verdict remains unverified. No successor commit or exact-commit acceptance is claimed.

## Post-fix verification checkpoint — 2026-09-16

A second adversarial pass over the rebuilt successor chain (production `6a38ee9`, support `ffc365a`) found no arbitrary-user drain and no reopened restriction. It found the ADR text lagging the ECON-001 repairs plus three smaller items. Dispositions below were applied to the local working trees of both repositories and are uncommitted; no commit, push, deployment, routing change, or activation was performed.

| Item | Severity | Disposition |
| --- | --- | --- |
| SPEC-001: same-reserve setoff undocumented in ADR 0008 and silent to reconciliation | medium | Kept (accepted mechanics). Added `debt_setoff(asset, b_tokens_burned, d_tokens_repaid)` at the setoff site in `pool/src/pool/bad_debt.rs`; ADR 0008 restriction 7, AC 3, section 8 archival rule, verification strategy and invariants amended; ADR 0009 terminal classification amended; Blend runbook event lists amended; class-7 stock control now asserts one `debt_setoff` event per setoff. |
| SPEC-002: `default_liabilities` no-denominator branch undeclared while ADR claimed the exact stock update | medium | Declared in ADR 0008 restriction 7 with the division-by-zero rationale. Class-7 stock control no longer absorbs it in the oracle: it asserts `b_rate` preservation explicitly, and a new `pool_direct_default_no_denominator` witness prepares a state that must hit that branch. |
| BRICK-001: admin `set_emissions_config` could permanently wedge orphan custody and final fills for a reserve | fix before release | Trapped: unconditional `BadRequest` as the first operation, ABI and generated spec unchanged (trait docs untouched, implementation body only). ADR 0008 section 3, restriction 7, AC 3 and invariants amended; ADR 0011 amended to stop describing it as retained-but-unreachable. Tests that used the admin route now expect rejection with an unchanged ledger; contradiction variants are planted by host preparation. |
| POLICY-001: post-activation disable requires the caller to pass the live stored `index` | operational | ADR 0008 section 4 now states the live-read caller obligation; hard checklist item for the policy contract review. No Blend change. |
| TOOLING-001: differential evidence only via manual env vars | low | `make differential` target builds the pinned baseline from git and runs the ignored runner; README updated. `make test` alone is not differential evidence. |
| Disclosure: unsettled-orphan cash is withdrawable by other suppliers under the utilization limit; settlement is permissionless at any instant `d_supply == 0` | disclosure | ADR 0008 section 8 keeper text and AC 13 amended. No Blend change. |
| Retracted: "share-inflation test deleted" | none | Misread of the diff stat. `test-suites/tests/test_backstop_inflation_attack.rs` retains the deposit/donate test; only the interest-auction variant was removed. No change. |
| Noted, no change: final fill and `bad_debt` now price the victim's residual positions, so a stale price for any residual asset blocks loss processing until the oracle recovers | liveness | Keeper runbooks should expect it. |

Verification on this checkpoint, run under the pinned 1.81 toolchain through devenv (the system toolchain cannot compile the locked dependency graph):

| Gate | Result |
| --- | --- |
| `cargo test -p pool --lib` | 342 passed, 0 failed |
| `cargo test -p backstop --lib` | 115 passed, 0 failed (unchanged crate) |
| `cargo test -p test-suites` ordinary suites | 16 suites, all passed; 1 intentionally ignored differential |
| Explicit differential (base built from `6cf6dd1`, fork rebuilt) | 1 passed in 263 s; all common-domain steps matched; 11 negative controls detected; 14 stock controls including the new no-denominator witness; 98 retained evidence directories |
| `cargo fmt --all` | clean |
| `adr --robot lint` | only the pre-existing untracked duplicate ADR 0006 |

Caveats: `stellar` CLI is absent on this host, so both artifact pairs are unoptimized release Wasm used for behavioral evidence only; they are not release bytes, and the closeout's optimized hashes no longer describe the amended source. The 10,000-case generated run was not repeated here. This checkpoint is not gate acceptance; the successor still needs a frozen source/evidence pair, independent optimized rebuilds, and fresh sequential reviews.
