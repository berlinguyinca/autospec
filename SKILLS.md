# Skill Index

## spec-to-roadmap

- Path: [`skills/spec-to-roadmap`](skills/spec-to-roadmap)
- Trigger: use when a user wants Codex to turn a product/spec issue or document into ordered GitHub issues, create or update the matching GitHub Project/view, and optionally continue through the implementation loop.
- Activation keywords: `spec to roadmap`, `turn this spec into GitHub issues`, `create roadmap issues from this spec`, `create/update the GitHub project for this spec`, `take this spec through implementation`, `ship this roadmap`, `roadmap this spec file`, `create project board from this spec`, `split this spec into LLM issues`, `materialize this spec`, `turn this doc into a GitHub project`, `generate implementation issues from docs/path.md`, `run spec-to-roadmap on docs/path.md`
- Status: governed roadmap workflow with dry-run validation, project reuse, issue idempotency, dependency metadata, and roadmap hygiene checks.

## autonomous-feature-shipping

- Path: [`skills/autonomous-feature-shipping`](skills/autonomous-feature-shipping)
- Trigger: use when a user asks the agent to ship a feature end-to-end — bootstrap a missing GitHub repo, brainstorm/design, decompose into linked GitHub issues with dependency metadata, and run an autonomous implementation loop with admin auto-merge until done.
- Activation keywords: `ship this feature`, `autonomous feature shipping`, `bootstrap repo and ship`, `decompose and auto-implement`, `run the autonomous loop`, `create issues and auto-merge`, `auto-implement this feature`, `ship end-to-end`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: 7-phase workflow (bootstrap → investigate → design → decompose → background monitor → status updates → final report) with admin-squash-merge of `auto-implement`-labeled PRs.

## Future Skill Checklist

1. Create `skills/<skill-name>/SKILL.md`.
2. Keep the skill body concise and move large details into `references/`.
3. Add deterministic helper code under `scripts/` when repeated shell/API work is error-prone.
4. Add UI metadata under `agents/openai.yaml` when useful.
5. Validate the skill before publishing.
6. Add a row to the README skill table and this index.
