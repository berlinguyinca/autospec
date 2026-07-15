# Workflows

## Plan A Feature

Use `/autospec-define` when you want a spec and issue queue before implementation starts.

Output:

- `docs/specs/*.md` design spec
- Parent issue
- Linked child issues
- Model-fit labels

## Ship Ready Issues

Use `/autospec-run` when issues already carry `auto-implement`.

Output:

- Branch per issue
- Pull request per issue
- Validation output
- Reviewer result
- Closeout report

## V62+ Core State

The Rust core adds a planning foundation beneath the existing workflows. Specs move through explicit lifecycle states: planned, ready, running, passed, failed, blocked, deferred, and superseded. Validation registry entries record command, working directory, timeout, and whether a gate is required.

The lifecycle store persists a schema-versioned document at `.autospec/state/specs.json`. A save first writes and synchronizes `specs.json.tmp`, then promotes it. On startup, a complete temporary document can recover a missing or malformed primary file; a malformed document without that recovery file is an error, never an empty state. The state store remains non-executing and does not replace `/autospec-run`; it gives later V66+ queue work a deterministic state model while current shell workflows remain the operational surface.

The V66 queue layer builds on that state with ordered entries, attempts, failure classification, blocked-spec handoff markdown, and final run-report summaries. Its local run model persists under `.autospec/runs/<run-id>/queue.json` and can select the newest incomplete valid run. `autospec run --run <id> --spec <id>` creates that local queue only. `autospec run --ingest <agent-result.json> ...` accepts a strict agent-result document plus an explicit typed outcome, persists it under an append-only result ID, and updates the matching queue entry exactly once. `autospec resume` reports the newest incomplete queue and its next entry. None of these commands launches an agent, invokes a shell, or runs validation; `/autospec-run` remains the operational execution workflow.

The Rust claim-control-plane cutover now owns `autospec claim`: strict parsing and rendering of
schema-1 GitHub run-state comments, lowest-comment-ID selection, duplicate collapse, linked-PR
reconciliation, typed safety eligibility, heartbeat/label ordering, lease CAS, and terminal
release transitions. The next step redirects every live caller to this one command family before
deleting the former shell authorities.

The Rust ready queue is `autospec queue ready`. It scans every GitHub page for open
`auto-implement` work and active claims, retains raw issue-page cardinality before excluding
pull-request records, cursor-paginates linked pull-request evidence, preserves check snapshots,
deduplicates unstable page results, and reports typed gate totals. A malformed or incomplete
later evidence page fails closed. A `scan_scope` of
`slice` means `AUTOSPEC_RUN_ONLY_ISSUES` constrained the selection; only a `repository` scan can
supply whole-queue completion evidence.

`autospec queue review-safety --repo OWNER/REPO --limit N [--issue N]` is the bounded Rust
writeback pass for unreviewed open queue work. `--issue N` targets the newly admitted issue
without relying on queue-list ordering. It writes a canonical passing block only after a typed re-read,
labels ambiguity as `autospec:needs-human`, quarantines blocking intent, and reports every
mutation or fail-closed conflict as structured totals.

Admission surfaces may only persist ordinary issue metadata and add interim
`auto-implement`; they must then call the exact Rust review command and must not
write a safety outcome themselves. Each applied grooming cycle also retries a
bounded set of interim issues, so a transient review failure remains recoverable.
`AUTOSPEC_GROOM_SAFETY_BIN` is a test-injection seam only; production resolves
the Rust `autospec` binary through `AUTOSPEC_BIN` or `PATH`.

## Rust Autonomous Conductor

The Rust conductor is a pure persisted control-plane state machine. Its phases
are `scan`, `review`, `select`, `claim`, `dispatch`, `dispatch_recorded`,
`retry`, `paused`, `slice_complete`, and `all_done`. It retains its repository,
queue scope, selected issue, serialization reasons, retry count, recorded
outcome, pause reason, and terminal reason in a schema-versioned state value.
It launches no process, mutates no GitHub state, and chooses no shell backend.

`SLICE_COMPLETE` means a constrained (`slice`) scan found no remaining work. It
does not establish repository completion, so the caller must discard that
constraint and perform a repository scan. `ALL_DONE` means a repository-scoped
scan found no work after completion reconciliation, including a rescan after a
serialized `priority:high` issue. `ALL_DONE` is a queue result only; it never
authorizes autonomous discovery to stop permanently.

The foreground CLI adapter persists this state under its repository-scoped
autonomous directory in separate repository and exact-slice files, and does not
delegate selection or dispatch to a script.
For the current cutover, its direct Rust `executor-result` child returns only a
blocked/deferred receipt because a typed implementation-agent protocol has not
been introduced. The adapter records that receipt before claim reconciliation,
then leaves the selected issue paused and claimed. It must not requeue or mark
the issue complete merely because the receipt process exited successfully.

## Rust CLI

The `autospec` Rust binary exposes the V62+ command surface while preserving the skill-first workflow. `doctor`, `init`, `status`, `plan`, `validate`, `run`, `resume`, `report`, `showcase`, and `growth-report` support `--json`. `autospec init --spec <id>` creates local planned state without executing work. Direct `autospec validate [--path <changed-path>]...` is a read-only affected-check planner, while `autospec validate --shadow-results <file>` aggregates pre-captured results without spawning a command. `autospec validate` remains the executor for shell options such as `--fast`. `run` and `resume` only create, ingest, and inspect local queue state; `benchmark` remains a non-zero stub.

See [`docs/cli-reference.md`](cli-reference.md) for the command table.

## Split An Existing Spec

Use `/autospec-split` when a design already exists under `docs/specs/`.

Output:

- Parent issue
- Child issues
- Classification
- Handoff to `/autospec-run`

## Audit Release Readiness

Use `/autospec-release` when you need a release verdict.

Output:

- Validation summary
- QA status
- Docs drift status
- Blocker list
- Release verdict

## Explain The Repository

Use `/autospec-story` when you need a cited narrative of what the repo does and what has shipped.

Output:

- Product story
- Implementation-state overview
- References to specs, docs, issues, PRs, and git history

## Stop Or Resume

Use `/autospec-stop` and `/autospec-resume` for long-running monitor control.

Output:

- Graceful stop, immediate pause, or resume action
- Preserved issue context
- Clean monitor state
