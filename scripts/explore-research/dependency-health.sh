#!/usr/bin/env bash
# scripts/explore-research/dependency-health.sh — deterministic researcher #7.
#
# Detects dependency manifests in the repo and identifies outdated deps via
# harness-aware tooling (npm/pip/go/cargo/gem). Best-effort: missing toolchains
# produce zero proposals rather than failing.
#
# Also usable directly by autospec-explore (extends 6→7 researchers) and by
# autospec-sweep as the dependency-health area researcher.
#
# Output: JSON to stdout matching the contract in
# docs/specs/2026-05-29-autospec-explore-design.md — Research cycle contract.

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
MAX_PROPOSALS=20

cd "$REPO_ROOT" || { echo '{"source":"dependency-health","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"dependency-health","proposals":[]}'
    exit 0
fi

# Allow tests to inject pre-computed outdated reports.
INJECTED_NPM="${AUTOSPEC_TEST_NPM_OUTDATED:-}"
INJECTED_PIP="${AUTOSPEC_TEST_PIP_OUTDATED:-}"

manifests=()
[ -f package.json ] && manifests+=("npm:package.json")
[ -f requirements.txt ] && manifests+=("pip:requirements.txt")
[ -f pyproject.toml ] && manifests+=("pip:pyproject.toml")
[ -f go.mod ] && manifests+=("go:go.mod")
[ -f Cargo.toml ] && manifests+=("cargo:Cargo.toml")
[ -f Gemfile ] && manifests+=("gem:Gemfile")

# Collect outdated reports (best-effort, capped time)
npm_json=""
pip_json=""
if [ -n "$INJECTED_NPM" ] && [ -f "$INJECTED_NPM" ]; then
    npm_json="$(cat "$INJECTED_NPM")"
elif [ -f package.json ] && command -v npm >/dev/null 2>&1; then
    npm_json="$(timeout 10 npm outdated --json 2>/dev/null || true)"
fi
if [ -n "$INJECTED_PIP" ] && [ -f "$INJECTED_PIP" ]; then
    pip_json="$(cat "$INJECTED_PIP")"
elif { [ -f requirements.txt ] || [ -f pyproject.toml ]; } && command -v pip >/dev/null 2>&1; then
    pip_json="$(timeout 10 pip list --outdated --format=json 2>/dev/null || true)"
fi

export AUTOSPEC_MANIFESTS="${manifests[*]:-}"
export AUTOSPEC_NPM_OUTDATED="$npm_json"
export AUTOSPEC_PIP_OUTDATED="$pip_json"
export AUTOSPEC_MAX_PROPOSALS="$MAX_PROPOSALS"

python3 - <<'PY'
import json, os

manifests = os.environ.get("AUTOSPEC_MANIFESTS", "").split()
npm_raw = os.environ.get("AUTOSPEC_NPM_OUTDATED", "").strip()
pip_raw = os.environ.get("AUTOSPEC_PIP_OUTDATED", "").strip()
cap = int(os.environ.get("AUTOSPEC_MAX_PROPOSALS", "20"))

proposals = []

def load_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            value = json.load(handle)
        return value if isinstance(value, dict) else {}
    except Exception:
        return {}

package_json = load_json("package.json")
package_lock = load_json("package-lock.json")

def npm_current_version(name, info):
    current = str((info or {}).get("current", "")).strip()
    if current:
        return current
    packages = package_lock.get("packages", {})
    if isinstance(packages, dict):
        installed = packages.get(f"node_modules/{name}", {})
        if isinstance(installed, dict):
            current = str(installed.get("version", "")).strip()
            if current:
                return current
    locked = package_lock.get("dependencies", {}).get(name, {})
    if isinstance(locked, dict):
        current = str(locked.get("version", "")).strip()
        if current:
            return current
    installed = load_json(os.path.join("node_modules", name, "package.json"))
    current = str(installed.get("version", "")).strip()
    if current:
        return current
    return ""

def add(title, evidence, complexity="small", confidence=0.7):
    if len(proposals) >= cap:
        return
    proposals.append({
        "title": title,
        "evidence": evidence,
        "estimated_complexity": complexity,
        "confidence": confidence,
    })

# npm outdated → object keyed by package name
if npm_raw:
    try:
        data = json.loads(npm_raw)
        if isinstance(data, dict):
            for pkg, info in data.items():
                cur = npm_current_version(pkg, info)
                latest = str((info or {}).get("latest", "")).strip()
                if not cur or not latest or cur == latest:
                    continue
                add(
                    f"chore(deps): bump {pkg} {cur} → {latest}",
                    f"npm outdated reports {pkg} current={cur} latest={latest}",
                )
    except Exception:
        pass

# pip list --outdated → list of {name, version, latest_version}
if pip_raw:
    try:
        data = json.loads(pip_raw)
        if isinstance(data, list):
            for item in data:
                name = item.get("name")
                cur = item.get("version", "?")
                latest = item.get("latest_version", "?")
                if not name:
                    continue
                add(
                    f"chore(deps): bump {name} {cur} → {latest}",
                    f"pip list --outdated reports {name} current={cur} latest={latest}",
                )
    except Exception:
        pass

# Fallback: if manifests exist but no outdated tooling produced output, emit a
# single low-confidence proposal noting toolchain coverage gap so the sweep
# does not silently report a clean dependency-health area.
if manifests and not proposals and not npm_raw and not pip_raw:
    add(
        "chore(deps): verify dependency freshness (no outdated tooling available)",
        f"Detected manifests: {', '.join(manifests)}. No outdated-check tooling produced output.",
        complexity="small",
        confidence=0.4,
    )

print(json.dumps({"source": "dependency-health", "proposals": proposals}))
PY
