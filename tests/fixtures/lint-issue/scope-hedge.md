# Scope-hedge — interim outcomes named inside scope sections

## Goal

Gate `ScaleDown` behind the deadline-kill rate in `scripts/scale-policy.sh`.

## Files to read first

- `scripts/lint-issue.sh`

## Implementation outline

- A conservative interim: gate `ScaleDown` now and leave the other 3 signals for later.

## Tests required

- [ ] `bats tests/lint-issue.bats` exits `0`

## Acceptance criteria

- [ ] `scripts/scale-policy.sh` allows `Rebalance` and `ScaleUp` for now

## Dependencies

none

## Verification

Primary smoke test (inner loop):

```
echo ok
```
