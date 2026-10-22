{ pkgs, inputs, ... }:

let
  # Single source of truth for the Rust version: the submodule's own
  # rust-toolchain.toml (1.81 + wasm32). Tracking `stable` instead breaks the
  # build — ethnum 1.5.0 fails E0512 on modern rustc.
  rustPkgs = import inputs.nixpkgs {
    inherit (pkgs.stdenv) system;
    overlays = [ (import inputs.rust-overlay) ];
  };
  toolchain = rustPkgs.rust-bin.fromRustupToolchainFile
    ./vendor/blend-contracts-v2/rust-toolchain.toml;

  # The Stellar CLI is not in nixpkgs; build it from its crate. stellar-cli
  # 22.6.0 depends on ethnum 1.5.0, which fails E0512 on current rustc, so each
  # version takes the toolchain it compiles with.
  stellarCli = { version, hash, cargoHash, rust, features ? [ ] }:
    (pkgs.makeRustPlatform {
      cargo = rust;
      rustc = rust;
    })
    .buildRustPackage {
      pname = "stellar-cli";
      inherit version cargoHash;
      src = pkgs.fetchCrate {
        pname = "stellar-cli";
        inherit version hash;
      };
      buildFeatures = features;
      nativeBuildInputs = with pkgs; [ pkg-config cmake perl ];
      buildInputs = with pkgs; [ openssl systemd dbus ];
      doCheck = false; # upstream tests want a live network
      meta.mainProgram = "stellar";
    };

  # `stellar` on PATH: optimizes the vendored contracts with the pinned toolchain.
  stellar-cli = stellarCli {
    version = "22.6.0";
    hash = "sha256-R+De07wD+R0lbooqA6ULqkxvUqE8gzlRQfKBdGfbnJ4=";
    cargoHash = "sha256-GR8OU+ZV++WRqqWcJYz2RKqOLqYhhe+oVZokvgAgDZg=";
    rust = toolchain;
    features = [ "opt" ];
  };

  # `tools/abtest` drives testnet with exactly this CLI, exported as
  # PRECEPT_STELLAR_BIN (off PATH, so `stellar` stays 22.6.0). Its market gate
  # refuses anything older: 26.0.0 rejects the explorer's
  # `--source <G> --sign-with-key <alias>` send before submission.
  stellar-cli-abtest = stellarCli {
    version = "26.1.0";
    hash = "sha256-LRBOSnQasqenMmtG+Mf3bxu2xUKulYG4KytEw/XIFIE=";
    cargoHash = "sha256-WF/FYa3JzIBzqCu7SDvIAoPt2PbidkrW8F6ZRDxjhqA=";
    rust = rustPkgs.rust-bin.stable."1.93.0".minimal; # the crate's rust-version
  };

  # The pool-oracle-adapter toolchain is defined by the crate's own
  # rust-toolchain.toml (1.89 minimal + wasm32v1-none), the same single-source
  # pattern as the vendored toolchain above. The default shell stays
  # 1.81 / CLI 22.6.0 for the vendored build; the adapter toolchain is
  # exposed ONLY through `pool-adapter-env`, which prepends its bin to PATH and
  # sets RUSTC so cargo/rustc spawned internally by Stellar CLI inherit it too.
  adapterToolchain = rustPkgs.rust-bin.fromRustupToolchainFile
    ./contracts/pool-oracle-adapter/rust-toolchain.toml;
  pool-adapter-env = pkgs.writeShellScriptBin "pool-adapter-env" ''
    export PATH="${adapterToolchain}/bin:$PATH"
    export RUSTC="${adapterToolchain}/bin/rustc"
    exec "$@"
  '';
  # Signed Lazer receiver: SDK 26 needs Rust >=1.91. Keep the frozen Blend and
  # adapter producers above unchanged.
  lazerToolchain = rustPkgs.rust-bin.fromRustupToolchainFile
    ./contracts/pyth-lazer-oracle/rust-toolchain.toml;
  lazer-oracle-env = pkgs.writeShellScriptBin "lazer-oracle-env" ''
    export PATH="${lazerToolchain}/bin:$PATH"
    export RUSTC="${lazerToolchain}/bin/rustc"
    exec "$@"
  '';
in
{
  packages = [
    toolchain
    pkgs.git
    pkgs.bashInteractive # adr-tools is bash; NixOS has no /bin/bash
    pkgs.graphviz        # `adr generate graph | dot`
    stellar-cli
    pkgs.just
    pool-adapter-env
    lazer-oracle-env
  ];

  env.PRECEPT_STELLAR_BIN = "${stellar-cli-abtest}/bin/stellar";

  enterShell = ''
    export PATH="$DEVENV_ROOT/bin:$PATH" # vendored adr-tools
  '';

  enterTest = ''
    adr help >/dev/null
    adr lint
    rustc --version | grep -q 1.81
    rustc --print target-list | grep -qx wasm32-unknown-unknown
    "$PRECEPT_STELLAR_BIN" --version | grep -q '^stellar 26\.1\.0 '
  '';
}
