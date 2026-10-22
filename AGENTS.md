# Agent Guide: blend

Working repo for Templar's Blend work. It holds decisions and changes *about* the Blend
contracts; the contracts themselves are vendored, not forked here.

Workspace policy (Ponytail gate, secrets, repo boundaries) lives in
`/home/common/projects/templar/AGENTS.md`. It applies here. Do not restate it.

## Layout

- `doc/adr/` — Architecture Decision Records. The decision log for all Blend work. ADR 0001
  defines the process itself. `doc/adr/templates/template.md` is the project template: born
  `Proposed`, sections Context / Decision / Alternatives / Consequences.
- `doc/lending-economics.md` — plain-language operator/lender guide to the ADR-0008 bridge's
  economics: reserves, health factor, liquidation auctions and partial fills, lender-borne
  default, oracle staleness and keepers. Not normative; the ADRs are.
- `bin/` — vendored [adr-tools](https://github.com/npryce/adr-tools) (bash, GPL-3.0,
  `bin/LICENSE.txt`) plus local patches. Keep this list current; re-apply it if adr-tools is
  ever re-vendored:
  - shebangs are `/usr/bin/env bash` (NixOS has no `/bin/bash`);
  - `adr --robot` flag in `bin/adr`; no-TTY editor guard in `adr-new`;
  - backticks escaped in `_adr_help_new` (upstream bug);
  - `adr-status`, `adr-accept`, `adr-lint` are ours.
- `.agents/skills/raladr/SKILL.md` — the `raladr` skill: evidence-grounded ADR drafting with
  sequential Architect + Critic review. Runtime-agnostic (Claude Code, Codex, oh-my-pi,
  prime-agent, …); `.claude/skills/raladr` is a symlink to the same directory. Add further
  runtime symlinks there rather than copying the file.
- `tools/abtest` — `precept-abtest`, a Rust CLI and operator TUI that deploys two pools on
  Stellar testnet (pinned stock vs the ADR-0008 fork, or two fork variants), drives identical
  scenarios against both in lockstep, and checks the ADR 0008/0009/0011 invariants after every
  step. Produces `report.md` with an ADR-keyed invariant ledger plus per-step JSON evidence.
  Standalone crate (its own `[workspace]`); every chain action shells out to the `stellar` CLI.
- `contracts/pool-oracle-adapter` — multi-asset SEP-40 `PoolOracleAdapter`: a thin view over the
  pinned Templar proxy-oracle runtime (no source IO, aggregation, freshness policy, or admin
  surface). Unlike read-only `vendor/`, this crate is ours: its own `justfile` gate and 32 KiB
  optimized budget live there. Scope, canary evidence, and remaining release gates:
  `doc/adr/reviews/adr-0007-pool-oracle-adapter-implementation-record.md`.
- `vendor/blend-contracts-v2` — git submodule, `Templar-Protocol/blend-contracts-v2`.
- `devenv.nix` / `.envrc` — [devenv](https://devenv.sh) shell, entered automatically by direnv.
  Provides Rust 1.81, `adr` on `PATH`, and the pinned Stellar CLI 22.6.0 on `PATH` —
  the contract builder only. `tools/abtest` market commands run on the exact CLI
  26.1.0 that devenv exports as `PRECEPT_STELLAR_BIN`; pair drives accept any CLI
  >= 23. `contracts/pool-oracle-adapter` is separate again: its gate builds through
  `pool-adapter-env` (Rust 1.89 + the frozen 26.0.0 adapter CLI), leaving these
  defaults untouched.

## Rules

- **Never edit anything under `vendor/`.** Changes to the contracts are PRs against
  `Templar-Protocol/blend-contracts-v2`. This repo only moves the pinned commit.
- Bumping the pin is its own commit: `git submodule update --remote vendor/blend-contracts-v2`,
  then say in the message which upstream commits came in and why.
- Anything that changes production contract behaviour, a protocol interface, or an operational
  assumption of a production deployment gets an ADR *before* the code. If you find yourself
  explaining such a decision in a PR description, it should have been an ADR.
- **Standard tooling needs no ADR.** Developer and testnet-only work goes straight to a PR with
  its tests and the Ponytail gate: `tools/abtest` features and recipes, justfiles, scripts,
  keepers for disposable testnet markets, mock tokens and testnet helpers such as
  `contracts/token-faucet`. Don't write an ADR or run `raladr` for it.
- Superseding beats editing: `adr new -s N "New decision"`. Accepted ADRs are a record of what
  we thought at the time, not a wiki page.
- **Agents propose, humans accept.** `adr new` creates a `Proposed` ADR; only a human runs
  `adr accept N`. An agent never accepts its own ADR and never edits an Accepted one.
- **Agents draft substantive ADRs through `raladr`, not by hand.** If a production decision
  touches contract behaviour, an interface, funds, auth, upgrades, liquidation, oracles, or an
  operational assumption, an agent runs the `raladr` skill and reports its review status. A
  hand-written ADR from an agent for such a decision is `UNREVIEWED` and must say so in its
  Status section until a human or a raladr run reviews it. Plain `adr --robot new` is for
  process and housekeeping decisions only.
- Fill every section of the template, Alternatives included. An ADR with "N/A" alternatives
  is a PR description, not a decision.
- `adr lint` must pass before commit (it also runs in `devenv test`).

## ADR workflow

Two paths. Pick by weight of the decision, not by who is asking:

- **`raladr`** (default for anything real): any production change to contract behaviour, an
  interface, funds flow, admin/auth, upgrades, liquidation, oracle inputs, or an operational
  assumption. Standard tooling needs no ADR at all (see Rules).
  Invoke it as `/raladr <decision>` or "use raladr to …" in whatever runtime you are in. It
  drafts the ADR with `adr --robot`, freezes it with `git hash-object`, runs an Architect
  review then a distinct Critic review on that exact hash, revises up to five pairs, and
  reports `CONSENSUS APPROVED` / `UNREVIEWED` / `CONSENSUS NOT REACHED` / `REVIEW BLOCKED`.
  It never accepts. Read the skill before running it.
- **Plain `adr --robot new`**: process and housekeeping decisions where two independent
  reviews would cost more than the decision. Still fill every section.

```sh
export PATH="$PWD/bin:$PATH"            # devenv/direnv does this for you
adr --robot status                       # number, status, title — read this first
adr --robot new "Title of the decision"  # next number, Proposed; prints the path, edit it
adr --robot new -s 7 "Replaces decision 7"   # supersede ADR 7 (7 is marked Superceded by)
adr --robot link 9 Amends 7 "Amended by" # cross-link
adr --robot lint                         # statuses known, numbers unique, links resolve
adr accept 9                             # HUMAN ONLY: Proposed -> Accepted
adr generate toc                         # index, paste into doc/adr/README.md if we want one
```

`--robot` never opens an editor or pager; agents and scripts always use it. (`adr new` also
skips the editor when stdin is not a TTY, as a backstop.) `adr new` prints the new file's
path to stdout. Known statuses for `adr lint`: `Proposed`, `Accepted`, `Rejected`,
`Deprecated`, `Superceded by …` (adr-tools' spelling; keep it so the tooling matches).

Lifecycle: `Proposed` (agent or human drafts) → human reviews → `adr accept N`, or the human
edits the status line to `Rejected`. Once Accepted, the file is frozen; a change of mind is
a new ADR with `-s N`.

## Environment

`direnv allow` gets you everything: Rust, the `wasm32-unknown-unknown` target, and `adr` on
`PATH`. The Rust version is read from `vendor/blend-contracts-v2/rust-toolchain.toml` (1.81) —
one source of truth for the vendored build; `tools/abtest` pins 1.81 too, matching the shell default.
`contracts/pool-oracle-adapter` carries its own `rust-toolchain.toml` (1.89) and is built through
`pool-adapter-env`. Do not switch the vendored pin to a `stable` channel:
`ethnum 1.5.0` fails to compile (E0512) on current rustc.

The shell builds Stellar CLI **26.1.0** and exports its exact path as
`PRECEPT_STELLAR_BIN` for the explorer recipes. For raw `precept-abtest` invocations,
set `STELLAR_BIN="$PRECEPT_STELLAR_BIN"`. Outside devenv, install the same version with
`cargo install --locked stellar-cli --version 26.1.0`. This runtime pin does not replace
the separate 22.6.0 and 26.0.0 contract-artifact producer pins.

## Soroban size gate

Each optimized Blend contract wasm (`pool`, `backstop`, `pool_factory`; the emitter is not
vendored here, only its README) must stay at or
below **128 KiB (`131072` bytes)**. That is the Templar-wide Soroban budget (same as the vault
runtime in `contracts/`); re-check it against the live network `contractMaxSizeBytes` after
every protocol upgrade and record a change here and in an ADR. Size is a shipping constraint,
not a cleanup task: any ADR or upstream PR that adds a public method, event field, storage
variant, generic helper, or dependency feature is a size decision and must report before/after
bytes.

```sh
cd vendor/blend-contracts-v2 && make build           # emits target/wasm32-unknown-unknown/optimized/*.wasm
stat -c '%s %n' target/wasm32-unknown-unknown/optimized/*.wasm
```

When a contract grows or nears the gate, attribute it before changing logic — measure, do not
guess:

```sh
cargo install twiggy                                  # once; wasm-objdump/wasm2wat come with wabt
wasm-objdump -h target/wasm32-unknown-unknown/optimized/pool.wasm      # code vs data vs custom sections
twiggy top        target/wasm32-unknown-unknown/release/pool.wasm -n 80
twiggy dominators target/wasm32-unknown-unknown/release/pool.wasm
twiggy monos      target/wasm32-unknown-unknown/release/pool.wasm
cargo tree -d; cargo tree -e features -i <crate>      # duplicate versions, feature unification
```

Order of attack: section growth → retained dominators/monos → duplicate crates and features →
ABI/spec/event surface → inlining (`#[inline(always)]`, cold helpers) → `format!`/panic
machinery (indexing, division, `unwrap()`) → allocator (`dlmalloc` high in `twiggy top`) →
LLVM IR only if a giant still has no source-level cause. Delete or narrow before adding a
helper layer; escalate to a contract split only after the low-delta levers are measured and
exhausted. The full playbook with rationale lives in
`../contracts/contract/vault/soroban/AGENTS.md` (“Codegen Reduction Tips”); do not duplicate
it here, link to it in the ADR.

## Invariants and formal verification

Every ADR that changes pool, backstop, or factory behaviour names the invariants it
preserves or deliberately changes. Blend's canonical ones, minimum (symbols are from
`pool/src/pool/reserve.rs`, `backstop/src/backstop/user.rs`, `backstop/src/storage.rs`):

- Pool: reserve `b_rate`/`d_rate` never decrease except by explicit bad-debt/`gulp` paths; a
  position is never liquidatable immediately after a healthy `submit`; per-reserve supplied
  ≥ borrowed in underlying; backstop-take and interest math never underflow or round in the
  protocol's disfavour; `flash_loan` leaves every reserve at least as solvent as before.
- Backstop: per-pool balance plus queued-for-withdrawal (`q4w`) shares plus emissions equal
  the backstop token balance; `queue_withdrawal`/`dequeue_withdrawal` lifecycle cannot be
  skipped; `draw`/`donate` only move funds to or from the named pool; emissions are never
  over-claimed.
- Factory: `deploy` is deterministic for a given salt and only deploys the pinned pool wasm.

Verify invariants in this order, cheapest first, and say which levels ran:

1. Unit and property tests in the submodule (`make test`; add `proptest` cases upstream for
   arithmetic and state-machine paths).
2. **Kani** model checking for bounded arithmetic and state-transition proofs
   (`cargo install --locked kani-verifier && cargo kani setup`). Harnesses are
   `#[cfg(kani)] #[kani::proof]` functions next to the logic they prove; they are upstream
   PRs to `blend-contracts-v2` like any other change. Run them with
   `cargo kani -p pool --harness <name>`. Use `kani::assume` for the concrete bounds Soroban
   already enforces (e.g. `i128` amounts within supply) and state the bound in the ADR;
   an unbounded harness that times out proves nothing.
3. Testnet smoke via the Stellar CLI for anything touching auth, upgrades, or cross-contract
   calls.

If a level cannot run (tool missing, build broken), say so in the ADR and the PR — do not
silently drop to the level below.

## Security notes (Blend)

- Review auth on every privileged entrypoint: pool `propose_admin`/`accept_admin`,
  `update_pool`, `set_status`, `queue_set_reserve`/`set_reserve`, `set_emissions_config`;
  backstop `add_reward`/`remove_reward`, `distribute`, `draw`, `drop`. Never rely on the
  caller contract's routing alone.
- Initialization and factory deploys are front-runnable; keep deployment assumptions
  explicit in the ADR.
- Oracle inputs, liquidation auctions, and bad-debt paths are where value leaks; changes
  there are `--deliberate` in `raladr` and need a pre-mortem.
- Storage-key, TTL, and serialization changes are upgrade-safety issues, not refactors.
  Keep wire formats, storage keys, and event payloads explicit.
- Prefer small, measurable upstream PRs over broad refactors; a change that cannot be
  measured for size and covered by an invariant is not ready.

## Verification

This repo carries no build of its own beyond `contracts/pool-oracle-adapter`. `devenv test`
only asserts the shell is sane. Real
checks run inside the submodule, and **`build` must precede `test`** — `pool` and
`pool-factory` use `contractimport!` against wasm artifacts, so a bare `cargo test` fails
with unresolved imports:

```sh
cd vendor/blend-contracts-v2 && make test    # runs build first; see its README.md
stat -c '%s %n' vendor/blend-contracts-v2/target/wasm32-unknown-unknown/optimized/*.wasm   # size gate
cargo kani -p <crate> --harness <name>       # when the change has a proof harness
```

`contracts/pool-oracle-adapter` has its own gate. Its recipes build through the `pool-adapter-env`
wrapper, which selects Rust 1.89 for this crate while the default shell stays Rust 1.81 for
`tools/abtest`; `STELLAR_BIN` must point at a 26.0.0 binary — devenv's own `stellar` on `PATH` is
still 22.6.0, which `preflight` rejects.

```sh
devenv shell
STELLAR_BIN=<path-to-stellar-26.0.0> just -f contracts/pool-oracle-adapter/justfile audit-ready
```

If a check cannot run, say so and why — do not silently skip it.
