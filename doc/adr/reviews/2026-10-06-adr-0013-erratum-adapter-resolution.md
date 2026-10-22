# ADR 0013 erratum — deployed adapter resolution is 1, not 300 — 2026-10-06

## Subject

- ADR: `doc/adr/0013-bound-original-spoon-testnet-mock-price-keeper-operations.md` (Proposed, dated 2026-10-05).
- Frozen candidate: commit `5d7777d078c654e7f76e56773ac8d8f494cb41f5`, blob `aabee036717eca593f8394cc3fbfec851882b955`.
- Review record: `doc/adr/reviews/2026-10-05-adr-0013-5d7777d.md` (CONSENSUS APPROVED, one pair).

This record corrects one factual premise of the candidate. It is evidence about the candidate and the keeper, not part of the reviewed ADR bytes; the frozen blob is not edited (ADR 0001: agents never edit reviewed records; corrections are dated, hash-referenced filings). The ADR remains Proposed and this erratum is not human acceptance.

## The incorrect premise

The candidate states the adapter projects timestamps into 300-second buckets for this market:

- Context: "The adapter deliberately buckets timestamps by its configured resolution … This original manifest uses 300 seconds." (line 17)
- Decision 2: "Require … its timestamp to equal `raw_timestamp - raw_timestamp % resolution` … a fresh selected timestamp such as 1800000020 may correctly appear as 1800000000 at resolution 300." (line 28)
- Acceptance: battery coverage listed as including "300-second adapter projection". (line 70)

The bucketing mechanism is real — `contracts/pool-oracle-adapter/src/lib.rs:217,221-222` floors `aggregated_latest` timestamps with `bucket_timestamp(timestamp, config.resolution)` — but the divisor is the **adapter's own immutable constructor config**, not the manifest's oracle resolution. The market deploys the two at different values:

- `tools/abtest/src/deploy.rs:1539-1540`: "the pool's price adapter — governance-owned, reading only the proxy runtime, resolution 1."
- `tools/abtest/src/deploy.rs:4770` (market config): `[oracle] … resolution = 300` and separately `[adapter] … resolution = 1`.
- `tools/abtest/src/deploy.rs:3439,3456-3458`: the deployer constructs the adapter with `cfg.adapter.resolution` and refuses unless it reads back that value.

At resolution 1, `bucket_timestamp(t, 1) = t`: the adapter echoes the raw accepted timestamp. Live confirmation on the spoon market (adapter `CAL44IMB252XGNFY3O3B6HGN6OGLOUVPNGS6SZ7D64P7LJGGN7IPYKVI`, read-only query, 2026-10-06): `lastprice` returned timestamp `1791284147`, and `1791284147 % 300 = 47 ≠ 0` — not a 300-second bucket.

Consequently the Decision-2 equality is unsatisfiable for the deployed adapter, and the "1800000020 may appear as 1800000000 at resolution 300" scenario cannot occur in this market.

## Reviewer blind spot (stated without blame)

Both reviewers verified the bucket predicate's internal coherence against the candidate's own premise and quoted adapter flooring at `lib.rs:187-222`; neither compared the premise against the deployed adapter's configured resolution. The review's evidence list already carried the distinguishing fact — "resolution 300" as a *manifest* property (line 80) — and disclaimed "These are offline evidence, not live-cycle proof" (line 79). The live end-to-end gate (ADR lines 74) would have caught this; it had not run.

## Operational consequence

Keeper commit `f132c54` (2026-10-05 20:30 BST) implemented the ADR predicate verbatim: both `_verify_adapter` call sites passed `resolution=manifest["oracle"]["resolution"]` (300). Every cycle thereafter failed at `refresh-adapter-readback`. Because one readback failure retains the unresolved marker (Decision 3, fail-closed by design), the first failure at 09:02:31 BST on 2026-10-06 blocked all subsequent timer cycles until operator reconciliation at approximately 11:57 BST — a roughly three-hour working-hours outage. Custody behaved exactly as designed; the premise was wrong.

## Corrected requirement

For this market the adapter readback must be verified at the adapter's own resolution, 1, where bucket equality degenerates to exact raw-echo equality:

- `tools/abtest/scripts/refresh_spoon_prices.py:32`: `ADAPTER_RESOLUTION = 1`, used at both `_verify_adapter` call sites (lines 634, 647) in place of the manifest oracle resolution.
- All other Decision-2 checks are retained unchanged and still bind: raw-accepted-timestamp membership in this cycle's publications, max-age/future-drift bounds, and expected-median price equality. The custody value of the ADR is unaffected; only the bucket-equality clause was void.
- The manifest's `oracle.resolution = 300` remains meaningful source-side configuration only.
- Regression test `test_full_cycle_verifies_adapter_at_adapter_resolution` pins the call-site resolution to the literal `1` through a full mocked cycle, asserting all twelve publishes and four refreshes are marker-wrapped; it fails under resolution 300 and passes under 1. `python3 -I tools/abtest/scripts/test_refresh_spoon_prices.py` exits 0 with `refresh_spoon_prices: offline boundary checks passed`.
- Keeper fix commit: `7097dc1` ("abtest: verify keeper adapter readback at the adapter's own resolution"), local on `feat/protected-market` at the time of filing; unsigned, matching branch practice.

Post-fix live observation on 2026-10-06: two green cycles recorded in the journal (one manual run, then the timer cycle completing at 12:10:40 BST), all four adapter `lastprice` quotes non-null and fresh, timer re-armed.

## Effect on acceptance

A human accepting ADR 0013 should accept Decision 2 as corrected here (adapter readback at resolution 1, raw-echo semantics). If instead the intent was a genuinely bucketed adapter, the deployed market configuration — not the keeper predicate — is the artifact that must change, under its own decision.
