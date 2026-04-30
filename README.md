# Codex Skills

Reusable Codex skills for turning repeated workflows into durable, discoverable automation.

## Skills

| Skill | Purpose |
| --- | --- |
| [`autospec`](skills/autospec/README.md) | Ship a feature end-to-end: bootstrap repo if missing, brainstorm/design, decompose into linked GitHub issues, then run an autonomous implementation loop with admin auto-merge. Multi-harness (Claude Code, OpenCode, Codex CLI). |

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

For multi-harness skills (Claude Code, OpenCode, Codex CLI), use the skill's
bundled installer. The fastest path is the one-line installer described in the
skill's README, e.g.:

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/codex-skills/main/skills/autospec/install.sh | sh -s -- --harness all
```

Or run it from a clone:

```bash
cd skills/autospec
./install.sh --harness all          # or claude / opencode / codex
```

For Codex-only skills, copy the skill directory into your Codex skills directory:

```bash
mkdir -p ~/.codex/skills
cp -R skills/<skill-name> ~/.codex/skills/
```

Start a new Codex session after installation so the skill metadata is loaded.
See each skill's own README for per-harness install paths and manual fallbacks.

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
