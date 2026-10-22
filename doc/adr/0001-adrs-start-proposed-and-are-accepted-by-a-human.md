# 1. ADRs start Proposed and are accepted by a human

Date: 2026-08-27

## Status

Accepted

Amended by [6. Review ADR bundles with durable evidence and preflight gates](0006-review-adr-bundles-with-durable-evidence-and-preflight-gates.md)

## Context

Most ADRs in this repo will be drafted by coding agents. Stock adr-tools creates every ADR
as `Accepted`, opens an editor (which hangs a non-interactive session), and has no way to
see statuses at a glance or to validate the log. That makes "ADR before code" unenforceable
when the author is an agent.

## Decision

We record architectural decisions as Nygard-style ADRs in `doc/adr/`, managed with the
vendored adr-tools in `bin/`, with these local rules:

- `adr new` creates ADRs as `Proposed` (project template in `doc/adr/templates/`).
  `adr --robot ...` never opens an editor or pager; agents always use it.
- Only a human moves an ADR to `Accepted`, with `adr accept N`. Agents never accept their own
  ADRs and never edit Accepted ones; they supersede.
- The template requires an Alternatives section.
- Agents draft substantive ADRs (contract behaviour, interfaces, funds, auth, upgrades,
  liquidation, oracles, operational assumptions) through the repo-local `raladr` skill
  (`.agents/skills/raladr/SKILL.md`): evidence-grounded draft, frozen by `git hash-object`,
  reviewed sequentially by a distinct Architect and Critic agent, revised to consensus or a
  five-pair cap, and reported with one of `CONSENSUS APPROVED`, `UNREVIEWED`,
  `CONSENSUS NOT REACHED`, `REVIEW BLOCKED`. The skill is runtime-agnostic; each agent
  runtime gets a symlink to the same file. Hand-written agent ADRs for such decisions are
  `UNREVIEWED` until reviewed. Plain `adr --robot new` remains for process and tooling
  decisions.
- `adr lint` (known status, unique numbers, resolvable links) runs in `devenv test` and
  must pass before commit. `adr status` is the canonical overview.

## Alternatives

- Switch to MADR or log4brains: heavier tooling and templates for no gain at this size.
- Keep stock adr-tools and rely on convention: agents cannot be told "remember to change
  Accepted to Proposed" reliably; the default has to be right.
- Let agents accept ADRs: removes the one human checkpoint the process exists to provide.
- Let agents write substantive ADRs freehand: a single agent reliably rationalises the
  option it already picked; the sequential Architect + Critic pair in `raladr` is the
  cheapest independent challenge available without a human in the loop for every draft.
- Rely on the host runtime's own planning mode: not portable across Claude Code, Codex,
  oh-my-pi, prime-agent; the skill file in-repo is the one thing all of them can read.

## Consequences

Substantive agent-drafted ADRs cost two extra review agents per revision; that is the
intended price. Human review becomes a required step per decision; a `Proposed` ADR blocking a PR is the
intended signal, not a defect. Vendored `bin/` now carries local patches, listed in
`AGENTS.md`, that must be re-applied if adr-tools is ever re-vendored.
