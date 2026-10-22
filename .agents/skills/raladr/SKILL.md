---
name: raladr
description: Use for evidence-grounded consensus drafting of an Architecture Decision Record in doc/adr, with sequential Architect and Critic review before a human accepts it
---

# Raladr

Draft a human-readable `Proposed` ADR, then have a read-only Architect and a distinct read-only
Critic review the same exact bytes in sequence. Raladr never implements the decision and never
accepts an ADR.

## Invocation

Use for `$raladr <decision>`, `/raladr <decision>`, or an explicit request to use Raladr. Also use
when `AGENTS.md` requires an ADR before changing contract behaviour, interfaces, funds flow,
authority, upgrades, liquidation, oracle inputs, or operational assumptions.

Optional modifiers:

- `--interactive`: pause after the draft and after final review.
- `--deliberate`: expand failure analysis for funds, auth, irreversible state, migrations, or public
  contract interfaces. Enable it automatically for those decisions.

A focused obvious fix or a request for immediate implementation does not implicitly invoke Raladr.
An explicit Raladr request always does.

## Outcome

Produce one concise Proposed ADR by default. Use coupled Proposed ADRs only when reviewing them
separately would make the decision incoherent. Leave implementation, generated artifacts, schemas,
record languages, and operational tooling for later implementation work.

Terminal outcomes are:

- `CONSENSUS APPROVED`: Architect and Critic approve the same unchanged ADR bytes.
- `UNREVIEWED`: the interactive user explicitly skips review.
- `CONSENSUS NOT REACHED`: the fifth complete review pair does not approve.
- `REVIEW BLOCKED`: required evidence, preflight, or reviewer execution cannot be recovered.

Approval is not acceptance. Only a human runs `adr accept N`.

## Non-negotiable simplicity

Raladr is a decision-review workflow, not an implementation language.

- Review evidence is Markdown written for humans.
- Exact-byte identity uses the candidate Git commit and each reviewed file's `git hash-object`.
- Reviewer responses are ordinary structured prose, not JSON.
- Do not create a manifest format, JSON receipt, schema, fixture/vector corpus, interpreter, or
  bespoke verification program for an ADR review.
- Existing repository checks may run when applicable. New implementation checkers belong in a
  later, separately reviewed implementation change after the ADR is accepted.
- One ADR is the default. Do not bundle files merely because they share a topic.

## Workflow

### 1. Ground the decision

1. Run `adr --robot status` and read the related ADRs.
2. Inspect the affected source and existing tests read-only. Cite concrete `path:line` evidence for
   behavioural claims. Use official sources for version-sensitive external behaviour.
3. State the decision, desired outcome, constraints, acceptance criteria, and stop condition.
4. Make a short inventory of requested or evidence-raised changes. Mark each `INCLUDE`, `DEFER`, or
   `REJECT` with one reason. Resolve material ambiguity from repository evidence before asking the
   user.
5. Prefer superseding or amending an existing Proposed ADR over creating a parallel decision.

### 2. Draft the ADR

Create the ADR with `adr --robot new`, using `-s N` when superseding. Fill every project-template
section:

- **Context:** scope, exclusions, current behaviour, invariants, assumptions, and evidence.
- **Decision:** exact changes, boundaries, ordering, ownership, and why this option wins.
- **Alternatives:** at least two real options with bounded tradeoffs.
- **Consequences:** acceptance criteria, smallest useful verification, operational burden, risks,
  recovery, and follow-ups.

For deliberate decisions, also include three credible failure scenarios with warning signs,
prevention, detection, and recovery. Cover unit, integration, testnet/end-to-end, and observability
checks where applicable; mark a layer inapplicable with a reason rather than inventing work.

Keep the ADR readable. Tables are useful for caps, authorities, state transitions, and release
gates. Do not embed executable policy or duplicate a future implementation.

### 3. Preflight and freeze

Before review:

1. Run `adr --robot lint` and `git diff --check`.
2. Run only repository checks that validate material claims already implemented or existing input
   artifacts. Do not write a checker just to review the ADR.
3. Apply the Ponytail gate to the complete pending diff. Remove redundant machinery and speculative
   requirements before review.
4. Commit the candidate ADR and any necessary cross-links on the requested branch. The candidate
   commit is the immutable review boundary.
5. Record the candidate commit SHA and `git hash-object` for every reviewed ADR file.

Do not edit reviewed files while a pair is running. A changed blob invalidates both reviews and
requires a new candidate commit and a fresh Architect-first pair.

### 4. Architect review

Launch one read-only Architect with:

- the complete user request and constraints;
- candidate commit SHA;
- reviewed paths and blob IDs;
- compact `claim -> path:line` evidence;
- change inventory and unresolved prior findings;
- the instruction to skip edits, formatting, builds, and tests.

Require a concise response with these headings:

- `Verdict: APPROVE` or `Verdict: ITERATE`
- `Reviewed commit and blobs`
- `Blockers` — numbered, with evidence and the minimum required correction
- `Prior blocker disposition`
- `Tradeoff assessment`
- `Security and maintenance assessment`
- `Evidence checked`

`APPROVE` means there are no blockers or hidden required changes. Suggestions must be explicitly
non-blocking. Preserve the complete response for the review record.

### 5. Critic review

After the Architect completes, launch a distinct read-only Critic against the same commit and blobs.
Provide the Architect response in addition to the same evidence. Require:

- `Verdict: APPROVE`, `ITERATE`, or `REJECT`
- `Reviewed commit and blobs`
- `Blockers`
- `Architect finding disposition`
- `Consequence simulations`
- `Requirement coverage`
- `Alternative fairness`
- `Security, maintenance, and verification assessment`
- `Evidence checked`

The Critic independently checks the ADR and does not inherit the Architect's verdict. Preserve the
complete response.

### 6. Record, revise, and converge

Write one human-readable review record under
`doc/adr/reviews/YYYY-MM-DD-adr-NNNN-<short-candidate-sha>.md` containing:

1. ADR paths, candidate commit, and reviewed blob IDs.
2. Exact preflight commands and results.
3. The complete Architect response.
4. The complete Critic response.
5. Applied and rejected finding dispositions.
6. The pair outcome.

Commit the review record. It is evidence about the candidate and is not itself part of the reviewed
ADR bytes.

If either reviewer finds blockers, count the pair, revise only the ADR/cross-links, rerun preflight,
commit a new candidate, and start again with Architect. Carry every prior blocker forward by its
number. Stop after five complete pairs. Reviewer failures do not consume a pair; retry once when
safe, then report `REVIEW BLOCKED` unless concrete recovery evidence appears.

Consensus exists only when both distinct reviewers approve one unchanged candidate. Do not change
the ADR after approval. Any accepted suggestion requires another pair.

### 7. Finalize

Before publishing or reporting success:

1. Verify current ADR blob IDs equal those in the approving review record.
2. Run final `adr --robot lint` and `git diff --check`.
3. Run Ponytail over the complete branch diff. If it changes reviewed ADR bytes, review again.
4. Confirm the branch contains only ADRs, necessary cross-links, and human-readable review records.
5. Push only when requested, then verify the remote branch points at the reported commit.

Report the ADR paths, candidate commit, reviewed blob IDs, review-record path, exact checks, both
verdicts, terminal status, material risks, and the human next step. Never claim deployment approval.

## Guardrails

- Agents draft Proposed ADRs only; they never run `adr accept`.
- Reviewers are distinct, sequential, read-only, and review identical bytes.
- No implementation, vendor edits, generated artifacts, or hidden follow-up machinery.
- No placeholder evidence, unverifiable command, or unspecified “add tests” consequence.
- Preserve concurrent user changes. Work in an isolated branch/worktree when requested or when the
  current tree contains unrelated changes.
- Five complete pairs is a hard stop.
