#!/usr/bin/env python3
"""Target stack detection and the confidence score gating stack-specific scaffolds.

Extracted from autospec-autonomy-v2-lib.py to bring that file under the repo's
file-size gate. Extracted rather than left in place because the worker refusal
path needs stack_confidence, and modules must never import the lib — its filename
contains hyphens and is not importable.

Behaviour is identical to the originals — this is a move, not a rewrite. The
ui_capabilities block is added afterwards by autospec-detect-stack-profile.sh.
"""

from __future__ import annotations

from pathlib import Path

from autospec_autonomy_io import load_json, reports, state, write_json, write_text

SKIP_PARTS = (".git", "node_modules")


def _source_files(root: Path) -> list[str]:
    return [p.relative_to(root).as_posix().lower() for p in root.rglob("*")
            if p.is_file() and ".git" not in p.parts and "node_modules" not in p.parts]


def _package_text(root: Path) -> str:
    path = root / "package.json"
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="ignore").lower()


def _profile(pid: str, confidence: float, evidence: list[str], recipes: list[str] | None = None) -> dict:
    return {"id": pid, "confidence": confidence, "evidence": evidence,
            "supported_recipes": recipes or [], "unsupported_recipes": [], "notes": []}


def _detect_profiles(root: Path) -> list[dict]:
    pkg = _package_text(root)
    files = _source_files(root)
    profiles = []
    if "react" in pkg and "vite" in pkg and ("typescript" in pkg or any(f.endswith(".tsx") for f in files)):
        profiles.append(_profile("react-vite-typescript", 0.95, ["package.json: react/vite/typescript"],
                                 ["playwright-viewport-matrix", "documentation-route-scaffold", "settings-page-scaffold"]))
    if "next" in pkg:
        profiles.append(_profile("nextjs-web-app", 0.9, ["package.json: next"],
                                 ["documentation-route-scaffold", "settings-page-scaffold"]))
    if "fastapi" in pkg or (root / "pyproject.toml").exists() or any(f.endswith(".py") for f in files):
        profiles.append(_profile("python-cli-tool", 0.65, ["python files or pyproject"], ["metadata-drift-test"]))
    if "@playwright/test" in pkg or any("playwright.config" in f for f in files):
        profiles.append(_profile("playwright", 0.9, ["Playwright dependency/config"],
                                 ["playwright-viewport-matrix", "accessibility-smoke"]))
    return profiles or [_profile("unknown", 0.1, ["no recognized stack evidence"], [])]


def detect_stack(root: Path) -> int:
    profiles = _detect_profiles(root)
    primary = max(profiles, key=lambda p: p["confidence"])
    payload = {"schema": 1, "profiles": profiles, "primary_profile": primary}
    write_json(state(root) / "stack-profile.json", payload)
    write_text(reports(root) / "stack-profile.md", "\n".join([
        "# Stack Profile",
        "",
        "## Summary",
        "",
        f"- Primary: `{primary['id']}` ({primary['confidence']})",
    ]))
    return 0


def stack_confidence(root: Path) -> float:
    data = load_json(state(root) / "stack-profile.json", {})
    if not data:
        detect_stack(root)
        data = load_json(state(root) / "stack-profile.json", {})
    return float(data.get("primary_profile", {}).get("confidence", 0))
