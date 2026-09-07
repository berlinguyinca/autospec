# Vacuous AC — absence-of-literal claim

## Goal

Add a rule to `scripts/lint-issue.sh` that rejects the vacuous fixture criterion.

## Files to read first

- `scripts/lint-issue.sh`

## Implementation outline

- Detect the absence-of-literal AC line and emit `AC_VACUOUS`.

## Tests required

- [ ] `bats tests/lint-issue.bats` exits `0`

## Acceptance criteria

- [ ] `scripts/lint-issue.sh` does not contain the string `vacuity`

## Dependencies

none

## Verification

Primary smoke test (inner loop):

```
echo ok
```
