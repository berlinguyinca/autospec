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

## Required system packages

The four solver stages resolve their binaries off `PATH` via `shutil.which`.
The default and every-PR path uses `$TMP/bin` shims — real solvers are **not**
required for unit tests or the mocked validation path. Real solvers are only
needed when running against an actual physical model.

| Stage | Script | PATH binary | APT package | Notes |
| --- | --- | --- | --- | --- |
| `fea` | `stage_fea.py` | `ccx` | `calculix-ccx` | CalculiX FEA solver |
| `cfd` | `stage_cfd.py` | `simpleFoam` | `openfoam2406` | OpenFOAM ESI edition; full solver set includes `simpleFoam` |
| `render` / geometry / section | `freecad_harness.py` | `freecadcmd` | `freecad-python3` | FreeCAD headless; requires mesa + xvfb for offscreen render |
| `render` (headless display) | `freecad_harness.py` | `Xvfb` | `xvfb` + `libgl1-mesa-dri` | Virtual framebuffer for FreeCAD GUI render path |
| `vision` (advisory) | `stage_vision.py` | `fab-vision` | — (see below) | Resolved via `$AUTOSPEC_FAB_VISION_CMD` or `PATH`; absent → stage skips cleanly |

**Bare-metal install hint (Ubuntu 24.04 / Debian):**

```bash
# CalculiX
sudo apt-get install -y calculix-ccx

# FreeCAD headless + offscreen render dependencies
sudo apt-get install -y freecad-python3 xvfb libgl1-mesa-dri

# OpenFOAM ESI edition (adds the apt repo first)
curl -s https://dl.openfoam.com/add-apt-repo.sh | sudo bash
sudo apt-get install -y openfoam2406-default

# fab-vision consumer CLI (from #1289)
pip install scripts/fab-vision-cli.py   # or point $AUTOSPEC_FAB_VISION_CMD
```

> **Note:** These packages are deferred / optional. The `$TMP/bin`-shim test
> path is the default; every unit suite and `validate.sh` run stays fully
> mocked. Install real solvers only when you want to run live physical
> simulations. The pinned container (forthcoming via #1300) ships all four
> binaries on `PATH` without any manual install.

## Container usage

> **Status:** The container image (`skills/autospec-fab/docker/Dockerfile`) is
> **forthcoming** — tracked in **#1300** (cost-gated, requires operator sign-off
> per §4 of the design spec). This section documents the intended workflow so
> operators can prepare; the bare-metal path described above works today.

Once #1300 ships the `Dockerfile`, the build + run workflow will be:

```bash
# Build (one-time; ~10–20 min on cold cache — OpenFOAM dominates)
docker build -f skills/autospec-fab/docker/Dockerfile -t autospec-fab .
```

```bash
# Run the release gate on a model directory
docker run --rm \
  -v "$PWD/model:/work/model" \
  -v "$PWD/stls:/work/stls" \
  autospec-fab \
  python3 /opt/autospec-fab/scripts/release_gate_stages.py \
    --in    /work/stls/part.stl \
    --model /work/model/metadata.json \
    --out   /work/release-gate.json
```

### Sibling-file convention inside the container

The container honors the same `MODELDIR` sibling-file convention as the bare-
metal path (see [Per-model input layout](#per-model-input-layout) above).
Map your model directory to `/work/model` and place the optional sidecar files
alongside `metadata.json`:

```text
$PWD/model/          ← mounted as /work/model inside the container
├── metadata.json    # required
├── circuit.json     # optional → runs vacuum-circuit
├── duct.json        # optional → runs dust-airflow
├── printer.json     # optional → per-model slicer override
├── load.json        # optional → runs FEA (ccx)
└── flow.json        # optional → runs CFD (simpleFoam)
```

**Present sidecar → real solver runs; absent sidecar → stage skips cleanly.**
No configuration change is needed — the engine's `extra_args_for` logic handles
the presence / absence of each file automatically.

### Vision stage environment variables

The vision stage (`stage_vision.py`) is **advisory and never blocks a release**.
It invokes an external CLI (`$AUTOSPEC_FAB_VISION_CMD` or `fab-vision` on PATH)
for a JUDGE pass followed by an adversarial `--verify` pass. The bundled
consumer is `scripts/fab-vision-cli.py`.

All five env vars are read by `fab-vision-cli.py` itself (not by the stage):

| Variable | Purpose | Default |
| --- | --- | --- |
| `AUTOSPEC_FAB_VISION_CMD` | Path or name of the vision CLI (consumed by the **stage**) | `fab-vision` (PATH lookup); unset → stage skips |
| `AUTOSPEC_FAB_VISION_BACKEND` | Force backend: `api`, `claude-cli`, or `none` | auto-detect (first usable wins) |
| `ANTHROPIC_API_KEY` | Credential for the `api` backend (Anthropic Claude vision) | unset → `api` backend unavailable |
| `AUTOSPEC_FAB_VISION_MODEL` | Model id for the `api` backend | `claude-opus-4-8` |
| `AUTOSPEC_FAB_VISION_MAX_IMAGES` | Cap on PNG image blocks sent per judge call | `16` |

#### Concrete runnable example

Point `$AUTOSPEC_FAB_VISION_CMD` at the bundled CLI, export your API key, then
run any fab gate command:

```bash
export AUTOSPEC_FAB_VISION_CMD=skills/autospec-fab/scripts/fab-vision-cli.py
export ANTHROPIC_API_KEY=sk-ant-...

# Run the full release gate (vision stage fires automatically)
python3 skills/autospec-fab/scripts/stl-release-gate.py \
  --in  build/stls/manifolds/inlet_manifold.stl \
  --model manifolds/inlet_manifold/metadata.json \
  --out  .autospec/fab/gates/inlet_manifold/release-gate.json
```

Vision results appear in the `vision_findings` key of the output JSON.
Findings are advisory (`warn`-severity at most); they never cause a hard reject.

#### One-off manual invocation

Run `fab-vision-cli.py` directly against any contact sheet (e.g. to inspect a
single render without running the full gate):

```bash
# Judge pass — returns {"observations": [...]}
python3 skills/autospec-fab/scripts/fab-vision-cli.py \
  .autospec/fab/renders/inlet_manifold/contact-sheet.html \
  [path/to/stl-modeling-rules.txt]   # optional rules file

# Verify pass — pipe one observation JSON on stdin
echo '{"observation": "...", "severity": "warn"}' | \
  python3 skills/autospec-fab/scripts/fab-vision-cli.py --verify \
    .autospec/fab/renders/inlet_manifold/contact-sheet.html
```

Both passes always exit 0. The verdict lives in the JSON output, never in the
exit code. Exit 2 is reserved for usage errors (bad flags / missing `<sheet>`).

#### Graceful-skip behavior

`fab-vision-cli.py` **never crashes and never fabricates a finding**. Every
failure mode degrades to an honest empty result:

| Condition | Stage outcome | CLI output |
| --- | --- | --- |
| `$AUTOSPEC_FAB_VISION_CMD` unset, `fab-vision` not on PATH | `status: skip`, empty `vision_findings` | *(CLI not invoked)* |
| CLI present, `ANTHROPIC_API_KEY` unset, no SDK | `status: pass`, empty `vision_findings` | `{"observations": []}` / `{"confirmed": false}` |
| `AUTOSPEC_FAB_VISION_BACKEND=none` (explicit opt-out) | `status: pass`, empty `vision_findings` | `{"observations": []}` / `{"confirmed": false}` |
| Sheet present but zero resolved PNGs on disk | `status: pass`, empty `vision_findings` | `{"observations": []}` |
| API call fails / timeout / malformed response | `status: pass`, empty `vision_findings` | `{"observations": []}` |
| Confirmed `warn` finding present | `status: warn` advisory (non-blocking) | `{"observations": [{...}]}` |

Vision never emits `status: fail`. Operators can inspect `vision_findings` in
the release-gate JSON and decide whether to act on advisory warnings.

Inside the container (#1300), `fab-vision` will be present on `PATH` so vision
runs automatically without setting `$AUTOSPEC_FAB_VISION_CMD`.

#### Opt-in real-API smoke (manual only — never runs in CI)

To verify the full Anthropic vision path end-to-end on a real contact sheet:

```bash
# Prerequisites: ANTHROPIC_API_KEY set, anthropic SDK installed, a real sheet
pip install anthropic

export ANTHROPIC_API_KEY=sk-ant-...
export AUTOSPEC_FAB_VISION_CMD=skills/autospec-fab/scripts/fab-vision-cli.py
# Optional: override model (default claude-opus-4-8)
# export AUTOSPEC_FAB_VISION_MODEL=claude-opus-4-8

python3 skills/autospec-fab/scripts/fab-vision-cli.py \
  .autospec/fab/renders/inlet_manifold/contact-sheet.html
```

This makes a real Anthropic API call and incurs token cost. It is **not run by
`validate.sh`** or any automated test. All automated paths use the
`$AUTOSPEC_FAB_VISION_STUB` test seam so no network call is required.

## License

MIT - see the repository [LICENSE](../../LICENSE) file.
