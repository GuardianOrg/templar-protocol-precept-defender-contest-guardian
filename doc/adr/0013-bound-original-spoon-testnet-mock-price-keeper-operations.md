# 13. Bound original-spoon testnet mock-price keeper operations

Date: 2026-10-05

## Status

Proposed

This record covers pre-existing uncommitted tooling and an already-installed host-local service. The implementation and installation preceded this ADR effort; this is a review-before-publication boundary, not a claim that the pre-code process was followed. Publication or consensus review is not human acceptance or authorization for a live run. Only a human may accept this decision.

## Context

Frontend development uses the disposable original `precept-abtest-evidence/market-live-20260930` market. Periodic timestamp renewal makes its protected oracle usable during working hours without changing synthetic mock price levels. These values are not external market observations. The README describes three sources, four assets, the working-hours schedule, and the generic journal's 256-coordinate limit (`tools/abtest/README.md:353-396`).

The dedicated script pins the original run, CLI/signer paths, exact testnet endpoint/passphrase, public admin, manifest SHA-256 `b02ef2a25cf0169d612d997c7e7e6307e715836688445b2149001181243b1b3f`, deployment ledger `4951021`, three sources, and `weth/usdc/xlm/stable`. It checks the manifest, signer public identity, RPC network and registered assets before publication (`tools/abtest/scripts/refresh_spoon_prices.py:19-41,239-292,550-564`). These checks do not prove current deployed Wasm provenance or exclusive on-chain authority.

The adapter deliberately buckets timestamps by its configured resolution (`contracts/pool-oracle-adapter/src/lib.rs:211-222`). This original manifest uses 300 seconds. The proxy dependency is pinned to contracts commit `6477b1d7ef0e17ed4ba6e6fcfa9493d049cca4eb` (`contracts/pool-oracle-adapter/Cargo.toml:17,21`). Its MedianLow selects one source price and retains that source's timestamp; the keeper must not confuse the adapter's rounded timestamp with the raw accepted quote timestamp.

ADR 0007 remains Proposed and concerns the production registry and canonical protected adapter, including fail-closed liveness (`doc/adr/0007-phase-1-permissionless-pool-registry-protected-oracle-and-optional-isolated-markets.md:45-53,118-160`). Adding one disposable host schedule to it would conflate separate decisions. Accepted ADR 0008's redundant funded keepers and oracle/contract TTL obligations remain untouched and are not satisfied by this proposal (`doc/adr/0008-launch-v1-on-a-narrowly-hardened-blend-v2-fork.md:297-306`). No contract, Wasm, adapter policy, or Accepted ADR change is proposed.

Host-local evidence is distinct from repository-controlled configuration. On 2026-10-05, read-only inspection of `~/.config/systemd/user/templar-spoon-prices.service` and `.timer` showed a Python oneshot, 20-minute timeout, `Restart=no`, control-group killing, `UMask=0077`, and Monday–Friday half-hourly starts from 09:00 through 17:30 Europe/London with `Persistent=false`. `systemctl --user show` reported the service inactive, timer active and matching file paths; `loginctl show-user common -p Linger` reported `yes`. No unit was started, stopped or reloaded. These facts do not establish a successful keeper cycle.

## Decision

Propose retaining a dedicated, original-market-only testnet refresh path rather than turning generic market trading into an unattended service:

1. **Preserve synthetic values.** Read all twelve positive source quotes and derive the expected adapter median from that snapshot. Require old quote timestamps to precede the RPC ledger clock. Before each source send, recheck network/signer and the exact prior source `(price, timestamp)` pair; publish the same price with an advancing RPC ledger timestamp, not workstation time. Before each runtime refresh, recheck all three exact newly published source pairs, even if a change would leave the median unchanged (`tools/abtest/scripts/refresh_spoon_prices.py:550-625`).
2. **Read back actual freshness.** Permit only pinned source `set_data` and runtime `refresh` sends. After source publication require exact source readback. After refresh, read the raw accepted `aggregated_latest` timestamp: it must be one of this asset's publication timestamps from this cycle and within the pinned maximum-age/future-drift bounds. Require the adapter price to equal the expected median and its timestamp to equal `raw_timestamp - raw_timestamp % resolution`. Thus a fresh selected timestamp such as 1800000020 may correctly appear as 1800000000 at resolution 300, but an unrelated timestamp 1800000015 cannot pass for publications at 1800000010/20/30. Reuse the adapter's price conversion; do not reimplement mantissa/exponent conversion in Python (`:414-434,523-541,626-650`). Transaction exit status alone is not proof of freshness.
3. **Retain single-operation uncertainty custody.** Create and fsync a separate unresolved-operation marker before each native send. Send once, and remove the marker only after matching readback and marker identity checks. A retained marker blocks the next cycle before manifest/network work. Never blindly retry a send, auto-clear uncertainty, or prune/bypass the generic journal cap. A later scheduled cycle after a pre-send failure is not a replay of an unresolved transaction (`:437-502,550-554`). The dedicated marker is not a full transaction-history service.
4. **Keep the schedule bounded.** Use the existing working-hours, non-persistent timer and non-restarting service. Night/weekend staleness is expected, not a promise of continuous availability. Stop the timer and allow an active job to finish before manual repricing or stale-price experiments. Installing, enabling or invoking the service is a separate explicit operator action, not an effect of importing the code, publishing this branch or approving this proposal.
5. **Require owner coordination.** The run directory and tooling must have a trusted owner, and authorized writers must coordinate exclusively. The local lock serializes cooperating keeper instances only. Read-check-send is not on-chain compare-and-swap; another authorized writer can race it. Separate runtime/adapter reads are not atomic either. Filesystem checks do not protect against a malicious same-owner process. This is not safe unattended mainnet operation.
6. **Make informational invocation non-mutating.** Parse arguments before the driver lock or cycle: `--help` exits 0, unsupported `--dry-run` exits 2, both without operational work. A no-argument invocation remains the explicit publishing mode used by the service (`:656-669`).

Scope inventory:

| Disposition | Item | Reason |
|---|---|---|
| INCLUDE | Original-market pins, preserved synthetic levels, RPC timestamps, exact readbacks, one-shot sends and durable uncertainty | Minimum bounded behavior for this development operation. |
| INCLUDE | Working-hours scheduling, owner coordination and separate offline/live verification | Makes the liveness and trust limits explicit. |
| DEFER | New markets, assets, sources, schedules or real market-data ingestion | Requires new evidence and authorization, not relaxed pins. |
| DEFER | Production redundancy, funding, TTL maintenance and continuous monitoring | Separate obligations under ADR 0008. |
| REJECT | Auto-confirmed generic trades, journal pruning, blind retries or marker removal | Widens authority or loses outcome custody. |

## Alternatives

- **Manual-only refresh:** least persistent machinery and suitable for occasional use, but requires repeated operator action rather than working-hours convenience. It remains the safe fallback while authorization or recovery is unresolved.
- **Auto-confirm generic `market call`:** reuses the wider transaction path but introduces unattended trading authority and pressure on the bounded journal. The dedicated two-method path has a narrower failure surface.
- **Adopt a production feed/keeper platform now:** could address real data, redundancy, funding and TTLs, but solves a different problem. It cannot turn recycled mock prices into economic observations.

## Consequences

The keeper changes neither economic mock values nor contract policy and adds no dependency or general scheduler. Costs are host-specific configuration, manual reconciliation, trusted CLI/RPC/tooling and expected out-of-hours staleness. A cycle is not atomic across twelve publications and four refreshes: earlier successful sends survive a later failure. Bounded stage/status logs and public quote metadata do not replace durable transaction-history evidence.

### Failure analysis

| Failure | Warning and detection | Prevention and recovery |
|---|---|---|
| An authorized writer reprices during a cycle, or a stale-price experiment overlaps the keeper | Exact source-pair checks/readbacks fail, including median-preserving changes; races after the last check can still escape detection. | Stop scheduling, finish the active job and coordinate all writers before experiments. On conflict, preserve evidence, inspect all affected quotes and reconcile intended prices before resuming. Do not claim CAS protection. |
| A send lands but the CLI times out, the process dies or readback fails | No completed cycle; an unresolved marker remains and blocks the next invocation. | Marker before one-shot send, no automatic restart/replay/clearing. Stop scheduling and ensure no active job; reconcile the specific operation against public transaction/chain state. Clear only after the operator establishes and records disposition. A failed CLI exit is not proof of non-submission. |
| RPC/identity/configuration drift or a schedule gap leaves unusable prices | Pin, ledger-clock, raw-runtime or adapter-readback refusal; stale or absent quotes. | Fixed pins and fail-closed bounds, no catch-up promise. Keep sends stopped, restore independently verified original configuration or obtain a new decision for changed scope. Never widen age/drift/pins to manufacture success. |

### Acceptance and verification

From repository root, run the existing offline boundary and mocked-integration check:

```sh
python3 -I tools/abtest/scripts/test_refresh_spoon_prices.py
```

Expected exit 0 with `refresh_spoon_prices: offline boundary checks passed`. It covers uncertain-send custody, exact source-change refusals, argument isolation, malformed normalized quotes, current-cycle raw timestamps, 300-second adapter projection, wrong prices, stale/future/non-member timestamps and invalid resolution. This check passed during publication preparation. An additional actual-script smoke in a disposable copied workspace verified help exit 0, unsupported flag exit 2, and default invocation refusing a synthetic unresolved marker with exit 1; deny-all fake CLIs saw zero calls, and the marker/manifest stayed unchanged. Only the default invocation created its empty local lock. Neither check used real identities or sent a transaction.

The ADR-only candidate uses existing `bin/adr --robot lint` and `git diff --check`, followed by distinct sequential Architect and Critic review of the same candidate commit/blob. Exact commands, results and reviewer responses belong in the review record. Consensus does not change Proposed status; only a human accepts.

Contract/Wasm regression and size measurement are inapplicable: this proposal changes neither. A successful live cycle and testnet end-to-end observability remain **unverified and not authorized by publication**. A separately authorized operator must reconcile any existing marker; verify the original deployment, signer, loaded units and exclusive ownership; and demonstrate twelve unchanged source prices with advanced timestamps, four accepted raw quotes from the current publications, matching adapter prices/bucketed timestamps, complete logs and no unresolved marker. This is an explicit remaining live gate, not an implementation placeholder or a production-readiness claim.

Stop publication of unsupported claims if source diverges from this contract or offline checks fail. Stop operational sends on uncertainty or a failed identity/quote boundary; require owner reconciliation rather than replay. The record remains Proposed until the human decision process completes.
