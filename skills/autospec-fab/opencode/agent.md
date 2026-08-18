---
name: autospec-fab
description: Use when the user wants to autonomously implement, validate, and release parametric 3D / CAD-as-code features whose output is 3D-printable STL — proven watertight, structurally sound, fluid/airflow-correct, and visually inspected — across any repo that opts in via `.autospec/fab.yml`.
mode: primary
---

# autospec-fab workflow

Run the full fabrication QA pipeline for a CAD-as-code project: regenerate STL
artifacts from source, then gate every printable model through geometry,
vacuum/pressure, gasket, airflow, slicer, FEA, CFD, render, and visual-inspection
stages before release.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-fab -->

## Self-update mode

If the feature-request argument matches `update` after trimming and lowercasing,
re-install the full autospec suite from `main`, show the before/after diff if the
harness exposes it, then stop. Do not run the fabrication pipeline.

## Required capabilities & harness adapter

| Capability | Claude Code | OpenCode | Codex CLI | Fallback if missing |
| --- | --- | --- | --- | --- |
| Subagent model tier | Tier A: `opus` + ultrathink | Tier A: top-tier `task` + max reasoning | Tier A: current top GPT + `reasoning_effort=high` | Run inline, but keep the same report contract |
<!-- autospec-block:harness-adapter-core -->
| Shell execution | Bash tool | shell tool | shell/apply_patch | Required for fab stage scripts |

**Model tier:** TIER_A for FEA/CFD analysis and vision advisory because geometry
defects and pressure failures are more expensive than extra review tokens.

## Harness detection

Detect the harness once at skill start:

1. Claude Code: `Agent` with `subagent_type` is available.
   - `TIER_A` = `opus` + ultrathink.
   - `TIER_B` = `sonnet`.
2. OpenCode: `task` tool is available.
   - `TIER_A` = top-tier task model + high reasoning.
   - `TIER_B` = smaller-tier task model + medium reasoning.
3. Codex CLI: `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT + `reasoning_effort=high`.
   - `TIER_B` = current cost-optimized Codex model + `reasoning_effort=medium`.

Prefer a Tier A subagent for FEA/CFD gate analysis and vision advisory pass.
If `TIER_A` is unavailable, silently fall back to the next available top-tier
model. If delegation is unavailable, run inline.

## When to use

- When a CAD-as-code project produces printable STL and needs autonomous
  fabrication QA: geometry validation, vacuum/pressure safety, slicer wall
  checks, FEA, CFD, render, and vision-advisory before release.
- When the repo opts in via `.autospec/fab.yml` and you need a repeatable,
  gated pipeline that blocks on geometry defects and surfaces visual findings
  as non-blocking advisories.
- When implementing a new parametric feature (`area:fab` / `autospec:fab-flow`
  issue) and you need the full-suite gate (clean regen → release-gate → unittest).

## When not to use

- Do not use against repos without `.autospec/fab.yml` — the skill exits with
  `code_health:fab_no_contract` rather than guessing at STL locations.
- Do not use when the CAD backend is not FreeCAD-scripted — the harness lib
  (`freecadcmd`) will not resolve and stages will error.
- Do not use to hand-edit generated artifacts (`build/`, BOM.md, PDFs, renders,
  section images, contact sheets) — the `NO_HANDEDIT_GENERATED` guard in
  `stage-docs.py` will detect and reject such edits.

## Composition map

Stage scripts and the release-gate engine live under `skills/autospec-fab/scripts/`
and install into `~/.autospec/scripts/` via `install.sh`. Real stage scripts land
in child issues #1222–#1234; this scaffold commits a `scripts/.gitkeep` placeholder.

| Component | Script / owner | Purpose |
|---|---|---|
| `stage-geometry.py` | #1222 | Reload STL + watertight + single-body connectivity (trimesh) |
| `stage-metadata.py` | #1223 | Schema-validate per-model metadata sidecar |
| `stage-vacuum-fitting.py` | #1224 | NPT access + ≥5 mm clearance + tower rejection |
| `stage-vacuum-circuit.py` | #1225 | Inlet→gasket reachability + isolation + no-relief |
| `stage-gasket.py` | #1226 | ≥5 mm wall + seal-face continuity |
| `stage-dust-airflow.py` | #1227 | Full-size openings + monotonic area + reject gate slots/PVC |
| `stage-slicer.py` | #1228 | Min wall + overhang-angle + section QA |
| `stage-fea.py` | #1229 | CalculiX structural FEA, anisotropic, geometry-hash cached |
| `stage-cfd.py` | #1230 | OpenFOAM fluid/airflow CFD, geometry-hash cached |
| `stage-render.py` | #1231 | 16-view FreeCAD headless render + contact sheet |
| `stage-vision.py` | #1232 | LLM-vision advisory pass (always warn/pass, never fail) |
| `stage-docs.py` | #1233 | Docs/PDF sync + `NO_HANDEDIT_GENERATED` guard |
| `stl-release-gate.py` | #1234 | Engine: sequence stages → `.autospec/fab/release-gate.json` |
| FreeCAD harness lib | #1221 | `freecadcmd` open/section/export/render helpers |

External solvers (`ccx`, OpenFOAM `simpleFoam`/`blockMesh`, `freecadcmd`, vision
CLI) are PATH-resolved so `$TMP/bin` mocks work in tests.

## STL Modeling Rules

### Gateable rules (deterministic — stages 1–12 above + FEA/CFD block on violation)

- **Watertight:** every exported body passes `trimesh.is_watertight`.
- **Single-body connectivity:** `mesh.split()` yields the expected body count
  (default 1); disconnected bodies are a hard reject (`code_health:disconnected_bodies`).
- **NPT access:** every NPT pilot preserves tap access, fitting-body clearance,
  entry relief, and usable thread depth. Narrow towers/ears/bosses around NPT ports
  are rejected (`code_health:blocked_npt`).
- **Vacuum fitting clearance:** every fitting/hose/socket/port has ≥5 mm radial
  free clearance unless a larger connector envelope is documented.
- **Vacuum circuit reachability:** graph reachability proves every intended inlet
  connects to each intended gasket/cavity/port; isolated circuits stay isolated
  unless declared shared-input. No relief valves, bleed ports, or restrictors on
  low-flow pump models (`code_health:disconnected_flow`).
- **Gasket wall:** every exposed gasket groove has ≥5 mm surrounding plastic
  outside the groove. Exposed gasket sides are a hard reject
  (`code_health:exposed_gasket`).
- **Dust/airflow openings:** full-size unobstructed hose/duct/socket openings;
  monotonic cross-section; reject blocked ports, disconnected ducts, printed gate
  slots, or PVC used as the printed airflow duct.
- **Slicer walls:** min wall = printer `min_perimeters × nozzle_width`; overhang
  ≤ `max_overhang_deg`; section QA at key planes.
- **FEA safety factor:** every load-critical part holds its intended load with the
  configured safety factor using material + print-orientation anisotropy from the
  metadata sidecar (`code_health:fea_below_safety`).
- **CFD targets:** every flow-critical part meets pressure-drop / velocity /
  no-stagnation targets (`code_health:cfd_target_miss`).

### Judgment rules (LLM review lens — advisory, never blocking gate)

- Prefer simple printable geometry over decorative forms.
- Keep vacuum/pressure parts conservative; optimize material only when
  fit/strength/sealing/tap-access/QA stay protected.
- Remove material from non-pressure accessories first (storage, brackets, covers,
  trays).
- After unioning caps/ribs/reinforcements/brackets, recut/reverify affected ports
  so bores/channels/gasket cavities are not refilled or clipped.

## Change workflow

1. **Regression test first:** any geometry-affecting change requires adding or
   updating a focused regression test before touching source.
2. **Smallest safe change:** make the minimal modeling/source change that fixes
   the issue without disturbing unrelated geometry.
3. **Never hand-edit generated artifacts:** `build/`, BOM.md, PDFs, renders,
   section images, and contact sheets are all generated — the `NO_HANDEDIT_GENERATED`
   guard detects and rejects hand edits.
4. **Update hand-maintained docs** (MANIFEST.md, README.md, FITTINGS_AND_SCREWS.md)
   when paths, dimensions, procedures, or rules change.
5. **Clean full catalog regen:** `rm -rf build && .venv/bin/python src/generate.py`
   (or the `generator` override in `.autospec/fab.yml`).
6. **Unit suite:** `.venv/bin/python -m unittest discover -s tests` — passes only
   when unittest reports OK. A known negative test prints a `fake.stl` metadata
   failure; ignore that line, honor the OK summary.
7. **Release gate:** run `stl-release-gate.py`; release only when every stage
   passes (FEA/CFD included) and the vision pass has no un-triaged blocking-looking
   finding.

## Release-gate / phase contract (stub — full stage prose lands in #1234)

The engine `stl-release-gate.py` sequences stages in order and aggregates
per-stage JSON fragments into `.autospec/fab/release-gate.json`. Each stage script
accepts a uniform CLI:

```
<script> --in <stl|dir> --model <metadata.json> --out <release-gate-fragment.json>
```

Exit non-zero only on harness error; gate pass/fail is expressed in the fragment
`status` field, not the exit code.

Stage order and owners:

1. geometry (#1222) — reload + watertight + single-body connectivity
2. metadata (#1223) — schema-validate sidecar
3. vacuum-fitting (#1224) — NPT access + clearance + tower rejection
4. vacuum-circuit (#1225) — inlet→gasket reachability + isolation + no-relief
5. gasket (#1226) — ≥5 mm wall + seal-face continuity
6. dust-airflow (#1227) — full-size openings + monotonic area
7. slicer (#1228) — min wall + overhang + section QA
8. fea (#1229) — CalculiX, anisotropic, geometry-hash cached
9. cfd (#1230) — OpenFOAM, geometry-hash cached
10. render (#1231) — 16-view FreeCAD headless render + contact sheet
11. vision (#1232) — LLM-vision advisory (always warn/pass, never fail)
12. docs (#1233) — docs/PDF sync + NO_HANDEDIT_GENERATED guard

Hard reject on: blocked NPT access (`blocked_npt`), exposed gasket sides
(`exposed_gasket`), disconnected flow (`disconnected_flow`), non-watertight mesh
(`non_watertight`), disconnected bodies (`disconnected_bodies`), FEA below safety
factor (`fea_below_safety`), or CFD target miss (`cfd_target_miss`).

Geometry gates **block**; the vision stage only **advises** — it never emits a
blocking status even when it observes a potential defect.

## Stop mode

If the request is exactly `stop` or `stop` plus `--<word>` flags after
normalization, dispatch to:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-stop.sh" <args>
```

Print the helper output and stop. Do not run the fabrication pipeline.

## Harness-aware handoff

Loop dispatch uses `lib/autospec-harness-detect.sh` to resolve the active AI
harness and pick the canonical `/autospec --autonomous` invocation form:

- Claude Code → `claude "/autospec" "--autonomous" "$PROMPT"`.
- Codex CLI → `codex exec --skip-git-repo-check "/autospec --autonomous $PROMPT"`.
- OpenCode → `opencode "/autospec" "--autonomous" "$PROMPT"` (best-effort).

Detection order: `AUTOSPEC_HANDOFF_DISPATCHER_KIND` env override → skill-mount
probe → PATH probe. Missing dispatcher exits 3 with
`code_health:loop_handoff_no_dispatcher_for_harness`.
