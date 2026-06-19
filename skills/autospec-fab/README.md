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

## .autospec/fab.yml

Target repositories opt in to `autospec-fab` by placing a `.autospec/fab.yml`
file at their root. The loader `skills/autospec-fab/scripts/load-fab-config.sh`
parses this file and emits normalized JSON with defaults applied.

### Shape

```yaml
# Entrypoint command that regenerates all STL artifacts from source.
# REQUIRED for --validate.
generator: "rm -rf build && .venv/bin/python src/generate.py"

# Directories to scan for output STL files.
# Default: ["build/stls/manifolds/"]
stl_roots:
  - "build/stls/manifolds/"
  - "catalog/manifolds/"

# FDM printer profile used by the slicer stage.
# All fields default if the printer key is absent.
printer:
  nozzle_width: 0.4      # default: 0.4 mm
  layer_height: 0.2      # default: 0.2 mm
  min_perimeters: 3      # default: 3
  max_overhang_deg: 45   # default: 45 degrees

# Path template to per-model metadata sidecar JSON files.
# Optional; null if absent.
metadata_sidecar: "manifolds/{name}/metadata.json"

# List of models to gate. REQUIRED (non-empty) for --validate.
models:
  - name: "inlet_manifold"
    stl: "build/stls/manifolds/inlet_manifold.stl"
    metadata: "manifolds/inlet_manifold/metadata.json"
    printable: true
    load_critical: true   # gates FEA stage
    flow_critical: false  # gates CFD stage
```

### Required keys (--validate mode)

| Key | Requirement |
| --- | --- |
| `generator` | Must be present and non-empty |
| `models` | Must be a non-empty list |

### Defaults applied at load time

| Key | Default value |
| --- | --- |
| `generator` | `rm -rf build && .venv/bin/python src/generate.py` |
| `stl_roots` | `["build/stls/manifolds/"]` |
| `printer.nozzle_width` | `0.4` |
| `printer.layer_height` | `0.2` |
| `printer.min_perimeters` | `3` |
| `printer.max_overhang_deg` | `45` |

### Loader CLI

```bash
# Emit normalized JSON (defaults applied):
skills/autospec-fab/scripts/load-fab-config.sh .autospec/fab.yml

# Strict validation (exits non-zero on missing required keys):
skills/autospec-fab/scripts/load-fab-config.sh --validate .autospec/fab.yml
```

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
