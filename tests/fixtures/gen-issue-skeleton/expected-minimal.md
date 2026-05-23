## Goal

Add scripts/gen-issue-skeleton.sh to render structured YAML input into an 11-section issue body.

## Source spec section anchor

`docs/specs/2026-05-22-autospec-tooling-optimization-design.md` — https://github.com/berlinguyinca/autospec/blob/main/docs/specs/2026-05-22-autospec-tooling-optimization-design.md

## Files to read first

- docs/specs/2026-05-22-autospec-tooling-optimization-design.md

## Local-LLM notes

Pure implementation; keep files within 200 lines so a 60-120k context window holds the full source during edits.

## Implementation scope

- scripts/gen-issue-skeleton.sh with --input <file> and stdin fallback

## Out of scope

- Model-fit block (owned by T2)

## Implementation outline

1. Add scripts/gen-issue-skeleton.sh with --input flag
2. Parse required YAML keys from input
3. Substitute into 11-section markdown template
4. Pipe output through scripts/lint-issue.sh

## Tests required

- bats tests/gen-issue-skeleton.bats with 3 fixtures

## Acceptance criteria

- [ ] scripts/gen-issue-skeleton.sh --input minimal.yaml emits 11-section body

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
