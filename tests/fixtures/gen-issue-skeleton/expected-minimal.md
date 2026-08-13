## Goal

Add scripts/gen-issue-skeleton.sh to render structured YAML input into a team-lensed issue body.

## Source spec

`docs/specs/2026-05-22-autospec-tooling-optimization-design.md` — https://github.com/berlinguyinca/autospec/blob/main/docs/specs/2026-05-22-autospec-tooling-optimization-design.md

## Team personality

- Tooling maintainers: architect, shell developer, test engineer
- Emphasis: deterministic issue rendering and lock-step validation compatibility

## Review counter-team

- Reliability review: release engineer, docs-drift reviewer, regression tester
- Challenge: generated issues must preserve all prompt-governance sections

## Files to read first

- docs/specs/2026-05-22-autospec-tooling-optimization-design.md
- scripts/lint-issue.sh

## Files touched

- scripts/gen-issue-skeleton.sh
- tests/gen-issue-skeleton.bats

## Local-LLM execution notes

- Pure implementation; keep the renderer and its tests in context together.

## Dependencies

none



## Implementation scope

- scripts/gen-issue-skeleton.sh with --input <file> and stdin fallback

## Out of scope

- Model-fit block (owned by T2)

## Implementation outline

1. Add scripts/gen-issue-skeleton.sh with --input flag
2. Parse required YAML keys from input
3. Substitute into team-lensed markdown template
4. Pipe output through scripts/lint-issue.sh

## Tests required

- bats tests/gen-issue-skeleton.bats with 3 fixtures

## Acceptance criteria

- [ ] scripts/gen-issue-skeleton.sh --input minimal.yaml emits a team-lensed issue body
- [ ] Output passes scripts/lint-issue.sh with exit 0
- [ ] Missing required YAML field exits non-zero with `MISSING_FIELD:<name>` on stderr

## Verification

### Primary smoke test

```
bats tests/gen-issue-skeleton.bats
```

### Operator full

```
bash scripts/gen-issue-skeleton.sh --input tests/fixtures/gen-issue-skeleton/minimal.yaml | scripts/lint-issue.sh /dev/stdin
```

## Branch name

`feat/tooling-opt-t1-gen-issue-skeleton`
