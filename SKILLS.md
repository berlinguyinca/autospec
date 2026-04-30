# Skill Index

## autospec

- Path: [`skills/autospec`](skills/autospec)
- Trigger: use when a user asks the agent to ship a feature end-to-end — bootstrap a missing GitHub repo, brainstorm/design, decompose into linked GitHub issues with dependency metadata, and run an autonomous implementation loop with admin auto-merge until done.
- Activation keywords: `autospec`, `ship this feature`, `autonomous feature shipping`, `bootstrap repo and ship`, `decompose and auto-implement`, `run the autonomous loop`, `create issues and auto-merge`, `auto-implement this feature`, `ship end-to-end`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: 7-phase workflow (bootstrap → investigate → design → decompose → background monitor → status updates → final report) with admin-squash-merge of `auto-implement`-labeled PRs.

## Future Skill Checklist

1. Create `skills/<skill-name>/SKILL.md`.
2. Keep the skill body concise and move large details into `references/`.
3. Add deterministic helper code under `scripts/` when repeated shell/API work is error-prone.
4. Add UI metadata under `agents/openai.yaml` when useful.
5. Validate the skill before publishing.
6. Add a row to the README skill table and this index.
