#!/usr/bin/env python3
"""Target stack detection and the confidence score gating stack-specific scaffolds.

A language is nominated only by a marker file resolved through
scripts/autospec-language-table.sh — never by substring matching on dependency
names — and its confidence is the share of tracked source lines the language
actually occupies, so a minority language cannot pass the scaffold gate.
The ui_capabilities block is added afterwards by autospec-detect-stack-profile.sh.
"""

from __future__ import annotations

import subprocess
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

TABLE_SCRIPT = Path(__file__).resolve().parent / "autospec-language-table.sh"

MARKER_BASENAMES = frozenset({
    "Cargo.toml", "go.mod", "pyproject.toml", "package.json",
    "pom.xml", "build.gradle", "Gemfile",
})

MIN_LINE_SHARE = 0.5
CLAMPED_CONFIDENCE = 0.5

_SUFFIX_LANGUAGES: dict[str, str] | None = None


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
    """Lowercased paths eligible to nominate a candidate; fixture-nested markers never vote."""
    lowered = [rel.lower() for rel in files]
    return [rel for rel in lowered if not FIXTURE_DIR_PARTS.intersection(rel.split("/")[:-1])]


def _table(fn: str, path: Path) -> str | None:
    """The language autospec-language-table.sh assigns to fn(path), or None."""
    try:
        proc = subprocess.run(["bash", str(TABLE_SCRIPT), fn, str(path)],
                              capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    lang = proc.stdout.strip()
    return lang if lang else None


def _git_ls_files(root: Path) -> list[str] | None:
    """Tracked paths in true case, or None when root is not a git worktree."""
    try:
        proc = subprocess.run(["git", "-C", str(root), "ls-files", "-z"],
                              capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    return [rel for rel in proc.stdout.split("\0") if rel]


def _tracked_entries(root: Path) -> "list[tuple[str, Path]]":
    """(repo-relative true-case path, path) for every file that may vote.

    A git worktree walks its index so an untracked file dilutes no line share;
    a plain directory falls back to the disk walk under the same exclusions.
    """
    tracked = _git_ls_files(root)
    if tracked is not None:
        return [(rel, root / rel) for rel in tracked
                if (root / rel).is_file() and not is_skipped(rel)]
    return [(path.relative_to(root).as_posix(), path) for path in root.rglob("*")
            if not is_skipped(path.relative_to(root).as_posix()) and path.is_file()]


def _line_counts(entries) -> dict[str, int]:
    """Tracked source lines per suffix, over every walked source file."""
    counts: dict[str, int] = {}
    for rel, path in entries:
        suffix = Path(rel).suffix.lower()
        if suffix not in SOURCE_SUFFIXES:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        counts[suffix] = counts.get(suffix, 0) + len(text.splitlines())
    return counts


def _package_text(root: Path) -> str:
    path = root / "package.json"
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="ignore").lower()


def _suffix_languages() -> dict[str, str]:
    """Suffix -> language per the table; suffixes the table refuses are absent."""
    global _SUFFIX_LANGUAGES
    if _SUFFIX_LANGUAGES is None:
        langs: dict[str, str] = {}
        for suffix in sorted(SOURCE_SUFFIXES):
            lang = _table("extension_language", Path("probe" + suffix))
            if lang:
                langs[suffix] = lang
        _SUFFIX_LANGUAGES = langs
    return _SUFFIX_LANGUAGES


def _confidence(share: float) -> float:
    """Fail-closed: under a 50% line share the language is weak evidence."""
    if share < MIN_LINE_SHARE:
        return CLAMPED_CONFIDENCE
    return min(0.95, round(0.5 + 0.45 * share, 2))


def _profile(pid: str, confidence: float, evidence: list[str], recipes: list[str] | None = None) -> dict:
    return {"id": pid, "confidence": confidence, "evidence": evidence,
            "supported_recipes": recipes or [], "unsupported_recipes": [], "notes": []}


def _detect_profiles(root: Path) -> dict:
    """{"languages": [...], "frameworks": [...]}; only a language may be primary.

    A language exists only when a non-fixture marker file resolves to it in the
    language table; playwright names a test framework, not the language a repo
    is written in, so it is reported but can never set primary_profile.
    """
    pkg = _package_text(root)
    entries = _tracked_entries(root)
    true_by_lower = {rel.lower(): rel for rel, _ in entries}
    marker_lowers = _marker_files([rel for rel, _ in entries])
    candidates: dict[str, list[str]] = {}
    for low in marker_lowers:
        rel = true_by_lower[low]
        base = Path(rel).name
        if base not in MARKER_BASENAMES and not base.endswith(".csproj"):
            continue
        lang = _table("marker_language", root / rel)
        if lang:
            candidates.setdefault(lang, []).append(rel)
    counts = _line_counts(entries)
    suffix_langs = _suffix_languages()
    total = sum(counts.values()) or 1
    languages = []
    for lang in sorted(candidates):
        lines = sum(n for s, n in counts.items() if suffix_langs.get(s) == lang)
        share = lines / total
        evidence = [f"marker {rel}" for rel in candidates[lang]]
        if share < MIN_LINE_SHARE:
            evidence.append(f"line share {share:.2f} below {MIN_LINE_SHARE}")
        languages.append(_profile(lang, _confidence(share), evidence))
    languages.sort(key=lambda p: (-p["confidence"], p["id"]))
    frameworks = []
    if "@playwright/test" in pkg or any("playwright.config" in rel for rel in marker_lowers):
        frameworks.append(_profile("playwright", 0.9, ["Playwright dependency/config"],
                                   ["playwright-viewport-matrix", "accessibility-smoke"]))
    return {"languages": languages or [_profile("unknown", 0.1, ["no recognized marker file"], [])],
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
