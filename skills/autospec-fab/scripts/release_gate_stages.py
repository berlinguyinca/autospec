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
