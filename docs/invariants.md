# Cross-cutting invariants

Home for invariants that bind **more than one component** or that outlive the
single file in which they are implemented. A comment in the implementing file
reaches only that file's readers; an entry here reaches the next component.

Rules (see [`runbooks/repository-retirement.md`](runbooks/repository-retirement.md)):

- Every entry states the invariant, **names the components it binds**, and
  links to where it is implemented and where it was decided.
- A new component bound by an existing invariant is added to that invariant's
  components-bound list in the same PR that introduces the component.
- When a component is retired, its invariants are migrated here (or to the
  successor's copy) *before* archival, with the successor added to the list.

## Invariants

| Invariant | Components bound | Implemented in | Decided in |
|---|---|---|---|
| Runtime state is private on Unix: `0700` directories, `0600` files; `RUNTIME_STATE_SYMLINK_REJECTED` and ownership ambiguity are fail-closed recovery signals; never delete an environment or session root manually. | `autospec runtime env` (CLI), `autospec-core` `runtime_env` module, `autospec` skill (Runtime resource isolation) | `crates/autospec-core/src/runtime_env/` | `AGENTS.md` §"Runtime resource isolation" |
| Code-intelligence results are cached per `workspace:revision:operation:request` and never reused across worktrees or revisions; degraded (`ast-grep`/`ripgrep`) results are never presented as semantic evidence. | `autospec-core` `code_intel` module, planner / implementer / reviewer gates in `autospec` and `autospec-run` skills | `crates/autospec-core/src/code_intel/`, `.autospec/code-intelligence.yaml` | `AGENTS.md` §"Semantic code intelligence", `docs/code-intelligence.md` |
| Skill lock-step: `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies stay identical (frontmatter may differ); the `## Startup self-update` block is byte-identical across multi-harness trios. | all multi-harness skills under `skills/`, `scripts/autospec-validate` (lock-step diff checks) | `skills/*/`, `scripts/` validation scripts | `CONTRIBUTING.md`, `AGENTS.md` §"Engineering standards" |
| Destructive remote actions (repo delete/archive, force-push to protected branches, `gh release delete`) are never bypassed: the autonomy gate surfaces a confirmation and gate exit 1 means "ask anyway". | `autospec` skill (Safety guardrails), `autospec-autonomy-gate.sh`, monitor/stop-mode paths | `scripts/autospec-autonomy-gate.sh`, `skills/autospec/SKILL.md` | `AGENTS.md` §"Autonomy charter", `docs/AUTONOMY-CHARTER.md` |
