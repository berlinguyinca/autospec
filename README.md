# autospec

Multi-harness skill suite for shipping a feature end-to-end across many GitHub
issues — design spec → decomposed `auto-implement` queue → autonomous
implementation loop with admin auto-merge — split across five cooperating
skills so each invocation runs only the phases you need. Works on **Claude
Code**, **OpenCode**, and **Codex CLI**.

## Skills

| Skill | Phases | Purpose |
| --- | --- | --- |
| [`autospec`](skills/autospec/README.md) | 0–6 (incl. **Phase 3.5**) | Full pipeline. Bootstrap repo if missing, design spec, decompose into issues, **review-and-label children with `ctx:*`/`reasoning:*` rubric (Phase 3.5)**, then run autonomous monitor with admin auto-merge. Also supports splitting an existing tracked `docs/specs/*.md` into issues. |
| [`autospec-define`](skills/autospec-define/README.md) | 0–3.5 | Planning half. Stops after Phase 3.5 review-and-label step and hands off to `/autospec-run`. Also supports splitting an existing tracked `docs/specs/*.md` into issues. |
| [`autospec-run`](skills/autospec-run/README.md) | 4–6 | Implementation half. Picks up the populated `auto-implement` queue and runs the autonomous monitor. Supports `--profile <name>` filtering against `~/.autospec/model-profiles.yml`. |
| [`autospec-classify`](skills/autospec-classify/README.md) | retro | Standalone retro-labeler for already-existing `auto-implement` issues; applies the Phase 3.5 rubric to a queue that pre-dates Phase 3.5. |
| [`autospec-listen`](skills/autospec-listen/README.md) | passive | Passive listener for chat-driven issue / spec triggers. On a phrase like "file an issue" or "write a spec", drafts a GitHub issue body for confirmation or routes to `/autospec-define`. |

Cost-aware **two-tier** subagent dispatch: spec/research/decompose/review subagents use the top model with extended thinking (Tier A — Claude Code: `opus` + `ultrathink`; Codex: top GPT + `reasoning_effort=high`; OpenCode: top task tier); implementer + LGTM-review subagents use the cheaper model with medium thinking (Tier B — Claude Code: `sonnet`; Codex: `gpt-5.1-codex-spark`; OpenCode: smaller task tier). Both tiers fall back UP on unavailability. The orchestrator runs whatever model you invoked the skill with — invoke it on top tier for best spec quality. See `AGENTS.md` for the full policy.

## Repository Layout

```text
skills/
  <skill-name>/
    SKILL.md                # canonical body (Claude Code skill format)
    agents/openai.yaml      # optional Codex UI metadata
    scripts/                # optional helpers
    references/             # optional supporting docs
    assets/                 # optional static assets

    # Multi-harness skills additionally include:
    README.md               # human-facing docs (what / why / install / usage)
    opencode/agent.md       # OpenCode-flavored variant
    codex/prompt.md         # Codex CLI prompt-library variant
    install.sh              # self-installer (--harness claude|opencode|codex|all)
    uninstall.sh            # symmetrical uninstaller
```

Each skill should be self-contained. Keep shared documentation in this repository minimal so future skills remain portable into `~/.codex/skills`.

Skills that target only Codex CLI can stick to the original layout (`SKILL.md` plus optional `agents/`, `scripts/`, `references/`, `assets/`). Skills that target multiple harnesses should add the `opencode/`, `codex/`, `install.sh`, `uninstall.sh`, and `README.md` files shown above.

## Installation

Install the whole suite into every supported harness in one call:

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
./install.sh --skill all --harness all
```

Or pick a subset:

```bash
./install.sh --skill autospec-run --harness claude   # one skill, one harness
./install.sh --skill all          --harness opencode # every skill, one harness
./install.sh --skill autospec     --harness all      # one skill, every harness
./uninstall.sh --skill all --harness all             # symmetric uninstall
```

Each per-skill installer remains standalone-callable, e.g.:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec/install.sh \
  | sh -s -- --harness all
```

Honors `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, and `CODEX_HOME` if set.

### Auto-update

Each `/autospec*` skill checks `main` for a newer commit at most once per 24 hours at
startup and reinstalls in place if there is one. The check is fail-open: any network
or install error logs one `WARN:` line to stderr and proceeds normally.
Set `AUTOSPEC_NO_SELF_UPDATE=1` to skip the check entirely. For the full contract
see `## Startup self-update` in `AGENTS.md`.

## Self-update

Each installed skill supports an in-place self-update: invoke the skill with
the literal argument `update` (case-insensitive) and it detects its harness,
re-runs the canonical install one-liner with `--update`, shows the diff, and
stops without entering the normal pipeline:

```
/autospec update
/autospec-define update
/autospec-run update
/autospec-classify update
```

You can also re-run the suite installer with `--update`:

```bash
./install.sh --skill all --harness all --update
```

The flag forces an idempotent overwrite of every (skill, harness) pair.

## Adding Future Skills

Use one folder per skill under `skills/`. A skill must include:

- `SKILL.md` with `name` and `description` frontmatter.
- Optional `agents/openai.yaml` for UI metadata.
- Optional `scripts/`, `references/`, and `assets/` only when they directly support the skill.

Avoid adding generated caches, local virtual environments, private credentials, or project-specific data.

## Validation

If the Codex skill-creator tools are installed locally, validate a skill with:

```bash
python3 ~/.codex/skills/.system/skill-creator/scripts/quick_validate.py skills/<skill-name>
```

Some environments require running that validator inside a virtual environment with `PyYAML` installed.

## Examples

Reference user-config files live under [`examples/`](examples/README.md):

- [`examples/model-profiles.yml`](examples/model-profiles.yml) — sample profile file consumed by `/autospec-run --profile <name>`.
- [`examples/project-map.yml`](examples/project-map.yml) — sample label-to-Projects-board map consumed by `/autospec-classify`.

See [`examples/README.md`](examples/README.md) for the full schema and the auto-init behavior that lets skills seed `~/.autospec/` from these samples on first run.

## Docs

- [`docs/user-manual.md`](docs/user-manual.md) — narrative walkthrough of all five skills (what each does, when to use it, example output).
- [`docs/architecture.md`](docs/architecture.md) — single-source-of-truth for the cross-cutting design rules: concurrency model, lock-step body rule, model tier policy, trigger keyword theory.

## Quality gate

Every issue filed by autospec is linted by `scripts/lint-issue.sh` before it reaches the implementation queue. The Phase 3 decomposer runs an adaptive retry loop (up to `MAX_LINT_RETRIES=5` attempts) that accumulates lint findings as prompt directives, skipping a child only if attempt 5 still fails. Phase 3.5 and `/autospec-classify` run a one-shot post-filing audit: issues that fail the lint get the `needs-quality-bar` label (color `#fbca04`), an idempotent `## Quality lint` block inserted into their body, and a comment with the findings. The `auto-implement` label is never removed — the operator decides whether to proceed or hand-fix. See [`docs/specs/2026-05-01-autospec-issue-quality-gate-design.md`](docs/specs/2026-05-01-autospec-issue-quality-gate-design.md) for the full contract.

## Existing Specs

`/autospec` and `/autospec-define` can split an already-written spec into the
same EPIC + `auto-implement` child issue queue used by the normal pipeline.
Invoke with phrases such as `split existing spec`, `split latest spec`, or
`turn docs/specs/2026-05-01-example-design.md into GitHub issues`. When no path
is provided, the skills choose the newest `docs/specs/*.md` by filename date as
the default; if multiple specs are available, they ask before filing issues.
The selected spec must already be tracked on `origin/main` so child issues can
cite a stable GitHub URL.

## Stopping a run

To halt a running autospec monitor, use the `/autospec-stop` skill or the inline sub-modes:

```bash
/autospec-stop --immediate          # abort at next step boundary; commit+push+mark paused
/autospec stop --graceful           # finish current issue then exit monitor
/autospec-run stop --status         # check current stop sentinel state
```

All three paths route through `scripts/autospec-stop.sh`. Use `--resume` to strip the `paused-by-user` label from paused issues and restart the queue. See [`AGENTS.md` § Stop mode authority](AGENTS.md) for the full contract.
