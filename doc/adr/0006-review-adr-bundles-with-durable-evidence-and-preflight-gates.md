# 6. Review ADR bundles with durable evidence and preflight gates

Date: 2026-09-08

## Status

Proposed

Amends [1. ADRs start Proposed and are accepted by a human](0001-adrs-start-proposed-and-are-accepted-by-a-human.md)

Superceded by [12. Review ADRs with human-readable exact-byte evidence](0012-review-adrs-with-human-readable-exact-byte-evidence.md)

## Context

Raladr originally froze one ADR and reported runtime-local reviewer result locations. That is
insufficient when a decision normatively depends on a companion ADR, schema, vectors, or another
reviewable design artifact. Runtime-local results also cannot prove consensus to a later PR reader.
In a recent deliberate run, reviewers spent several pairs discovering mechanically testable schema,
digest, lifecycle, timing, and vocabulary inconsistencies. Repository cleanup after the last review
then changed the reviewed bytes, demonstrating that required local gates must run before freezing.

## Decision

Raladr will review a deterministic **decision bundle**. The default remains one primary Proposed ADR.
A primary ADR may add a small number of Proposed companion ADRs and normative supporting artifacts
only when it explains why separate review would be incoherent. A canonical manifest records the
primary path and every bundle path with its `git hash-object`; the manifest's own Git object ID is
the bundle hash attested by both reviewers.

Before every freeze, the drafter runs `adr lint`, repository-required gates including Ponytail, and
every applicable constructibility check for bundled schemas, vectors, digests, signatures,
lifecycle/replay chains, endpoint bounds, and ADR vocabulary. A failed or missing applicable check
blocks review rather than consuming a review pair.

Architect and Critic return compact mandatory JSON against the same bundle hash. Their complete
results, roles, run/pair identity, manifest, gate evidence, byte-custody location, and terminal
status are persisted in `doc/adr/reviews/<bundle-hash>.json`; repeated reviews append rather than
overwrite. Before revision, reviewed bytes remain reachable through an authorized custody commit
or exact repository snapshot copies. Runtime-local results and dangling Git blob IDs are not
durable proof. Later pairs disposition prior blockers, inspect the revision delta, and rerun the
fixed global invariant checklist without repeating narrative already present in the ADR.

When explicitly requested, Raladr may use an isolated branch or worktree containing only the
decision bundle, cross-links, and review receipts. It still never implements or accepts the
decision. The five-pair cap remains unchanged. Before final output or commit, Raladr re-creates the
manifest and requires the current bundle hash to equal the applicable candidate or terminal
reviewed hash. Pre-cap drift restarts preflight and Architect review. Drift after five complete
pairs is reported as `UNREVIEWED POST-CAP REVISION`; it cannot be committed or claimed as terminal,
and the fifth receipt preserves its actual reviewed outcome until a human starts a new run.

## Alternatives

- Keep one-file review and describe companions in prose: reviewers cannot verify normative bytes.
- Store only runtime result URLs: convenient during one session but not durable PR evidence.
- Add more reviewer roles: raises cost without catching deterministic defects as cheaply as a
  constructibility gate.
- Remove the five-pair cap for small final fixes: invites unbounded loops and unreviewed drift.

## Consequences

Review input and consensus evidence become reproducible from repository bytes. Mechanical defects
fail before expensive review, and required cleanup cannot invalidate an approval unnoticed. Coupled
decisions and explicit branch custody become supported rather than ad hoc.

Raladr now owns small manifest/receipt bookkeeping and applicable preflight commands. Supporting
artifacts increase bundle size, so coupling must be justified and receipts must stay compact.
Human acceptance remains unchanged: every ADR stays Proposed until a human runs `adr accept`.
