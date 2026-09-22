# Runbook: the Woodpecker gate

autospec's CI gates run on Woodpecker at <https://ci.metabolomics.us>. This
runbook says what runs, where it runs, how to reproduce a failure by hand, and
which parts of the previous TeamCity setup are **not** reproduced.

## What replaced what

Seven TeamCity build configurations published seven separate commit statuses
onto every pull request. They are now steps of one pipeline, so the repository
has one check.

| TeamCity build configuration       | Woodpecker step        | `woodpecker-gates.sh` argument |
| ---------------------------------- | ---------------------- | ------------------------------ |
| `Autospec_AccessibilityWorkstream` | `accessibility-workstream` | `accessibility`            |
| `Autospec_ArchitectureFitness`     | *(not in the pipeline)* | `architecture-fitness`        |
| `Autospec_FileSizeRatchet`         | `file-size-ratchet`    | `file-size-ratchet`            |
| `Autospec_PythonSuites`            | `python-suites`        | `python-suites`                |
| `Autospec_SecurityWorkstream`      | `security-workstream`  | `security-workstream`          |
| `Autospec_StackGuard`              | `stack-guard`          | `stack-guard`                  |
| `Autospec_UxUiWorkstream`          | `ux-ui-workstream`     | `ux-ui-workstream`             |

Configuration lives in two files:

- `.woodpecker.yml` — when the pipeline runs, the checkout, and the step fan-out.
- `ops/ci/woodpecker-gates.sh` — every gate's actual commands.

## Running a gate by hand

The gate script takes no Woodpecker-specific input it cannot default. From a
clean checkout:

```bash
bash ops/ci/woodpecker-gates.sh                               # every gate the pipeline runs
bash ops/ci/woodpecker-gates.sh file-size-ratchet stack-guard # a subset
bash ops/ci/woodpecker-gates.sh architecture-fitness          # not in the pipeline; red today
```

A bare invocation runs the **six** gates the pipeline runs, so it passes on a
clean checkout. `architecture-fitness` is implemented in the same script but
is not in that list and runs only when named — it fails on `main` today, and a
bare run that inherited that failure would make the documented
reproduce-it-locally command useless.

Outside CI the `CI_*` variables are unset and each gate falls back to the
defaults the TeamCity steps used: base branch `main`, and `HEAD`'s own sha in
place of `GITHUB_SHA`.

Thresholds are environment variables, set on the step in `.woodpecker.yml` and
defaulted in the script:

| Variable                   | Default | Gate               |
| -------------------------- | ------- | ------------------ |
| `MAX_LOC`                  | `600`   | file-size-ratchet  |
| `HARD_LOC`                 | `2000`  | file-size-ratchet  |
| `AUTOSPEC_PR_SIZE_STRICT`  | `0`     | stack-guard        |
| `WOODPECKER_JOURNAL_DIR`   | unset   | all (log journal)  |

## What an agent actually is

A Woodpecker agent here is a Slurm job on a Rocky 9 node, but the agent execs
itself into an Apptainer image (`woodpecker/images/ci.sif`) and runs every step
as a child process. **A step therefore sees the image, not the node**:

- Debian 13 (trixie), `python3` 3.13.5, plus `git`, `jq`, `node`, `npm`, `cargo`,
  and `shellcheck` 0.10.0 (added by the image rebuild of 2026-09-22).
- **Not** present: `gh`, `python3.12`.
- No docker, no `sudo`, no `apt`. Anything a step needs is either in the image
  already or installed by the step into a temp dir (a venv is fine).
- One container per *workflow*, not per step: all steps share a workspace.

`image:` in `.woodpecker.yml` is a **host executable**, not a container image —
`image: bash` means the job's own shell.

## Reading a failure

Woodpecker's agent reproducibly drops a step's final log chunk, so a gate that
fails in under about two seconds can show an empty step log. Every gate
therefore prints a journal path as its first line and tees its whole run there:

```
journal: /home/wohlgemuth/woodpecker/logs/autospec-gates-<sha>-<ts>-<gate>.log
```

That path is on the cluster; read it from `whiteale`. The Woodpecker server's
sqlite `log_entries` table holds the same output when the step log survived.

## Not reproduced from TeamCity

These are deliberate gaps, not oversights. Each has an issue.

- **`Autospec_ArchitectureFitness` is not in the pipeline.** The gate runs fine
  on these agents, but `rust_core_cli_direction` has been failing on `main`
  continuously (73 occurrences against a threshold of 0), so adding it would
  make the new check red from its first commit and stall the auto-merger.
  TeamCity keeps asserting it until the debt is cleared. The registry filter
  that drops `latency_budget_validate_fast` is implemented and commented in
  `gate_architecture_fitness`, ready for the day the gate is green.
- **`stack-guard`'s linearity half is inert.** `scripts/stack-guard.sh` calls
  `gh pr list` to check that a PR's base is the head of another open PR. There
  is no `gh` in the image, so the open-head list is always empty and any PR
  based on something other than the default branch reads as non-linear —
  advisory at `AUTOSPEC_PR_SIZE_STRICT=0`, so it does not block. The per-layer
  `PR_SIZE` half is fully reproduced.
- **`python-suites` runs on 3.13, not 3.12.** TeamCity asserted
  `sys.version_info[:2] == (3, 12)` against its Ubuntu 24.04 agent. This image
  is Debian 13 and has only 3.13.5, and a step cannot install another
  interpreter. The assertion is kept and the number moved, so an image rebuild
  onto a different python fails loudly instead of silently changing what the
  suites exercise.
- **No gate runs `shellcheck`.** It is in the image as of 2026-09-22, but none
  of the seven TeamCity configurations used it — the two that lint shell run
  `bash -n` and stop there. Adding it would be a new check arriving under cover
  of a migration, and with 400+ tracked shell scripts the severity floor it
  starts at is a decision of its own. `bash -n` is reproduced as-is.
- **`latency_budget_validate_fast` is excluded**, as it was on TeamCity. It is
  `command_max_ms` against `/usr/bin/true` with a 50 ms budget: it measures
  process-spawn overhead on whichever node Slurm picked, not anything about
  this repository's code.
