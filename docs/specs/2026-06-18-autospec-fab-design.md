# autospec-fab — generative 3D-model fabrication QA skill (design)

Status: proposed
Owner: berlinguyinca
Slug: autospec-fab

## Goal

Add a generic `autospec-fab` skill to the autospec family that lets the autospec
pipeline (define → run → review → Phase 5.5) autonomously implement **parametric
3D / CAD-as-code** features whose output is **3D-printable STL** that is proven
watertight, structurally sound under its intended loads, fluid/airflow-correct,
and **automatically visually inspected** before release. The skill is repo-generic:
it drives any CAD-as-code project through a `.autospec/fab.yml` contract.

## Locked decisions

1. **Scope:** generic `autospec-fab` skill in the autospec family (not a one-repo
   fork). Target repos opt in via `.autospec/fab.yml`.
2. **CAD backend:** **FreeCAD** scripting (headless `freecadcmd`/`FreeCAD` Python
   API) for geometry ops, sections, and renders. STL is exported from FreeCAD and
   mesh-level checks use `trimesh`.
3. **Physical-test fidelity:** **full FEA + CFD from the start** — CalculiX (`ccx`)
   for structural/load FEA and OpenFOAM for fluid/airflow CFD, on every part the
   contract flags load- or flow-critical. Results are cached by geometry hash so
   unchanged parts skip re-analysis (gate-runtime control).
4. **Visual-inspection authority:** deterministic geometry checks are the
   **blocking** gate; the LLM-vision pass over the 16-view contact sheet runs and
   surfaces findings as **non-blocking** warnings/follow-ups ("geometry gates,
   vision advises").

## Architecture (mirrors autospec-harmonize / autospec-upgrade)

### Lock-step trio skill `skills/autospec-fab/`
`SKILL.md` (authoritative) + byte-identical `codex/prompt.md` + `opencode/agent.md`
(via `derive-trio.sh --in-place`) + regenerated skill goldens; ships
`install.sh`/`uninstall.sh`/`README.md`; `autospec-block` startup-self-update +
harness-adapter. Encodes the STL Modeling Rules (below), the change workflow, and
the release-gate contract. Validated by `check_autospec_fab_contract` in
`scripts/validate.sh`, which also runs the fab bats/test suite + a dogfood model.

### `.autospec/fab.yml` contract (per target repo)
Declares: generator entrypoint (default `rm -rf build && .venv/bin/python
src/generate.py`), STL output roots (`build/stls/manifolds/`, catalog
`manifolds/...`), per-model **metadata sidecars** (dims, material + print
orientation, ports, gaskets, load/flow-critical flags), printer profile (nozzle,
layer height, min perimeters, max overhang), and which models are printable.

### Release-gate engine `stl-release-gate.py`
Sequences ordered, individually-tested stages; emits a schema-validated
`.autospec/fab/release-gate.json` (single source of truth). Stages, in order:

1. **geometry reload** — re-open every STL; parse OK.
2. **watertight** — `trimesh.is_watertight` per body.
3. **single-body connectivity** — `mesh.split()` yields the expected body count
   (default 1); reject disconnected bodies.
4. **metadata** — sidecar validates against `autospec-fab-model.schema.json`.
5. **vacuum fitting QA** — every NPT pilot preserves tap access, fitting-body
   clearance, entry relief, usable thread depth; every fitting/hose/socket/port has
   ≥5 mm radial free clearance unless a larger connector envelope is documented;
   NPT ports integrated into the body/reinforced block (reject narrow towers/ears/
   bosses).
6. **vacuum circuit QA** — graph reachability proves every intended inlet connects
   to each intended gasket/cavity/port; isolated circuits stay isolated unless
   declared shared-input self-clamping (full-size internal branches); reject relief
   valves / bleed ports / intentional leaks / restrictors on low-flow pump models.
7. **gasket leak sim** — every exposed gasket groove has ≥5 mm surrounding plastic
   outside the groove; seal-face continuity / pressure-boundary check; reject
   exposed gasket sides.
8. **dust leak/airflow QA** — full-size unobstructed hose/duct/socket openings;
   monotonic cross-section; reject blocked ports, disconnected ducts, printed gate
   slots, or PVC used as the printed airflow duct.
9. **slicer wall checks** — min wall = printer min-perimeters × nozzle width;
   overhang-angle limit; section QA at key planes.
10. **structural FEA** (CalculiX) — every load-critical part holds its intended
    load with the configured safety factor, using material + **print-orientation
    anisotropy** from the metadata. Cached by geometry hash.
11. **fluid/airflow CFD** (OpenFOAM) — every flow-critical part meets pressure-drop
    / velocity / no-stagnation targets; dust-hood outlet transitions analyzed.
    Cached by geometry hash.
12. **render QA + 16-view contact sheet** — headless FreeCAD render of the 16+
    required angles (right/left/front/back/top/bottom, the eight 45° diagonals, plus
    extra diagonals to expose ports/sockets/gaskets/lugs/overhangs/transitions) and
    slicer-like rotated/top-side views; assemble a contact sheet.
13. **visual inspection (vision-advises)** — LLM-vision pass judges the contact
    sheet against the modeling rules (exposed gasket sides, blocked NPT,
    disconnected bodies, overhangs); adversarially verified; emits **non-blocking**
    findings.
14. **docs + PDFs** — MANIFEST.md / README.md / FITTINGS_AND_SCREWS.md sync;
    BOM/PDF/render regen consistent; `NO_HANDEDIT_GENERATED` guard forbids hand
    edits to `build/`, `BOM.md`, PDFs, renders, section images, contact sheets.

Hard reject (gate fail) on: blocked NPT access, exposed gasket sides, disconnected
flow, non-watertight mesh, disconnected bodies, known leakage, FEA below safety
factor, or CFD target miss.

## STL Modeling Rules — split

- **Gateable (deterministic, stages 1-12 above + FEA/CFD):** all clearance,
  watertightness, connectivity, NPT-access, full-size-duct, gasket-wall, isolation,
  wall-thickness, load, and airflow rules.
- **Judgment (LLM review lens + guardian RULE_IDs):** prefer simple printable
  geometry over decorative forms; keep vacuum/pressure parts conservative, optimize
  material only when fit/strength/sealing/tap-access/QA stay protected; remove
  material from non-pressure accessories first (storage, brackets, covers, trays);
  after unioning caps/ribs/reinforcements/brackets, recut/reverify affected ports so
  bores/channels/gasket cavities are not refilled or clipped.

## Change workflow (Phase contract — encodes "What To Do For Changes")

1. Geometry-affecting change → add/update a focused regression test first (TDD).
2. Smallest safe modeling/source change.
3. Never hand-edit generated artifacts (`build/`, BOM.md, PDFs, renders, sections,
   contact sheets).
4. Update hand-maintained docs (MANIFEST.md, README.md, FITTINGS_AND_SCREWS.md) when
   paths/dimensions/procedures/rules change.
5. Clean full catalog regen: `rm -rf build && .venv/bin/python src/generate.py`.
6. Unit suite: `.venv/bin/python -m unittest discover -s tests` — counts as passing
   only when unittest reports OK (a known negative test prints a `fake.stl` metadata
   failure; ignore that line, honor the OK summary).
7. Run `stl-release-gate.py`; release only when every stage passes (FEA/CFD included)
   and the vision pass has no un-triaged blocking-looking finding.

## autospec-run / define / Phase 5.5 wiring

- **Phase 4 implementer:** `area:fab` / `autospec:fab-flow` issues route to a fab
  implementer whose **full-suite gate** = clean regen → `stl-release-gate.py` on
  affected models → unittest; **Primary smoke** = the model's focused regression
  test. Team personality = DfAM + test-safety; Review counter-team = vacuum/pressure/
  dust QA.
- **Phase 5.5:** add a **fab-completeness** dimension — every printable model has the
  16-view contact sheet + a green release-gate.json; surviving gaps file as
  `gap-remediation`.
- **autospec-define decomposition:** CAD features decomposed small + regression-test-
  first, modeling rules as hard constraints, metadata/ports pre-staged.

## Schemas

- `schemas/autospec-fab-model.schema.json` — per-model metadata sidecar.
- `schemas/autospec-fab-release-gate.schema.json` — release-gate.json (per-stage
  status, FEA/CFD results, vision findings, freshness).
- `.autospec/fab.yml` documented shape (generator, roots, printer profile, models).

## Decomposition (small, regression-test-first issues)

1. fab trio skill scaffold + install/uninstall/README + `check_autospec_fab_contract`
   validate gate (mirror autospec-upgrade #1172).
2. `autospec-fab-model` + `autospec-fab-release-gate` JSON schemas + bats.
3. `.autospec/fab.yml` loader + contract doc.
4. FreeCAD headless harness lib (open/section/export/render) + fixture model.
5. geometry stages: reload + watertight + single-body connectivity (trimesh).
6. metadata stage (schema-validate sidecars).
7. vacuum fitting QA (NPT access + 5 mm clearances + tower rejection).
8. vacuum circuit QA (inlet→gasket reachability + isolation + no-relief).
9. gasket leak sim (≥5 mm wall + seal continuity).
10. dust leak/airflow QA (full-size openings + monotonic area + reject gate slots/PVC).
11. slicer wall + overhang + section QA.
12. structural FEA via CalculiX (anisotropic, geometry-hash cached) + fixture part.
13. fluid/airflow CFD via OpenFOAM (geometry-hash cached) + fixture part.
14. 16-view render harness + contact sheet (FreeCAD headless).
15. LLM-vision advisory pass + adversarial verify (non-blocking findings).
16. docs/PDF stage + `NO_HANDEDIT_GENERATED` guard.
17. `stl-release-gate.py` engine wiring all stages → release-gate.json.
18. autospec-run Phase 4 fab routing + full-suite gate.
19. Phase 5.5 fab-completeness dimension.
20. autospec-define fab decomposition lens + dogfood model end-to-end.
21. Phase 5.5 audit + remediation — autospec-fab.

## Risks / honest constraints

- **Heavy toolchain:** FreeCAD + CalculiX + OpenFOAM make the gate slow and infra-
  heavy. Mitigations: containerized toolchain image; geometry-hash result caching so
  only changed parts re-run FEA/CFD; per-stage timeouts; FEA/CFD skipped for parts
  not flagged load/flow-critical.
- **FDM anisotropy:** print orientation dominates real strength. FEA must consume
  per-axis material properties + orientation from metadata; safety-critical pressure
  vessels still warrant a physical proof print — the gate gates, it does not certify.
- **Determinism:** renders + CFD must be seed/version-pinned for reproducible QA;
  pin tool versions in the container.
- **Vision authority:** vision is advisory only (per decision 4); a real defect that
  is purely visual and not encoded in a deterministic check will warn, not block —
  so new visual failure modes should be promoted into deterministic stages over time.
