# 12. Review ADRs with human-readable exact-byte evidence

Date: 2026-09-10

## Status

Proposed

Supercedes [6. Review ADR bundles with durable evidence and preflight gates](0006-review-adr-bundles-with-durable-evidence-and-preflight-gates.md)

## Context

ADR 0006 made review evidence durable, but implemented that goal with canonical manifests,
mandatory JSON reviewer responses, hash-named JSON receipts, embedded runtime payloads, and optional
schemas and checkers inside decision bundles. In practice this displaced review from the protocol
decision into a bespoke record language and its verification machinery. The resulting evidence is
exact but difficult for a human reviewer to inspect and expensive to maintain.

The useful requirements remain: Architect and Critic must be distinct and sequential; both must
review identical bytes; findings must survive revisions; passing local checks must precede review;
and approval must remain separate from human acceptance.

This is a process decision only. It does not change any Blend contract, deployment policy, or
existing ADR's substantive protocol decision. Existing JSON receipts remain historical records and
need not be rewritten.

## Decision

Raladr will use Git's existing object model as the exact-byte boundary and Markdown as the durable
review format.

Before review, the drafter commits the Proposed ADR and necessary cross-links on its working branch.
The candidate commit SHA and each reviewed ADR file's `git hash-object` identify the review input.
One read-only Architect reviews that candidate, followed by a distinct read-only Critic reviewing
the same commit and blobs. Any ADR-byte change creates a new candidate commit and restarts the pair
with the Architect.

Each pair is recorded in
`doc/adr/reviews/YYYY-MM-DD-adr-NNNN-<short-candidate-sha>.md`. The record contains reviewed paths and
blob IDs, exact preflight commands and results, both complete reviewer responses, finding
resolutions, and the outcome. Reviewer responses use short, fixed Markdown headings and
`APPROVE`/`ITERATE`/`REJECT` verdicts; they do not use a machine schema.

Raladr will not create manifests, JSON receipts, schemas, fixture corpora, interpreters, or bespoke
verification programs. Existing repository checks may validate existing implementation or inputs,
but a production verifier belongs in a later implementation change after human acceptance.
Decision bundles default to one ADR; coupling requires a concrete reason that separate review would
be incoherent.

The five-pair cap, explicit blocked/unreviewed outcomes, Ponytail preflight, Proposed status, and
human-only acceptance remain unchanged.

## Alternatives

- Keep ADR 0006 unchanged. This gives strong machine reproducibility but retains the review and
  maintenance burden that obscured the underlying protocol decisions.
- Store only reviewer session URLs. This is smaller but not durable and does not bind later readers
  to exact reviewed bytes.
- Require signed review attestations. This adds key custody and signature policy without improving
  the immediate human-review problem; Git identity plus repository history is sufficient here.
- Put verifier implementation in each ADR bundle. This repeats the failure mode: the ADR becomes a
  second policy implementation whose correctness dominates review.

## Consequences

Review evidence becomes directly readable in pull requests. Exact-byte custody remains reproducible
from ordinary Git commits and blob IDs, with no parallel manifest or receipt protocol. Reviewers can
focus on security, maintenance, alternatives, and observable consequences rather than a bespoke
serialization format.

Candidate and review-record commits add visible history, and Markdown is less convenient for
automated aggregation. That is accepted: ADR review is low-volume and human judgment is the primary
consumer. Exact blob comparison still detects drift mechanically.

Existing JSON receipts are retained as historical evidence but are not extended. New Raladr runs
must use the Markdown record. A reviewer result that does not name the candidate commit and blobs,
or hides a required correction outside `Blockers`, is invalid and must be rerun.

Acceptance checks:

1. `.agents/skills/raladr/SKILL.md` contains no mandatory JSON response, manifest, receipt schema,
   Python verifier, or bundled-checker workflow.
2. The skill requires a committed candidate, per-file blob IDs, sequential distinct read-only
   Architect and Critic reviews, invalidation on byte drift, and a human-readable Markdown record.
3. `PATH="$PWD/bin:$PATH" adr --robot lint` and `git diff --check` pass.
4. A fresh ADR review can be understood from its Markdown record and reproduced by checking the
   named commit and blob IDs without running project-specific review software.

Operational rollback is a normal Git revert of this process ADR and skill change. It does not alter
or invalidate historical review evidence or accepted protocol decisions.
