# Contributing

This repository stores portable Codex skills.

## Skill Requirements

- Use lowercase hyphenated skill names.
- Keep each skill in `skills/<skill-name>/`.
- Include exactly one required `SKILL.md` per skill.
- Keep the frontmatter `name` aligned with the folder name.
- Keep descriptions focused on when Codex should use the skill.
- Do not commit private credentials, local data, generated caches, or virtual environments.

## Preferred Structure

```text
skills/<skill-name>/
  SKILL.md
  agents/openai.yaml
  scripts/
  references/
  assets/
```

Only include optional directories when they are useful.

## Review Checklist

- Skill has a clear trigger.
- Instructions are actionable and not project-specific unless intentionally scoped.
- Helper scripts have been syntax-checked or run in dry-run mode.
- Large examples or templates are moved out of `SKILL.md`.
- README and `SKILLS.md` are updated.
