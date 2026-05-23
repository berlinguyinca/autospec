---
name: project-autospec-mutation-testing
description: Queued for after pipeline-hardening completes — mutation testing + assertion-density floor + negative-path-pair lint as test-of-tests layer
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2d7883e9-9977-428f-8919-ef9b88df12a4
---

User asked 2026-05-22 (mid-pipeline-hardening): "how do you feel about the idea of testing tests?"

Tied directly to the #368 failure mode where a test claimed to exercise "malformed × 4 then valid" adaptive retry but only ran the happy path. The pipeline-hardening covers two pieces of this already:

- **AC-stub-still-skipped detector** (folded into #391 amendment via `gen-ac-tests.sh --verify` mode) — blocks push if any AC bats stub remains `skip "auto-stub"`. Closes the trivial vacuous-truth path.
- **Pre-commit lint hook** (#388) — catches deterministic anti-patterns before review.

What remains for full test-of-tests coverage:

1. **Mutation testing gate** in Phase 4 QA — flip `==` to `!=`, drop one assertion, replace a literal with a different literal; run tests against each mutant; gate at ≥80% mutants caught. Tools: Stryker (JS), mutmut (Python), `go-mutesting`. Start scoped to `area:safety` / `area:hardening` issues; broaden later.
2. **Assertion-density floor** — minimum assertions per test, minimum tests per public function. Pre-commit lint.
3. **Negative-path coverage** — every "this should succeed" test pairs with a "this should fail" test. Per-language linter heuristic.

**Sequencing:** ship pipeline-hardening (#387–#392) first, then this work. Tooling-optimization (per [[project_autospec_tooling_optimization]]) and init-skill-amendment (per [[project_autospec_init_skill]]) both still queued; mutation-testing slots before or after them depending on user preference at the time.

**How to apply:** When pipeline-hardening completes, run `/autospec-define` for a `docs/specs/<DATE>-autospec-mutation-testing-design.md` spec covering the 3 items above. Decompose via /autospec-split. Implement via /autospec-run with hardened pipeline.
