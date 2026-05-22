# Autospec Pipeline Hardening — Implementer Constraints + CI-Wait Sentinel + Adaptive Retry

**Status:** Draft design (2026-05-22)
**Author:** berlinguyinca + diagnostic
**Scope:** Amends `autospec-run` skill + extends `lint-implementation.sh` + new `ci-wait` shell driver. No new skill.

## 1. Goal & non-goals

### Goal
Eliminate the two failure modes observed across the v1+v2+docs-amendment runs (Sessions 2026-05-21/22):
- **Monitor crashes during long waits** (CI checks, npm install, test runs) — wrapper agent dies at ~75–180 tool calls because it's blocking foreground on poll loops it can't outlast.
- **Implementer ships code that violates saved-memory rules** that already exist in `~/.claude/projects/-Users-wohlgemuth-IdeaProjects-autospec/memory/feedback_*.md` — the rules live in orchestrator context but never reach the implementer's prompt. Reviewer catches violations after the fact, forcing rework.

Five fixes ship as one cohesive amendment.

### Non-goals
- Replacing the LLM implementer with a deterministic codegen (out of scope; LLM is needed for the creative parts)
- Removing the fused guardian+LGTM reviewer (still needed as a backstop)
- Restructuring the autospec skill family (no new skills, no skill splits)
- Distributed/multi-host monitor coordination (single-host design only)

## 2. Architecture & where each fix lands

| Fix | File(s) modified | Mechanism |
|---|---|---|
| #1 Implementer-prompt enrichment | `skills/autospec-run/SKILL.md` (Phase 4 implementer prompt template) | Pre-load saved memory + RULE_IDs + AC bats tests into the implementer prompt before "begin coding" line |
| #2 Pre-commit lint in worktree | `$AUTOSPEC_SCRIPTS_DIR/install-implementer-precommit.sh` (new) + implementer prompt amendment | Implementer-side `.git/hooks/pre-commit` runs `lint-implementation.sh --pre-commit` against the staged diff and blocks the commit on findings |
| #3 CI-wait sentinel | `$AUTOSPEC_SCRIPTS_DIR/ci-wait.sh` (new) + `skills/autospec-run/SKILL.md` (replace synchronous `gh pr checks --watch` invocations) | Background `nohup` poller writes `~/.autospec/ci-state/<PR>.signal` when CI settles; monitor agent reads sentinel + exits to parking state; orchestrator relaunches on signal |
| #4 Adaptive-retry in implementer | `skills/autospec-run/SKILL.md` (Phase 4 implementer prompt) + `lint-implementation.sh` (new `--directives` output mode) | When pre-commit lint fails, capture findings as directives and re-prompt the implementer (up to MAX_IMPL_RETRIES=5) with cumulative directive context |
| #5 Smaller batch + reasoning:deep gating | `skills/autospec-run/SKILL.md` (monitor outer loop) + AGENTS.md (durable preference) | Default `AUTOSPEC_BATCH_SIZE=3` for `reasoning:medium` and lower; force `BATCH_SIZE=1` when the next ready issue carries `reasoning:deep` |

**New shared tooling** (lives at `$AUTOSPEC_SCRIPTS_DIR`, NOT vendored):

- `ci-wait.sh` — spawn background poller, exit immediately; signal file at `~/.autospec/ci-state/<PR>.signal` (JSON `{state, checks, settled_at}`)
- `install-implementer-precommit.sh` — installs the pre-commit hook into a worktree

**Extended tooling:**

- `lint-implementation.sh` — gains `--pre-commit` mode (operates on staged diff, not PR diff) and `--directives` output mode (emits per-finding directive lines suitable for re-prompting)

## 3. Fix #1 — Implementer-prompt enrichment

### 3a. Saved-memory injection

Before the implementer's "begin coding" line, inject the verbatim content of every `~/.claude/projects/-Users-wohlgemuth-IdeaProjects-autospec/memory/feedback_*.md` file that matches at least one tag from the issue body. The implementer prompt template gains:

```
## Project rules you MUST honor

<verbatim concatenation of relevant feedback_*.md bodies>

## RULE_IDs (from AGENTS.md ## Implementation-quality contract)

<verbatim copy of the RULE_ID table>

## Acceptance criteria as constraints

<verbatim copy of the issue's ## Acceptance criteria — every checkbox must be green before push>
```

### 3b. Tag-based memory selection

Memory files declare tags in frontmatter (`tags: [bash, async, autospec-run]`). Implementer prompts include only memories whose tags intersect with the issue's labels + the files-to-read-first language hints. Avoids dumping all 30+ memory files into every prompt.

**Bootstrap:** the first issue in the hardening chain (extend `lint-implementation.sh`) adds `tags:` frontmatter to every existing memory file under the autospec memory dir.

### 3c. AC-as-runnable-constraints

When the implementer prompt assembler sees the issue's `## Acceptance criteria` checkbox list, it converts each `- [ ] <criterion>` into a bats test stub via `$AUTOSPEC_SCRIPTS_DIR/gen-ac-tests.sh <issue-body> > tests/ac/issue-<N>.bats`. The implementer must make all stubs green before it can declare done.

Stub example:
```
- [ ] All bats tests pass
```
becomes:
```bash
@test "AC#1: All bats tests pass" {
  run bash -c "cd skills/autospec-shared && npm test"
  [ "$status" -eq 0 ]
}
```

For criteria that aren't trivially auto-translatable, the stub is `skip "auto-generated stub: edit before declaring AC met"` — implementer must replace with a real assertion before push. (This handles the "claims AC done without verifying" failure mode from #368.)

## 4. Fix #2 — Pre-commit lint hook in implementer worktree

`install-implementer-precommit.sh` writes `.git/hooks/pre-commit` into the worktree:

```bash
#!/usr/bin/env bash
set -euo pipefail
STAGED=$(git diff --cached --name-only)
[ -z "$STAGED" ] && exit 0

OUT=$(mktemp -t autospec-precommit.XXXXXX)
trap 'rm -f "$OUT"' EXIT

if ! bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/lint-implementation.sh" --pre-commit --staged > "$OUT" 2>&1; then
  echo "Pre-commit lint FAILED. Findings:" >&2
  cat "$OUT" >&2
  echo "" >&2
  echo "Run 'lint-implementation.sh --pre-commit --directives' to get re-prompt directives, OR fix the listed RULE_IDs and re-stage." >&2
  exit 1
fi
```

Implementer prompt template gains a step right after `git worktree add ...`:
```
6.5. Install pre-commit hook:
     bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/install-implementer-precommit.sh" .
```

When a `git commit` is attempted with violations, the hook blocks. Implementer must run the directive output and apply fixes before re-staging. This catches every RULE_ID before the PR exists.

## 5. Fix #3 — CI-wait sentinel

`ci-wait.sh`:

```
ci-wait.sh <PR> [--timeout SECONDS] [--required-only]
  Exits immediately. Spawns nohup background poller writing JSON to:
    ~/.autospec/ci-state/<PR>.signal
  Schema: { pr, state: pending|pass|fail|stalled, checks: [...], settled_at: ISO }

ci-wait-poll.sh <PR>
  Reads ~/.autospec/ci-state/<PR>.signal. Returns:
    exit 0 + state=pass
    exit 1 + state=fail
    exit 2 + state=pending (sentinel exists but CI not settled)
    exit 3 + sentinel missing (call ci-wait.sh first)

ci-wait-cleanup.sh <PR>
  Kills background poller, removes sentinel.
```

Background poller (spawned by `ci-wait.sh`):
```bash
nohup bash -c '
  while true; do
    rollup=$(gh pr view '"$PR"' --json statusCheckRollup --jq ...)
    [ "$bad" -gt 0 ] && { write_signal fail; exit; }
    [ "$pending" -eq 0 ] && [ "$total" -gt 0 ] && { write_signal pass; exit; }
    [ "$(elapsed)" -gt "'"$TIMEOUT"'" ] && { write_signal stalled; exit; }
    sleep 30
  done
' > ~/.autospec/ci-state/<PR>.log 2>&1 &
echo $! > ~/.autospec/ci-state/<PR>.pid
```

**Monitor amendment:** the existing `gh pr checks --watch` blocking call in `autospec-run` Phase 4 is replaced with:

```
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/ci-wait.sh" <PR>  # fire-and-forget
# monitor exits to parking state HERE — orchestrator relaunches on signal
```

Orchestrator (the outer agent loop) gains a small wakeup-on-sentinel pattern: poll signal files at start of each tick; if any settled, resume the parked monitor for that PR.

## 6. Fix #4 — Adaptive-retry in the implementer

Mirrors `feedback_llm_validator_adaptive_retry` already applied to the reviewer.

Implementer prompt template gains a retry loop wrapper:

```
attempt=1
MAX=5
directive_context=""
while [ $attempt -le $MAX ]; do
  generate_code_with_prompt($base_prompt + $directive_context)
  if pre_commit_lint passes AND ac_bats_tests green:
    break  # success
  findings = pre_commit_lint --directives + ac_bats_failures
  directive_context += "\n\n## Retry attempt $attempt findings\n$findings\n\nFix these BEFORE the next code generation."
  attempt=$((attempt + 1))
done
if [ $attempt -gt $MAX ]; then
  comment "Implementer hit max retries; manual intervention needed" on issue
  release lock label
  exit failure
fi
```

## 7. Fix #5 — Smaller batch + reasoning:deep gating

Monitor outer loop, before claim:

```
ISSUE = ready[0]
ctx_lbl = labels(ISSUE).ctx_label or "ctx:64k"
reasoning_lbl = labels(ISSUE).reasoning_label or "reasoning:medium"

effective_batch_size = (reasoning_lbl == "reasoning:deep") ? 1 : AUTOSPEC_BATCH_SIZE
```

Net effect: high-blast-radius issues get a fresh monitor each time, limiting crash blast radius.

## 8. Testing

### 8a. Unit tests
- `ci-wait.sh`: fixture PR + stubbed `gh pr view` returning sequences of (pending/pass/fail) — assert sentinel state transitions
- `install-implementer-precommit.sh`: install in a temp git repo, attempt commits with known-bad + known-good diffs, assert correct block/allow
- `lint-implementation.sh --pre-commit --staged`: table of staged diffs → expected RULE_ID findings
- `lint-implementation.sh --directives`: per-RULE_ID → expected directive text
- `gen-ac-tests.sh`: fixture issue bodies → expected bats stub trees

### 8b. Integration tests
- Synthetic "bad-implementer" target: a PR diff with deliberate RULE_ID violations + an AC check that won't pass → run the full adaptive-retry loop with stubbed implementer (returning known bad/good outputs across attempts) → assert correct retry count + final outcome
- Synthetic "long-CI" target: stubbed `gh pr view` returning pending forever then pass → assert monitor parks correctly + signal arrives + orchestrator relaunches

### 8c. Self-test
The hardening chain itself ships through the (currently broken) pipeline. Crash recovery may be needed mid-chain. After the chain merges, re-run a single issue through the now-hardened pipeline as a smoke check — verify the implementer prompt grew, pre-commit hook is installed, sentinel-based CI wait is active.

## 9. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| Existing `lint-implementation.sh` | live | extended (new modes), backward-compatible |
| Existing `autospec-run/SKILL.md` | live | amended (prompt template + outer-loop changes) |
| `gh` CLI | live | required |
| Bash 4+, jq | live | required |

### Out of scope
- Removing the fused guardian+LGTM reviewer (still needed)
- Multi-host monitor coordination
- LLM model selection / cost tuning (separate concern)
- Doc-drift gate (separate amendment, already specced)

## 10. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Where do saved-memory rules live for implementer? | Pre-loaded into implementer prompt template, tag-filtered | Implicit knowledge → explicit working set |
| Pre-commit vs review-time enforcement? | Both — pre-commit blocks first, reviewer still runs as backstop | Defense in depth; reviewer catches semantic issues lint can't |
| CI wait: sentinel vs in-agent loop? | Sentinel (out-of-agent poller) | In-agent polling burns tool calls; sentinel is free |
| Adaptive retry in implementer? | Yes, mirroring reviewer pattern | `feedback_llm_validator_adaptive_retry` is the proven pattern |
| Batch size for deep issues? | 1 (vs 3 default) | Limits crash blast radius for highest-risk issues |
| Bootstrap problem (ship hardening through unhardened pipeline)? | Accept some crash recovery during the chain; smoke test after | Lower friction than building a separate bootstrap path |

## 11. Open follow-ups (separate specs)

1. **Tooling optimization full scope** (per [[project_autospec_tooling_optimization]]) — gen-issue-skeleton, classify-model-fit deterministic, gen-pr-report from templates. Builds on this hardening.
2. **Doc-drift gate integration** — separate spec already filed; orthogonal.
3. **AC-bats coverage check** — measure what % of AC items had real assertions vs `skip` stubs; ratchet over time.
