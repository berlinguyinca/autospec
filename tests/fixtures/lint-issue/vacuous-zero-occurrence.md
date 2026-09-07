# Vacuous AC — zero-occurrence claim

## Goal

Add a rule to `scripts/lint-issue.sh` that rejects the vacuous fixture criterion.

## Files to read first

- `scripts/lint-issue.sh`

## Implementation outline

- Detect the zero-occurrence AC line and emit `AC_VACUOUS`.

## Tests required

- [ ] `bats tests/lint-issue.bats` exits `0`

## Acceptance criteria

- [ ] The linter reports `0` occurrences of the `todo` literal

## Dependencies

none

## Verification

Primary smoke test (inner loop):

```
echo ok
```
