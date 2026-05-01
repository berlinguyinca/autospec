# autospec

Multi-harness skill suite for shipping a feature end-to-end across many GitHub
issues — design spec → decomposed `auto-implement` queue → autonomous
implementation loop with admin auto-merge — split across four cooperating
skills so each invocation runs only the phases you need. Works on **Claude
Code**, **OpenCode**, and **Codex CLI**.

## Skills

| Skill | Phases | Purpose |
| --- | --- | --- |
| [`autospec`](skills/autospec/README.md) | 0–6 (incl. **Phase 3.5**) | Full pipeline. Bootstrap repo if missing, design spec, decompose into issues, **review-and-label children with `ctx:*`/`reasoning:*` rubric (Phase 3.5)**, then run autonomous monitor with admin auto-merge. |
| [`autospec-define`](skills/autospec-define/README.md) | 0–3.5 | Planning half. Stops after Phase 3.5 review-and-label step and hands off to `/autospec-run`. |
| [`autospec-run`](skills/autospec-run/README.md) | 4–6 | Implementation half. Picks up the populated `auto-implement` queue and runs the autonomous monitor. Supports `--profile <name>` filtering against `~/.autospec/model-profiles.yml`. |
| [`autospec-classify`](skills/autospec-classify/README.md) | retro | Standalone retro-labeler for already-existing `auto-implement` issues; applies the Phase 3.5 rubric to a queue that pre-dates Phase 3.5. |

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
