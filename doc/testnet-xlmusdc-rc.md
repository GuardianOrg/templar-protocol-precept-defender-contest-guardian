# XLM / Circle USDC testnet RC — signed Pyth Lazer

The active RC verifies **Pyth Lazer signatures on-chain**, using Pyth's existing
Stellar testnet verifier. The keeper transports signed messages; its fee-paying key
cannot invent an accepted price. This is testnet validation, not audit or launch approval.

[Current manifest](testnet-xlmusdc-rc/market-manifest.toml) ·
[Signed cutover evidence](testnet-xlmusdc-rc/signed-lazer-evidence.json) ·
[Historical backend deployment evidence](testnet-xlmusdc-rc/evidence.json)

## Active addresses

| Role | Address |
| --- | --- |
| **Pool** | `CCA3GD33RD3ZEEFCLO5W3JLNABOPOPQKHW4VWXXHN5KCYOHTTMMG3LO2` |
| Administrator | `GCNQ4ZLK45KOYNIQLIYI7GT6TYWGK2OZEVZEY7A3W5L7QSPBNCBWSJAQ` |
| **Signed receiver / SEP-40 oracle** | `CAZYNT3QQQZP75BBBN5G56PD6Z45USODMBGHKSHOKSX6LU3Z3SYMA3IH` |
| Official Pyth testnet verifier | `CAYFT5JE3UQTKT4Q6ZOZK4FXVYVT6RE3MFC7STA4UB6WAEGBT65MRU52` |
| Delivery fee account | `GATYQZTJL6LTCFC4FMR3FAEFSCCMS5Z6ECGC6WPGN5NTWIS6UEGM7DTP` |
| XLM, reserve 0 | `CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC` |
| Circle testnet USDC, reserve 1 | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |

USDC issuer: `GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5`.
Both reserve tokens have seven decimals. The manifest's `runtime` and `adapter`
labels deliberately point to the **same receiver**, not two deployed contracts.
Its `sources` entry names the official cryptographic verifier. There is no oracle
governance contract or unsigned price setter.

The former backend-priced RC (`CCWMK5KJ3ICQ7QXI3RWIHUP3O3ARJJMMF6PESTZXSQHL2EJSUD6YMOBE`)
has an immutable oracle pointer, so it could not be repointed or natively upgraded.
It was empty at cutover. The replacement deployed only a receiver and pool, reusing
its factory, backstop, activation tokens and existing admin LP balance. The old RC
price timer was disabled and its updater units removed; its contracts, private
journals and historical evidence remain intact. The separate original four-asset lab pool was not changed.

## Authentication and freshness

The active path is **signed Lazer update → official verifier → receiver → pool**.
Verification happens during publication; borrowing reads the accepted stored prices.
The receiver has no administrator, upgrade function or configurable verifier.
Its constructor accepts only the official Stellar testnet network ID.

The keeper makes authenticated HTTPS requests to
`https://pyth-lazer-0.dourolabs.app/v1/latest_price` for feed **23 (XLM/USD)** and
**7 (USDC/USD)**, channel `fixed_rate@200ms`, format `leEcdsa`, with price, exponent
and per-feed update timestamps. The original envelope is passed unchanged to
`pyth-lazer-stellar-sdk` 0.4.0 and the verifier. API JSON previews are client
expectations, never the on-chain authentication authority.

Both feeds must be present exactly once, with exponent -8 and positive values
that remain positive after integer division by ten to USD-7. Freshness uses the
**signed per-feed microsecond timestamps**, never a replacement bundle timestamp.
Writes require age ≤60 seconds and future skew ≤15 seconds; reads expire after
300 seconds. Older/conflicting updates fail; identical authenticated updates leave
prices unchanged. Both feeds are validated before either accepted price changes.
This authenticates one Pyth provider; it does not manufacture independent quorum
or weaken the production proxy's three-source rule.

## Exercised on testnet

- A testnet **simulation** (`--send no`) rejected a forged signed-price byte:
  receiver error **3** wraps verifier error **6**. No forged transaction was
  submitted; both stored prices remained unchanged.
  [An ordinary user then submitted a genuine update successfully](https://stellar.expert/explorer/testnet/tx/f3b05cd0639777d975b3444ba02b1a43f772ce7eb0551066ace3ef73982046dd).
- Supply 10 USDC → collateral 50 XLM → [borrow 1 USDC](https://stellar.expert/explorer/testnet/tx/d749e5c94b8d1b1b326d59bb98026ad91e0d9d01e4fa3c65ee439ed922e2af55)
  → [full repayment](https://stellar.expert/explorer/testnet/tx/93b33343c750dbdb03150affebc4f8724d1a6edc655ee3eb4eaabedd60bb7f19)
  → full withdrawals. Final actor positions, reserve supply, debt and backstop credit were zero.
- Genuine observations expired to `null` for both feeds without artificial repricing;
  restarting the keeper restored fresh signed observations. Disabled bad-debt and
  interest auction simulations refused with error **1200**.
- The actual read-only explorer loaded the replacement pool ABI and exited cleanly.
  Public evidence retains these readbacks, transaction identities and operational observations.

## Keeper and credentials

Run directory:
`/home/common/projects/templar/precept-abtest-evidence/rc-xlmusdc-lazer-20261009`.

The supplied `PYTH_LAZER_API_KEY` was provisioned from the workspace `.env` into
`$RC/.credentials/pyth-lazer-api-key` with mode 0600 in a private directory.
`templar-rc-lazer-prices.service` uses **systemd LoadCredential**, passing only the
credential-file path to Python. The key is not in argv, child environments, journals,
public evidence or Git. Credential-bearing HTTP redirects are refused, and HTTP
error responses are closed before their finalizers could log a server-supplied reason.
Changing `.env` later does not rotate this private copy automatically: replace the
credential file securely and restart the service. Do not paste its contents into logs.

```sh
systemctl --user status templar-rc-lazer-prices.timer
journalctl --user -u templar-rc-lazer-prices.service --since '10 minutes ago' -o cat
systemctl --user start templar-rc-lazer-prices.service
# Park / resume this RC only:
systemctl --user stop templar-rc-lazer-prices.timer
systemctl --user start templar-rc-lazer-prices.timer
```

Stopping the timer does not interrupt an in-flight service cycle. Let that cycle
finish before manual updates or shared-account signing; do not kill it to clear a lock.

The enabled timer runs 30 seconds **after each completed cycle**, without overlap.
The service is oneshot, `Restart=no`, `KillMode=control-group`, timeout four minutes,
umask 0077. Before publishing, it checks the exact testnet RPC identity, canonical
assets, on-chain pool/receiver/verifier code hashes and fixed verifier binding.
Successful publication requires matching readback of both prices and original
floor-second timestamps. A Nix GC root at `$RC/tooling/stellar-cli` retains CLI 26.1.0.
The service executes the tested copy at `$RC/tooling/refresh_lazer_prices.py`,
SHA-256 `0b25a0edf4535aca987749c27d10d676c9407446104aad6caf06cc3f84d0ffd2`,
independently of future checkout changes. Source edits do not deploy automatically:
park the timer, let the service finish, test the new source and explicitly replace
the pinned copy and its provenance before resuming.

Unresolved transaction custody fails closed. The pre-send
`.lazer-price-keeper-unresolved` retains the exact public signed invocation; its
`.result` sidecar retains bounded exit/stdout/stderr (streams are base64-encoded),
including partial output on child timeout or service SIGTERM. The keeper refuses
either retained object and clears them only after both exact price readbacks.
Never delete them to make a timer green or replay a transport failure without
reconciliation. Deployment uses settled-command journals and serializes the
temporarily shared old/new fee signer. The original lab
admin timer must also be parked and any active cycle allowed to finish before admin
signing, then restored. At cutover the original lab had its own pre-existing unresolved
marker: a later settled admin sequence proved its old-sequence send could no longer
apply, but that marker was **preserved**, not silently cleared or retried.

## Inspection

From the repository root inside `devenv shell`:

```sh
RC=/home/common/projects/templar/precept-abtest-evidence/rc-xlmusdc-lazer-20261009
CLI="$PRECEPT_STELLAR_BIN"
LAB=tools/abtest/target/release/precept-abtest
"$LAB" --out "$RC" --stellar-bin "$CLI" \
  --rpc-url https://soroban-testnet.stellar.org \
  --passphrase 'Test SDF Network ; September 2015' market contracts
"$LAB" --out "$RC" --stellar-bin "$CLI" \
  --rpc-url https://soroban-testnet.stellar.org \
  --passphrase 'Test SDF Network ; September 2015' market tui --mode explore
python3 -I tools/abtest/scripts/refresh_lazer_prices.py \
  --run "$RC" --stellar-bin "$CLI" \
  --credential-file "$RC/.credentials/pyth-lazer-api-key" --check
```

The public manifest can be copied to an otherwise empty directory for read-only
exploration. Signed actor operations still use the isolated QA identities in the
original RC's private `.stellar` directory. Preserve that directory when archiving
old evidence. Pool administration remains the separate `ralplan-check` lab identity.

The protected four-asset `market status` readiness check remains **not applicable**:
its mock assets, proxy cache/breaker views and protected-deployment journal are absent.
Use direct receiver prices and the signed keeper's completed checks; do not weaken
that unrelated readiness gate or call the RC production-ready.

## Reproduction and limits

The pool remains the reviewed Blend fork commit
`ffeca2683162e8e32321c27f1652903d7990d7ac`, 54,016 bytes, SHA-256
`441e95a552c7397ce65ba2cef7332e02ec85ee0d3da1d944d587702c21266c6b`.
The receiver is 13,543 bytes, SHA-256
`0bdb400ee239adb6b3965da9da0d144111a36b5059293eab90ecbff3900b3e59`.
The fetched official verifier is 7,363 bytes, SHA-256
`2cb81d38107d5c1a66c7d01c5a36b394cbcac150bda43898fd9260b33be5e145`.

```sh
devenv shell -- lazer-oracle-env cargo test --locked \
  --manifest-path contracts/pyth-lazer-oracle/Cargo.toml
devenv shell -- lazer-oracle-env bash -c \
  '"$PRECEPT_STELLAR_BIN" contract build --manifest-path contracts/pyth-lazer-oracle/Cargo.toml --profile release-soroban-checked'
python3 -I tools/abtest/scripts/test_refresh_lazer_prices.py
```

The new wrapper uses Rust 1.93.0 and SDK 26.1.0; the official Pyth SDK/verifier source
is pinned to [commit `e0f2cf3b4dbb185c4b6e02ef3ca0ae11dccd38e5`](https://github.com/pyth-network/pyth-lazer-public/tree/e0f2cf3b4dbb185c4b6e02ef3ca0ae11dccd38e5/contracts/stellar).
Keep Cargo.lock: the upstream host's loose ed25519-dalek constraint otherwise selects an incompatible major for
its test utilities. The frozen Blend builder (Rust 1.81 / CLI 22.6.0) and historical
qualified adapter producer (Rust 1.89 / CLI 26.0.0) are unchanged.

`deploy_lazer_rc.py` takes `--base-run`, a **new** `--out`, `--stellar-bin`,
`--credential-file` and `--admin-config-dir`. Prepare hash-checked Wasm pins
for exactly `pool`, `backstop`, `factory`, `token`, `comet`, `emitter`, `oracle`,
`verifier`. The keeper roles must already be staged at
`$OUT/artifacts/pool.wasm`, `$OUT/artifacts/pyth-lazer-oracle.wasm` and
`$OUT/artifacts/pyth-verifier.wasm`; their pin paths must resolve to those files.
Misplaced artifacts are refused before any transaction, not silently copied.
The recipe refuses a nonempty base pool, wrong network/identity or insufficient
existing activation LP. Without `--yes` it submits no transactions. Pause the
old RC timer and join its current service cycle before the initial replacement
publication; preserve/restore it on failure, retire it only after verified success.
Settled deployment steps are not resubmitted during a resumed run.

Supply caps remain `i128::MAX`; XLM factors are 75%/80%, USDC 90%/95%, max utilization
95%, no backstop take or rewards. Borrow interest still accrues. Synthetic activation
LP is **not insurance**; bad-debt and interest auctions remain disabled, and the fork
can socialize uncollectible debt. Frontend limits and production audit/readiness
remain separate work. The signed receiver is new integration code, not an audited
production oracle merely because it uses Pyth's verifier.
