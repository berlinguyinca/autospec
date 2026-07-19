---
name: project-turbo-integration-design
description: "Turbo↔autospec integration LIVE on origin/main 2026-05-17 (16 commits); smoke test on real install caught two install.sh bugs, both fixed"
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 0a77c1fd-c243-4bf9-b3fb-4f83ae5f9830
---

Integration design + implementation landed 2026-05-17. Zero forks. Iterated from 11 forks → 5 forks → zero forks after user pushed back on token-burn ([[feedback_roi_check_new_components]]).

**Final shape:** two independent skill families (autospec + turbo) coexist with a shared install/update entrypoint. autospec absorbs turbo's *practices* inline at the highest-leverage points (Phase 4 implementer + planning self-review). Zero forks; zero drift management.

## Implementation state

**origin/main:** at `53ab331`. 16 commits total since the integration started. Latest CI for this commit polled in background as of this save.

**Worktree:** removed via `ExitWorktree action: remove` after the local-merge to main; only `main` checkout remains at `/Users/wohlgemuth/IdeaProjects/autospec`.

**Artifacts:**
- Spec: `docs/superpowers/specs/2026-05-17-turbo-autospec-integration-design.md`
- Plan: `docs/superpowers/plans/2026-05-17-turbo-autospec-integration.md`

**Status:** integration shipped end-to-end (local main → origin/main → smoke-tested on real `~/.claude/skills/`). Two follow-up bugs caught by smoke test, both fixed in `53ab331`. CI run for that commit polling in background.

## What landed (3 phases, 13 commits)

**Phase A — install machinery (7 commits):**
- New: `scripts/lib/install-helpers.sh` (merge_marked_block + command_present + ensure_line_in_file)
- New: `scripts/lib/claude-md-block.txt` (canonical CLAUDE.md content)
- `install.sh` extended with `bootstrap_turbo` (clone or ff-pull `~/.turbo/repo`, symlink into `~/.claude/skills/`), `check_codex` (graceful-skip with install hint), `merge_claude_md` (idempotent `<!-- autospec-block -->` marker), `offer_gitignore` (`.autospec/` entry with `AUTOSPEC_AUTO_YES=1`), `pull_autospec` (only on `--update`)
- bootstrap_turbo tolerates offline / no-remote turbo (warns, continues)
- README documents turbo bootstrap + `--update` both-stacks behavior
- 6 dry-run + idempotency tests under `tests/install/`

**Phase B — `/autospec-define` absorbs turbo planning (1 commit, lock-step synced to codex/prompt.md + opencode/agent.md):**
- Phase 2 self-review expanded into concrete placeholder/consistency/ambiguity/scope checks
- Phase 3 decomposer asks Produces/Consumes/Covers as internal questions (NOT YAML in issue body)
- Phase 3 labels: every child carries `autospec:v2-flow` (idempotent label creation)

**Phase C — Phase 4 absorbs turbo discipline (3 commits, lock-step synced):**
- New: `skills/autospec-run/prompts/phase4-implementer.md` — self-contained prompt embedding expand → implement → finalize → peer-review (via `codex exec`) → evaluate-findings → lock-step compliance. Explicitly forbids `Skill` tool calls from within the subagent to avoid upstream prompt drift.
- `skills/autospec-run/SKILL.md` label-routing block: issues with `autospec:v2-flow` load the new prompt; legacy issues stay on inline prompt
- SKILLS.md notes integration on the autospec umbrella entry
- 1 prompt structure test under `tests/phase4/`

## Post-merge fixes (commit 5f5a6ac)

Independent code-reviewer subagent caught two real issues after local-merge; both fixed in one follow-up commit on main.

1. **`merge_marked_block` leaked `content_file` on `mv` failure** (`scripts/lib/install-helpers.sh`). Under `set -e` inherited from `install.sh`, a failed `mv` (read-only target, cross-device move) aborted the function before `rm -f "$content_file"` could run. Fix: `trap 'rm -f "$tmp" "$content_file"' RETURN` at the top of the function; dropped the trailing explicit rm.
2. **Phase 4 implementer lock-step check used unreliable `gh pr list --search "linked:#<N>"`** (`skills/autospec-run/prompts/phase4-implementer.md`). The monitor's outer loop checks issue state (CLOSED); the implementer was checking linked PRs (which only works for PRs that used a `Closes` keyword). Mismatch would soft-loop on deps closed manually or via non-`Closes` PRs. Fix: use `gh issue view <N> --json state --jq .state` and confirm `CLOSED` — same predicate as the monitor.

Reviewer also confirmed: no autospec/turbo skill name collisions, symlink approach clean, `bootstrap_turbo`'s offline-tolerance correct, the awk END "repair unclosed block" clause earns its weight (test exercises that exact path).

## Smoke-test follow-up fixes (commit 53ab331)

After push, ran `bash install.sh` against the user's real `~/.claude/skills/` (which already had turbo skills installed by hand as real directories). Smoke test caught two real `install.sh` bugs the unit/bats tests had missed:

1. **`ln -sfn src dest` silently creates nested broken symlinks when `dest` is a pre-existing directory**, not a replacement. On the user's machine, 71 of 72 turbo skill directories got a useless `<skill>/<skill> -> ~/.turbo/repo/claude/skills/<skill>` nested symlink. Fix: `bootstrap_turbo` now (a) cleans nested-symlink corruption from earlier broken runs, (b) refreshes symlinks it created itself, (c) symlinks into empty slots, and (d) skips pre-existing real directories with an info line telling the user how to opt in (`rm -rf $dir + re-run`).
2. **`[ "$counter" -gt 0 ] && info "..."` aborts under `set -e` when the counter is 0.** This silently skipped `check_codex`, `merge_claude_md`, and `offer_gitignore` for any install where `skipped_dir==0` and `cleaned_nested==0` (i.e., the bats test's empty-turbo scenario). Fix: rewrote as `if [ ... ]; then info ...; fi`. Captured as durable lesson in [[feedback_bash_set_e_short_circuit]].

Both bugs would have been invisible without running install.sh against a real `~/.claude/skills/`. Memory captures the lesson: bats/unit tests are necessary but not sufficient; integration tests against real filesystem state catch a different class of bug.

## Key implementation discoveries (gotchas)

- `install.sh` is a meta-orchestrator (delegates to per-skill installers), NOT a flat install script as the plan originally assumed. Integration concerns added as pre-skill-loop steps, reusing existing `--dry-run` flag. No new `--check` flag needed.
- `awk -v c="$content"` REJECTS multi-line content. Fixed merge_marked_block to write content to a temp file and `getline` it from inside awk.
- `validate.sh` lock-step rule: any change to `skills/<name>/SKILL.md` body MUST be mirrored verbatim into `codex/prompt.md` and `opencode/agent.md` (per [[feedback_validate_sh_lockstep_checks]] and [[feedback_autospec_decomposer_gotchas]]). codex/prompt.md needs leading blank line preserved — use `printf '%s\n' "$body" > codex/prompt.md` (no extra leading `\n`).
- bootstrap_turbo's git pull needs `|| warn` fallback so offline/no-tracking-branch users don't get a `set -e` exit mid-install.

## Verbatim user direction that shaped the final cut (2026-05-17)

> "sounds all good, make sure that all these things actually improve the code and are not just burning tokens here"

> "keep iterating, make sure that we have the turbo gunctionality and concetps inside autospec correctly. So that we get the best of both worlds. Streamline it and make it easy to install/update"

(After Task A1 ran the full subagent-driven workflow with ~5 invocations for a 50-line bash file, the user approved a "light loop" for mechanical tasks: I implement directly + verify via tests, reserve full implementer+spec+quality cycle for genuine design surface. Saved ~30 subagent invocations across A2-C3.)

## How to apply (when resuming)

- Worktree still active. ExitWorktree status: still inside `.claude/worktrees/turbo-integration`.
- Do NOT regrow forks. Zero-fork shape is final. If a future requirement seems to demand a fork, name the consumer first per [[feedback_roi_check_new_components]].
- The `autospec:v2-flow` label is canonical across `/autospec-define`, `/autospec-run` SKILL.md routing block, and `phase4-implementer.md`. Renaming requires updating all three.
- Phase 4 implementer prompt is intentionally self-contained (no Skill tool calls from subagent). Don't "improve" it by extracting steps into other skills — that's the whole point of zero-fork.
- Lock-step sync after every SKILL.md edit: `body=$(awk '/^---$/{c++; next} c>=2' SKILL.md); printf '%s\n' "$body" > codex/prompt.md; { print frontmatter; printf '%s\n' "$body"; } > opencode/agent.md`. Then `autospec validate`.
- All install tests are self-contained and use tmpdirs (no real `~/.claude` or `~/.turbo` touched). Run with `bash tests/install/test_*.sh`.
