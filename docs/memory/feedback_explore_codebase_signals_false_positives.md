---
name: feedback_explore_codebase_signals_false_positives
description: "autospec-explore codebase-signals researcher greps (TODO|FIXME|XXX|HACK)[: ] which matches prose (\"TODO list CLI\", \"FIXME found\"), non-.md assets (.cast/.html), and its OWN source + lint tooling that documents the markers — these noise proposals dominate ranking and the constitution gate (45→45) drops none"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: c279ed81-38f7-4098-ab47-a0a192f0366f
---

A bounded `explore-research-cycle.sh` round (deterministic 4-source, no internet/LLM)
on the autospec repo returned 8/8 top-ranked proposals as `codebase-signals` false
positives: demo copy `"build me a TODO list CLI"` in docs/quickstart.cast +
docs/site/index.html, the researcher's OWN comments in codebase-signals.sh, and
lint-implementation.sh help text describing the TODO_LEFT check.

Root cause `scripts/explore-research/codebase-signals.sh:30`:
`git grep -n -I -E '(TODO|FIXME|XXX|HACK)[: ]' -- ':!*.md'` — the `[: ]` allows a
trailing SPACE (so "TODO list"/"FIXME found" prose matches), only `*.md` is excluded
(so `.cast`/`.html`/asset files slip), and nothing excludes the researcher's own dir or
the lint tooling that documents the markers.

**Why:** noise crowds out the genuine spec-drift / prior-report proposals (small +
weight 0.7 → score 0.385 dominates), and the constitution gate is toothless here —
`proposals_after_constitution: 45` dropped 0/45. Unsupervised auto-filing would have
opened ~8 garbage "chore: address TODO in marketing copy" issues. This is the concrete
argument for surface-before-file discipline on explore rounds.

**How to apply:** tighten the marker regex to require a comment-marker shape
(`(TODO|FIXME|XXX|HACK)(\(|:)`, not a bare space); exclude `scripts/explore-research/`,
`scripts/lint-*.sh`, `docs/`, and non-source asset extensions (`.cast`,`.html`,`.svg`);
and give the constitution a rule that culls low-value `chore: address <marker>` proposals
(or down-weights self/doc matches). Related: [[feedback_installer_excludes_runtime_libs]]
(the DOA install that hid this until now), [[feedback_self_consistent_test_fixtures_mask_bugs]].
