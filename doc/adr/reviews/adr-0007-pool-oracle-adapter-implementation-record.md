# ADR 0007 pool oracle adapter implementation record

Date: 2026-09-22

Status: **LOCAL GATE, CI, AND DISPOSABLE TESTNET CANARY COMPLETE — independent review,
ADR 0007 human acceptance, and production/mainnet release decision remain blocked.**

This record describes what was built and what is still owed. It changes no ADR status. ADR 0007
remains `Proposed` and its file is unmodified by this work; `adr accept` is human-only.

The dependency, surface, and semantics sections describe the implementation in this branch.
Main supplied the local and CI measurements; this documentation lane inspected the committed canary
receipt and bundle notes without rerunning commands.

The raw and optimized Wasm artifacts are under
`contracts/pool-oracle-adapter/target/wasm32v1-none/release-soroban-checked/`.

Source citations refer to the inspected proxy governance, runtime, kernel, and reference adapter
under `../contracts`. Passing gates are not acceptance and not a production release decision.

## Scope and the location divergence

[ADR 0007](../0007-phase-1-permissionless-pool-registry-protected-oracle-and-optional-isolated-markets.md)
specifies one thin multi-asset SEP-40 `PoolOracleAdapter` over the existing Soroban proxy runtime,
with no source IO, aggregation, independent freshness policy, or circuit-breaker evaluation.

ADR 0007's text still places that contract in `../contracts/contract/proxy-oracle/soroban`
(its Context, Decision, and Verification-strategy sections). The adapter was instead implemented in
**Precept**, at `contracts/pool-oracle-adapter`, by explicit user direction for this slice. That is a
recorded divergence from the Proposed text, not an amendment of it: relocating the contract
permanently needs a superseding ADR. Nothing under `doc/adr/0*.md` was edited.

Everything else in ADR 0007 (registry constructor checks, pool construction, backstop zero-take,
issuance/emissions disposition, the production manifest) is out of scope here and remains unimplemented.

## Pinned dependencies

| Item | Pin |
| --- | --- |
| `templar-proxy-oracle-soroban-common` (runtime) | `Templar-Protocol/contracts` rev `6477b1d7ef0e17ed4ba6e6fcfa9493d049cca4eb` |
| `templar-proxy-oracle-soroban-contract` (dev, `testutils`) | same rev |
| `soroban-sdk` | declared `=25.3.0` (`alloc`; `testutils` in dev); lock resolves `25.3.0` |
| `soroban-sdk-macros` (lock) | `25.3.0` |
| `stellar-access` (lock) | `0.7.1` |
| Rust | `rust-toolchain.toml` channel `1.89.0`, target `wasm32v1-none` |
| Stellar CLI (qualification) | `26.0.0`, supplied through `STELLAR_BIN` |

The crate is standalone (own `[workspace]`, `publish = false`, `GPL-3.0-only`) and defines the proxy
package's `release-soroban` / `release-soroban-checked` profiles locally (`Cargo.toml:40-57`; only
`release-soroban-checked` is built by the gate).

## Immutable v1

The adapter exposes only its constructor, `config`, `interface_version`, `storage_version`,
`extend_ttl`, and the seven `PriceFeedTrait` methods. There is no owner, setter,
transfer/accept/renounce, decommission, upgrade, token, or source method.

Evidence that an adapter-side authority surface would be dead:

- the proxy governance contract stores exactly one runtime target: its `__constructor` rejects when
  `DataKey::ProxyOracle` already exists (`governance-contract/src/lib.rs:50-52`) and line 58 is the
  only write of that key in the crate — no runtime-target setter exists;
- every governance action resolves that single key (`engine.rs:71-73`) and dispatches to it,
  including `TransferOwnership`, `AcceptOwnership`, `RenounceOwnership`, and `Upgrade`
  (`engine.rs:74-220`);
- therefore no governance path can authorize an adapter upgrade or ownership change, so such a
  surface would be unreachable authority that still costs Wasm against the 32 KiB budget.

`deployment_owner` is a **construction-time provenance snapshot**: the constructor requires the
parent's `get_owner()` to equal it before the single config write, and the stored value is never an
authorization input.

That is a **second, explicit divergence** from ADR 0007's Proposed text, which the same superseding
ADR must reconcile. ADR 0007:148 says the adapter *may* retain an owner-authorized same-address
upgrade surface **and** that "its owner must equal the proxy runtime's governance owner"; :154 and
acceptance criterion 11 (:228) make registry genesis enforce that owner equality when it checks the
adapter. A v1 adapter with no owner role cannot satisfy a live owner-equality read, and a snapshot
taken once at construction does not. Changing the adapter later therefore means deploying a new
address under a reviewed decision, which is also what keeps `decimals`, `resolution`, `base`, and
`parent_oracle` from silently rescaling pool economics.

## Inherited latest-versus-history semantics

The adapter performs no price policy of its own; it mirrors the parent runtime:

- `aggregated_latest` (`contract/src/lib.rs:420-431`) returns the cached price only if it is
  `Accepted` and no older than that asset's `max_age_secs`. `lastprice` maps it through
  `normalized_to_sep40` and returns `None` on parent error, absence, non-acceptance, or staleness.
- `aggregated_history` (`contract/src/lib.rs:433-450`) requires nonzero `records`, a loadable breaker
  set that is not blocking, and nonempty history; it never consults latest freshness. Accepted
  history is bounded by the asset's `history_len` ring
  (`kernel/src/proxy/circuit_breaker/set.rs:241-242`, reported as `history_len:
  breakers.accepted_history().capacity()` at `contract/src/lib.rs:388`) and truncated to the newest
  requested records (`contract/src/lib.rs:448-449`).
  The divergence this leaves: latest staleness and a non-blocking failed refresh (`ResolveFailed` /
  `SourceUnavailable`) hide `lastprice` while accepted history stays readable through
  `price`/`prices`.
- A manual trip behaves differently. It makes `is_blocking()` true, blocking history immediately
  (`kernel/src/proxy/circuit_breaker/set.rs:353-357`), and clears latest in the same call:
  `set_manual_trip` runs through `with_breakers` (`contract/src/lib.rs:341-343`), which ends in
  `invalidate_cache(env, asset)` (`contract/src/lib.rs:106-124`, call at :123) removing the cache
  entry (`contract/src/storage.rs:52-55`). So `lastprice`, `price`, and `prices` all return `None`
  at once, with no refresh required.
- Refresh while tripped returns `Blocked`; untrip followed by an accepted refresh restores all three
  reads. The crate's regression
  `real_runtime_breaker_trip_clears_every_read_and_untrip_refresh_restores_it`
  (`contracts/pool-oracle-adapter/src/tests.rs`) asserts that sequence plus the failed-refresh
  history divergence, and runs inside the native suite reported under Gates and artifacts.

`price`/`prices` use `aggregated_history` only and never consult `aggregated_latest` to synthesize
freshness. Inheritance versus Precept choice is worth separating: `price` pins its own lookup window
at `MAX_HISTORY_RECORDS = 32` (`contracts/pool-oracle-adapter/src/lib.rs:150`), so a bucket older
than the parent's retained tail is simply absent; `prices` forwards the caller's `records` verbatim
to the parent (`src/lib.rs:167-168`), which then truncates to that retained tail. `price` rejects a
non-bucket-aligned timestamp and scans newest-first for an exact bucket match; `prices` converts
all-or-none, oldest-first, deduping a repeated bucket to its newest entry. Every price read first
confirms the parent's `source_base()` still equals the
configured `base` (drift yields `None`). Conversion reuses the reference adapter's extra guard: a
non-zero mantissa projecting to `price == 0` fails. `assets()` returns the parent's
`registered_assets()` verbatim — same values, same order, no sort, filter, or cache — and traps on
parent failure because SEP-40 has no fallible return. Budget exhaustion is an unrecoverable abort,
never mapped to `None`. `extend_instance_ttl` on reads is the only permitted state effect.

## Gates and artifacts

Run from the default `devenv shell`; the cargo and Stellar invocations run under the
`pool-adapter-env` wrapper, which selects Rust 1.89 for this crate while the default shell stays
1.81 for `tools/abtest`. `build` and `optimize` use the Stellar CLI as the artifact producer, so
`STELLAR_BIN` must point at a 26.0.0 binary — devenv's own `stellar` on `PATH` is still 22.6.0:

```sh
devenv shell -- bash -c 'STELLAR_BIN=<path-to-stellar-26.0.0> just -f contracts/pool-oracle-adapter/justfile audit-ready'
```

`audit-ready` runs `preflight`, `fmt`, `clippy`, `test-native`, `build`, `optimize`,
`test-optimized-wasm`, `spec-check`, `size-check` (that justfile is the ToolingCI lane's). The
central local run reported by Main **passed** every step: `preflight` (Stellar CLI 26.0.0,
cargo/rustc 1.89.0, `wasm32v1-none` std), `cargo fmt --check`, `clippy -D warnings`, native
`cargo test --locked` at 11 passed / 0 failed, build and optimize, the deployed-optimized-Wasm
suite at 4 passed / 0 failed, `SPEC CHECK PASSED: exactly 12 functions and 4 UDTs with frozen
shapes`, and the size check at 13562 / 32768 bytes.

The spec gate validates the referenced UDTs — `PoolOracleConfig`, `Asset`, `PriceData`,
`AdapterError` — as well as the 12 functions.

Artifacts (Stellar CLI 26.0.0 producer, Rust 1.89.0, profile `release-soroban-checked`, target
`wasm32v1-none`; producer binary SHA-256
`70a548f0c171aa498277f7a31c51a2601f07102d3c4d87aa4b6b98f8897ac734`):

| Artifact (crate-relative) | Bytes | SHA-256 |
| --- | --- | --- |
| `target/wasm32v1-none/release-soroban-checked/pool_oracle_adapter.wasm` | 15326 | `1b9a61d82e5c021bc898c9989821de9eae9f49a5fb15e1b8c2032c20643ef21e` |
| `target/wasm32v1-none/release-soroban-checked/pool_oracle_adapter.optimized.wasm` | 13562 | `bbdeee7b29d0b6e86af86cbf4ff1e9b76c34a2d27b088134a61151d46e3e17fb` |

The 32 KiB ceiling is the proxy package's own adapter budget (ADR 0007's consequences section, and
`adapter_size_limit := "32768"` in the parent `contract/proxy-oracle/soroban/justfile:44`).

The path-scoped `.github/workflows/pool-oracle-adapter.yml` ran on draft PR
[Templar-Protocol/precept#16](https://github.com/Templar-Protocol/precept/pull/16) (branch
`feat/pool-oracle-adapter`): **[run 35727243602](https://github.com/Templar-Protocol/precept/actions/runs/35727243602)**
reports "Audit-ready gate" PASS in 3m6s. Every step is green — checkout, Rust 1.89.0 +
`wasm32v1-none`, `just`, Stellar CLI 26.0.0, the `pool-adapter-env` shim, `just audit-ready`, and the
lock assertion of a single `soroban-sdk 25.3.0` / `soroban-sdk-macros 25.3.0` /
`stellar-access 0.7.1`. CI's gate output matches the local run: native 11 passed / 0 failed,
deployed-Wasm 4 passed / 0 failed, `SPEC CHECK PASSED: exactly 12 functions and 4 UDTs with frozen
shapes`, `Optimized WASM: 13562 / 32768 bytes`.

Reproducibility is measured, not asserted: the gate prints the CLI `PRODUCER:` sha256 and an
`ARTIFACT:` size/sha256 line for both artifacts (`justfile:31,68-70`), so the CI log carries the
identity of what it built. Main's CI report records those lines matching the local run exactly —
producer `70a548f0c171aa498277f7a31c51a2601f07102d3c4d87aa4b6b98f8897ac734`, raw 15326 /
`1b9a61d8…`, optimized 13562 / `bbdeee7b…`. The workflow has no hash-comparison step; byte-equality
comes from comparing the printed digests across the two runs.

Both triplets (native 11/0, deployed-Wasm 4/0, 12 functions + 4 UDTs, 13562 bytes) bind to the
post-dead-guard-removal tree: the supersession note in the canary state records that the earlier
13576-byte / `6878437f…` build still carried a dead guard in `src/lib.rs` and was superseded before
upload, so that digest belongs to no CI or deployed artifact.

Operational note: the first CI run was red because `permissions: {}` left the token unscoped and
checkout could not clone the private repository; the workflow now requests `contents: read`.

## Disposable testnet canary

The canary ran on the qualified bytes: `receipt.json` records
`artifact_provenance.pool_oracle_adapter` as the 13,562-byte
`bbdeee7b29d0b6e86af86cbf4ff1e9b76c34a2d27b088134a61151d46e3e17fb` optimized Wasm — the same artifact
as the Gates table — with `steps` identities → `deploy_oracles` → `proxy_adapter` → `pool` →
`activate`. Its `pending_reconciliations` holds only two closed entries: the mock SEP-41 upload that
testnet already had installed (no transaction submitted), and `deploy.runtime`, whose transaction was
recovered from public Horizon tx `df551060…`. The adapter's own upload was a real transaction,
`be5ed056…`, and its deploy `f4a370a1…` installed it at `CAR3CRG…`; pool creation was `a2b1c310…`.

Two distinct `Asset::Stellar` reserves — a 7-decimal collateral wrapped asset and the 7-decimal
stable `CADNYW…` — were served by that one adapter instance. `assets()` returned the parent's
registered assets and `base()` / `decimals()` / `resolution()` came from stored config; the collateral
reserve carried `util=5000000`, `reactivity=20`, `max_util=9500000`, `supply_cap=1e18`.

Proven on testnet:

- funded healthy lifecycle: collateral supply `10000000000`, borrow `100000000` at 20% LTV,
  pre-liquidation health factor 1.050 (receipt `liquidation.prefill_offset: 209`, collateral base
  `10500000000`, liability base `9473684244` at 7 decimals);
- the exact liquidation fixture: a `200000000000` (20,000 token) lot against a `8999999658` bid, 95%
  of the `9473684244` base — creation `5b3bd262…`, then a nonzero-bid **fill** in `cb381d8f…`;
- governance circuit-breaker trip, rejection while tripped, and recovery: with the trip active the
  pool saw `lastprice` return void and both `submit` and `new_auction` were rejected
  (`evidence_kind: simulation_rejected`); trip transaction `b59d3f61…`, recovery borrow `f7ed5e21…`;
- the retained divergence: latest staleness and a non-blocking failed refresh (`ResolveFailed` /
  `SourceUnavailable`) hide `lastprice` while accepted `price` / `prices` history keeps serving;
- permissionless `extend_ttl`, transaction `05a8e8fa…`.

**Operator finding, kept deliberately:** with no accepted price, `submit` and `new_auction` fail as a
Wasm `UnreachableCodeReached` trap (pool `CA24OLH2…`), **not** as the pool's timestamp error `#1210`.
These are two distinct fail-closed classes; both reject with no state change, and this record does
not merge them.

### Successor Blend consumer target

The canary finding above remains historical evidence bound to the adapter target
`5f40291f8f395cd61b6c67103543977817bfe5d2` / tree
`367c089b57b04ccc160a50b06396a2effc0b9030` and the deployed pool bytes recorded by
that canary. It has not been rewritten or relabeled.

[Templar-Protocol/blend-contracts-v2#7](https://github.com/Templar-Protocol/blend-contracts-v2/pull/7)
supersedes the pool-side failure class for a returned SEP-40 `lastprice` `None`:

- reviewed base: `7b63bafd459d992a3bcc4ded71fe2ce8bc86e734` / tree
  `33c3026c08f4ba3389bf6e7c503c9fea72492729`;
- behavior commit: `2eacdc6`;
- PR head: `acdc3208baf71ad14a9c3db7ac2b185fc7fc6b7d` / tree
  `fbf0dd5f5111372f78ac1c0cddaa7a81942e0d3d`;
- optimized pool Wasm: `441e95a552c7397ce65ba2cef7332e02ec85ee0d3da1d944d587702c21266c6b`
  (base `0594a17e9aed0258ec833e9a30edd22c3d00712299ab67cf4afd039275e45999`);
- public pool XDR unchanged at
  `e2d569e8851317e6b36971f38d615e40b1bfb0286499ea1b540eddb79e8bb853`;
- production Rust scope: 5,125 → 5,127 NSLOC under the retained `scc` 3.7.0
  exclusions.

On the successor target, health-dependent `submit` and user-liquidation
`new_auction` return pool contract error `#1210` when the adapter returns `None`.
Native and optimized-Wasm regressions assert the exact error, complete ledger /
TTL / token / allowance equality, and no committed success event. Restoring the
old `unwrap_optimized` behavior turned those regressions red with host
`InvalidAction`; restoring the successor change returned them green. The full
native suite passed 383 tests with 1 ignored, and the separate differential gate
matched every common-domain step and detected all 11 negative controls.

This changes the pool consumer, not the adapter bytes or configuration. `#1210`
still combines multiple `None` causes — including missing or stale accepted
latest data, blocked/source-failed parent reads, active-config/base mismatch, and
conversion failure — so adapter and parent-runtime telemetry remain the
root-cause authority.

Bundle: kept **out of tree** by decision — the 3.9 MB raw canary corpus does not belong in this
repository. Two verified copies exist on the workstation: `/data/backups/pool-oracle-adapter-canary-2026-09-22/`
(durable) and `/tmp/pool-adapter-canary/` (scratch, reap-prone). Identity is this record's anchor:
1,137 hashed entries, manifest SHA-256
`954d2e294ac308105bd17ef7f7628162d49be5a58ac4ed87fc77da7a204c8fe9`, re-verified 1,137/1,137 with zero
failures against the archived copy. The throwaway CLI home, the pinned build trees, and the
9 `artifacts/*.wasm` inputs (pinned by SHA-256 in `artifacts/manifest.json`) were excluded by design and
named in the bundle's `BUNDLE-NOTES.md`; a seed scan over exactly the hashed files found nothing.
`receipt.json` seals 123 transaction records — every one carrying a real 64-hex hash — plus 2
`simulation_rejected` records with no hash and an unchanged-state proof, 5 `wasm_preinstalled` uploads
that submitted nothing, 6 Friendbot funding requests, and 2 closed pending reconciliations recovered
from the public chain. Disposable-testnet evidence only — not production readiness.

## Pending

- **Independent implementation review** — outstanding.
- **ADR 0007 human acceptance** — still `Proposed`. The two divergences above (crate location, live
  owner-equality check) need a superseding ADR, and the slice leaves the remaining Phase 1
  acceptance criteria untouched.
- **Production / mainnet release** — blocked on that review plus a separate, explicit release
  decision.
