# autospec

Autospec is a multi-harness AI workflow suite for turning product intent into
tracked, reviewable, and explainable software changes.

It helps an agent move from a feature request to a written spec, from that spec
to a tree of GitHub issues sized for different model capabilities, and from
those issues to autonomous pull requests with validation, review, and merge
control. It also keeps the story behind the code available afterward: specs,
issues, PRs, commits, and implementation state can be synthesized into a cited
repo overview.

Autospec works across **Claude Code**, **OpenCode**, and **Codex CLI**.

## Getting Started

Pick the skill that matches where you are:

| I want to... | Use | What happens |
| --- | --- | --- |
| Turn a feature idea into shipped PRs | [`/autospec`](skills/autospec/README.md) | Write the spec, create issues, implement, review, test, and merge. |
| Plan only, then stop | [`/autospec-define`](skills/autospec-define/README.md) | Create the spec and issue queue without running implementation. |
| Split an existing spec into issues | [`/autospec-split`](skills/autospec-split/README.md) | Turn `docs/specs/*.md` into classified GitHub issues. |
| Work through ready issues | [`/autospec-run`](skills/autospec-run/README.md) | Process the `auto-implement` queue and merge passing PRs. |
| Check whether the repo is ready to ship | [`/autospec-release`](skills/autospec-release/README.md) | Run sweep, review, implementation, tests, QA proof, docs sync, and legacy cleanup gates. |
| Keep specs, docs, tests, and code aligned over time | [`/autospec-sweep`](skills/autospec-sweep/README.md) | Configure and run recurring repo sweeps. |
| Prove the running app actually matches the spec | [`/autospec-qa`](skills/autospec-qa/README.md) | Revalidate UI controls, validation, API behavior, accessibility, and no-mock smoke paths. |
| Stop or resume a long-running monitor | [`/autospec-stop`](skills/autospec-stop/README.md) | Gracefully stop, immediately pause, check status, or resume. |
| Understand what has been built | [`/autospec-story`](skills/autospec-story/README.md) | Produce a cited repo story from specs, issues, PRs, and docs. |

If you are unsure, start with `/autospec-release` for an existing repo or
`/autospec` for a new feature.

## What It Solves

AI-generated code can move fast without leaving a good trail of why it exists,
what spec produced it, which model worked on it, and which parts actually work.
Autospec makes that process auditable.

It gives you:

- A durable spec before implementation starts.
- A linked 1:n issue tree instead of one oversized task.
- Model-fit metadata on each issue with `ctx:*` and `reasoning:*` labels.
- Implementation queues that can be filtered by model profile.
- Quality gates for issue shape and implementation scope.
- Autonomous PR creation, counter-team review, full-suite validation, CI checks, and admin squash-merge.
- Stop/resume controls for long-running monitors.
- A read-only story mode that explains the current application state from local
  docs plus GitHub issues and PRs.

## Core Workflow

```text
feature request
  -> investigation
  -> design spec
  -> EPIC issue
  -> linked child issues
  -> Phase 3.5 model-fit classification
  -> implementation monitor
  -> PR per issue
  -> review + checks + merge
  -> final report / repo story
```

Child issues are written for small and large models alike. They include staged
context, files to read first, implementation scope, acceptance criteria, and one
primary smoke test. The goal is to make every unit of work small enough for the
selected model and harness to execute reliably.

## Skills

| Skill | Use it when | Result |
| --- | --- | --- |
| [`autospec`](skills/autospec/README.md) | You want the full path from feature request to merged PRs. | Bootstraps if needed, investigates, writes a spec, creates issues, classifies them, runs implementation, and reports completion. |
| [`autospec-release`](skills/autospec-release/README.md) | You want to know whether the current repo is ready to ship. | Runs the release-readiness loop across sweep, review, run, test, QA, docs sync, proof artifacts, and legacy cleanup. |
| [`autospec-sweep`](skills/autospec-sweep/README.md) | You want first-run configuration or continuous improvement across specs, docs, tests, and code. | Creates `.autospec/autospec.yml`, runs a configured sweep, writes `.autospec/sweep/latest.json`, and routes recurring gaps back through specs, issues, and `/autospec-run`. |
| [`autospec-define`](skills/autospec-define/README.md) | You want planning only before implementation starts. | Produces a design spec plus classified `auto-implement` issues, then hands off to `/autospec-run`. |
| [`autospec-split`](skills/autospec-split/README.md) | You already have a tracked `docs/specs/*.md` design spec. | Turns the existing spec into an EPIC plus linked child issues, then stops after classification. |
| [`autospec-run`](skills/autospec-run/README.md) | You already have an `auto-implement` queue. | Runs the implementation monitor, opens PRs, reviews, validates, and merges. |
| [`autospec-fleet`](skills/autospec-fleet/README.md) | You want to prepare multi-repo autospec supervision. | Provides config schemas/linting, URL path planning, dry-run `/autospec-run` command generation, JSON status, stop forwarding, and smoke tests. |
| [`autospec-classify`](skills/autospec-classify/README.md) | Existing issues need model-fit labels. | Adds `ctx:*` and `reasoning:*` labels, inserts a `## Model fit` block, and promotes `needs-classify` issues. |
| [`autospec-listen`](skills/autospec-listen/README.md) | You want chat phrases like "file an issue" to become tracked work. | Drafts issues for approval or routes spec requests into `/autospec-define`. |
| [`autospec-story`](skills/autospec-story/README.md) | You need a repo-level product and implementation-state overview. | Produces a cited Markdown story from local specs, docs, issues, PRs, and git history. |
| [`autospec-stop`](skills/autospec-stop/README.md) | You need to halt or resume an active monitor. | Writes the shared stop sentinel, pauses issues safely, reports status, or resumes paused work. |
| [`autospec-review`](skills/autospec-review/README.md) | You want to close the spec-vs-code feedback loop. | Audits specs against issues, finds gaps, files `[REGRESSION]` issues with `priority:high`. |
| [`autospec-test`](skills/autospec-test/README.md) | You want every Phase 4 PR gated on unit + E2E coverage with auto-heal. | Runs a two-stage coverage gate, auto-heals gaps within a 60-min budget, and blocks assertion-loosening rewrites before auto-merge. |
| [`autospec-qa`](skills/autospec-qa/README.md) | You want to revalidate a running app against its spec and regenerate weak tests. | Builds a spec traceability matrix, exercises UI/API/accessibility/validation flows, and turns gaps into stronger tests or follow-up issues. |
| [`autospec-design`](skills/autospec-design/README.md) | You want the repo's UI anchored to a known vendor design language (Apple, Linear, Stripe, etc.). | Fetches a `DESIGN.md` from the `berlinguyinca/awesome-design-md` catalog via `suggest`, `apply`, and `migrate` subcommands, writes `DESIGN.md` to the project root on a feature branch, and optionally hands off a per-component migration spec to `/autospec-define`. |

See [`SKILLS.md`](SKILLS.md) for activation keywords and per-skill routing
details.

## Install

One-line install for the full suite into every supported harness:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Windows PowerShell can use the native bootstrap, which installs Git/Bash through
`winget`, `choco`, or `scoop` when available:

```powershell
irm https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.ps1 | iex
```

The bootstrap clones (or fast-forwards) autospec into `~/.autospec/repo` and then runs `./install.sh --skill all --harness all`. Re-run any time to update.

Forward flags to the underlying installer:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh \
  | bash -s -- --skill autospec --harness claude
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh \
  | bash -s -- --skill all --harness opencode
```

The bootstrap honors:

| Variable | Purpose |
| --- | --- |
| `AUTOSPEC_HOME` | Base dir for the autospec checkout + state (default: `~/.autospec`). |
| `AUTOSPEC_REF` | Git ref (branch, tag, or sha) to install (default: `main`). |
| `AUTOSPEC_REPO_URL` | Override the remote URL — useful for forks or mirrors. |

The installer also honors:

| Variable | Purpose |
| --- | --- |
| `CLAUDE_CONFIG_DIR` | Override Claude Code install root. |
| `OPENCODE_CONFIG_DIR` | Override OpenCode install root. |
| `CODEX_HOME` | Override Codex install root. |
| `AUTOSPEC_NO_STAR_PROMPT=1` | Skip the optional adoption star prompt. |
| `AUTOSPEC_SKIP_SYSTEM_TOOLS=1` | Skip best-effort installation of required/recommended CLIs. |
| `AUTOSPEC_SKIP_ECOSYSTEM_BOOTSTRAP=1` | Skip peer ecosystem bootstrap. |
| `AUTOSPEC_SKIP_SUPERPOWERS=1` | Skip Superpowers clone/link/OpenCode plugin setup. |
| `AUTOSPEC_SKIP_OH_MY_CODEX=1` | Skip `oh-my-codex` npm install/setup. |
| `AUTOSPEC_SKIP_OH_MY_OPENCODE=1` | Skip `oh-my-opencode` npm install/setup. |
| `AUTOSPEC_SKIP_OH_MY_CLAUDE=1` | Skip `oh-my-claude`/OMC npm install/setup. |

After a successful interactive suite install, the top-level installer asks
whether you want to star `berlinguyinca/autospec` to support adoption. It is
prompted through `/dev/tty`, so one-line `curl | bash` installs can still ask
when a terminal is available. The prompt is skipped for headless installs,
`--update`, CI, missing `gh`, or `AUTOSPEC_NO_STAR_PROMPT=1`.

### Manual install (from a checkout)

If you'd rather manage the checkout yourself — e.g. for development on
autospec itself — clone and run the installer directly:

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
./install.sh --skill all --harness all
```

Install a subset:

```bash
./install.sh --skill autospec-run --harness claude
./install.sh --skill autospec-split --harness all
./install.sh --skill all --harness opencode
./install.sh --skill autospec --harness all
```

Uninstall symmetrically:

```bash
./uninstall.sh --skill all --harness all
```

`uninstall.sh --skill all` removes every suite skill listed in the install
matrix, including `autospec-design`.

To enable Claude hook mode (PreCompact trigger instead of tmux polling):

```bash
bash install.sh --hook-mode claude
```

### Single-skill curl install (advanced)

Each per-skill installer is also callable standalone over curl:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec/install.sh \
  | sh -s -- --harness all
```

This installs only that skill's files and the small shared-script set listed
inside its installer. It does **not** fetch `skills/autospec-shared/scripts/**`,
so skills that depend on shared helpers (gap remediation, doc-drift scanning,
etc.) may fail at runtime. Prefer the one-line `bootstrap.sh` install above
unless you specifically want a single-skill, no-checkout footprint.

### Target repo setup

If you run autospec against your own codebase, see
[`docs/target-repo-setup.md`](docs/target-repo-setup.md) for the
operator-side opt-ins that pair with the Phase 4 implementer's
cross-session safety gates: branch protection on `main` plus a
migration-replay test convention. Both are voluntary; without them
autospec keeps working but loses the safety net for issue #307's
cross-session CI rot.

### Turbo integration

`install.sh` also bootstraps the recommended peer ecosystem on macOS, Linux, and
Windows hosts:

- Ensures common system tools best-effort: `git`, `bash`, `curl`, `jq`, `yq`, `gh`, `node`, `npm`, `bun`, `bats`, `codex`, `claude`, `opencode`, `omx`, `omc`, `oh-my-opencode`, `mempalace`, and `ajv`.
- Uses platform package managers when present: Homebrew, apt/dnf/yum/pacman/apk, winget, Chocolatey, Scoop, npm, pipx, uv, and pip.
- Installs/updates `oh-my-codex`, `oh-my-opencode`, and OMC (`oh-my-claude-sisyphus`) through npm, runs idempotent setup for OMX/OMC, and initializes `oh-my-opencode` only when its config is missing.
- Clones (or fast-forward pulls) [obra/superpowers](https://github.com/obra/superpowers), exposes its Codex skills at `~/.agents/skills/superpowers`, and adds the OpenCode plugin entry `superpowers@git+https://github.com/obra/superpowers.git`.
- Clones (or fast-forward pulls) `~/.turbo/repo` and symlinks turbo skills into `~/.claude/skills/`.
- Checks for the [Codex CLI](https://github.com/openai/codex). When present, autospec's Phase 4 implementer (issues labelled `autospec:v2-flow`) runs an inline peer-review pass on each diff before opening a PR. When absent, peer-review skips gracefully and the implementer continues.
- Idempotently merges an `<!-- autospec-block -->` section into `~/.claude/CLAUDE.md` documenting both stacks' entrypoints.
- Inside a git repo, offers to add `.autospec/` to `.gitignore` (auto-accept via `AUTOSPEC_AUTO_YES=1`).

Peer ecosystem bootstrap failures are non-fatal: offline or locked-down hosts continue with cached tools or the autospec install itself.

## Update

Each installed suite skill runs a startup self-update check at most once every
24 hours. It reinstalls from `main` when a newer commit is available by running
the curl-safe bootstrap with `--skill all --harness all --update`, so newly added
autospec skills are picked up during ordinary self-update. The check is
fail-open: network or install errors log a `WARN:` line and the installed skill
continues to run.

Disable startup self-update:

```bash
AUTOSPEC_NO_SELF_UPDATE=1
```

Force an in-place suite update — for a bootstrap install, re-run the one-liner:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --update
```

Or, from a manual checkout:

```bash
./install.sh --skill all --harness all --update
```

`--update` also fast-forwards the autospec checkout itself and refreshes the peer ecosystem (`superpowers`, `oh-my-*`, and `~/.turbo/repo`), so a single command keeps the whole autospec toolchain current.

Or invoke any installed skill with `update`:

```text
/autospec update
/autospec-define update
/autospec-split update
/autospec-run update
/autospec-classify update
/autospec-listen update
/autospec-story update
/autospec-stop update
/autospec-sweep update
```

## Model Fit and Profiles

Autospec classifies implementation issues along two axes:

| Axis | Labels | Meaning |
| --- | --- | --- |
| Context window | `ctx:32k`, `ctx:64k`, `ctx:120k` | How much staged context the issue needs. |
| Reasoning depth | `reasoning:shallow`, `reasoning:medium`, `reasoning:deep` | How much derivation the implementation requires. |

`/autospec-run --profile <name>` filters the queue against
`~/.autospec/model-profiles.yml`, so a smaller local model can pick only issues
that fit its limits while larger cloud models can take deeper work.

Example profile config:

```bash
mkdir -p ~/.autospec
cp examples/model-profiles.yml ~/.autospec/model-profiles.yml
```

## Quality Gates

Autospec validates both the work items and the resulting implementation.

Issue quality is checked by [`scripts/lint-issue.sh`](scripts/lint-issue.sh):

- `## Goal` must be concrete and one sentence.
- Acceptance criteria must be checkbox items with machine-checkable anchors.
- The primary smoke test must be one executable line.

Implementation quality is checked by
[`scripts/lint-implementation.sh`](scripts/lint-implementation.sh):

- Scope must match the issue body.
- Required tests must be present.
- Complexity, security, TODO, mock DB, invented config, duplicate code, and doc
  drift rules are enforced.

Shared helper scripts are installed into `~/.autospec/scripts`, so target repos
do not need to carry this repository's `scripts/` directory.

## Running Implementation

Typical split workflow:

```text
/autospec-define "add OIDC support behind a feature flag"
# review generated spec and issues
/autospec-run --profile claude-sonnet-cloud
```

Existing-spec workflow:

```text
/autospec-split split latest spec
/autospec-run
```

Full end-to-end workflow:

```text
/autospec "add OIDC support behind a feature flag"
```

Fleet helper workflow:

```text
bash skills/autospec-fleet/scripts/fleet-init.sh --dry-run --workspace .autospec-fleet/repos \
  https://github.com/org/repo-a https://github.com/org/repo-b
bash skills/autospec-fleet/scripts/fleet-config-lint.sh --config path/to/autospec-fleet.yml
bash skills/autospec-fleet/scripts/fleet-run.sh --config path/to/autospec-fleet.yml --dry-run --once
bash skills/autospec-fleet/scripts/fleet-status.sh --config path/to/autospec-fleet.yml --json
```

`autospec-fleet` currently exposes helper scripts for planning and dry-run
coordination. Live repository clone/sync and worker launch are not implemented
yet.

The monitor:

- Reconciles stale process heartbeats at startup.
- Prints live queue status after scans.
- Prints an issue summary before working on each issue.
- Claims one ready issue at a time.
- Opens a branch and PR for each issue.
- Runs self-review and implementation guardian checks.
- Admin squash-merges `auto-implement` PRs when the full target-repo test suite
  passes, required checks pass, and review is `LGTM`.

## Stop and Resume

Gracefully stop after the current issue:

```text
/autospec-stop --graceful
/autospec-run stop --graceful
```

Abort at the next major step boundary:

```text
/autospec-stop --immediate
```

Check or resume:

```text
/autospec-stop --status
/autospec-stop --resume
```

Stop state is stored in `~/.autospec/stop.flag`. Immediate stops preserve resume
context on the issue and mark it with `paused-by-user`.

## Usage-Limit Recovery

Autospec-run can hand off quota pauses to a shell supervisor before the harness
runs out of usable LLM turns. Set `AUTOSPEC_RESUME_COMMAND` to the exact command
that relaunches the same run, then arm the helper with the reset time or wait
duration:

```bash
"${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-usage-limit.sh" \
  arm --harness codex --repo-dir "$PWD" \
  --command "$AUTOSPEC_RESUME_COMMAND" --wait-seconds 1800
```

The supervisor stores state in `~/.autospec/usage-limits/`, polls every five
minutes, and relaunches after the reset time. `/autospec-run` uses the same
helper when a harness exposes `AUTOSPEC_USAGE_LIMIT_RESUME_AT` or
`AUTOSPEC_USAGE_LIMIT_WAIT_SECONDS`.

## Repo Story Mode

Use `/autospec-story` when you need to understand what a repository is, what has
been built, what remains conceptual, and which sources support that conclusion.

```text
/autospec-story
/autospec-story --output docs/autospec-story.md
/autospec-story --since 2026-05-01 --limit 200 --output docs/autospec-story.md
```

The report reconciles:

- Local specs and docs.
- Open and closed GitHub issues.
- Open, merged, and closed PRs.
- Recent git history.
- Dirty worktree state.

It separates evidence from inference so the output can be used in planning,
reviews, handoffs, or leadership updates.

## Repository Layout

```text
skills/
  <skill-name>/
    SKILL.md              # Claude Code skill format and canonical body
    README.md             # human-facing docs
    opencode/agent.md     # OpenCode variant
    codex/prompt.md       # Codex CLI variant
    install.sh            # per-skill installer
    uninstall.sh          # per-skill uninstaller

scripts/
  lint-issue.sh
  lint-implementation.sh
  autospec-stop.sh
  autospec-watchdog.sh
  listener-match.sh
  sizing-check.sh

examples/
  model-profiles.yml
  project-map.yml
```

The lock-step rule keeps multi-harness skill bodies byte-identical across
`SKILL.md`, `opencode/agent.md`, and `codex/prompt.md`. Only frontmatter may
differ. [`scripts/validate.sh`](scripts/validate.sh) enforces this.

## Validation

This repository has no language-level test runner. Validation is shell and Bats
based:

```bash
bash scripts/validate.sh
bats tests/unit tests/smoke
```

The validation suite checks:

- Lock-step multi-harness skill bodies.
- Frontmatter parsing.
- Installer and uninstaller syntax.
- Startup self-update block consistency.
- Shared helper installation.
- Model-tier directives.
- Issue and implementation lint helpers.
- Stop-mode and guardian invariants.
- Top-level install and uninstall smoke behavior.

## Docs amendment

Every autospec run produces first-class documentation artifacts committed to the target repo.
The reverse-engineer pipeline + doc generators are part of the autospec-shared tooling:

| Artifact | Description |
|---|---|
| `docs/USER_MANUAL.md` | Operator-facing narrative (skills, installation, usage) |
| `docs/API_REFERENCE.md` | Per-symbol reference for all public CLI surfaces and scripts |
| `docs/ARCHITECTURE.md` | System shape, module graph (mermaid), component responsibilities |
| `docs/ASSISTANT_PROMPT.md` | Paste-ready system prompt for Claude/GPT repo assistant |
| `docs/.llm-manifest.json` | Structured per-symbol manifest (schema v1.0) |
| `llms.txt` | Short curated index ≤200 lines (llmstxt.org convention) |
| `llms-full.txt` | Full concatenated doc content for context-window ingestion |

`/autospec-sweep` can also enforce a documentation matrix in
`.autospec/autospec.yml`. `documentation.audiences[]` names reader groups such
as users, developers, operators, and security reviewers; `documentation.scopes[]`
names product or operational surfaces such as API reference, runbooks, and
troubleshooting. Missing target files or required `autospec-doc-scope` markers
are emitted as separate docs gaps so the normal autospec loop can build deep
documentation per audience and scope.

**Doc-drift gate:** every Phase 4 PR is checked by `check-doc-drift.sh`. Source changes that
match a `<!-- autospec-doc-scope: ... -->` block must be accompanied by doc updates. Use
`docs: skip` in the issue body to demote drift to warnings for a single PR.

## Telemetry Dashboard

Generate an HTML dashboard from the autospec telemetry log (`~/.autospec/telemetry.jsonl`):

```bash
bash skills/autospec-shared/scripts/gen-telemetry-dashboard.sh \
  --input ~/.autospec/telemetry.jsonl \
  --output ~/.autospec/telemetry-dashboard.html
open ~/.autospec/telemetry-dashboard.html
```

The dashboard shows: daily cache hit-rate trend (Chart.js), per-role token cost breakdown,
LGTM first-pass rate, and top-10 cost outliers by issue. To publish to GitHub Pages, push
`~/.autospec/telemetry-dashboard.html` to your `gh-pages` branch manually.

## More Docs

- [`docs/USER_MANUAL.md`](docs/USER_MANUAL.md) - operator-facing narrative walkthrough of the
  suite skills (generated).
- [`docs/API_REFERENCE.md`](docs/API_REFERENCE.md) - per-symbol reference for all scripts (generated).
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) - concurrency model, lock-step rule, module graph (generated).
- [`docs/user-manual.md`](docs/user-manual.md) - legacy narrative walkthrough.
- [`docs/architecture.md`](docs/architecture.md) - legacy architecture notes.
- [`examples/README.md`](examples/README.md) - config file schemas for model
  profiles and project board mapping.
- [`AGENTS.md`](AGENTS.md) - repository operating contract and merge authority
  rules.

## Auto context rollover (opt-in)

Wraps `claude`, `codex`, and `opencode` in a tmux session monitored by
`autospec-context-monitor`. At 50% context usage the monitor injects
`/compact`; at 80% it triggers `/create-handoff` → `/clear` → resume — same
terminal, same process, new conversation.

**Enable:** run `bash install.sh` and answer `y` to the auto-rollover prompt.

**Disable at any time:**
- `bash install.sh --disable-auto-rollover` — removes the shim permanently.
- `AUTOSPEC_AUTO_ROLLOVER=0 claude` — single-session bypass.
- `command claude` — bypasses the shim entirely.
- `touch ~/.autospec/no-auto-rollover.flag` — global kill-switch without reinstalling.

Check current status with the `/autospec-rollover-status` skill. See the
[design spec](docs/specs/2026-05-31-auto-context-rollover-design.md) for full
architecture details.
