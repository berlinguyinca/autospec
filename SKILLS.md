# Skill Index

## autospec

- Path: [`skills/autospec`](skills/autospec)
- Trigger: use when a user asks the agent to ship a feature end-to-end — bootstrap a missing GitHub repo, brainstorm/design, decompose into linked GitHub issues with dependency metadata, split an existing `docs/specs/*.md` into issues, and run an autonomous implementation loop with admin auto-merge until done.
- Activation keywords: `autospec`, `ship this feature`, `autonomous feature shipping`, `bootstrap repo and ship`, `decompose and auto-implement`, `run the autonomous loop`, `create issues and auto-merge`, `auto-implement this feature`, `ship end-to-end`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`, `split existing spec`, `split latest spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: 7-phase workflow (bootstrap → investigate → design → decompose → background monitor → status updates → final report) with admin-squash-merge of `auto-implement`-labeled PRs.
- Turbo integration (since 2026-05-17): `install.sh` bootstraps [tobihagemann/turbo](https://github.com/tobihagemann/turbo) as a peer skill family and `--update` keeps both stacks current. Issues filed by `/autospec-define` carry the `autospec:v2-flow` label, which routes the Phase 4 implementer to a prompt at `skills/autospec-run/prompts/phase4-implementer.md` that absorbs turbo's expand → implement → finalize → peer-review → evaluate-findings discipline inline. Peer-review uses Codex CLI; gracefully skips when absent. See [`docs/superpowers/specs/2026-05-17-turbo-autospec-integration-design.md`](docs/superpowers/specs/2026-05-17-turbo-autospec-integration-design.md).

## autospec-listen

- Path: [`skills/autospec-listen`](skills/autospec-listen)
- Trigger: passive listener that fires mid-conversation when the user mentions filing an issue or starting a spec — drafts a GitHub issue body for confirmation (issue trigger) or routes to `/autospec-define` (spec trigger). Bare nouns ("issue", "spec", "ticket") are NOT triggers.
- Activation keywords: `file an issue`, `file this as an issue`, `new issue`, `open an issue`, `create a ticket`, `make an issue`, `write a spec`, `design spec`, `new spec`, `start a spec`, `write a design spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: trigger-listener for chat-driven issue / spec creation. Files issues with `needs-classify` label so `/autospec-classify` can transition them onto the `auto-implement` queue. See [`skills/autospec-listen/README.md`](skills/autospec-listen/README.md).

## autospec-story

- Path: [`skills/autospec-story`](skills/autospec-story)
- Trigger: use when a user asks for a complete repo story, implementation overview, product history, or state report from local specs plus GitHub issues and PRs.
- Activation keywords: `autospec-story`, `repo story`, `application story`, `implementation state`, `what has been built`, `complete overview`, `state of the application`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: read-only synthesis mode. Produces a cited Markdown report that separates evidence, inference, open work, completed work, and unknowns. See [`skills/autospec-story/README.md`](skills/autospec-story/README.md).

## autospec-review

- Path: [`skills/autospec-review`](skills/autospec-review)
- Trigger: use when a user wants to audit design specs against open and closed issues to find gaps, file high-priority regression issues, and feed them back through autospec. Auto-fires after each autospec-run batch unless `~/.autospec/no-review.flag` exists.
- Activation keywords: `autospec-review`, `audit specs`, `find spec gaps`, `regression review`, `spec vs issues`, `find missing coverage`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: closes the spec-vs-code feedback loop; renders regression specs, dispatches Tier A reviewer subagent, and hands off to `/autospec-split` to file `[REGRESSION]` issues with `priority:high`.

## autospec-split

- Path: [`skills/autospec-split`](skills/autospec-split)
- Trigger: use when a user asks to split, materialize, roadmap, decompose, or turn an existing tracked `docs/specs/*.md` design spec into GitHub issues.
- Activation keywords: `autospec-split`, `split existing spec`, `split latest spec`, `turn this spec into GitHub issues`, `roadmap this spec`, `materialize this spec`
- Harnesses: Claude Code (`SKILL.md`), OpenCode (`opencode/agent.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: existing-spec shortcut for Phase 3 plus Phase 3.5 with startup self-update before normal execution.

## autospec-test

- Path: [`skills/autospec-test`](skills/autospec-test)
- Trigger: use when you want every Phase 4 PR gated on unit + E2E test coverage with an auto-heal loop, or to run ad-hoc coverage validation against any branch.
- Activation keywords: `autospec-test`, `coverage gate`, `e2e gate`, `unit coverage`, `test gate`, `auto-heal tests`, `enforce test coverage`, `coverage enforcement`
- Harnesses: Claude Code (`SKILL.md`), Codex CLI (`codex/prompt.md`). Bundled `install.sh` / `uninstall.sh` handle per-harness placement.
- Status: inline Phase 4 gate (runs after build + lint, before auto-merge) plus standalone `/autospec-test [PR#]` invocation. Two-stage: unit coverage → E2E coverage. Self-heal loop up to 5 iterations / 60 min coding time. Assertion-shift guardrail blocks LOOSENING rewrites. Mode II (scoped production) opt-in with mandatory backup/restore.

## Docs amendment (Phase 10c)

The autospec docs amendment ships first-class documentation artifacts to every target repo.
Run `bash skills/autospec-shared/scripts/reverse-engineer.sh --repo-root .` to regenerate.

Generated artifacts committed to this repo (autospec dogfooding its own pipeline):

| File | Generator |
|---|---|
| [`docs/USER_MANUAL.md`](docs/USER_MANUAL.md) | `gen-docs-from-spec.mjs` |
| [`docs/API_REFERENCE.md`](docs/API_REFERENCE.md) | `gen-docs-from-spec.mjs` |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | `gen-docs-from-spec.mjs` + `gen-arch-diagram.mjs` |
| [`docs/ASSISTANT_PROMPT.md`](docs/ASSISTANT_PROMPT.md) | `gen-assistant-prompt.mjs` |
| [`docs/.llm-manifest.json`](docs/.llm-manifest.json) | `gen-llm-manifest.mjs` |
| [`llms.txt`](llms.txt) | `gen-llms-txt.sh` |
| [`llms-full.txt`](llms-full.txt) | `gen-llms-txt.sh` |

`scripts/validate.sh` enforces presence of `docs/USER_MANUAL.md`, `llms.txt`, and
`docs/.llm-manifest.json` on every CI run. Deleting any of these artifacts causes
`validate.sh` to exit non-zero.

## Future Skill Checklist

1. Create `skills/<skill-name>/SKILL.md`.
2. Keep the skill body concise and move large details into `references/`.
3. Add deterministic helper code under `scripts/` when repeated shell/API work is error-prone.
4. Add UI metadata under `agents/openai.yaml` when useful.
5. Validate the skill before publishing.
6. Add a row to the README skill table and this index.
