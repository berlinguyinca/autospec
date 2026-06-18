# autospec-upgrade

Behavior-locked, project-independent framework upgrades for Angular, Next.js,
and React projects.

Use `autospec-upgrade` when a project needs to be upgraded to the latest
official framework versions safely. The skill locks observable behavior before
touching any versions, upgrades one major at a time via official codemods, and
gates completion on mutation score ≥ pre-upgrade baseline — not line coverage
alone.

## Invocation

```text
/autospec-upgrade --repo .
/autospec-upgrade --repo . --framework angular
/autospec-upgrade update
```

## Result

- Detection JSON identifying frameworks, versions, package manager, and test
  runners.
- Behavior-lock status and recorded mutation baseline.
- Per-hop upgrade log (codemod applied, build/test result, tag created).
- Mutation gate result (score vs baseline).
- `autospec-qa --no-heal` verdict.
- Migration doc from `autospec-doc --full`.
- Tags and `.autospec/upgrade-report.json`.

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-test`](../autospec-test/README.md) | Test authoring + coverage gate reused for behavior-lock. |
| [`autospec-playwright`](../autospec-playwright/README.md) | Playwright run reused for golden-master snapshots. |
| [`autospec-qa`](../autospec-qa/README.md) | Post-upgrade no-mock smoke + console/network gate. |
| [`autospec-doc`](../autospec-doc/README.md) | Migration documentation (docs-as-tests). |
| [`autospec-run`](../autospec-run/README.md) | Worktree / PR-aware ladder / CI-wait. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
