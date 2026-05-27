---
name: project-gap-remediation-keyword-routing
description: Two autospec features shipped 2026-05-24 (end-of-run gap remediation + keyword auto-routing) and the parallel-wave pipeline execution pattern that built them
metadata:
  node_type: memory
  type: project
  originSessionId: 720b91a2-3664-4f6b-a9ba-ebc4f439de76
---

Shipped 2026-05-24 in one session, driven end-to-end through autospec's own pipeline per the user directive "use autospec as much as possible / keep it moving":

1. **End-of-run gap remediation loop** — after a run drains, `/autospec-review --remediation --emit-gaps` does a broad review (spec-coverage + correctness + test-quality + integration-wiring + docs) with a false-positive filter, then `gap-remediation-loop.sh` files surviving gaps as `auto-implement,gap-remediation,priority:high` issues, re-looped up to `AUTOSPEC_GAP_MAX_ROUNDS` (default 2). New `## Phase 5.5` section in the autospec-run trio replaced the report-only post-batch audit. Scripts: `run-batch-start.sh`, `gap-json-lib.sh`, `emit-gaps.sh`, `gap-remediation-loop.sh`. Spec `docs/specs/2026-05-24-autospec-end-of-run-gap-remediation-design.md` + plan in `docs/superpowers/plans/`. Issues #530-536, PRs #542/#543/#545/#546/#548/#549/#550, tracker #540 (closed).
2. **autospec-listen keyword auto-routing** — `listener-match.sh --classify` (verb→skill map + imperative intent gate, biased to false-negatives) + a `## Keyword auto-routing` trio section. Imperative `design`/`new feature`/`spec`→`/autospec-define`, `implement`/`build`/`ship`→`/autospec-run`, `review`→`/autospec-review`, `autospec …`→`/autospec`, with a one-line opt-out. Spec `docs/specs/2026-05-24-autospec-listen-keyword-routing-design.md`. Issues #537-538, PRs #544/#547, tracker #541 (closed).

**Origin:** "review everything for gaps now + always do this at the end" → a deep review found G1/G2/G3 (G1 = a real GNU-vs-BSD grep divergence in `cross-repo-search.sh`), fixed via #525/#526/#527/#528, then institutionalized as feature 1. Keyword-routing from "route common verbs into the autospec loop."

**Parallel-wave execution pattern (the reusable workflow win):** decomposed both specs into 9 dep-linked issues, then drained them as **dependency-ordered waves of parallel opus single-issue monitors** operating on disjoint files. Per monitor: explicit single issue, `run_in_background`, isolated `/tmp/wt-*` worktree off origin/main, primary checkout pinned to `main`, **watchdog reconciliation disabled** (so concurrent siblings don't reclaim each other's claims), and **no shared `~/.autospec/batch-done.json`** (relied on task-completion notifications instead). Concurrent merges were absorbed by the rebase-and-retest gate (BEHIND→rebase→retest). Result: ~zero recovery needed, far faster than sequential. **Opus monitors were 100% reliable across all 16 single-issue runs this session; sonnet truncates** — see [[feedback_monitor_silent_exit]]. Codex peer-review caught 3 real bugs in #532 + a regex-wildcard bug in #535 — keep it in the loop.

**How to apply:** when implementing a decomposed spec via autospec, launch the no-dependency wave as parallel opus single-issue monitors on non-overlapping files, then fire each dependent wave as its deps close. Disable the watchdog and skip the shared batch-done signal for parallel runs. Related: [[project_memory_consumers_epic]] (prior single-monitor run).
