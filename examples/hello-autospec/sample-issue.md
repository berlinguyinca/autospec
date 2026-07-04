# Add hello autospec smoke command

## Goal

Add `src/hello.sh` that prints exactly `hello autospec`.

## Files to read first

- `examples/hello-autospec/spec.md`

## Implementation outline

- Create `src/hello.sh`.
- Create `tests/hello.bats`.
- Keep the output exact and newline-terminated.

## Acceptance criteria

- [ ] `src/hello.sh` exists and is executable.
- [ ] `tests/hello.bats` contains one output assertion.
- [ ] `bats tests/hello.bats` passes.

### Primary smoke test (inner loop)

```bash
bats tests/hello.bats
```

