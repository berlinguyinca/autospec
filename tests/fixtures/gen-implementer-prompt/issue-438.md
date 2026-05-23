## Goal

Add a simple example script that prints "hello world".

## Files to read first

- `scripts/lint-issue.sh`

## Implementation scope

- `scripts/hello.sh`

## Acceptance criteria

- [ ] `bash scripts/hello.sh` prints "hello world"
- [ ] `bats tests/hello.bats` passes

## Verification

### Primary smoke test

```
bats tests/hello.bats
```

## Branch name

`feat/example-hello`
