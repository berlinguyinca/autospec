# Non-vacuous AC — absence paired with a positive post-condition

## Goal

Add a rule to `scripts/lint-issue.sh` that accepts the paired fixture criterion.

## Files to read first

- `scripts/lint-issue.sh`

## Implementation outline

- Detect the paired AC line and emit no finding.

## Tests required

- [ ] `bats tests/lint-issue.bats` exits `0`

## Acceptance criteria

- [ ] `scripts/lint-issue.sh` does not contain the string `vacuity` and prints `done`

## Dependencies

none

## Verification

Primary smoke test (inner loop):

```
echo ok
```
