# Hello AutoSpec Spec

## Goal

Add a command that prints `hello autospec` and validates the output with one smoke test.

## Acceptance Criteria

- [ ] `src/hello.sh` prints exactly `hello autospec`.
- [ ] `tests/hello.bats` checks the command output.
- [ ] The closeout report cites the smoke test command.

## Primary Smoke Test

```bash
bats tests/hello.bats
```

