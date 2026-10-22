# templar-price-source-contract

Admin-gated synthetic SEP-40 price publisher for the protected testnet market.

Tests (run from this directory):

```sh
pool-adapter-env cargo test --locked
```

Deployable WASM needs Stellar CLI v25.2.0+ for the build, not plain
`cargo build`: `stellar-access` enables SDK spec shaking and the CLI injects
the contract spec. The artifact currently pinned for new market deployments
was built with Rust 1.89.0 and Stellar CLI 26.1.0, then optimized with
the available Stellar CLI 22.6.0:

```sh
pool-adapter-env ~/.cargo/bin/stellar contract build --locked --profile release-soroban-checked
stellar contract optimize \
  --wasm target/wasm32v1-none/release-soroban-checked/templar_price_source_contract.wasm \
  --wasm-out target/wasm32v1-none/release-soroban-checked/templar_price_source_contract.optimized.wasm
```

This mixed-toolchain artifact is **not** qualified for production. The
implementation and tests are unchanged apart from rustfmt, and its exported
ABI spec matches the old build, but its WASM hash differs
from the publisher already deployed on testnet. The expected **new-deployment**
artifact path and hash are pinned in `tools/abtest/markets/protected-testnet.toml`.
Do not mistake rebuilding at another path or with another CLI for byte
equivalence: verify the rebuilt hash and re-pin deliberately.
