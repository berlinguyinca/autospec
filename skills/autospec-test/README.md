# autospec-test

Phase 4 unit and E2E coverage gate for autospec implementation PRs.

`autospec-test` runs after build/lint and before auto-merge. It checks unit
coverage, E2E coverage, UI element coverage, behavior taxonomy coverage, and
assertion-shift safety. Its self-heal loop can strengthen tests or fix product
bugs within the configured budget.

## Invocation

```text
/autospec-test [PR#]
/autospec-test --init
```

## Result

- Unit and E2E gate status.
- Coverage and behavior-taxonomy findings.
- Assertion-shift blocking for loosened tests.
- Optional self-heal commits within the configured loop budget.

Use [`autospec-qa`](../autospec-qa/README.md) when you need a broader
spec-to-running-app revalidation pass and regenerated tests from user-visible
behavior.

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
