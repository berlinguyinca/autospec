---
name: per-pr-lgtm-misses-integration
description: Per-PR self-review LGTM caught zero integration bugs across 19 PRs; Phase 5.5 broad audit caught 7 highs. Always run Phase 5.5; per-PR review is necessary but not sufficient.
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 37f5bea4-90df-4cf3-8298-d8158b15d2ca
---

Per-PR self-review LGTM (the autospec Phase 4 inline reviewer) caught **0 of 7** high-severity integration bugs across the 19-PR context-monitor batch shipped 2026-06-01. Phase 5.5 broad-audit caught them after the fact:

- g-001: PreCompact hook argparse crashes immediately (`--hook-event` wasn't a flag).
- g-002: `install.sh` never `pip install`s the package (silent post-install failure).
- g-003: handoff dir mismatch (`~/.turbo` vs `<cwd>/.turbo`) — validation gate vacuously passed.
- g-004: `wait_for_handoff` was dead code; daemon never waited.
- g-005: engine skipped compact when context jumped >50% direct to >80%.
- g-006: daemon dispatched literal `/clear` instead of calling `adapter.command()`.
- g-007: `validate.sh` failing on new skill's lockstep duo.

**Why per-PR review missed everything:** each PR looked fine in isolation. The bugs lived at integration boundaries (daemon ↔ adapter, daemon ↔ install.sh, daemon ↔ handoff). Per-PR reviewer only sees the diff for one PR.

**How to apply:**

1. **Never skip Phase 5.5** — even when the queue is fully merged and all per-PR LGTMs passed, Phase 5.5 is what catches integration. `~/.autospec/no-review.flag` should rarely if ever exist.
2. **Smoke tests must be executable, not descriptive.** Issue templates have `### Primary smoke test (inner loop)` — make it an actual shell command that hits the binary, not prose. Framework issue #821 codifies this as a Phase 4 merge gate.
3. **Pattern survey before code** — implementer should always know what already exists. Framework issue #820 codifies a `## Pattern survey` step before code generation.
4. **Subprocess mocks ARE allowed** — `tmux`/`osascript`/`notify-send` are EXTERNAL boundaries (not internal services). The "no mocks" rule applies to internal services only. A batch-11 monitor stalled this session because it tried to run real tmux in tests; batch 12 succeeded after explicit "mock external subprocesses" guidance.
5. **For multi-bug integration fixes, use opus**. Sonnet silent-exited twice on #779 (6 interconnected bugs); opus completed in one shot. Hard cases: `priority:high` + integration-shaped + >2 files touched.

Related: [[autospec-split-origin-main-gate]], [[autospec-decomposer-gotchas]], [[monitor-silent-exit]].
