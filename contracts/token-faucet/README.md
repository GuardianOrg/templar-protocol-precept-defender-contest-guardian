# token-faucet

Testnet-only faucet: anyone may mint any amount of any token that has appointed
this contract as its admin. The owner can hand admin back. Not for mainnet.

## Tests

```sh
pool-adapter-env cargo test --locked
```

## Build

```sh
pool-adapter-env ~/.cargo/bin/stellar contract build --locked --profile release-soroban-checked
```

(Stellar CLI 26.1.0 at `~/.cargo/bin/stellar`.)
