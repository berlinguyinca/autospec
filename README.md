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

## What It Solves

AI-generated code can grow quickly without a clear record of why it exists, what
spec produced it, which model worked on it, and which parts are actually
implemented. Autospec makes that process auditable.

It gives you:

- A durable spec before implementation starts.
- A linked 1:n issue tree instead of one oversized task.
- Model-fit metadata on each issue with `ctx:*` and `reasoning:*` labels.
- Implementation queues that can be filtered by model profile.
- Quality gates for issue shape and implementation scope.
- Autonomous PR creation, counter-team review, CI checks, and admin squash-merge.
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
| [`autospec-define`](skills/autospec-define/README.md) | You want planning only before implementation starts. | Produces a design spec plus classified `auto-implement` issues, then hands off to `/autospec-run`. |
| [`autospec-split`](skills/autospec-split/README.md) | You already have a tracked `docs/specs/*.md` design spec. | Turns the existing spec into an EPIC plus linked child issues, then stops after classification. |
| [`autospec-run`](skills/autospec-run/README.md) | You already have an `auto-implement` queue. | Runs the implementation monitor, opens PRs, reviews, validates, and merges. |
| [`autospec-classify`](skills/autospec-classify/README.md) | Existing issues need model-fit labels. | Adds `ctx:*` and `reasoning:*` labels, inserts a `## Model fit` block, and promotes `needs-classify` issues. |
| [`autospec-listen`](skills/autospec-listen/README.md) | You want chat phrases like "file an issue" to become tracked work. | Drafts issues for approval or routes spec requests into `/autospec-define`. |
| [`autospec-story`](skills/autospec-story/README.md) | You need a repo-level product and implementation-state overview. | Produces a cited Markdown story from local specs, docs, issues, PRs, and git history. |
| [`autospec-stop`](skills/autospec-stop/README.md) | You need to halt or resume an active monitor. | Writes the shared stop sentinel, pauses issues safely, reports status, or resumes paused work. |
| [`autospec-review`](skills/autospec-review/README.md) | You want to close the spec-vs-code feedback loop. | Audits specs against issues, finds gaps, files `[REGRESSION]` issues with `priority:high`. |
| [`autospec-test`](skills/autospec-test/README.md) | You want every Phase 4 PR gated on unit + E2E coverage with auto-heal. | Runs a two-stage coverage gate, auto-heals gaps within a 60-min budget, and blocks assertion-loosening rewrites before auto-merge. |

See [`SKILLS.md`](SKILLS.md) for activation keywords and per-skill routing
details.

## Install

Install the full suite into every supported harness:

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

The installer honors:

| Variable | Purpose |
| --- | --- |
| `CLAUDE_CONFIG_DIR` | Override Claude Code install root. |
| `OPENCODE_CONFIG_DIR` | Override OpenCode install root. |
| `CODEX_HOME` | Override Codex install root. |
| `AUTOSPEC_NO_STAR_PROMPT=1` | Skip the optional adoption star prompt. |

After a successful interactive suite install, the top-level installer asks
whether you want to star `berlinguyinca/autospec` to support adoption. It is
skipped for non-interactive installs, `--update`, CI, missing `gh`, or
`AUTOSPEC_NO_STAR_PROMPT=1`.

Each per-skill installer is also standalone-callable:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec/install.sh \
  | sh -s -- --harness all
```

### Target repo setup

If you run autospec against your own codebase, see
[`docs/target-repo-setup.md`](docs/target-repo-setup.md) for the
operator-side opt-ins that pair with the Phase 4 implementer's
cross-session safety gates: branch protection on `main` plus a
migration-replay test convention. Both are voluntary; without them
autospec keeps working but loses the safety net for issue #307's
cross-session CI rot.

### Turbo integration

`install.sh` also bootstraps [tobihagemann/turbo](https://github.com/tobihagemann/turbo) as a peer skill family:

- Clones (or fast-forward pulls) `~/.turbo/repo` and symlinks turbo skills into `~/.claude/skills/`.
- Checks for the [Codex CLI](https://github.com/openai/codex). When present, autospec's Phase 4 implementer (issues labelled `autospec:v2-flow`) runs an inline peer-review pass on each diff before opening a PR. When absent, peer-review skips gracefully and the implementer continues.
- Idempotently merges an `<!-- autospec-block -->` section into `~/.claude/CLAUDE.md` documenting both stacks' entrypoints.
- Inside a git repo, offers to add `.autospec/` to `.gitignore` (auto-accept via `AUTOSPEC_AUTO_YES=1`).

Turbo bootstrap failures are non-fatal: offline or no-remote setups continue using the cached turbo checkout.

## Update

Each installed suite skill runs a startup self-update check at most once every
24 hours. It reinstalls from `main` when a newer commit is available. The check
is fail-open: network or install errors log a `WARN:` line and the installed
skill continues to run.

Disable startup self-update:

```bash
AUTOSPEC_NO_SELF_UPDATE=1
```

Force an in-place suite update:

```bash
./install.sh --skill all --harness all --update
```

`--update` also fast-forwards the autospec checkout itself and pulls `~/.turbo/repo`, so a single command keeps both autospec and turbo current.

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

The monitor:

- Reconciles stale process heartbeats at startup.
- Prints live queue status after scans.
- Prints an issue summary before working on each issue.
- Claims one ready issue at a time.
- Opens a branch and PR for each issue.
- Runs self-review and implementation guardian checks.
- Admin squash-merges `auto-implement` PRs when required checks pass and review
  is `LGTM`.

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
