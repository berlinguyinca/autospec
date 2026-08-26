#!/usr/bin/env bash
# scripts/autospec-detect-stack-profile.sh — stack profile + UI capability detection.
#
# Runs the stack detector, then enriches .autospec/state/stack-profile.json with a
# ui_capabilities block: accessible component primitives, a motion library, a
# prefers-reduced-motion reset, and the resulting gaps list.
#
# Most judgment-class accessibility findings — custom-widget keyboard operability,
# focus management, screen-reader announcements — come from hand-rolled widgets. A
# repo that already depends on accessible primitives does not produce them, so the
# implementer needs to know which of the three are missing before writing UI code.
#
# The enrichment lives here rather than inside detect_stack because
# autospec-autonomy-v2-lib.py is already far past the repo file-size gate.
# set -eu means a failure here fails the command instead of leaving a half-formed
# profile.
#
# Usage: autospec-detect-stack-profile.sh [--repo-root <dir>]
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
python3 "$SCRIPT_DIR/autospec-autonomy-v2-lib.py" --command detect-stack "$@"
# The detector owns the one exclusion list; PYTHONPATH lets this walker import it
# instead of keeping a second copy that can drift.
export PYTHONPATH="$SCRIPT_DIR${PYTHONPATH:+:$PYTHONPATH}"
exec python3 - "$@" <<'PY'
import argparse
import json
from pathlib import Path

from autospec_autonomy_stack import is_skipped

ACCESSIBLE_PRIMITIVE_PREFIXES = (
    "@radix-ui", "react-aria", "@headlessui", "@ariakit",
    "@chakra-ui", "@mui/material", "@base-ui-components",
)
MOTION_LIBRARIES = {"framer-motion", "motion", "gsap", "@formkit/auto-animate", "auto-animate"}
MOTION_PREFIXES = ("@react-spring", "@motionone")
UI_SOURCE_SUFFIXES = {".css", ".scss", ".sass", ".less", ".tsx", ".jsx", ".ts", ".js", ".html", ".vue", ".svelte"}
WEB_UI_PROFILES = {"typescript", "javascript"}
CAPABILITY_NAMES = ("accessible_primitives", "motion_library", "reduced_motion_reset")


def read_json(path):
    """Parsed JSON object, or an empty dict when absent or malformed."""
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="ignore"))
    except (ValueError, OSError):
        return {}
    return data if isinstance(data, dict) else {}


def declared_dependencies(root):
    """Dependency names from package.json, parsed rather than substring-matched.

    Substring matching against the raw file text — the surrounding detector's
    style — would let "motion" match an unrelated package name.
    """
    data = read_json(root / "package.json")
    sections = (data.get(key) for key in
                ("dependencies", "devDependencies", "peerDependencies", "optionalDependencies"))
    return {str(name).lower()
            for section in sections if isinstance(section, dict)
            for name in section}


def ui_source_files(root):
    """UI sources under the detector's shared exclusion list.

    The exclusion is matched on the repo-relative path, so a checkout living
    under a directory named e.g. "build" no longer hides its own sources.
    """
    return (p for p in root.rglob("*")
            if p.suffix.lower() in UI_SOURCE_SUFFIXES
            and not is_skipped(p.relative_to(root).as_posix())
            and p.is_file())


def reduced_motion_evidence(root):
    """First source path declaring a prefers-reduced-motion guard, if any."""
    for path in ui_source_files(root):
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        if "prefers-reduced-motion" in text:
            return [path.relative_to(root).as_posix()]
    return []


def ui_capabilities(root, profile_ids):
    """UI primitives present, and the gaps worth adopting.

    Non-UI repos report is_web_ui false and no gaps, so CLI, library, and Python
    projects are never nagged about UI dependencies.
    """
    is_web_ui = bool(profile_ids & WEB_UI_PROFILES)
    deps = declared_dependencies(root)
    primitives = sorted(d for d in deps if d.startswith(ACCESSIBLE_PRIMITIVE_PREFIXES))
    motion = sorted(d for d in deps if d in MOTION_LIBRARIES or d.startswith(MOTION_PREFIXES))
    reset = reduced_motion_evidence(root) if is_web_ui else []
    found = {
        "is_web_ui": is_web_ui,
        "accessible_primitives": {"present": bool(primitives), "evidence": primitives},
        "motion_library": {"present": bool(motion), "evidence": motion},
        "reduced_motion_reset": {"present": bool(reset), "evidence": reset},
    }
    found["gaps"] = [n for n in CAPABILITY_NAMES if not found[n]["present"]] if is_web_ui else []
    return found


parser = argparse.ArgumentParser()
parser.add_argument("--repo-root", default=".")
args, _ = parser.parse_known_args()
repo_root = Path(args.repo_root).resolve()
profile_path = repo_root / ".autospec/state/stack-profile.json"
# detect-stack ran immediately above under set -eu, so it either wrote this
# profile or the command already aborted. No missing-file branch is reachable.
profile = read_json(profile_path)
ids = {p.get("id") for p in profile.get("profiles", []) if isinstance(p, dict)}
profile["ui_capabilities"] = ui_capabilities(repo_root, ids)
profile_path.write_text(json.dumps(profile, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
