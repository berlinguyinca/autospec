# Skill Index

## spec-to-roadmap

- Path: [`skills/spec-to-roadmap`](skills/spec-to-roadmap)
- Trigger: use when a user wants Codex to turn a product/spec issue or document into ordered GitHub issues, create or update the matching GitHub Project/view, and optionally continue through the implementation loop.
- Activation keywords: `spec to roadmap`, `turn this spec into GitHub issues`, `create roadmap issues from this spec`, `create/update the GitHub project for this spec`, `take this spec through implementation`, `ship this roadmap`
- Status: governed roadmap workflow with dry-run validation, project reuse, issue idempotency, dependency metadata, and roadmap hygiene checks.

## Future Skill Checklist

1. Create `skills/<skill-name>/SKILL.md`.
2. Keep the skill body concise and move large details into `references/`.
3. Add deterministic helper code under `scripts/` when repeated shell/API work is error-prone.
4. Add UI metadata under `agents/openai.yaml` when useful.
5. Validate the skill before publishing.
6. Add a row to the README skill table and this index.
