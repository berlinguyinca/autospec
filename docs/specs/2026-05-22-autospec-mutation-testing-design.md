# Autospec Mutation Testing — Test-of-Tests Layer

**Status:** Draft design (2026-05-22)
**Author:** berlinguyinca + accumulated diagnostic from this session
**Scope:** Closes the vacuous-truth gap class. Extends `lint-implementation.sh`, adds mutation testing gate to Phase 4 QA, ships a vacuous-assertion detector + assertion-density floor + negative-path heuristic. Tracker issue #420.

## 1. Goal & non-goals

### Goal
Add a deterministic, language-aware "test-of-tests" layer that catches **tests claiming to verify behavior they don't actually exercise**. The motivating bug was PR #397's `tests/assemble-impl-prompt.bats` test 15 (`grep -qv "X" || true` — always passes regardless of exclusion logic) — exactly what `gen-ac-tests.sh --verify` was designed to prevent, written into the very PR that built that detector. Future-proof the pipeline so this class of vacuous truth is caught structurally.

### Non-goals
- Replacing the LLM reviewer (still needed for semantic correctness review)
- Mutation testing on dependencies or vendor code
- Cross-language mutation operators beyond what each language's standard tooling supports
- Property-based test generation (separate concern, future)

## 2. Architecture

Three deterministic layers plus one runtime gate, ordered by cost:

1. **Pre-commit lint** (`lint-implementation.sh --pre-commit`) — extends with vacuous-assertion detector + assertion-density floor. Runs in implementer worktree. ~1 sec per commit.
2. **PR-time gate** (mutation testing) — opt-in via `area:safety` / `area:hardening` label on issue; new `scripts/run-mutation-test.sh` invoked in Phase 4 QA chain after build/lint/test passes. ~30s–5min depending on diff size.
3. **Negative-path pair heuristic** (`scripts/check-negative-path-pairs.sh`) — for each `should pass` test name, look for a sibling `should fail` test. Warn (not fail) if missing.

All three layers respect a `mutation-testing: skip` issue-body escape hatch (mirrors the existing `docs: skip` pattern for the drift gate).

## 3. Component 1 — Vacuous-assertion detector (extends `lint-implementation.sh`)

New `--vacuous-assertions` mode (also bundled in `--pre-commit`):

| Pattern | RULE_ID | Severity |
|---|---|---|
| `grep -qv "X" \|\| true` | `VACUOUS_GREP_INVERSE_OR_TRUE` | BLOCK |
| `\|\| true` at end of any test assertion line | `VACUOUS_OR_TRUE` | BLOCK |
| `expect(true).toBe(true)` / `expect(1).toBe(1)` | `VACUOUS_TAUTOLOGY` | BLOCK |
| `assert(1 === 1)` / `assert True` / `xit(...)` | `VACUOUS_TAUTOLOGY` | BLOCK |
| `@test ... { skip "auto-stub" }` in `tests/ac/` | `VACUOUS_AC_STUB` | BLOCK |
| Empty test body `it("...", () => {})` | `VACUOUS_EMPTY_TEST` | BLOCK |
| `assert.ok(true)` / `t.true(true)` | `VACUOUS_TAUTOLOGY` | BLOCK |
| `for ... { /* no asserts */ }` test bodies | `VACUOUS_NO_ASSERT` | WARN |

Detector is bash + grep + per-language AST helper where regex is insufficient (e.g., `tree-sitter` query for empty test bodies).

Each finding emits a structured directive line for the adaptive-retry implementer prompt:

```
RULE_ID:VACUOUS_GREP_INVERSE_OR_TRUE:<file>:<line>: `grep -qv` succeeds when ANY line lacks the pattern; `|| true` makes the assertion a no-op. Use `! grep -q` instead.
```

## 4. Component 2 — Mutation testing gate (`run-mutation-test.sh`)

Per-language adapter pattern:

| Language | Tool | Adapter |
|---|---|---|
| Node/TS | `stryker-mutator` | `mutation-adapters/stryker.sh` |
| Python | `mutmut` | `mutation-adapters/mutmut.sh` |
| Go | `go-mutesting` | `mutation-adapters/go-mutesting.sh` |
| Bash | `bash-mutate.mjs` (custom — small mutator: flip `==` ↔ `!=`, drop one assertion, swap literals) | `mutation-adapters/bash-mutate.sh` |

Gate flow:
1. Detect changed files since base ref
2. Group by language; dispatch adapter per group
3. Each adapter mutates only changed source files (not tests, not vendor) and runs the existing test suite
4. Aggregate: total mutants vs killed
5. Gate threshold: ≥80% mutants killed per changed file (configurable per repo via `.autospec/mutation-testing.yml`)

**Opt-in scoping:** gate fires only when issue has `area:safety`, `area:hardening`, or explicit `mutation-testing: required` label. Defaults to disabled to avoid token cost on every PR.

## 5. Component 3 — Assertion-density floor

Pre-commit lint adds:
- Every modified test file must have ≥1 `assert`/`expect`/equivalent per logical test block.
- Every new public function in product code must have ≥1 test that references it (already covered by `function-presence` from v1 Stage 1, but extended here to ALL changed code, not just unit-coverage scope).

## 6. Component 4 — Negative-path pair heuristic

New `scripts/check-negative-path-pairs.sh`:
- Scan changed test files for test names matching `(should|when).*(success|valid|pass|works|returns)`
- For each, look for a sibling test name matching `(should|when).*(fail|invalid|reject|throws|errors)`
- Missing pair → `WARN` finding (not block)

Operator-tunable via per-language `negative-path-patterns.yml` overlay in the target repo.

## 7. Failure semantics

| Layer | Block? | Self-heal loop? |
|---|---|---|
| Vacuous-assertion BLOCK | Yes (pre-commit) | Yes (adaptive retry with directive) |
| Vacuous-assertion WARN | No | Surfaced in PR comment |
| Mutation testing gate fail | Yes (Phase 4 gate) | Yes (loop classifier gets `missing_mutation_coverage` category — adds new tests for uncovered mutants) |
| Negative-path WARN | No | Surfaced in PR comment |

## 8. Decomposition (5 phases for /autospec-split)

1. **Phase M1** — Vacuous-assertion detector in `lint-implementation.sh` + 8 RULE_IDs + bats coverage. ~1 PR. `priority:high` (closes the immediate gap from PR #397).
2. **Phase M2** — `bash-mutate.mjs` + `mutation-adapters/bash-mutate.sh` (bash mutation is custom-built since no off-the-shelf tool exists; ship first to validate the architecture on autospec itself). ~1 PR.
3. **Phase M3** — Per-language adapters for Stryker (Node), mutmut (Python), go-mutesting (Go). ~1 PR. Each adapter is a thin wrapper invoking the language-native tool with per-file scoping.
4. **Phase M4** — `scripts/run-mutation-test.sh` orchestrator + `scripts/qa-phase4.sh` Phase 4 QA chain wiring + opt-in scoping + `tests/run-mutation-test.bats` (7 cases). ~1 PR.
5. **Phase M5** — `scripts/check-negative-path-pairs.sh` + `--assertion-density` flag in `scripts/lint-implementation.sh` + `negative-path-patterns.yml` overlay + integration tests (`tests/mutation-integration.bats`, 8 cases) against 3 synthetic targets (`tests/fixtures/mutation-integration/{bash,node,python}`). ~1 PR.

All 5 phases carry `priority:high` so they ship before queued docs-amendment-style work.

## 9. Testing

### 9a. Unit tests (per phase)
- Per RULE_ID: positive (detected) + negative (not flagged) fixture
- Per language adapter: stubbed tool output → expected mutation report shape
- Assertion-density floor: empty-test fixture, asserts-present fixture
- Negative-path: paired + unpaired fixture sets

### 9b. Integration tests (Phase M5)
- Synthetic bash target with a deliberate vacuous test → Phase M1 catches it
- Synthetic Node target with a function where a mutant survives all tests → Phase M3 catches it
- Combined: target with 2 vacuous + 1 mutation gap → both surface

## 10. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| `lint-implementation.sh` | live (#388) | extended in M1 |
| Pre-commit hook installer | live (#388) | no change needed |
| Phase 4 QA chain | live | M4 hooks before LGTM |
| Stryker / mutmut / go-mutesting | external tools | absent → adapter exits 0 with warning, doesn't block |
| `bash-mutate.mjs` | new in M2 | own implementation |

### Out of scope
- Property-based test generation
- Cross-file dataflow mutation
- Fuzz testing integration
- LLM-driven mutant generation

## 11. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Mutation testing on every PR or opt-in? | Opt-in via label (`area:safety`/`area:hardening`/`mutation-testing: required`) | Token cost too high for every PR; high-risk surfaces benefit most |
| Where does vacuous-assertion detector live? | `lint-implementation.sh` extension, NOT a new script | Reuses existing pre-commit hook + adaptive-retry directive path |
| Bash mutation testing source? | Custom `bash-mutate.mjs` | No off-the-shelf bash mutator; small enough to build |
| Default threshold? | ≥80% mutants killed per changed file | Industry standard for mutation gates |
| Negative-path pair heuristic — block or warn? | Warn only | Pattern matching is heuristic; false positives common |
| Escape hatch? | `mutation-testing: skip` in issue body | Mirrors existing `docs: skip` pattern |

## 12. Open follow-ups (separate specs)

- Property-based test generation (future v3)
- LLM-driven mutant generation (advanced; ship traditional mutation gate first to measure baseline)
- Cross-file dataflow mutation (research territory)
