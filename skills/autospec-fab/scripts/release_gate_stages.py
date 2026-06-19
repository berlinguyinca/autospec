"""release_gate_stages.py — STAGE_ORDER table + script resolver for the engine.

Extracted from stl-release-gate.py to keep each module small (repo lints file
size + function length hard). Single source of truth for the 12-stage fab
composition order and how the engine locates each stage script.

Stage order matches skills/autospec-fab/SKILL.md (§Release-gate / phase
contract). Every stage is BLOCKING except `vision`, which is ADVISORY: its
status never blocks the gate; its observations flow to release-gate.json
["vision_findings"].

The real geometry stage lives at `stage-geometry.py` (hyphenated); the rest are
`stage_<name>.py`. Resolution tries the listed filename first, then the
alternate naming convention, so the engine works whether stage authors used a
hyphen or an underscore.
"""

import os

# Ordered list of {name, script, blocking}. `name` is the schema stage name;
# `script` is the canonical filename; `blocking=False` only for vision.
STAGE_ORDER = [
    {"name": "geometry", "script": "stage-geometry.py", "blocking": True},
    {"name": "metadata", "script": "stage_metadata.py", "blocking": True},
    {"name": "vacuum-fitting", "script": "stage_fitting.py", "blocking": True},
    {"name": "vacuum-circuit", "script": "stage_circuit.py", "blocking": True},
    {"name": "gasket", "script": "stage_gasket.py", "blocking": True},
    {"name": "dust-airflow", "script": "stage_dust.py", "blocking": True},
    {"name": "slicer", "script": "stage_slicer.py", "blocking": True},
    {"name": "fea", "script": "stage_fea.py", "blocking": True},
    {"name": "cfd", "script": "stage_cfd.py", "blocking": True},
    {"name": "render", "script": "stage_render.py", "blocking": True},
    {"name": "vision", "script": "stage_vision.py", "blocking": False},
    {"name": "docs", "script": "stage_docs.py", "blocking": True},
]


# Per-stage SIBLING-FILE input convention. The engine resolves each stage's
# stage-specific input relative to MODELDIR (the directory holding the --model
# sidecar) and threads it ONLY when the file exists. A featureless model (no
# sibling file) gets no extra flag, so the stage correctly skips. This is what
# makes a FEATUREFUL model actually RUN vacuum-circuit / dust-airflow / fea /
# cfd instead of always skipping (Phase 5.5 audit D2).
#   stage name -> (flag, sibling filename relative to MODELDIR)
_SIBLING_INPUTS = {
    "vacuum-circuit": ("--circuit", "circuit.json"),
    "dust-airflow": ("--duct", "duct.json"),
    "slicer": ("--printer", "printer.json"),
    "fea": ("--load", "load.json"),
    "cfd": ("--flow", "flow.json"),
}


def render_dir_for(out_path):
    """Deterministic per-run render dir: <out-dir>/renders/.

    Keyed to the gate `--out` directory so a run's renders land in a stable,
    predictable location the engine can hand to render (via --render-dir) and
    then read back for the vision handoff. The render stage writes its
    contact sheet under <render-dir>/<slug>/contact-sheet.html.
    """
    out_dir = os.path.dirname(os.path.abspath(out_path))
    return os.path.join(out_dir, "renders")


def contact_sheet_for(render_dir, stl_path):
    """Path the render stage writes its sheet to: <render-dir>/<slug>/...html.

    `slug` is the STL stem (matches stage_render._model_slug). The engine
    threads this path as the vision stage's --in so vision inspects the real
    renders instead of the STL.
    """
    base = os.path.basename(stl_path)
    slug = os.path.splitext(base)[0] or "model"
    return os.path.join(render_dir, slug, "contact-sheet.html")


def extra_args_for(stage_name, stl_path, model_path, render_dir=None):
    """Return the extra CLI args to thread into <stage_name>, or [].

    `docs` is the ONE stage that takes the model DIRECTORY (not the stl) as
    `--in`, plus an optional `--baseline MODELDIR/baseline.json`. `render`
    gets `--render-dir <render_dir>` (the deterministic per-run renders dir)
    when one is supplied, so its contact sheet lands at a known path. `vision`
    takes that produced contact sheet as its `--in` (NOT the stl) when render
    actually emitted one — handled by the engine via override, see run_gate.
    Every other stage keeps `--in <stl>`; this resolver only adds
    stage-specific flags when the sibling descriptor exists under MODELDIR.
    """
    model_dir = os.path.dirname(os.path.abspath(model_path))
    if stage_name == "docs":
        args = ["--in", model_dir]
        baseline = os.path.join(model_dir, "baseline.json")
        if os.path.isfile(baseline):
            args += ["--baseline", baseline]
        return args
    if stage_name == "render" and render_dir:
        return ["--render-dir", render_dir]
    spec = _SIBLING_INPUTS.get(stage_name)
    if spec is None:
        return []
    flag, filename = spec
    sibling = os.path.join(model_dir, filename)
    if os.path.isfile(sibling):
        return [flag, sibling]
    return []


def _candidate_dirs(stages_dir):
    """Resolution order: --stages-dir, $AUTOSPEC_FAB_STAGES_DIR, engine dir."""
    dirs = []
    if stages_dir:
        dirs.append(stages_dir)
    env_dir = os.environ.get("AUTOSPEC_FAB_STAGES_DIR")
    if env_dir:
        dirs.append(env_dir)
    dirs.append(os.path.dirname(os.path.abspath(__file__)))
    return dirs


def _filename_variants(script):
    """Yield the canonical name plus the alternate hyphen/underscore form."""
    variants = [script]
    if script.startswith("stage-"):
        variants.append("stage_" + script[len("stage-"):].replace("-", "_"))
    elif script.startswith("stage_"):
        alt = "stage-" + script[len("stage_"):].replace("_", "-")
        variants.append(alt)
    return variants


def resolve_stage_script(script, stages_dir):
    """Return an absolute path to the stage script, or None if not found."""
    for directory in _candidate_dirs(stages_dir):
        for name in _filename_variants(script):
            path = os.path.join(directory, name)
            if os.path.isfile(path):
                return path
    return None
