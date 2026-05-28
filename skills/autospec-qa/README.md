# autospec-qa

Spec-to-running-app QA revalidation for autospec projects.

Use `autospec-qa` when an implementation is believed to be complete but needs a
broad proof pass against the source spec. The skill audits every requirement,
UI control, validation rule, state transition, API effect, accessibility path,
and cross-feature workflow, then regenerates missing or weak automated tests.

## Invocation

```text
/autospec-qa --spec docs/specs/<feature>.md --url http://localhost:3000
/autospec-qa --spec docs/specs/<feature>.md --run "npm run dev"
/autospec-qa revalidate latest spec
```

## Result

- A spec traceability matrix with PASS/FAIL/PARTIAL/NOT TESTED status.
- UI control and validation coverage findings.
- Accessibility, responsive, API, and negative-path findings.
- Regenerated or strengthened tests where the repo has an existing harness.
- Follow-up issue drafts for remaining implementation gaps.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-test`](../autospec-test/README.md) | Deterministic Phase 4 unit/E2E coverage gate. |
| [`autospec-review`](../autospec-review/README.md) | Spec-vs-issues audit and regression issue filing. |
| [`autospec-run`](../autospec-run/README.md) | Implementation loop that produces PRs. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
