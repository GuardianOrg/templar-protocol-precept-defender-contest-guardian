# blend

Templar Protocol's working repo for [Blend](https://github.com/Templar-Protocol/blend-contracts-v2).

- **Decisions:** [`doc/adr/`](doc/adr) — every architectural decision, newest wins.
- **Contracts:** [`vendor/blend-contracts-v2`](vendor/blend-contracts-v2) — git submodule, read-only here.
- **Conventions:** [`AGENTS.md`](AGENTS.md).

## Setup

```sh
git clone --recurse-submodules git@github.com:Templar-Protocol/blend.git
cd blend
direnv allow          # devenv: rust 1.81 + wasm32 + adr on PATH
adr status
```

Already cloned? `git submodule update --init --recursive`.
No direnv? `devenv shell`. No devenv? `export PATH="$PWD/bin:$PATH"` gets you `adr` alone.

## Pool oracle adapter

[`contracts/pool-oracle-adapter`](contracts/pool-oracle-adapter) is the first first-party Soroban
contract in this repo: a thin multi-asset SEP-40 view over the Templar proxy-oracle runtime. Its
optimized Wasm budget is **32 KiB (`32768` bytes)**.

Inside `devenv shell`, run `just -f contracts/pool-oracle-adapter/justfile audit-ready` with
`STELLAR_BIN` pointing at the pinned Stellar CLI 26.0.0 (devenv's own is 22.6.0). Its cargo recipes
build through the `pool-adapter-env` wrapper, which selects Rust 1.89.

Implementation scope, the disposable testnet canary, and the remaining release gates are in
[`doc/adr/reviews/adr-0007-pool-oracle-adapter-implementation-record.md`](doc/adr/reviews/adr-0007-pool-oracle-adapter-implementation-record.md).

## Protected market price source

[`contracts/price-source`](contracts/price-source) is the disposable testnet
market's admin-published synthetic SEP-40 source. It is not part of the
production contracts repository. Build instructions and the distinction between
source code and the deployed artifact are in the crate's README.

The testnet-only [market explorer and guarded action CLI](tools/abtest/README.md#disposable-protected-market)
is in `tools/abtest`; it does not maintain a live quote feed automatically.
