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

## Rust CLI

The `autospec` Rust binary exposes the V62+ command surface while preserving the skill-first workflow. `doctor`, `init`, `status`, `plan`, `validate`, `run`, `resume`, `report`, `showcase`, and `growth-report` support `--json`. `autospec init --spec <id>` creates local planned state without executing work. Direct `autospec validate [--path <changed-path>]...` is a read-only affected-check planner; `scripts/validate.sh` remains the executor for shell options such as `--fast`. `run` and `resume` only create, ingest, and inspect local queue state; `benchmark` remains a non-zero stub.

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
