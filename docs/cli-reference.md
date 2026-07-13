# CLI Reference

The Rust CLI is additive. Existing `/autospec-*` skills and shell scripts remain the operational surface while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init` | no | documented stub, exits non-zero |
| `autospec doctor --json` | yes | implemented |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec status --json` | yes | persisted local spec-lifecycle counts |
| `autospec plan [--input <package-dir>] [--json]` | yes | read-only inspection of generated spec metadata |
| `autospec validate [--path <changed-path>]... [--json]` | yes | read-only affected-check planner; shell wrapper remains the executor |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec run` | no | documented stub, exits non-zero |
| `autospec resume` | no | documented stub, exits non-zero |
| `autospec report --json` | yes | local release summary from persisted spec state |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark` | no | documented stub, exits non-zero |
| `autospec growth-report --json` | yes | local-only metrics stub |

`autospec plan` only reads and parses Markdown from one generated package. It does not
execute validation, calculate an execution order, or report persisted lifecycle state.
