# Turbo ↔ Autospec Integration — Design

**Date**: 2026-05-17
**Status**: Design, pending implementation plan
**Author**: berlinguyinca (with Claude)

## Goal

Bring the high-leverage practices from [tobihagemann/turbo](https://github.com/tobihagemann/turbo) — peer-review via Codex, expand-before-implement, finalize-as-QA-gate, spec self-review — into autospec's autonomous Phase 4 implementer, without forking any turbo skill and without disrupting operator-facing UX (`/autospec-define`, `/autospec-run`).

Two design constraints govern every decision:

1. **No ceremony, no token-burn.** Every added component must name a concrete consumer that benefits today. Default to invoking upstream over forking.
2. **Operator UX is sacred.** Existing entrypoints behave the same; integration is internal.

## Non-goals

- Replacing autospec-define's brainstorm or decomposer wholesale with turbo equivalents.
- Forking turbo skills into the autospec repo. (Considered, then cut — see "What we considered and dropped" below.)
- Adding a parallel improvements backlog (`.turbo/improvements.md`) or handoff directory. Autospec uses GitHub issues for backlog and the monitor relaunch loop for continuity.
- Embedding a shell-YAML metadata block (Produces/Consumes/Covers) in every issue body. Deferred until at least one downstream consumer (autospec-review or autospec-split) is being changed to read those fields.

## Architecture

Two independent skill families coexist on the operator's machine:

```
~/.claude/skills/
├── (autospec skills — symlinked from this repo)
└── (turbo skills — symlinked from ~/.turbo/repo)
```

The operator picks the entrypoint that fits the work:

- `/autospec-define` + `/autospec-run` — autonomous, multi-issue pipelines (autospec)
- `/turboplan`, `/draft-spec`, `/finalize`, `/peer-review`, etc. — interactive, single-task discipline (turbo)

Integration is one-directional: autospec's Phase 4 implementer absorbs turbo's per-task discipline inline. Turbo skills remain available independently for operators who prefer that flow.

### Planning side: concept absorption inside /autospec-define

`/autospec-define` keeps its existing brainstorm + decomposer UX. It adopts three turbo practices as inline phases of the existing flow:

1. **Spec self-review pass.** After the brainstorm and before issues are filed, run a fresh-eyes pass on the spec checking for: placeholders (TBD/TODO), internal contradictions, ambiguous requirements (multiple valid interpretations), scope creep (does this need decomposition into sub-specs). Fix inline.
2. **Spec output path.** Write the validated spec to `docs/specs/<date>-<slug>-design.md`. (Autospec already uses this path; no change.)
3. **Shell-structure questions.** The decomposer asks itself, per candidate issue: what does this Produce (new files / new exports), what does it Consume (existing files / outputs of earlier issues), what does it Cover (which spec sections). These questions shape decomposition quality. They are not written out as YAML metadata in the issue body (deferred — see Non-goals).

No cross-tool skill calls. `/autospec-define` does not invoke `/draft-spec`, `/draft-shells`, or any other turbo skill.

### Implementation side: Phase 4 implementer prompt

The high-leverage piece. A new prompt template lives at:

```
skills/autospec-run/prompts/phase4-implementer.md
```

This is **not a skill** (it is not operator-facing) — it is the prompt the Phase 4 monitor subagent loads when assigned an issue. It embeds turbo's discipline inline, in this order:

1. **Expand** — Before any code change, the subagent:
   - Verifies that files referenced in the issue still exist at the cited paths.
   - Runs a quick pattern survey (`grep`/`find`) for analogous existing implementations.
   - Escalates ambiguity back to the issue (as a comment) instead of guessing, if a referenced file was renamed or a contract changed.

2. **Implement** — Tier-aware budget control (existing autospec behavior, retained). The subagent reads `ctx:*` / `reasoning:*` labels on the issue and adapts its context budget and reasoning depth accordingly.

3. **Finalize** — Tests must pass; lint must pass; commit follows autospec's existing commit-style rules. (This step replaces the looser "implement-then-commit" current behavior with a gate.)

4. **Peer-review** — `codex exec` is invoked on the diff with a second-opinion prompt. If Codex CLI is not on PATH, the step prints a one-line warning and skips. Output is captured.

5. **Evaluate-findings** — The implementer triages peer-review output, separating must-fix (correctness, security, broken tests) from noise (style preferences, scope creep). Only must-fix findings are applied; the rest are dropped or logged as comments on the resulting PR for human review.

6. **Lock-step compliance** — Existing autospec behavior; retained.

The prompt is self-contained: no `Skill` tool calls inside the subagent, no dependency on turbo skills being installed at runtime. This isolates Phase 4 from upstream turbo prompt drift.

### Install / update — single entrypoint

`install.sh` becomes the only command the operator needs to keep both stacks current.

| Invocation | Effect |
|---|---|
| `install.sh` | Bootstrap turbo if missing (clone `tobihagemann/turbo` → `~/.turbo/repo`, symlink turbo skills into `~/.claude/skills/`). Symlink autospec skills (current behavior). Check for Codex CLI; if absent, print one-line install instruction and note that peer-review will gracefully skip. Merge an autospec section into `~/.claude/CLAUDE.md` via `<!-- autospec-block -->` … `<!-- /autospec-block -->` markers (idempotent — re-running edits in place). If invoked inside a git repo, offer to add `.autospec/` to `.gitignore`. |
| `install.sh --update` | `git pull` in both `~/.turbo/repo` and the autospec repo. Re-run install steps. Re-check Codex. Report what changed. |
| `install.sh --check` | Dry-run: report what would change without making edits. |
| `install.sh --uninstall` | Delegate to existing `uninstall.sh`. |

Because there are zero forked skills, there is no upstream-drift script to maintain.

## Concept mapping

| Turbo concept | Autospec equivalent | Notes |
|---|---|---|
| Spec file | `docs/specs/<date>-<slug>-design.md` | Already aligned |
| Shell file | GitHub issue with `auto-implement` label | Issue body is canonical |
| `.turbo/` scratch directory | `.autospec/` (gitignored) | Implementer writes transient scratch here |
| `/finalize` QA gate | Inline section in Phase 4 implementer prompt | Tests + lint + commit-style |
| `/peer-review` (Codex) | Inline `codex exec` step before PR open | Highest-leverage absorbed practice |
| `/self-improve` | Operator-invoked upstream skill | Not absorbed; operator runs `/self-improve` directly |
| `.turbo/improvements.md` | Not absorbed | Autospec uses GitHub issues for backlog |
| `.turbo/handoff/*.md` | Not absorbed | Autospec monitor relaunch handles continuity |
| Plan → Shells → Expand → Implement → Finalize | `/autospec-define` (plan, shells) → `/autospec-run` Phase 4 implementer (expand, implement, finalize, peer-review) | Mental model preserved end-to-end |

## Detection: how Phase 4 routes issues

Existing in-flight issues (filed before this change) must continue on the legacy implementer path. New issues use the absorbed-discipline path.

**Detection mechanism**: a label `autospec:v2-flow` on the issue. `/autospec-define` adds it to every issue it files after this change lands. The monitor subagent reads the label and loads either the new `phase4-implementer.md` prompt or the legacy path.

The label-based switch is cleaner than a body-marker comment and surfaces in `gh issue list --label autospec:v2-flow` for operator visibility.

## What we considered and dropped

| Considered | Why dropped |
|---|---|
| 11 forked skills (full plan + per-issue absorption) | Too many forks to maintain; drift management overhead; most forks added zero value because the relevant skills run interactively on Opus where tier-awareness doesn't matter. |
| 5 forked skills (Phase 4 only) | Cleaner but still wrong: Phase 4 is one capability ("implement an issue end-to-end"), not five. Per skill-per-capability rule, one inline prompt template is correct. |
| Shell-YAML metadata block in issue bodies | Deferred. Adds maintenance burden today with no consumer reading the fields. Re-evaluate when autospec-review or autospec-split is being changed. |
| `/autospec-define` delegates to `/draft-spec` etc. | Would disrupt the autospec brainstorm UX the operator has tuned. Concept absorption inline gives the practice value without the UX disruption. |
| Forked `/peer-review` skill | Unnecessary. The peer-review step is two lines of `codex exec` + an evaluate prompt — inline in the implementer prompt is simpler than a dedicated skill. |

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Codex CLI not installed on Phase 4 monitor host → peer-review step never runs | Graceful skip with one-line warning. Operator informed at `install.sh` time. Phase 4 still completes; peer-review absence is logged in the PR description so reviewer knows. |
| Turbo upstream prompt improvements never reach Phase 4 (no auto-sync) | Acceptable trade for zero drift management. Periodic manual review of turbo CHANGELOG; port specific improvements only if they earn their token cost. |
| Mixed v1/v2 issue queue causes monitor confusion | Label-based routing (`autospec:v2-flow`). Backwards-compat retained until all v1 issues drain. |
| `install.sh` complexity grows unbounded as it absorbs turbo bootstrap | Keep the script declarative: each step is independently testable, no implicit state. `--check` dry-run validates the script itself. |

## Open questions for the implementation plan

- Exact wording of the peer-review prompt sent to Codex (likely "review this diff for correctness, security, and consistency with surrounding code; flag must-fix vs nice-to-have").
- Whether `evaluate-findings` triage runs as a separate LLM call or as part of the implementer's same turn (parallel to budget).
- Specific `<!-- autospec-block -->` content for CLAUDE.md merge.

## Next step

Invoke `writing-plans` to produce a detailed implementation plan against this design.
