# Stall handoff, partial-work capture, and model rotation

Issue: #3563. Module: [`crates/autospec-core/src/stall`](../crates/autospec-core/src/stall).

## The problem this fixes

An implementer that wedges used to leave three things behind, all wrong:

1. **The issue stayed claimed.** A label is not a lease. Nothing said who held it
   or when that holding ended, so a dead agent's issue never came back to the queue.
2. **"No diff, please retry" when a diff existed.** Commits on the agent's branch
   and uncommitted edits in its worktree were read into a local variable and
   dropped. The next agent was told nothing had been produced and started over,
   discarding real work.
3. **"Same model, one retry."** There was no roster to rotate to, so a model that
   cannot do a particular task got the same task again with the same
   configuration, and after that the issue simply stopped.

The module's contract is the inverse: capture before teardown, record exactly what
was produced, rotate to a different model when one is configured, and escalate to
spec repair with the whole history when attempts run out.

## Components

| File | Responsibility |
|---|---|
| `lease.rs` | `IssueLease` — a claim with an expiry, renewed by observed progress rather than by a timer the agent controls. A wedged agent cannot renew, so its lease expires on its own. |
| `liveness.rs` | `LivenessMonitor` — classifies a run as `Deliberating`, `Producing`, `Quiet`, or `Hung` from two external signals (transcript growth, output growth) sampled over time. |
| `partial_work.rs` | `WorktreeEvidence` trait + `GitWorktreeEvidence`; `capture_partial_work` collects commits ahead of base, a commit patch, the working-tree patch, and a bounded transcript tail. `ArtifactStore` writes them `0700`/`0600`. |
| `attempts.rs` | `ModelRoster` (deduped, family-inferred, `select_next` skips models already attempted) and `AttemptHistory` (per-attempt model, configuration, duration, work produced, outcome). |
| `tracker.rs` | `IssueTracker` trait — `release_to_queue`, `bump_attempt_counter`, `attach`, `add_label`, `escalate_to_spec_repair`. GitHub is one implementation; nothing in `stall/` knows that. |
| `release.rs` | `StallPolicy`, `plan_release` — the pure decision: `Completed`, `Requeue { next_model, changed_family }`, or `Escalate { report }`. |
| `note.rs` | `StallNote` (the block written to the issue before teardown) and `SpecRepairReport` (the handoff when attempts run out). |
| `mod.rs` | `StallRelease::finish` — the orchestration: capture, store, attach, then talk to the tracker. |

## Order of operations in `StallRelease::finish`

The order is the fix, so it is asserted by tests rather than left to review:

1. **Capture** evidence from the worktree. Nothing has touched the tracker yet, so
   a tracker outage cannot race the worktree out of existence.
2. **Store** every artifact locally under the artifact root
   (`attempt-N-commits.patch`, `attempt-N-working-tree.patch`,
   `attempt-N-transcript-tail.txt`, `attempt-N-stall-note.md`).
3. **Attach** each artifact to the tracker. A failure here is recorded in
   `ReleaseResult::attachment_errors` and does not abort the release: the local
   store still holds the work.
4. **Decide and report**: requeue with the next model and a `StallNote`, or apply
   `stalled-attempts-exhausted` + `spec-repair` and hand the
   `SpecRepairReport` — attempts, models, durations, work produced, artifact
   paths — to the spec-repair path.

A completed attempt touches the tracker not at all.

## Retry and escalation rules

* Roster has an untried model → `Requeue` onto it. `changed_family: true` when the
  family differs, which is what makes the rotation meaningful rather than cosmetic.
* Exactly one model configured → **one** same-model retry, then escalate with the
  reason stated as *rotation unavailable*, not a silent give-up.
* Every model tried, or `max_attempts` reached → escalate. The attempt limit is
  checked before rotation is chosen: a limit that only fires once the roster runs
  out is not a limit.
* Empty roster → degraded to a same-model retry, and the note says the roster is empty.

## Environment variables

See [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) for the table. `AUTOSPEC_GIT_PROGRAM`
exists because capture often runs where the PATH `git` is not the one you want:
container images, remote runners, or a pinned git build. Capture is
read-only (`git log`, `git diff`, `git ls-files`) and never mutates the worktree.

## Scope

This is the core library. The CLI subcommands that call it are wired separately —
`StallRelease` takes an `IssueTracker`, an `ArtifactStore`, a `StallPolicy` and a
`ModelRoster`, so the wiring is an adapter, not a change to these decisions.
