# autospec-fab

Autonomous fabrication QA pipeline for parametric 3D / CAD-as-code projects that
produce 3D-printable STL. The skill regenerates artifacts from source, then gates
every printable model through geometry, vacuum/pressure, gasket, airflow, slicer,
FEA, CFD, render, and visual-inspection stages before release.

Target repos opt in via `.autospec/fab.yml`.

## Invocation

```text
/autospec-fab --repo .
/autospec-fab update
```

## Result

- Clean catalog regen (`rm -rf build && .venv/bin/python src/generate.py`).
- Per-model release-gate JSON (`.autospec/fab/release-gate.json`) with stage
  results for geometry, vacuum/pressure, gasket, airflow, slicer, FEA, CFD,
  render, and vision advisory.
- Hard reject on: blocked NPT access, exposed gasket sides, disconnected flow,
  non-watertight mesh, disconnected bodies, FEA below safety factor, or CFD
  target miss.
- Non-blocking vision advisory findings surfaced for operator triage.
- Unit suite result (`.venv/bin/python -m unittest discover -s tests`).

## Related skills

| Skill | Purpose |
| --- | --- |
| [`autospec-run`](../autospec-run/README.md) | Worktree / PR-aware ladder / CI-wait. |
| [`autospec-define`](../autospec-define/README.md) | Decompose CAD features into regression-test-first issues. |
| [`autospec-qa`](../autospec-qa/README.md) | Post-gate no-mock smoke + proof artifacts. |
| [`autospec-doc`](../autospec-doc/README.md) | Docs/PDF sync and `NO_HANDEDIT_GENERATED` guard. |

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
