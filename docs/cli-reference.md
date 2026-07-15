# CLI Reference

The Rust CLI is additive. Runtime environment management is implemented by the
Rust command family below; existing `/autospec-*` skills and unrelated shell
scripts remain operational surfaces while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init --spec <id> [--spec <id>]... [--json]` | yes | initialize persisted planned state without executing work; refuses existing state |
| `autospec doctor --json` | yes | implemented |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec status --json` | yes | persisted local spec-lifecycle counts |
| `autospec plan [--input <package-dir>] [--json]` | yes | read-only inspection of generated spec metadata |
| `autospec validate [--path <changed-path>]... [--json]` | yes | read-only affected-check planner; shell wrapper remains the executor |
| `autospec validate --shadow-results <captured-results.json> [--json]` | yes | aggregates captured shell outcomes without executing commands; returns non-zero when a required captured result failed |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec runtime env init [--repo <path>] [--manifest agent\|autospec] [--force]` | no | creates a conservative v1 runtime manifest; refuses an existing manifest without `--force` |
| `autospec runtime env up [--repo <path>] [--mode <mode>]` | no | provisions or reuses the selected environment, runs its manifest command on first provision, and prints the sourceable environment protocol |
| `autospec runtime env status [--repo <path>] [--mode <mode>]` | no | prints a provisioned environment or returns status `3` when it is inactive |
| `autospec runtime env down [--repo <path>] [--mode <mode>]` | no | runs the selected optional teardown command and removes its state after successful teardown |
| `autospec runtime env exec [--repo <path>] [--mode <mode>] -- <command> [args...]` | no | provisions or reuses state, then runs one direct child with the runtime environment |
| `autospec runtime env session [--repo <path>] [--mode <mode>] [--keep-alive] -- <command> [args...]` | no | runs one direct child with lifecycle cleanup, manifest auto-init/bypass controls, and Unix interruption cleanup |
| `autospec claim state read\|upsert\|clear\|reconcile-linked-pr ...` | yes | manages the schema-1 GitHub run-state comment using lowest-comment-ID selection |
| `autospec claim acquire\|release ...` | yes | applies the typed safety gate, heartbeat/label ordering, lease CAS, and terminal release transitions |
| `autospec queue ready [--repo OWNER/REPO] [--batch-size N]` | yes | scans every Rust-owned GitHub issue page and returns typed eligibility, gate totals, and scan scope |
| `autospec run --run <id> --spec <id>... [--json]` | yes | creates a local persisted queue only; it does not launch an agent or validation command |
| `autospec run --ingest <agent-result.json> --run <id> --spec <id> --result-id <id> --outcome <passed\|failed\|blocked> [--failure-kind <kind>] [--retry-limit <n>] [--json]` | yes | validates and records an explicit local agent result; it does not launch an agent or validation command |
| `autospec resume [--json]` | yes | reports the newest incomplete local queue and its next entry; it does not execute it |
| `autospec report --json` | yes | local release summary from persisted spec state |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark` | no | documented stub, exits non-zero |
| `autospec growth-report --json` | yes | local-only metrics stub |

`autospec plan` only reads and parses Markdown from one generated package. It does not
execute validation, calculate an execution order, or report persisted lifecycle state.

`autospec validate --shadow-results` accepts the strict schema-1 captured-result shape used
by `crates/autospec-cli/tests/fixtures/validation-results/`: each row supplies a unique name,
Boolean `required`, and signed `exit_code`. Rust computes the pass/fail aggregate only; it never
spawns the captured command. The compatibility wrapper still delegates all real validation to
`autospec validate` until a full fixture-backed cutover is approved.

`autospec run` is deliberately a state-management command, not an execution engine. Queue
creation requires an explicit run ID and one or more spec IDs. Result ingestion requires a
strict `schemas/autospec-agent-result.schema.json` document plus an explicit outcome and
result ID; `failed` also requires `--failure-kind` (`validation`, `environment`, `agent`,
`dependency`, or `safety`). Results are retained append-only below
`.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json`, so a retry can safely
replay the same result ID without consuming another queue attempt. `resume` only reports the
current queue position. Use `/autospec-run` for the existing agent-execution workflow.

For the v1 runtime-manifest grammar, state behavior, child-command semantics, and cleanup
procedure, see [Agent runtime manifests](runbooks/agent-runtime-manifest.md).

`autospec claim state` is the Rust-owned transport and codec for the existing GitHub
run-state comment protocol. `read` fails closed when the lowest marked comment is malformed
or bound to a different issue; `upsert` patches that lowest comment and removes higher-ID
duplicates; `clear` removes only marked run-state comments; and `reconcile-linked-pr` records
one eligible linked PR before posting the idempotent post-PR handoff reminder. `acquire` validates
the current issue safety review before it writes a startup heartbeat and moves labels, then uses
the lowest GitHub comment ID plus a server-side timestamp to decide the lease. `release` writes
terminal merge evidence before state and label transitions. Legacy script entrypoints remain only
as compatibility surfaces until every caller is redirected to this command family.

`autospec queue ready` follows every GitHub REST page for open `auto-implement` work and active
claims, counts raw issue-page records before filtering pull requests, and cursor-paginates linked
pull-request evidence while preserving check snapshots. A malformed or incomplete later evidence
page blocks selection rather than shortening the scan. Its JSON includes a stable `gate_counts`
object for discovered, candidate, reviewed,
blocked, dependency-blocked, linked-PR-blocked, path-conflicted, ready, claimed, and selected
issues. `scan_scope` is `repository` for a full scan and `slice` when
`AUTOSPEC_RUN_ONLY_ISSUES` constrains the result, so callers cannot mistake a completed slice for
whole-queue completion.
