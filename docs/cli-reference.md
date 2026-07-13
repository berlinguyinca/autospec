# CLI Reference

The Rust CLI is additive. Existing `/autospec-*` skills and shell scripts remain the operational surface while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init --spec <id> [--spec <id>]... [--json]` | yes | initialize persisted planned state without executing work; refuses existing state |
| `autospec doctor --json` | yes | implemented |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec status --json` | yes | persisted local spec-lifecycle counts |
| `autospec plan [--input <package-dir>] [--json]` | yes | read-only inspection of generated spec metadata |
| `autospec validate [--path <changed-path>]... [--json]` | yes | read-only affected-check planner; shell wrapper remains the executor |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec run --run <id> --spec <id>... [--json]` | yes | creates a local persisted queue only; it does not launch an agent or validation command |
| `autospec run --ingest <agent-result.json> --run <id> --spec <id> --result-id <id> --outcome <passed\|failed\|blocked> [--failure-kind <kind>] [--retry-limit <n>] [--json]` | yes | validates and records an explicit local agent result; it does not launch an agent or validation command |
| `autospec resume [--json]` | yes | reports the newest incomplete local queue and its next entry; it does not execute it |
| `autospec report --json` | yes | local release summary from persisted spec state |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark` | no | documented stub, exits non-zero |
| `autospec growth-report --json` | yes | local-only metrics stub |

`autospec plan` only reads and parses Markdown from one generated package. It does not
execute validation, calculate an execution order, or report persisted lifecycle state.

`autospec run` is deliberately a state-management command, not an execution engine. Queue
creation requires an explicit run ID and one or more spec IDs. Result ingestion requires a
strict `schemas/autospec-agent-result.schema.json` document plus an explicit outcome and
result ID; `failed` also requires `--failure-kind` (`validation`, `environment`, `agent`,
`dependency`, or `safety`). Results are retained append-only below
`.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json`, so a retry can safely
replay the same result ID without consuming another queue attempt. `resume` only reports the
current queue position. Use `/autospec-run` for the existing agent-execution workflow.
