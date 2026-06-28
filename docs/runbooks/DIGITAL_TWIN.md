# Autospec Digital Twin

The Digital Twin is version-controlled repository intelligence under
`.autospec/state`. It summarizes what the repository appears to contain, how
capabilities connect to files/docs/tests, and what evidence supports each
inference.

Digital Twin v1 is heuristic and evidence-scored. It is useful for worker,
verifier, and supervisor decisions, but it is not a perfect semantic model.

## Build

```bash
bash scripts/autospec-build-digital-twin.sh
```

The build is local and read-only. It does not require network, GitHub API,
GitHub Actions, cron, or a background scheduler.

## Generated Metadata

- `.autospec/state/repository-inventory.json`
- `.autospec/state/technology-registry.yml`
- `.autospec/state/capability-registry.json`
- `.autospec/state/api-surface.json`
- `.autospec/state/ui-surface.json`
- `.autospec/state/data-surface.json`
- `.autospec/state/settings-registry.json`
- `.autospec/state/permission-model.json`
- `.autospec/state/ai-capabilities.json`
- `.autospec/state/mcp-registry.json`
- `.autospec/state/domain-model.json`
- `.autospec/state/workflow-map.json`
- `.autospec/state/knowledge-graph.json`
- `.autospec/state/digital-twin.json`

Human-readable reports are written under `.autospec/reports`.

## Reading Reports

Start with `.autospec/reports/digital-twin.md` for the state of the repo. Use
`.autospec/reports/knowledge-graph.md` to inspect connected capabilities,
orphan docs/tests, and isolated files. Use `.autospec/reports/technology-registry.md`
to spot library sprawl.

## Impact Analysis

```bash
bash scripts/autospec-impact-analysis.sh --dry-run --file src/example.ts
bash scripts/autospec-impact-analysis.sh --dry-run --capability create-project
```

Impact analysis reports related files, capabilities, tests, docs, workflows,
validation hints, risk hints, and missing evidence.

## Metadata Drift

```bash
bash scripts/autospec-metadata-drift.sh
```

Drift detection reports stale references, missing required metadata, orphan docs,
orphan tests, and capability references to missing implementation files. It does
not modify files.

## Confidence And Evidence

Every inferred fact should carry evidence and confidence. Low-confidence findings
are allowed, but reports should make that uncertainty visible.

## Future Use

Future Constitution rule interpretation should read the Digital Twin instead of
hand-coded repository heuristics. Worker and verifier flows can use impact
analysis to choose validation commands and detect risk before implementation.
