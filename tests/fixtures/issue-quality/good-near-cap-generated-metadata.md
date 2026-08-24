## Goal

Exempt marker-bounded generated metadata from the `scripts/lint-issue.sh` 400-word budget.

## Files to read first

- `scripts/lint-issue.sh` — the `strip_generated_metadata` helper and the BODY_TOO_LONG rule.
- `skills/autospec-split/SKILL.md` — the Phase 3.5 `## Model fit` block template.
- `skills/autospec-classify/SKILL.md` — the same template carried by the standalone skill.
- `skills/autospec-define/SKILL.md` — the same template carried by the define pipeline.
- `tests/unit/test_lint_issue.bats` — the fixture-driven rule tests this fixture feeds.

## Local-LLM execution notes

32k routine, single-pass. The change is one awk block plus matching template and
fixture edits; no cross-crate reasoning and no new abstractions are required.

## Implementation scope

Track a third marker family inside `strip_generated_metadata` and exclude those
lines from the authored word count alongside the classification and
shared-contract families. Keep the three families independent so a malformed or
half-written block in one family never suppresses counting for another. Update
every skill template so the generated heading itself sits inside the marker pair
rather than above it, because the exemption is line-bounded and a heading placed
above the opening marker is counted as authored prose.

## Out of scope

Changing the 400-word budget itself, altering the set of ui-feature sections that
the separate `strip_ui_sections` helper removes before counting, and touching any
rule other than BODY_TOO_LONG.

## Implementation outline

1. `scripts/lint-issue.sh` — record `quality_begin` and `quality_end` line numbers.
2. `scripts/lint-issue.sh` — add `in_quality` to the line-retention predicate.
3. `tests/fixtures/issue-quality/` — extend the fixtures to carry a quality block.

## Tests required

- `bats tests/unit/test_lint_issue.bats` covering a body that stays within the
  budget only because the generated blocks are exempt from counting.
- A fixture whose authored prose alone approaches the cap, so that a regression in
  the exemption logic changes the exit code instead of passing unnoticed.
- Real script execution against the fixture files; no stubbed linter output.

## Acceptance criteria

- [ ] `scripts/lint-issue.sh` exits 0 for this fixture.
- [ ] `strip_generated_metadata` references `autospec-quality` markers.
- [ ] `bats tests/unit/test_lint_issue.bats` passes with 0 failures.

## Verification

### Primary smoke test

```bash
bash scripts/lint-issue.sh tests/fixtures/issue-quality/good-near-cap-generated-metadata.md
```

### Operator/full verification

```bash
bats tests/unit/test_lint_issue.bats
```

## Files touched

- `scripts/lint-issue.sh`
- `tests/fixtures/issue-quality/good-near-cap-generated-metadata.md`
- `tests/unit/test_lint_issue.bats`

## Branch name

`feat/quality-block-word-budget`

<!-- autospec-classify:begin -->
## Model fit

- **ctx:** `ctx:32k` — one awk block plus fixture edits, three files staged.
- **reasoning:** `reasoning:medium` — mirrors two existing marker families.

*Auto-classified by Phase 3.5 on 2026-08-23.*
<!-- autospec-classify:end -->

<!-- autospec-shared-contracts:begin -->
## Shared contracts

- `strip_generated_metadata` owns every marker family; callers never filter markers themselves.
- Marker names follow `autospec-<family>:begin` and `autospec-<family>:end`.
- Generated headings live inside their marker pair so they are exempt from the budget.
<!-- autospec-shared-contracts:end -->

<!-- autospec-quality:begin -->
## Quality lint

- **GOAL** — the goal sentence names a concrete script path and a numeric budget,
  so the concreteness rule is satisfied without further rewriting here.
- **AC#1** — every acceptance item names a path, a backticked span, or an integer,
  so each one is machine-checkable rather than a matter of reviewer judgement.
- **SMOKE** — the primary smoke test is a single command line inside one fenced
  block, so the inner-loop rule is satisfied and no chaining is required.

*Auto-linted by Phase 3.5 on 2026-08-23.*
<!-- autospec-quality:end -->

## Dependencies

none
