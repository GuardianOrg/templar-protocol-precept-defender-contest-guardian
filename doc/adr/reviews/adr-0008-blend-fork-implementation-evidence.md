# ADR 0008 / ADR 0011 local implementation evidence

Date: 2026-09-13

Status: **LOCAL, IN PROGRESS — not implementation approval or publication evidence.**

This record preserves pre-edit authority/source custody, the successful baseline, and Main's executed development checkpoints. The baseline passed before production edits; the changed development tree now passes the migrated workspace suite, but the final exact-commit candidate is not frozen or accepted. Development success is not independent rebuild equality, complete differential/property coverage, implementation review, or publication approval.

## Human authorization and bounded deviations

The user explicitly accepted ADR 0008 and ADR 0011 in the current conversation and authorized their combined **local-only** implementation in the existing checkouts. The user prohibited new worktrees and push/publication. No PR, deployment, repository-ruleset mutation, or vendor-pin change is authorized. These instructions supersede the plans' worktree and publication execution instructions for this local phase, not their technical acceptance criteria.

The user also explicitly authorized moving the commercial delivery comparison and hard abandonment date from before implementation to **before publication**. This is a recorded timing exception to ADR 0008 section 10 and the combined plan's start gate, not a claim that the unchanged ADR already permits that timing. The comparison must still demonstrate a real bridge-versus-successor schedule advantage, account for the required delivery/operations/migration/tail work and reserve-disable timelock, and name an owner-approved hard abandonment date. Neither requirement is waived or satisfied here. The expected `adr-0008-bridge-vs-registry-successor-comparison.md` file was absent at inspection; no estimates, audit availability, finalized grant terms, or date are invented.

Human acceptance is presently local working-file state, not an acceptance commit on published main. The authorizing human is the user in this conversation; no separate named approver, signed attestation, or acceptance commit is asserted. The local ADR 0009 file already says Accepted, but this conversation authorizes acceptance of **only ADR 0008 and ADR 0011**. ADR 0009's local status is preserved without attributing authority to it. The reviewed ADR 0009 blob remains the bundle reference; its human acceptance is a registry-cutover gate, not a gate to this combined contract implementation.

## Governing material

- [ADR 0008](../0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md).
- [ADR 0011](../0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md).
- [ADR 0009 bundle reference](../0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md).
- [Sequential pair-3 review](2026-09-10-adr-0011-96cfa6b.md), containing `CONSENSUS APPROVED`, Architect `APPROVE`, and Critic `APPROVE`, all for the exact candidate below. Those are historical ADR reviews, not reviews of this implementation or of future accepted bytes.
- Combined implementation plan: `/home/common/projects/templar/2026-09-11-adr-0008-blend-security-fork-plan.md`.
- ADR-0011-only plan: `/home/common/projects/templar/2026-09-11-adr-0011-blend-backstop-fork-plan.md`.

The combined plan governs one candidate: seven pool restrictions plus six backstop traps. The isolated ADR-0011-only plan's requirement that optimized **pool** Wasm remain byte-equal does not apply to that combined candidate. Factory source/Wasm equality, generated pool/backstop/factory interface equality, the backstop six-body boundary, unchanged Blend manifests/lockfile, and the remaining combined verification obligations still apply. The older isolated plan's all-three-ADRs-accepted implementation gate is not substituted for the combined plan's ADR 0008/0011 acceptance gate.

## Exact ADR custody

The reviewed candidate is `96cfa6b12481fa25c8e51c78055a05cfb5874705`.

The merged Precept main anchor is `d25bf98199f81663bacbc584e340452b693cbaff`. Local `HEAD` and the locally stored `origin/main` both resolved to that commit during inspection. No remote refresh was performed by this evidence-writing lane; a local remote-tracking ref is not a new remote verification.

`git ls-tree` at both commits and `git hash-object` over the current files returned:

| Document | Reviewed candidate blob | Merged-main blob | Current local file blob |
| --- | --- | --- | --- |
| ADR 0008 | `75deb4185d3b79419eeb99d1db9fdb1be554c68f` | `75deb4185d3b79419eeb99d1db9fdb1be554c68f` | `d5e305d20dae7f1cc9475c39d48eb260483488ee` |
| ADR 0009 | `ea44d39954b2fcadbc6a820e36d05c25a201a6c3` | `ea44d39954b2fcadbc6a820e36d05c25a201a6c3` | `b6cbf7b518f031e30104be6d094e6ac9e3a2ae9a` |
| ADR 0011 | `911c070cc154261ead3637e8b67ed8f673bb2c0c` | `911c070cc154261ead3637e8b67ed8f673bb2c0c` | `07d6a489d96aed8ecf69264966989ada23c3e4a3` |

The pair-3 review record's current file and merged-main blob are both `f0ec8f3f6f1880b912dff12885b919d4e46ebec9`. Its embedded candidate and ADR blob IDs agree with the actual candidate-tree objects.

The current ADR 0008 and ADR 0011 IDs above bind the locally accepted files. They are **not** the reviewed Proposed blobs. No distinct committed acceptance snapshot exists in the inspected history; acceptance-to-current is the same observed local snapshot, not an independently committed comparison.

### Exact reviewed/current differences

The following command returned no output for all three named ADR paths:

```sh
git -C /home/common/projects/templar/precept diff --no-ext-diff --unified=0 96cfa6b12481fa25c8e51c78055a05cfb5874705 d25bf98199f81663bacbc584e340452b693cbaff -- doc/adr/0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md doc/adr/0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md doc/adr/0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md
```

Separate `git diff --no-ext-diff --unified=0` comparisons from each of those commits to the current files returned exactly the following three hunks and no others:

```diff
--- doc/adr/0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md (reviewed / merged main)
+++ doc/adr/0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md (current local)
@@ -7 +7 @@ Date: 2026-09-03
-Proposed
+Accepted
--- doc/adr/0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md (reviewed / merged main)
+++ doc/adr/0009-migrate-adr-0008-legacy-pools-to-registry-native-successors.md (current local)
@@ -7 +7 @@ Date: 2026-09-07
-Proposed
+Accepted
--- doc/adr/0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md (reviewed / merged main)
+++ doc/adr/0011-disable-activation-shim-backstop-emissions-while-preserving-its-abi.md (current local)
@@ -7 +7 @@ Date: 2026-09-09
-Proposed
+Accepted
```

Classification:

- **Status:** ADR 0008 and ADR 0011 each have the single user-authorized local acceptance transition. ADR 0009 has the same single-line transition, but its authorizing context is not established here; do not include it in this lane's acceptance claim or silently publish it as authorized.
- **Linkage:** no linkage hunk exists. The reciprocal ADR 0008/0011 selection and ADR 0009 migration references are unchanged from the reviewed candidate.
- **Substance:** no decision, number, acceptance criterion, or other prose changed in these three comparisons. No substantive reviewed-versus-current drift blocker was found. This result comes from the exact diffs, not inferred equivalence or status normalization.
- **Existing historical wording:** ADR 0008 still says this revision requires fresh review before human acceptance; ADR 0011 still describes the previously unresolved emissions disposition and denies preservation of review for a changed bundle. These sentences were already in the reviewed blobs, not introduced by local acceptance. This record links the later pair-3 review explicitly instead of editing or treating that wording as a fresh implementation approval.

## Existing checkouts and baseline boundary

| Purpose | Path / identity (opening observations unless stated otherwise) |
| --- | --- |
| Precept evidence checkout | `/home/common/projects/templar/precept`, branch `chore/accept-adr-8-11`, HEAD `d25bf98199f81663bacbc584e340452b693cbaff` |
| Blend implementation checkout | `/home/common/projects/templar/blend-contracts-v2`; opening branch `main`, baseline HEAD `6c3bbb46f89653b13bec7c39e4b811161c82c765`; current implementation branch `adr/0008-0011-local-security-fork` as reported by Main |
| Pinned contract baseline | `6cf6dd1ca712ea3cf17a767fe49b7da01f80b909` |
| Local scratch root | `/tmp/adr8-local-20260913.RLSEWPiF` |
| Installed isolated CLI prefix | `/tmp/adr8-local-20260913.RLSEWPiF/tools` |

The supplied preflight classified the opening Blend fork HEAD versus the pinned baseline as docs-only. This lane's read-only `git diff --no-ext-diff --name-status 6cf6dd1ca712ea3cf17a767fe49b7da01f80b909 6c3bbb46f89653b13bec7c39e4b811161c82c765` also returned only paths under `docs/emergency-runbooks/`: modified `docs/emergency-runbooks/README.md`, seven Blend runbook relocations, a Blend README, and Templar runbooks/README. No repository-root README change was returned. This is a committed source-tree scope observation, not a build or Wasm-equivalence result; the two commits remain distinct custody identities.

### Baseline run completed before production edits

Main reports that, in `/home/common/projects/templar/blend-contracts-v2` at `6c3bbb46f89653b13bec7c39e4b811161c82c765`, with `/tmp/adr8-local-20260913.RLSEWPiF/tools/bin` first on PATH, the following sequence passed:

```sh
stellar --version
cargo --version
rustc --version
cargo fmt --all --check
make build
make test
```

The retained [complete baseline transcript](evidence/adr-0008/logs/baseline-build-test.log) records Stellar `22.6.0 (f4fd838380408dcad3e39f704e8d56d6408c63e2)`, Cargo `1.81.0 (2dbb1af80 2024-08-20)`, and rustc `1.81.0 (eeb90cda1 2024-09-04)`. It shows successful optimized contract production, the second build performed by `make test`, and `cargo test --all --tests`: 20 result summaries, all `ok`, totaling **481 passed, 0 failed**. The silent format-check success and complete command exit status are Main's execution report; no separate formatter banner is fabricated from the transcript.

Main installed Rust 1.81 and built the isolated Stellar CLI `22.6.0` with `--features opt` using Cargo 1.86 inside a `nix-shell` providing `openssl pkg-config cmake clang perl systemd dbus`. These are host-side CLI/compiler dependencies, not changes to Blend's contract dependency graph. The retained baseline run uses Cargo/rustc 1.81 as shown above. Main reports Blend build inputs remained unchanged.

The opening scratch inspection found only the tools directory; the following evidence was retained afterward. Paths below are relative to `doc/adr/reviews/evidence/adr-0008/`; SHA-256 values were computed from these retained files by this evidence-writing lane:

| Retained file | SHA-256 |
| --- | --- |
| [logs/baseline-build-test.log](evidence/adr-0008/logs/baseline-build-test.log) | `0ee5a53e6dc4e2db92b2aa844501749b13e4616c1c42d35821e23903aceb914d` |
| [baseline-artifacts.json](evidence/adr-0008/baseline-artifacts.json) | `fb8b001c66d4202ffa6aa3649474181f5c1ea29f7ab796eb5959ac3dd8769db2` |
| [baseline-build-input-hashes.json](evidence/adr-0008/baseline-build-input-hashes.json) | `dae0528b4469f16030ce749cda8d8c6f230d78951ba60b00cbde026e7912d52f` |
| [logs/baseline-pool-normal-dependencies.log](evidence/adr-0008/logs/baseline-pool-normal-dependencies.log) | `aeca5fb0f47a6aba742525436d30f8212896f726667183ca9ac37f5755812ba6` |
| [logs/baseline-backstop-normal-dependencies.log](evidence/adr-0008/logs/baseline-backstop-normal-dependencies.log) | `ce41ee5a910c5d4ebecae7b7d665a72f93b5ae94ea5864a57ff3ba289c55555e` |

The input record binds the baseline manifests, lockfile, toolchain, Makefile, and workflows. In particular, recorded `Cargo.lock` SHA-256 is `5ca629e352621b7fd68a8a8f657faa6e8349846d2179abc56ebcc2a806f9069c`. The two dependency logs retain the baseline normal dependency graphs; candidate graph equality has not yet been established.

The artifact record reports these baseline optimized outputs and specification hashes. Sizes also agree with the optimization transcript. This lane read the retained record, not independently rebuilt or rehashed Wasm:

| Contract | Optimized bytes | Optimized Wasm SHA-256 | Recorded spec SHA-256 |
| --- | ---: | --- | --- |
| pool | 57247 | `712486df0f76267e46aecc74c8c2d8ffb3bb72a2a3fdc80b301a60108ba0cfc8` | `e2d5e75029a044fdaf107a6124d2e3a346f5cf180e1dbbbff51dbd4b34a89d95` |
| backstop | 30691 | `eef956ba07f26f5f1186330a4f14a3923193ca1829a24eb6a526793e64bf6d91` | `a0bfed1ea59382ebf9573867eb177d0e1da958adac51e037f8f981ec69ed853e` |
| pool_factory | 3149 | `80abaf3c68fd15adade9f82165baa9b50d85800e497c99b45809e1fbd7397de2` | `28bfa6de93228ce3af8c08e4df13f014594d4c6a741834ee00c74fb922b8eda4` |

All three reported baseline sizes are below 131072 bytes. These are not candidate sizes or a base/fork comparison. Canonical compared specification bytes under `adr-0008-blend-fork-spec/` were not present at this update; recording baseline spec hashes alone does not satisfy the final spec custody/equality gate.

The green baseline satisfied Main's production-start gate. All three production source lanes have settled. Main subsequently repaired and exercised the retained ordinary scenarios; the full development workspace passes 486 tests across 20 suites. Remaining integration and final-verification obligations are distinguished below.

Use the existing checkouts and preserve unrelated dirty files, including the untracked duplicate ADR 0006. This lane neither changes ADR text/status nor edits Blend. It creates no new worktree, branch, acceptance commit, or toolchain/build configuration.

## Executed development checkpoints

These are actual command executions against the uncommitted development tree at their respective invocations, not a claim that a final source/evidence commit has been frozen. The [checkpoint receipt](evidence/adr-0008/development-checkpoints.json) records complete retained log hashes and sizes.

| Executed surface | Result | Complete transcript |
| --- | --- | --- |
| Migrated whole workspace, `cargo test --all --tests --no-fail-fast` | 486 passed, 0 failed; 20 suites | [Workspace log](evidence/adr-0008/logs/checkpoint-full-workspace-486-passing.log) |
| Public default and six-trap/lifecycle checks, each looping native and optimized Wasm | 5 tests passed | [Public smoke log](evidence/adr-0008/logs/checkpoint-public-default-and-six-traps-passing.log) |
| Backstop native unit suite, including retained no-data emission hooks | 115 passed | [Backstop log](evidence/adr-0008/logs/checkpoint-backstop-native-passing.log) |
| Kani runtime-shared disable-only reserve predicate | 254 checks, 0 failures; one harness | [Bounded-predicate log](evidence/adr-0008/logs/kani-disable-only-transition.log) |

The public price-collapse default witness checks exact debt-share burn, pool-owned collateral parking, supplier-rate loss, unchanged underlying balances, and event units/order. The migrated liquidation suite retains multi-asset partial/full fills, repayments, stale-auction deletion, restored-position restrictions, supplier default, and queued backstop withdrawals. Its zero-bid full-fill witness checks actual residual default rather than confusing a requested 100% fill with a 100% bid. Interest conservation uses a bound derived from the number of accrual/deposit floors, not an empirically widened tolerance. Per-operation events are captured before subsequent getters.

The predicate proof covers the actual pure reserve-transition helper only. It does **not** prove accounting arithmetic, the Soroban host, transactional rollback, or the complete protocol. This historical development checkpoint did not satisfy the later differential, generated-case, clean-producer, or implementation-review gates. The exact-commit checkpoint below supersedes its verification status without widening the bounded proof's scope.

## Local history and remaining gates

The requested Blend history is now recorded locally: production commit `23abc74` followed by the single test/support head `273a5131da1f7407887a4a40867c992790809f3d`. Production-only index blobs kept embedded test changes out of the first commit. The existing docs-only ancestry was preserved. Precept evidence is not yet committed; no remote publication is claimed.

The final exact-commit build/test, differential, 10,000-case property, unchanged build-input/dependency, ABI, factory-byte, size, and independent rebuild gates have now passed; receipts appear below. These remaining gates must not be represented as passed:

- Final evidence payload commit and fresh sequential independent security and correctness/maintenance acceptance reviews of the frozen source/evidence pair. Earlier development reviews and the historical Architect/Critic ADR pair do not substitute for final acceptance.
- The commercial comparison and explicit hard abandonment date, now due before publication by the recorded user exception.
- Published acceptance custody, remote payload verification, authorized and verified protected publication controls, PR review/CI, and evidence-before-source publication. Local worktree prohibition does not certify the plans' independent clean-producer or remote-replay requirements; their actual method and evidence must be resolved before claiming final compliance.
- All later deployed-authority-tree, policy/proxy/shim, canary, active-network size, operations, maintenance, migration, and activation gates.

**Publication gates are not satisfied. No push, PR, ruleset change, vendor-pin update, deployment, routing, or activation is authorized by this record.** The original evidence-writing lane performed only read-only custody work and recorded Main's baseline execution. Main subsequently updated this record with the retained, executed development checkpoints above; no final candidate acceptance is claimed.

## Local source freeze and issue history

This update records completed development work and the local source freeze, not final acceptance. The exact candidate is `273a5131da1f7407887a4a40867c992790809f3d`; [exact-local-candidate.json](evidence/adr-0008/exact-local-candidate.json) identifies three independent local clones used for the verifier, rebuild, and generated-case consumer. Local clones replace forbidden worktrees and remote cloning for this phase. They do not satisfy remote custody or publication requirements.

After pinned Rust formatting, `rustup run 1.81 make test` passed from the settled working tree; see [final local build/test log](evidence/adr-0008/logs/local-freeze-final-passing-build-test.log). The named ignored differential test then passed against the rebuilt artifacts; see [local differential log](evidence/adr-0008/logs/local-freeze-differential.log). The latter generated corrected stock add/remove-reward evidence with actual `result=()`, and exercised the executable-role TTL-swap negative control. Final exact-commit replay remains a separate gate.

All Blend manifests, Cargo.lock, Makefile, and toolchain bytes matched the pinned baseline in [input comparison](evidence/adr-0008/local-freeze-input-comparison.json). Normal pool/backstop dependency trees matched after replacing only the respective source-root path; see [dependency comparison](evidence/adr-0008/local-freeze-dependency-comparison.json) and its four `local-freeze-*-normal-dependencies.log` inputs.

### Problems encountered and their disposition

| Problem | Classification and disposition |
| --- | --- |
| A development lane used the wrong Rust toolchain and contaminated lockfile bytes | Tooling/process failure. Restore the verified lockfile rather than update dependencies; use explicit `rustup run 1.81` with Stellar CLI 22.6.0. Current input comparisons are byte-equal. The rejected lockfile is historical diagnostic material, not a build input. |
| Legacy fixtures called disabled emissions and reward-zone methods or initialized nonzero backstop take | Planned integration migration, not permission to weaken the fork. Use emissions-free, economically matching fixtures and zero take; replace forbidden success-only scenarios with exact rejection/no-effect coverage while retaining ordinary lifecycle coverage. |
| Factory happy-path tests still supplied nonzero take | Test adaptation required by the new pool constructor. Three test-local constants were changed, including a max-position error fixture so it still reaches its intended error. Factory runtime source and optimized bytes remain unchanged in development comparisons. |
| A sealed-reserve stock-positive control expected the wrong error during an intermediate repair; its queue-stage observation was absent in an older runner | Harness/versioning defect. Correct the stock/fork control and rerun with the retained `fork-after-queue.json` observation. Do not attribute an old runner's behavior to current source. |
| Auction fixtures advanced timestamp without advancing ledger sequence | Fixture defect. Auction maturity is sequence-based and creation starts at sequence plus one. Mature existing final-fill auctions consistently and assert the inclusive 400-block boundary. |
| Generated liquidation-fill expectations confused self-fill, missing auctions, and status gating | Oracle defect. Self-fill rejects before auction lookup; a non-victim missing-auction fill reaches the stock trap, and request type 6 is not gated by the assumed status check. Correct caller identities, exact outcomes, and rollback checks rather than change production to match the test. |
| An attempted liability-emissions discriminator predicted the wrong final-fill outcome | Oracle defect. Neither a liability-token heuristic nor broad acceptance of error 1200 is adequate. Replace it with a pre-state residual-collateral emissions guard and explicit independent loss/custody/reserve/event expectations. Retain deterministic failing prefixes. |
| The initial outcome-tolerant final-fill oracle accepted error 1200 without proving its precondition | Security-review finding in verification code. Strengthen the discriminator and success accounting checks; do not count the weaker earlier run as final evidence. |
| Failed-call event checks confused SDK ledger snapshots with full snapshots and invocation-local events | Harness API/observation defect. Use actual SDK types and full snapshot event records, distinguishing failed-call diagnostic copies from committed contract events. Several compile errors during this repair were fixed before the targeted and generated tests passed. |
| Source mtimes suggested optimized Wasm might be stale | Investigated custody concern, not a confirmed production regression. Rebuilding from the then-current source reproduced the earlier optimized bytes exactly. Mtime alone was insufficient; exact-commit producers still remain required. |
| Differential code-row normalization could mask swapped pool/backstop TTLs | Maintenance-review finding in the comparator. Keep distinct executable roles and add a negative control. The first swap fixture had equal TTLs, making the mutation a no-op; create distinct starting TTLs and explicitly assert that the comparator detects the swap. |
| Common-domain raw and normalized observations were not retained | Evidence-custody gap. Persist both sides and normalized comparison outputs, not only a passing console message or divergence controls. Final durable archival must use the exact-commit run. |
| An earlier 10,000-case process disappeared without a retained terminal verdict | Lost verification evidence. Treat that run as unverified, not passed and not a proven timeout cause. Relaunch with supervised lifecycle and durable tee logging. A later wait timeout did not stop the live runner, which continued appending successful cases. |
| Artifact metadata labeled dirty-tree output with checkout HEAD as if that were its source commit | Provenance defect. Rename it to `checkout_head_at_build` and explicitly mark dirty-tree binding. The new candidate manifest binds an actual local source commit; independent proof still must complete. |
| Stock add/remove-reward evidence recorded constructed vectors as returns of void methods | Precommit evidence defect. Record unit `()` and retain reward-zone state assertions separately. The corrected differential run retained `result=()` for both calls. |
| A final-fill regression checked persistent storage for an auction held in temporary storage | Vacuous test assertion. Replace the hand-built check with the public `try_get_auction` failure after successful fill, preserving the earlier failed-fill auction-retention assertion. The corrected regression passed in the full suite. |
| Redundant Wasm hash computation, unused private arguments/mutability, formatting drift, and executable Rust-source modes | Contained maintenance cleanup. Reuse the already computed artifact hashes, remove seven unused helper arguments and five unused mutable bindings, apply pinned rustfmt, and retain ordinary source modes. No runtime dependencies or suppression attributes were introduced. |
| The hash-reuse edit duplicated a struct field and dropped its label | Integration mistakes in a repair. Actual pinned compilation detected E0062 and E0063; restore one hash field and the label, then rerun the full suite and differential. |
| Read-only agents produced unsupported details or violated the no-validation instruction | Review/process failures, not accepted evidence. Reject fabricated file/hash/coverage claims and re-ground in actual files. One repair agent ran unauthorized toolchain checks and reported misleading environment conclusions; Main's explicit pinned builds, not those claims, determine the verdict. |

The precommit Ponytail reviewer closed all five reported findings after the repairs. This is a development/precommit review, not the plan's final exact-source/evidence-pair security and correctness approval. Known dormant stock internals and public parameter names were deliberately retained: deleting them would widen the six-body/ABI-preserving boundary. No new suppression attributes were used to hide them.

### Plan conformance distinctions

- **Authorized timing/method exceptions:** local-only operation in existing checkouts; no new worktrees, pushes, PRs, deployment, or vendor-pin movement; commercial comparison and owner-approved hard abandonment date deferred to before publication. The original authority section records why these exceptions are permitted for this phase.
- **Combined versus isolated plan:** ADR 0008 includes ADR 0011. Pool Wasm changes are expected for the combined fork, so the isolated backstop-only plan's pool-byte-equality criterion is inapplicable. Factory-byte and all public-spec equality remain required.
- **Stronger verification is not a weakened contract:** full event snapshots, role-specific executable normalization, corrected pre-state oracles, public deletion checks, and stock-positive controls repair evidence gaps; they do not authorize additional production behavior.
- **Not waived:** exact committed-source replay, independent artifact equality, 10,000 generated cases, complete final evidence custody, and final sequential reviews. Remote payload custody, protected publication, commercial approval, and later deployment gates remain outstanding under the local-only boundary.

### Exact-commit verification checkpoint

The independent verifier clone at `273a5131da1f7407887a4a40867c992790809f3d` passed pinned formatting and `make test`: **490 passed, 0 failed, 1 intentionally ignored differential test across 22 suites**. The differential test was then invoked explicitly and passed: **1 passed, 0 failed**, 243.32 seconds; [executed differential transcript](evidence/adr-0008/logs/exact-local-differential-executed.log). All 532 raw/normalized common and stock/fork divergence artifacts are retained in the [exact differential archive](evidence/adr-0008/exact-local-differential.tar.gz), with [member-level SHA-256 manifest](evidence/adr-0008/exact-local-differential-manifest.json). Archival replay compared every member to its original bytes and confirmed all 84 normalized common observation pairs byte-identical.

All three clean producers independently built byte-identical optimized artifacts. Pool: **53,921 bytes**, SHA-256 `351257924a5499a6c7c947a7c81e27ffce51ed013650cd52ce188d50cff5c96d`; backstop: **22,701 bytes**, `6bd13007c020b617c43c718c2ad3d2d13605d4d42fa8f739da2293f8a4e293a0`; factory: **3,149 bytes**, `80abaf3c68fd15adade9f82165baa9b50d85800e497c99b45809e1fbd7397de2`. Factory bytes equal baseline; all three contract specifications equal baseline byte-for-byte and are retained in the canonical [spec directory](adr-0008-blend-fork-spec/). All optimized artifacts are below 131072 bytes. See [exact artifact receipt](evidence/adr-0008/exact-local-artifacts.json).

Two further acceptance-runner issues were caught rather than counted as success:

- The standalone harness's nested local Blend clone was auto-selected into the outer Cargo workspace, breaking inherited dependency resolution. Add `exclude = ["blend-src"]` only to the standalone harness manifest; leave every Blend manifest and lockfile unchanged. `cargo metadata --locked` then showed only the harness as its workspace member, and the actual property test launched.
- An incorrect differential filter against the property-only harness exited zero while executing **zero tests**. That attempt, retained in `exact-local-differential.log`, is rejected evidence. The actual named ignored test in the exact verifier clone produced the one-test passing transcript cited above.

The exact-commit **10,000-case property run passed**, supervised as `adr8-exact-property-10000`: seed `9110008`, 1 test, 0 failures, 28184.17 seconds. The terminal line records Wasm-registered fixtures with at most 64 operations, 4 users, 3 reserves, amount selectors 0–5, and time selectors 0–8. The candidate receipt binds its arguments, harness source hashes, and complete durable log hash. The separate development run also passed 10,000 cases in 28354.85 seconds; it is retained as development evidence, not substituted for the committed-source gate. Final evidence commit and sequential acceptance reviews remain pending.
