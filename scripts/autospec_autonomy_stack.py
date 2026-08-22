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

# One exclusion list for both walkers: this module's detector and the
# ui_capabilities walker in autospec-detect-stack-profile.sh, which imports it.
# Matched against repo-relative paths so a repo checked out under a directory
# literally named "build" is not erased by its own parent path.
SKIP_DIRS = frozenset({".git", "node_modules", "dist", "build", ".next", "out",
                       "coverage", "target"})
SKIP_PATH_PREFIXES = (".claude/worktrees",)

# A marker under one of these trees describes a fixture, not this repository —
# tests/fixtures/evidence/react-vite-with-playwright/playwright.config.ts is the
# defect this list closes. Line share is deliberately NOT filtered this way: a
# real .rs or .sh file under tests/ is still this repo's source.
FIXTURE_DIR_PARTS = frozenset({"fixtures", "fixture", "testdata", "vendor", "third_party"})

SOURCE_SUFFIXES = frozenset({
    ".py", ".rs", ".sh", ".bash", ".go", ".java", ".kt", ".rb", ".php", ".swift",
    ".scala", ".c", ".h", ".cpp", ".hpp", ".cs", ".lua", ".ex", ".exs", ".pl",
    ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte",
})

# Which suffixes a language profile is actually written in, for line share.
_WEB_SUFFIXES = (".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte")
LANGUAGE_SUFFIXES = {
    "react-vite-typescript": _WEB_SUFFIXES,
    "nextjs-web-app": _WEB_SUFFIXES,
    "python-cli-tool": (".py",),
}
MIN_LINE_SHARE = 0.5
CLAMPED_CONFIDENCE = 0.5


def is_skipped(rel_path: str) -> bool:
    """True when a repo-relative path lies under an excluded directory."""
    parents = rel_path.split("/")[:-1]
    if SKIP_DIRS.intersection(parents):
        return True
    return any(rel_path.startswith(prefix + "/") for prefix in SKIP_PATH_PREFIXES)


def _walk(root: Path):
    """(repo-relative lowercased path, path) for every file outside the exclusions.

    Replaces the old _source_files, which skipped only .git and node_modules and
    so walked target/, dist/, and every nested .claude/worktrees copy of a fixture.
    """
    for path in root.rglob("*"):
        rel = path.relative_to(root).as_posix()
        if is_skipped(rel) or not path.is_file():
            continue
        yield rel.lower(), path


def _marker_files(files: list[str]) -> list[str]:
    """Paths eligible to nominate a candidate; fixture-nested markers never vote."""
    return [f for f in files if not FIXTURE_DIR_PARTS.intersection(f.split("/")[:-1])]


def _line_counts(entries) -> dict[str, int]:
    """Tracked source lines per suffix, over every walked source file."""
    counts: dict[str, int] = {}
    for rel, path in entries:
        suffix = Path(rel).suffix
        if suffix not in SOURCE_SUFFIXES:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        counts[suffix] = counts.get(suffix, 0) + len(text.splitlines())
    return counts


def _clamped(profile: dict, counts: dict[str, int]) -> dict:
    """Confidence capped below the scaffold gate when the language is a minority.

    Fail-closed: a marker that nominates a language covering under half the
    tracked source lines is weak evidence, not a licence to scaffold.
    """
    suffixes = LANGUAGE_SUFFIXES.get(profile["id"])
    total = sum(counts.values())
    if not suffixes or not total or profile["confidence"] <= CLAMPED_CONFIDENCE:
        return profile
    share = sum(counts.get(s, 0) for s in suffixes) / total
    if share >= MIN_LINE_SHARE:
        return profile
    return {**profile, "confidence": CLAMPED_CONFIDENCE,
            "evidence": profile["evidence"] + [f"line share {share:.2f} below {MIN_LINE_SHARE}"]}


def _package_text(root: Path) -> str:
    path = root / "package.json"
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="ignore").lower()


def _profile(pid: str, confidence: float, evidence: list[str], recipes: list[str] | None = None) -> dict:
    return {"id": pid, "confidence": confidence, "evidence": evidence,
            "supported_recipes": recipes or [], "unsupported_recipes": [], "notes": []}


def _detect_profiles(root: Path) -> dict:
    """{"languages": [...], "frameworks": [...]}; only a language may be primary.

    playwright names a test framework, not the language a repo is written in, so
    it is reported but can never set primary_profile. nextjs-web-app and
    react-vite-typescript stay language-tier: they are the only profiles carrying
    TypeScript/JavaScript language evidence, and the runtime adapter/generator
    pipeline resolves its adapter from primary_profile. Demoting them before the
    follow-up marker-table issue lands a TS/JS language marker would leave every
    such repo at unknown @ 0.1.
    """
    pkg = _package_text(root)
    entries = list(_walk(root))
    markers = _marker_files([rel for rel, _ in entries])
    languages, frameworks = [], []
    if "react" in pkg and "vite" in pkg and ("typescript" in pkg or any(f.endswith(".tsx") for f in markers)):
        languages.append(_profile("react-vite-typescript", 0.95, ["package.json: react/vite/typescript"],
                                  ["playwright-viewport-matrix", "documentation-route-scaffold", "settings-page-scaffold"]))
    if "fastapi" in pkg or (root / "pyproject.toml").exists() or any(f.endswith(".py") for f in markers):
        languages.append(_profile("python-cli-tool", 0.65, ["python files or pyproject"], ["metadata-drift-test"]))
    if "next" in pkg:
        languages.append(_profile("nextjs-web-app", 0.9, ["package.json: next"],
                                  ["documentation-route-scaffold", "settings-page-scaffold"]))
    if "@playwright/test" in pkg or any("playwright.config" in f for f in markers):
        frameworks.append(_profile("playwright", 0.9, ["Playwright dependency/config"],
                                   ["playwright-viewport-matrix", "accessibility-smoke"]))
    counts = _line_counts(entries)
    languages = [_clamped(p, counts) for p in languages]
    return {"languages": languages or [_profile("unknown", 0.1, ["no recognized stack evidence"], [])],
            "frameworks": frameworks}


def detect_stack(root: Path) -> int:
    detected = _detect_profiles(root)
    languages, frameworks = detected["languages"], detected["frameworks"]
    primary = max(languages, key=lambda p: p["confidence"])
    payload = {"schema": 1, "profiles": languages + frameworks, "languages": languages,
               "frameworks": frameworks, "primary_profile": primary}
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
