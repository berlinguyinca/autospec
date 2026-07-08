#!/usr/bin/env python3
"""Classify changed paths for autospec autonomous blast-radius quarantine."""

import argparse
import fnmatch
import json
import re
from pathlib import Path

LEGACY_REGISTRY = [
    {"id": "autonomous-control-plane", "severity": "fenced", "reason": "autonomous conductor or guardrail control plane", "paths": ["scripts/autospec-autonomous.sh", "scripts/autonomous-*.sh", "scripts/autospec-autonomous-run-drain.sh", "scripts/worktree-guard.sh", "scripts/claim-guard.sh", "scripts/autospec-autonomy-gate.sh"]},
    {"id": "skill-contracts", "severity": "high", "reason": "autospec skill public contracts", "paths": ["skills/autospec*/SKILL.md", "skills/autospec*/codex/prompt.md", "skills/autospec*/opencode/agent.md"]},
    {"id": "release-and-ci", "severity": "high", "reason": "release, install, or CI surface", "paths": [".github/workflows/*", "install.sh", "bootstrap.sh", "uninstall.sh"]},
    {"id": "schema-package-core", "severity": "high", "reason": "schema/package/crate core surface", "paths": ["schemas/*", "packages/*", "crates/*", "Cargo.toml", "Cargo.lock"]},
    {"id": "trading-money-risk", "severity": "fenced", "reason": "trading system money/risk/execution paths", "paths": ["trading-system/money/**", "trading-system/risk/**", "trading-system/execution/**"]},
    {"id": "sensitive-keywords", "severity": "high", "reason": "migration/auth/secret/token path keyword", "paths": ["*migration*", "*secret*", "*auth*", "*token*"]},
]


def scalar(value):
    value = value.strip()
    if (value.startswith('"') and value.endswith('"')) or (value.startswith("'") and value.endswith("'")):
        return value[1:-1]
    return value


def changed_paths(path):
    rows = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        item = line.strip()
        if item:
            rows.append(item[2:] if item.startswith("./") else item)
    return rows


def load_registry(path):
    if not path or not Path(path).exists():
        return []
    rows, active, base_indent, cur, in_paths = [], False, 0, None, False
    for raw in Path(path).read_text(encoding="utf-8").splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        indent = len(raw) - len(raw.lstrip(" "))
        stripped = raw.strip()
        if not active:
            active = bool(re.match(r"^fenced_surfaces\s*:", stripped))
            base_indent = indent if active else base_indent
            continue
        if indent <= base_indent and not re.match(r"^fenced_surfaces\s*:", stripped):
            break
        if re.match(r"^-\s+id\s*:", stripped):
            if cur:
                rows.append(cur)
            cur = {"id": scalar(stripped.split(":", 1)[1]), "severity": "high", "reason": "configured fenced surface", "paths": []}
            in_paths = False
            continue
        if cur is None:
            continue
        if re.match(r"^paths\s*:", stripped):
            in_paths = True
        elif in_paths and stripped.startswith("- "):
            cur["paths"].append(scalar(stripped[2:]))
        elif ":" in stripped:
            key, val = stripped.split(":", 1)
            if key.strip() in {"id", "severity", "reason"}:
                cur[key.strip()] = scalar(val)
            in_paths = False
    if cur:
        rows.append(cur)
    return [row for row in rows if row.get("id") and row.get("paths")]


def match_registry(paths, registry):
    matches = []
    for changed in paths:
        for surface in registry:
            for pattern in surface.get("paths", []):
                pat = str(pattern).lstrip("./")
                if fnmatch.fnmatch(changed, pat) or (pat.endswith("/**") and changed.startswith(pat[:-3].rstrip("/") + "/")):
                    matches.append({"path": changed, "surface": surface.get("id", "unknown"), "severity": surface.get("severity", "high"), "reason": surface.get("reason", "configured fenced surface"), "pattern": pat})
                    break
    return matches


def classify(paths, registry_path):
    matches = match_registry(paths, load_registry(registry_path) or LEGACY_REGISTRY)
    fenced = bool(matches)
    label = "blast:fenced" if any(m.get("severity") == "fenced" for m in matches) else "blast:high" if fenced else "blast:medium" if len({p.split('/')[0] for p in paths}) > 3 or len(paths) > 10 else "blast:low"
    return {
        "decision": "quarantine" if fenced else "allow",
        "reason": "fenced_surface" if fenced else None,
        "label": label,
        "fenced": fenced,
        "reversibility": "requires-review" if any(re.search(r"(migration|schema|auth|secret|token)", p, re.I) for p in paths) else "reversible",
        "paths": paths,
        "fenced_matches": matches,
        "registry": str(registry_path) if registry_path else "legacy-defaults",
        "exit_status": 1 if fenced else 0,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--changed-files", required=True)
    parser.add_argument("--fenced-surfaces", default="")
    args = parser.parse_args()
    print(json.dumps(classify(changed_paths(args.changed_files), args.fenced_surfaces), sort_keys=True))


if __name__ == "__main__":
    main()
