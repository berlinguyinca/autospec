---
name: feedback_self_consistent_test_fixtures_mask_bugs
description: "Tests that build fixtures with the same expression as the code-under-test can't catch a bug in that expression; pin against the real external convention"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d039abad-07c8-4595-980c-99dc83e9788b
---

When a function derives a value from an external convention (a filesystem path,
an API slug, a third-party encoding), do NOT build the test fixture by reusing
the same derivation expression — the test becomes self-consistent with the bug
and passes regardless of correctness.

**Concrete case (2026-06-02, context-monitor):** `ClaudeAdapter.find_transcript`
computed the Claude project-dir slug as `cwd.replace("/","-").lstrip("-")`. The
`.lstrip("-")` was wrong — Claude Code's real `~/.claude/projects/<slug>` keeps
the leading dash (`/Users/x/repo` → `-Users-x-repo`) and maps EVERY
non-alphanumeric char to `-` (verified: `.../m_/...` → `...-m--...`). Correct:
`re.sub(r"[^a-zA-Z0-9]", "-", cwd)`. The unit tests had constructed their fixture
dirs with the same buggy expression, so they were green while the deployed
PreCompact hook hit `TranscriptNotFoundError` on every compaction and the
auto-context-rollover silently no-opped for months.

**Why:** the bug was in the cwd→slug mapping; the test encoded the same mapping,
so both agreed on the wrong answer.

**How to apply:**
- Pin against ground truth: assert known real values (an actual live
  `~/.claude/projects` slug), or a hand-written golden, NEVER the SUT's own formula.
- For "did it actually run end-to-end" bugs, reproduce against the real
  environment (run the real hook against the real cwd and read the log) — that
  is what surfaced this, not the unit tests.
- Pair every external-convention mapping with one test that would fail if the
  formula drifts from the real convention (here: leading-dash, dots, underscores/
  specials cases).

Related: [[feedback_llm_validator_adaptive_retry]] (validators need independent
ground truth too), [[feedback_monitor_silent_exit]] (context-monitor quirks).
