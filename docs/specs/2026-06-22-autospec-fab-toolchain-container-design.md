# autospec-fab: Pinned Container for the Real FreeCAD / CalculiX / OpenFOAM / Vision Toolchain — Design

**Status:** Draft design (2026-06-22)
**Author:** berlinguyinca + brainstorm
**Tracking issue:** #1270 (`area:fab`) — surfaced by the 2026-06-20 whole-project review (ship-completeness / ROI dimension)
**Related:** #1271 (real vision CLI consumer — vision stage currently always skips)
**Classification:** INFRASTRUCTURE with real, ongoing cost (image size + CI minutes + registry storage). NOT a free-to-implement skill change. The heaviest child must be explicitly cost-flagged and gated on operator sign-off (§4).

---

## 1. Problem

`autospec-fab` gates printable STL through twelve stages. Four shell out to **real external engineering solvers**, resolved off `PATH`:

| Stage | Script | Binary (PATH name) | Invocation contract |
| --- | --- | --- | --- |
| `fea` | `scripts/stage_fea.py` | `ccx` (CalculiX) | `shutil.which("ccx")`; runs `ccx <job_base>`; reads `<job_base>.dat`, parses `SAFETY_FACTOR <value>` |
| `cfd` | `scripts/stage_cfd.py` | `simpleFoam` (OpenFOAM) | `shutil.which("simpleFoam")`; runs `simpleFoam` in the case dir; reads `cfd_results` (`PRESSURE_DROP`/`MIN_VELOCITY`/`STAGNATION`) |
| `render` (+ geometry/section) | `scripts/freecad_harness.py` | `freecadcmd` (FreeCAD headless) | `shutil.which("freecadcmd")`; runs `freecadcmd <script.py>`; render drives `FreeCADGui` for offscreen PNG |
| `vision` (advisory) | `scripts/stage_vision.py` | `fab-vision` / `$AUTOSPEC_FAB_VISION_CMD` | judge `fab-vision <sheet> <rules>` → `{"observations":[…]}`; `--verify` on stdin → `{"confirmed":bool}` |

Today every one resolves to **nothing** on a normal host; each stage's absent-binary branch is deliberate and graceful (`ccx`/`simpleFoam` → "real solver deferred to container", `status: skip`; vision → `skip`; freecad → sentinel/`FileNotFoundError`). The unit suites prove wiring via `$TMP/bin` shims and are green; that mocked path must stay the default.

What's missing — what #1270 asks for — is a **reproducible, pinned environment supplying the real binaries** plus **one end-to-end smoke** proving the real toolchain integrates with the existing stage CLIs. There is no `Dockerfile` in the repo (CI is `python.yml`, `pages.yml`, `release-cli.yml` only) and the README has zero solver install instructions.

## 2. Goal

Ship a **pinned, reproducible container** carrying the real toolchain whose binaries land on `PATH` under the exact resolver names (`freecadcmd`, `ccx`, `simpleFoam`, `fab-vision`), plus:

1. A **real-solver integration smoke** driving one tiny part through `fea`+`cfd`+`render`+`vision` end-to-end inside the image, asserting a non-`skip`, structurally-valid release-gate fragment from each.
2. **Documented system packages** in `skills/autospec-fab/README.md` (container path + bare-metal recipe).
3. A clear operator workflow (`docker run` mapping a model dir, honoring `MODELDIR/{circuit,duct,load,flow,…}.json`).

### Non-goals
Changing any stage's resolution logic / result-file contract / skip semantics (image satisfies the **existing** `shutil.which` contract; no Python stage changes); running real solvers on every PR (§3); a real vision model beyond a thin contract-conformant CLI (the model decision is #1271; this ships a minimal compatible CLI so the smoke exercises a non-skip vision pass); GPU / MPI OpenFOAM / solver tuning; publishing the image to a public registry as a supported artifact (open decision §4).

## 3. Cost analysis (read before approving)

Conservative `linux/amd64` estimates.

### 3.1 Image size

| Layer | Source | Est. on-disk |
| --- | --- | --- |
| `ubuntu:24.04` base | apt | ~80 MB |
| FreeCAD headless (+ xvfb + mesa/llvmpipe) | apt | ~1.3–1.8 GB |
| OpenFOAM (`openfoam2406`/`openfoam12`, full solver set) | OpenFOAM apt repo | ~2.0–4.0 GB |
| CalculiX `ccx` | apt | ~30–80 MB |
| `fab-vision` CLI (python wrapper + deps) | pip | ~150–400 MB |
| Python 3.12 + fab scripts + test deps | apt/pip | ~150 MB |
| **Total (single-stage naive)** | | **~4–6.5 GB** |
| **Total (multi-stage, stripped)** | | **~3–4.5 GB** |

**This image is multi-GB and there is no way around it** — OpenFOAM dominates. Multi-stage + prune (`*-dev`, OpenFOAM tutorials/doc, apt lists) saves ~1–2 GB but can't get under ~3 GB.

### 3.2 Build time
Clean build (cold cache, both heavy repos): **~10–20 min** on a GitHub runner; ~5–10 min locally warm. The OpenFOAM apt repo is the long pole — pin a point release to keep it cacheable. Layer ordering: solvers early (cache-stable), `COPY skills/autospec-fab` last (iterating the smoke doesn't rebuild solver layers).

### 3.3 CI minutes / registry storage
Building on **every PR** would add ~15 min × per-PR fab churn — unacceptable. Registry storage for ~3–4.5 GB is cheap in $ but every consumer pays the **pull** (~3–4.5 GB egress + disk) on first use. The real-solver smoke (real FreeCAD + meshing + simpleFoam iterations) is minutes of CPU even on a tiny part.

### 3.4 Recommendation (proposed; operator confirms in §4)
1. **Default stays mocked** — the `$TMP/bin`-shim unit suites remain the every-PR gate via `python.yml`/validate.sh. The default path doesn't change or slow.
2. **Container build + real smoke is OPT-IN, dedicated workflow** — `workflow_dispatch` + nightly `schedule`, `paths: skills/autospec-fab/**`. **Never** on `pull_request`.
3. **Build fresh in the nightly job, run the smoke, do NOT push by default** — pushing to a registry is a separate explicitly-enabled step behind §4.
4. **Pin everything** — base by digest, OpenFOAM/FreeCAD/ccx by exact apt version, pip deps with hashes.

## 4. Open decisions — needs operator sign-off

Resolve before the cost-incurring child (C/E) runs.

1. **Run the real-solver smoke in CI at all?** Proposed: yes, **nightly + manual-dispatch only**, never per-PR. (Confirm cadence.)
2. **Publish the image?** (a) build-and-discard, never pushed [proposed default]; (b) push to GHCR (`ghcr.io/berlinguyinca/autospec-fab`) tagged date+SHA; (c) Docker Hub. (If pushed: public/private? retention/prune for old multi-GB tags?)
3. **Accept a multi-GB image (~3–4.5 GB)?** No small variant exists.
4. **OpenFOAM flavor + version pin:** ESI (`openfoam2406`) vs Foundation (`openfoam12`). Both ship `simpleFoam`. Proposed: ESI `openfoam2406`.
5. **`fab-vision` real consumer scope:** ship a minimal contract-conformant CLI here (enough for non-skip smoke) vs block on #1271. Proposed: ship minimal here; #1271 upgrades the model behind the same contract.
6. **Architecture matrix:** `linux/amd64` only, or also `linux/arm64`? arm64 ~doubles build time + OpenFOAM arm64 apt is shakier. Proposed: amd64 only for v1.

## 5. Architecture

### 5.1 New files
```
skills/autospec-fab/docker/
  Dockerfile                  # multi-stage, pinned
  fab-vision                  # minimal contract-conformant vision CLI (judge + --verify)
  requirements.txt            # pip deps, hash-pinned
  smoke/
    run_smoke.sh              # drives one tiny part through fea+cfd+render+vision
    part.stl                  # tiny fixture part
    model/{metadata.json,load.json,flow.json}   # MODELDIR sibling layout
.github/workflows/fab-container.yml             # opt-in: workflow_dispatch + nightly, NOT pull_request
```

### 5.2 Dockerfile strategy (multi-stage, pinned)
Minimize final size; land each binary on `PATH` under the resolver's exact name; keep heavy solver layers above the `COPY` of fab scripts. Stages: `base` (ubuntu pinned by digest + python/xvfb/curl), `solvers` (apt `calculix-ccx`, `freecad-python3`+mesa, OpenFOAM ESI repo `openfoam2406-default`), `runtime` (COPY only needed binaries/libs from `solvers`, set OpenFOAM `PATH`/`WM_PROJECT_DIR`/`FOAM_LIBBIN` env so non-login shells resolve `simpleFoam`, prune `doc`/`tutorials`, COPY fab scripts + `fab-vision` last).

**PATH-resolution mapping (the contract the image must satisfy):** `which("ccx")` → `/usr/bin/ccx`; `which("simpleFoam")` → OpenFOAM `platforms/.../bin` on `ENV PATH`; `which("freecadcmd")` → `/usr/bin/freecadcmd`; `which("fab-vision")`/`$AUTOSPEC_FAB_VISION_CMD` → `/usr/local/bin/fab-vision`.

**Result-file conformance (the crux — beyond `apt install`):**
- `ccx` writes `<job>.dat`; the stage parses a `SAFETY_FACTOR <value>` line. Real ccx emits stress/displacement, not a safety factor — so the container ships a thin **post-process wrapper** deriving `SAFETY_FACTOR` from the `.dat` MAX_MISES vs material yield. This is the one place the real solver meets the stage's parse contract; spec + test it explicitly.
- `simpleFoam` workflow writes `cfd_results` (`PRESSURE_DROP`/`MIN_VELOCITY`/`STAGNATION`). Real OpenFOAM emits field data under time dirs; the case build + a post step must reduce fields into the `cfd_results` file the stage reads. **This reduction is the real integration work** and lives in the container's case/post pipeline.

> **Load-bearing note:** the gap between real solver output and the `.dat`/`cfd_results` line format the stages already parse is the crux. Keep stages unchanged; ship post-process wrappers in `docker/` (bundled into child C).

### 5.3 Pinned versions (pin all at build)
`ubuntu:24.04` by `@sha256`; `calculix-ccx` exact apt version; `freecad-python3` exact; `openfoam2406-default` pinned; `requirements.txt` `--require-hashes`.

## 6. Real-solver integration smoke + where it runs

### 6.1 `docker/smoke/run_smoke.sh` (inside the image, one tiny fixture part with `metadata.json`+`load.json`+`flow.json`)
Run the gate / each stage, then assert: **FEA** non-`skip` with structured `safety_factor` (real `ccx` + `.dat` parse); **CFD** non-`skip` with parsed `PRESSURE_DROP`/`MIN_VELOCITY` (real `simpleFoam` + `cfd_results`); **render** real PNG + `contact-sheet.html` (headless `freecadcmd` + xvfb); **vision** non-`skip` with a structurally-valid (possibly empty) `vision_findings` (real `fab-vision` judge+verify). Exit non-zero on any `skip`-where-real-expected or malformed fragment. This is a **wiring/integration** assertion, not numerical-accuracy.

### 6.2 Where (`.github/workflows/fab-container.yml`)
`on: workflow_dispatch` + `schedule: cron "0 6 * * *"` (nightly) — **NOT `pull_request`**. Job: checkout, `docker build`, `docker run … run_smoke.sh`. Registry push is a separate step behind §4, disabled by default. Mirror `python.yml` conventions (`ubuntu-latest`, `actions/checkout@v4`, `set -euo pipefail`, `concurrency`, `timeout-minutes`).

## 7. Operator usage
Bare-metal: install solvers, `/autospec-fab --repo .` (stages resolve binaries off PATH). Container: `docker build -f skills/autospec-fab/docker/Dockerfile -t autospec-fab .` then `docker run --rm -v "$PWD/model:/work/model" -v "$PWD/stls:/work/stls" autospec-fab python3 …/release_gate_stages.py --in /work/stls/part.stl --model /work/model/metadata.json --out /work/release-gate.json`. Sibling-file convention unchanged: present sidecar → real solver runs; absent → stage skips cleanly.

## 8. Testing
- **Default (unchanged, every-PR):** existing `$TMP/bin`-shim unit suites stay green and authoritative; no regression to the mocked path.
- **New host-portable lint (cheap, in validate.sh):** assert the Dockerfile pins every version (no floating `:latest`, no unpinned apt), the smoke fixture `MODELDIR` is well-formed, `run_smoke.sh` asserts non-`skip`. Runnable without building the image.
- **New real-integration smoke:** §6, only in `fab-container.yml` (nightly/dispatch).
- **`fab-vision` CLI unit test:** the shipped minimal CLI conforms to the §1 contract (reuse `test_stage_vision.py`'s shim contract as oracle).
- Adding `docker/` + a workflow does not touch the trio → goldens unaffected; verify validate.sh stays green.

## 9. File pointers
- Stage solver contracts: `stage_fea.py` (`_run_ccx`, `_parse_safety_factor`), `stage_cfd.py` (`_run_solver`, `_parse_cfd_results`), `freecad_harness.py` (`resolve_freecadcmd`, `_script_render`), `stage_vision.py` (`resolve_vision_cmd`, judge/verify).
- Engine + sibling input: `release_gate_stages.py` (`STAGE_ORDER`, `_SIBLING_INPUTS`, `extra_args_for`).
- OpenFOAM case builder (snappyHexMesh-on-real-STL belongs here): `openfoam_case.py`.
- Test shim patterns to mirror: `tests/test_stage_fea.py` (`_make_ccx_shim`), `tests/test_stage_cfd.py` (`_make_solver_shim`), `tests/test_stage_vision.py` (`_install_vision_shim`), `tests/freecad_shim.py`.
- CI style: `.github/workflows/python.yml`.
- README to extend: `skills/autospec-fab/README.md`.

## 10. Decomposition hint (auto-implement children)

Ordered; each small and independently mergeable. **Children C and E are cost-incurring — gate behind §4 sign-off.**

1. **A — README system-packages + container-usage docs (cheap, no cost).** "Required system packages" table (`calculix-ccx`, `freecad-python3`, `openfoam2406`, mesa/xvfb) + "Container usage" section + `$AUTOSPEC_FAB_VISION_CMD` doc. Closes the doc half of #1270 and the README ask of #1271. No image build.
2. **B — minimal `fab-vision` CLI + conformance test (cheap).** `docker/fab-vision` (judge + `--verify`) conforming to `stage_vision.py`'s contract, unit test reusing `test_stage_vision.py`'s shim as oracle. No heavy deps. Unblocks a non-skip vision pass in the smoke. (Overlaps #1271; coordinate — prefer #1271's `fab-vision-cli.py` as the real consumer and have the container install it.)
3. **C — Dockerfile + result-contract wrappers [COST-INCURRING: multi-GB build, ~10–20 min CI].** Multi-stage pinned Dockerfile landing the 4 binaries on PATH + the `ccx`→`SAFETY_FACTOR` and OpenFOAM-fields→`cfd_results` post-process wrappers. **Requires §4 decisions 3,4,6.** `ctx:` high.
4. **D — real-solver smoke fixture + `run_smoke.sh` (cheap to author; runs heavy only in workflow).** Tiny fixture part + `MODELDIR` + `run_smoke.sh` asserting non-`skip` fragments. Depends on C for a runnable image; the script/fixture itself is light.
5. **E — `fab-container.yml` opt-in workflow [COST-INCURRING: nightly build minutes].** `workflow_dispatch` + nightly `schedule`, builds C's image, runs D's smoke, NOT on `pull_request`. Registry-push present but disabled pending §4 decision 2. Plus the host-portable Dockerfile-pin lint wired into validate.sh.

> A, B, D are safe auto-implement (no real cost). **C and E must not run until §4 is signed off** — they incur the multi-GB image + nightly CI minutes. Combine C's Dockerfile with its result-contract wrappers in one child.

---

**Note:** the single hardest piece of real work (beyond `apt install`) is bridging real solver output to the existing `.dat`/`cfd_results` parse contracts the stages already depend on — flagged in §5.2, bundled into child C. Issue #1271 (real vision CLI) overlaps and is satisfied by/coordinated with child B.
