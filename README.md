# Codex Skills

Reusable Codex skills for turning repeated workflows into durable, discoverable automation.

## Skills

| Skill | Purpose |
| --- | --- |
| [`spec-to-roadmap`](skills/spec-to-roadmap/SKILL.md) | Convert a product/spec issue or document into ordered GitHub issues, a GitHub Project, and an end-to-end branch/PR/merge execution loop. |

## Repository Layout

```text
skills/
  <skill-name>/
    SKILL.md
    agents/openai.yaml
    scripts/
    references/
    assets/
```

Each skill should be self-contained. Keep shared documentation in this repository minimal so future skills remain portable into `~/.codex/skills`.

## Installing A Skill

Copy a skill directory into your Codex skills directory:

```bash
mkdir -p ~/.codex/skills
cp -R skills/spec-to-roadmap ~/.codex/skills/
```

Start a new Codex session after installation so the skill metadata is loaded.

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
