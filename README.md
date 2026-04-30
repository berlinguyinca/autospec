# Codex Skills

Reusable Codex skills for turning repeated workflows into durable, discoverable automation.

## Skills

| Skill | Purpose |
| --- | --- |
| [`spec-to-roadmap`](skills/spec-to-roadmap/SKILL.md) | Convert a product/spec issue or document into a governed roadmap: master spec, LLM-ready issues, dependencies, GitHub Project/view routing, and optional implementation loop. |
| [`autonomous-feature-shipping`](skills/autonomous-feature-shipping/README.md) | Ship a feature end-to-end: bootstrap repo if missing, brainstorm/design, decompose into linked GitHub issues, then run an autonomous implementation loop with admin auto-merge. Multi-harness (Claude Code, OpenCode, Codex CLI). |

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

## Installing A Skill

For Codex-only skills, copy the skill directory into your Codex skills directory:

```bash
mkdir -p ~/.codex/skills
cp -R skills/spec-to-roadmap ~/.codex/skills/
```

Start a new Codex session after installation so the skill metadata is loaded.

For multi-harness skills (Claude Code, OpenCode, Codex CLI), use the skill's
bundled installer:

```bash
cd skills/autonomous-feature-shipping
./install.sh --harness all          # or claude / opencode / codex
```

See the skill's own README for the per-harness install paths and manual
fallbacks.

## Adding Future Skills

Use one folder per skill under `skills/`. A skill must include:

- `SKILL.md` with `name` and `description` frontmatter.
- Optional `agents/openai.yaml` for UI metadata.
- Optional `scripts/`, `references/`, and `assets/` only when they directly support the skill.

Avoid adding generated caches, local virtual environments, private credentials, or project-specific data.

## Validation

If the Codex skill-creator tools are installed locally, validate a skill with:

```bash
python3 ~/.codex/skills/.system/skill-creator/scripts/quick_validate.py skills/spec-to-roadmap
```

Some environments require running that validator inside a virtual environment with `PyYAML` installed.

The `spec-to-roadmap` helper also supports a local dry run before mutating
GitHub:

```bash
python3 skills/spec-to-roadmap/scripts/roadmap_plan_scaffold.py \
  --spec docs/path.md \
  --repo owner/name \
  --project-owner owner \
  --project-title "Project Title" \
  --output /tmp/roadmap.json

python3 skills/spec-to-roadmap/scripts/create_github_roadmap.py \
  /tmp/roadmap.json \
  --dry-run
```
