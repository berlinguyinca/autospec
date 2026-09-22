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

The pipeline also reproduces one **GitHub Actions** job, which is a separate
matter from the TeamCity migration and the reason the pipeline is now minutes
rather than seconds:

| GitHub Actions job (`.github/workflows/rust.yml`) | Woodpecker steps | `woodpecker-gates.sh` arguments |
| --- | --- | --- |
| `build-test` | `rust-tools` → `rust-clippy` → `rust-ownership-contracts` → `rust-workspace-test` → `rust-catalog-parity` → `rust-validate` → `rust-build` → `rust-behaviour-probes` | the same eight names |

`build-test` is the check `main`'s branch protection requires. Its last GitHub
run was 2026-09-17 and it failed; nothing has produced the status since, so
every pull request has been blocked on a check that is neither passing nor
being recomputed. The eight steps above are that job, split by its own step
names so a failure is attributable, and running in sequence because they share
one cargo target directory. See **The `build-test` job** below for the parts
of it that do not run here.

Configuration lives in three files:

- `.woodpecker.yml` — when the pipeline runs, the checkout, and the step fan-out.
- `ops/ci/woodpecker-gates.sh` — the TeamCity gates' actual commands, plus the
  dispatch table and the shared helpers.
- `ops/ci/woodpecker-rust-gates.sh` — the eight `rust-*` gates. **Sourced** by
  the file above, never run on its own. It is a separate file because the
  file-size ratchet this repository gates on caps a file at 600 lines, and the
  two together would be ~875; the split follows the seam between one GitHub
  Actions job and the TeamCity migration.

## Running a gate by hand

The gate script takes no Woodpecker-specific input it cannot default. From a
clean checkout:

```bash
bash ops/ci/woodpecker-gates.sh                               # every gate the pipeline runs
bash ops/ci/woodpecker-gates.sh file-size-ratchet stack-guard # a subset
bash ops/ci/woodpecker-gates.sh architecture-fitness          # not in the pipeline; red today
```

A bare invocation runs the **fourteen** gates the pipeline runs — the six
workstream gates plus the eight `rust-*` gates — so it passes on a clean
checkout, and takes the same time the pipeline does.
`architecture-fitness` is implemented in the same script but is not in that
list and runs only when named — it fails on `main` today, and a bare run that
inherited that failure would make the documented reproduce-it-locally command
useless.

Every gate prints `ELAPSED <gate>: <n>s` when it finishes, so the cost of a
change to one of them is visible in its own log rather than only as a wall
clock number on the server.

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
| `CARGO_BUILD_JOBS`         | `4`     | every `rust-*` gate |

The `rust-*` gates also set three variables themselves, in `rust_env`, and
none of them can be left at its default in this image: `CARGO_HOME` and
`CARGO_TARGET_DIR` move into the workspace because the image's `CARGO_HOME`
is on the read-only SIF, and `CI_TOOLS` (`.ci-tools`) is the one PATH entry
the pinned tools install into. All three are gitignored.

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

## The `build-test` job

### The tools it pins, and where each one comes from here

`build-test` installs its own tools rather than trusting the runner image,
verifying each archive's sha256 before unpacking it. That pattern transfers
unchanged; the versions and digests in `ops/ci/woodpecker-rust-gates.sh` are copied
from `.github/workflows/rust.yml` and must be changed in both places at once,
or the two CIs test different software while both report green.

| Tool | On GitHub | Here |
| --- | --- | --- |
| `codex` + `bwrap` | sha256-pinned musl tarball | same |
| `gitleaks` | sha256-pinned tarball | same |
| `trivy` | sha256-pinned tarball | same |
| `semgrep` | **docker image digest** | **not installed — see below** |
| `gh` | preinstalled on the runner | sha256-pinned tarball |
| `ripgrep` | `sudo apt-get install ripgrep` | sha256-pinned tarball |
| `bats` | `sudo apt-get install bats` | cloned at its tag, verified by commit |
| `ajv`, `license-checker` | npm, via `dev-bootstrap.sh` | npm, version-pinned |
| `pytest`, `pyyaml` | `sudo apt-get install python3-…` | already in the image |

`bats-core` publishes no binary asset, and a GitHub source-archive tarball is
regenerated on demand so its sha256 is not a stable pin. The tag's commit is,
and it is the stronger pin: a moved tag changes it and the gate stops.

`scripts/dev-bootstrap.sh` still runs, after those installs rather than
instead of them. It prefers `apt` whenever `apt-get` exists — which it does in
this image — and then calls `sudo apt-get`, and there is no `sudo`. Every
`install_*` function in it is idempotent, so pre-installing is what lets it
reach `check_tools`, which is the part worth having: the repository's own
statement of what a working checkout needs.

### Environment the image forces

- `CARGO_HOME` defaults to `/usr/local/cargo`, which is on the **read-only**
  SIF. The first `cargo fetch` dies with `could not create temp file …:
  Read-only file system`, which names the filesystem and not the cause.
  `CARGO_HOME` and `CARGO_TARGET_DIR` therefore move into the workspace.
  `RUSTUP_HOME` is left alone: the pinned 1.91.0 toolchain is baked into the
  image, and re-downloading it per pipeline would be pure cost.
- npm's global prefix is `/usr/local`, on the same read-only SIF, so the npm
  tools install with `--prefix` into the workspace and are symlinked onto the
  one PATH entry the gates add.
- `CARGO_BUILD_JOBS` is capped at 4, below the 8 CPUs an agent asks Slurm for.
  An agent gets `--mem=18G` and rustc's peak is per codegen unit; an
  OOM-killed run is *unverified*, not red, and reports a signal rather than a
  test result.
- There is **no cargo cache between pipelines.** The workspace is a fresh
  `mktemp -d` per workflow, so every run compiles the dependency graph from
  scratch. GitHub's `build-test` has `actions/cache` and does not. This is the
  single largest component of the pipeline's duration, and the first thing to
  change if it needs to come down.

### Not reproduced: `semgrep`

`build-test` pins semgrep as a **docker image digest**
(`semgrep/semgrep@sha256:44dd022c…`) and puts a `docker run` wrapper on PATH.
There is no docker in this image — `ci.def` refuses it deliberately — and a
container runtime is the only way to run that exact artifact. Two alternatives
were considered and rejected:

- `pip install semgrep==1.173.0` is a different artifact with no digest pin.
  Substituting it silently would mean the two CIs scan with different software
  while both say "semgrep".
- A shim on PATH satisfying `command -v semgrep` would turn the tests that
  require it green by fabrication.

`install.sh` declares `AUTOSPEC_EXECUTOR_SCANNERS="gitleaks semgrep trivy
license-checker"` `readonly`, so the install suites that assert the scanner
set cannot pass with semgrep absent. What that costs, exactly, is in the issue
linked from the pull request that added these steps. GitHub Actions keeps
asserting `build-test`, semgrep included, until it is resolved.

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
