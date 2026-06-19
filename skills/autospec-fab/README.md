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

## Fab model directory & output layout

Two conventions keep the producers (the release-gate engine + its stages) and
the consumer (the Phase 5.5 `fab-completeness.sh` audit) aligned. They live in
`scripts/release_gate_stages.py` (`_SIBLING_INPUTS`, `extra_args_for`) and
`skills/autospec-run/scripts/fab-completeness.sh`.

### Per-model input layout

`MODELDIR` is the directory holding a model's `--model` metadata file. It always
contains `metadata.json` (validated against `autospec-fab-model.schema.json` by
the `metadata` stage). Alongside it the engine looks for OPTIONAL stage sidecars
and threads each one into its stage automatically **only when the file exists**.
An absent sidecar means the engine adds no extra flag, so that stage skips
cleanly — correct for a model that lacks the feature.

| Sidecar (`MODELDIR/…`) | Stage | Engine flag |
| --- | --- | --- |
| `metadata.json` | `metadata` | `--model` (required) |
| `circuit.json` | `vacuum-circuit` | `--circuit` |
| `duct.json` | `dust-airflow` | `--duct` |
| `printer.json` | `slicer` | `--printer` |
| `load.json` | `fea` | `--load` |
| `flow.json` | `cfd` | `--flow` |
| `baseline.json` | `docs` | `--baseline` (the `docs` stage gets the model DIR as `--in`) |

```text
manifolds/inlet_manifold/
├── metadata.json     # required; validated vs the model schema
├── circuit.json      # optional → runs vacuum-circuit
├── duct.json         # optional → runs dust-airflow
├── printer.json      # optional → per-model slicer override
├── load.json         # optional → runs FEA
├── flow.json         # optional → runs CFD
└── baseline.json     # optional → docs NO_HANDEDIT baseline
```

### Output layout

The engine writes the aggregated `release-gate.json` to its `--out` path. It also
threads a deterministic render dir (`<out-dir>/renders/`) into the `render` stage
via `--render-dir`; render writes its contact sheet to
`<render-dir>/<slug>/contact-sheet.html` (`slug` = STL stem), which the engine
then hands to the `vision` stage as its `--in` (the render→vision handoff).

The Phase 5.5 `fab-completeness.sh` audit consumes one canonical per-model
layout under the fab dir (`.autospec/fab`). Keep the STL stem equal to the model
name so the `<model>` segments stay aligned:

```text
.autospec/fab/
├── gates/
│   └── <model>/
│       └── release-gate.json     # green + geometry_hash matches current STL
└── renders/
    └── <model>/
        └── contact-sheet.html    # produced by render, read by vision
```

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
