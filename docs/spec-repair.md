# Spec repair loop

When the pipeline judges an issue or spec unusable — most commonly the #3533
dead-end where every acceptance criterion is already true, so nothing the agent
could do would falsify them — the repair loop turns a bare refusal into one
structured repair attempt. The loop never edits the issue body: adoption of new
wording is a maintainer authority act, expressed through comment replies, not
agent edits.

The implementation lives in `crates/autospec-core/src/spec_repair.rs` (pure
logic, tracker behind the `IssueRepairTracker` trait) and
`crates/autospec-cli/src/commands/spec_repair.rs` (the `gh`-backed CLI).

## The five-part proposal comment

`autospec spec-repair propose` posts exactly one comment and labels the issue
`needs-spec-clarification`. The comment carries five parts:

1. **The defect** — a machine-readable classification from `SpecDefectShape`:
   `no_falsifiable_criterion`, `missing_precondition`, `ambiguous`,
   `over_scoped`, `unverifiable`, or `contradictory`.
2. **What we think it means** — the best available interpretation, stated
   plainly. Not a hedge: it is the reading the agent would act on.
3. **Proposed acceptance criteria** — each criterion rewritten to be
   falsifiable, with the check command that decides it and the exit status
   observed on the current tree. A criterion that passes today is rejected at
   validation: a repair must show what fails now.
4. **The question** — one specific question whose answer unblocks the issue,
   naming the plausible answers so a maintainer can settle it in one word.
5. **What was checked** — the evidence gathered before concluding the issue is
   unusable, so the proposal is visibly not a guess.

The comment opens by stating it is a proposal, not a decision, and that the
issue body was not modified. A maintainer replies with a line
`spec-repair: approved` (adopt the proposal as the requirement of record;
`autospec spec-repair check` then removes the label and allows re-dispatch) or
`spec-repair: rejected: <reason>` (the label stays on; a human rewrites the
issue). Decision markers only count at the start of a line, so quoting the
banner inside another comment cannot forge a decision.

## Stopping the loop

Two guardrails keep the loop from converging on nothing:

- **Proposal escalation.** After two recorded proposals for the same issue,
  `propose` posts an escalation notice instead of a third proposal: a
  maintainer must rewrite the issue (or close it). The notice is idempotent.
- **Stall spec review.** Two consecutive no-output runs on one issue require
  spec review: `autospec spec-repair stall --consecutive-runs N` reports
  `spec_review_required` so the monitor can act on it.

## Stated assumptions (low stakes only)

Some ambiguity is cheap to be wrong about. `autospec spec-repair assumption`
records such an assumption under `## Stated assumption (reviewable)` in a PR
body, with the alternatives considered and how to undo work built on it.
High-stakes ambiguity — a security boundary, credential, data migration,
public interface / breaking change, or destructive / irreversible action —
always blocks with exit 2 instead; it is never resolved by assumption.

## Commands

| Command | Purpose |
| --- | --- |
| `autospec spec-repair propose --repo OWNER/REPO --issue N --proposal-file <path.json> [--ledger-file <path>] [--origin-template <t>] [--origin-command <c>] [--origin-author <a>] [--judged-unusable]` | post the five-part proposal, label the issue, record the defect in the ledger; exits 0 with `outcome` `posted`, `already-proposed`, or `escalated-to-human`; exits 2 when the issue is not in the mechanical dead-end state unless `--judged-unusable` is passed |
| `autospec spec-repair check --repo OWNER/REPO --issue N` | classify the maintainer reply after the latest proposal; exits 0 with `outcome` `no-proposal`, `awaiting-maintainer`, `approved` (label removed), or `rejected` (label stays) |
| `autospec spec-repair assumption --statement <text> --pr-body-file <path> [--context-file <path>] [--alternative <text>]... [--rollback <text>]` | append the reviewable assumption section (low stakes) or block with exit 2 (high stakes); idempotent per PR body |
| `autospec spec-repair stall --consecutive-runs N [--issue N]` | report whether spec review is required at the two-run threshold |
| `autospec spec-repair ledger --file <path> [--issue N] [--shape <id>] [--template <t>]` | query the JSONL ledger; reports totals, events, and summaries by defect shape and origin template |

All commands print one JSON line on stdout.

## Repair ledger

Every posted proposal appends one JSON line to the ledger (default
`.autospec/spec-repair-ledger.jsonl`) recording: when, which repository and
issue, the defect classification, and the issue's origin — template, creating
command, and author. One badly written issue is noise; the same defect
recurring from one template is a systemic problem, and the ledger makes that
recurrence queryable via `autospec spec-repair ledger` summaries. Escalations
derive from this count, so the ledger is the loop's memory.
