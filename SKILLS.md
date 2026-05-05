# Skill Index

## autospec

- Path: [`skills/autospec`](skills/autospec)
- Trigger: use when a user asks the agent to ship a feature end-to-end — bootstrap a missing GitHub repo, brainstorm/design, decompose into linked GitHub issues with dependency metadata, split an existing `docs/specs/*.md` into issues, and run an autonomous implementation loop with admin auto-merge until done.
- Activation keywords: `autospec`, `ship this feature`, `autonomous feature shipping`, `bootstrap repo and ship`, `decompose and auto-implement`, `run the autonomous loop`, `create issues and auto-merge`, `auto-implement this feature`, `ship end-to-end`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`, `split existing spec`, `split latest spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: 7-phase workflow (bootstrap → investigate → design → decompose → background monitor → status updates → final report) with admin-squash-merge of `auto-implement`-labeled PRs.

## autospec-listen

- Path: [`skills/autospec-listen`](skills/autospec-listen)
- Trigger: passive listener that fires mid-conversation when the user mentions filing an issue or starting a spec — drafts a GitHub issue body for confirmation (issue trigger) or routes to `/autospec-define` (spec trigger). Bare nouns ("issue", "spec", "ticket") are NOT triggers.
- Activation keywords: `file an issue`, `file this as an issue`, `new issue`, `open an issue`, `create a ticket`, `make an issue`, `write a spec`, `design spec`, `new spec`, `start a spec`, `write a design spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: trigger-listener for chat-driven issue / spec creation. Files issues with `needs-classify` label so `/autospec-classify` can transition them onto the `auto-implement` queue. See [`skills/autospec-listen/README.md`](skills/autospec-listen/README.md).

## autosplit

- Path: [`skills/autosplit`](skills/autosplit)
- Trigger: use when a user asks to split, materialize, roadmap, decompose, or turn an existing tracked `docs/specs/*.md` design spec into GitHub issues.
- Activation keywords: `autosplit`, `split existing spec`, `split latest spec`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: existing-spec shortcut for Phase 3 plus Phase 3.5 with startup self-update before normal execution.

## Future Skill Checklist

1. Create `skills/<skill-name>/SKILL.md`.
2. Keep the skill body concise and move large details into `references/`.
3. Add deterministic helper code under `scripts/` when repeated shell/API work is error-prone.
4. Add UI metadata under `agents/openai.yaml` when useful.
5. Validate the skill before publishing.
6. Add a row to the README skill table and this index.
