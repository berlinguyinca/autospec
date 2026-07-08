#!/usr/bin/env bash
# scripts/autospec-discover-metadata.sh — discover local repository metadata.
#
# Local filesystem only. Writes deterministic repository intelligence under
# .autospec/state and .autospec/reports for later Baseline and Constitution
# analysis. No network or GitHub API calls.

set -eu

usage() {
    cat <<'EOF'
Usage:
  autospec-discover-metadata.sh [--repo-root <dir>]

Writes:
  .autospec/state/product-purpose.md
  .autospec/state/technology-registry.yml
  .autospec/state/repository-inventory.json
  .autospec/state/docs-coverage-map.json
  .autospec/state/test-coverage-map.json
  .autospec/state/api-surface.json
  .autospec/state/ui-surface.json
  .autospec/reports/metadata-discovery.json
  .autospec/reports/metadata-discovery.md
EOF
}

die() {
    printf 'autospec-discover-metadata: %s\n' "$*" >&2
    exit 2
}

REPO_ROOT="$(pwd)"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) [ "$#" -ge 2 ] || die "--repo-root requires a value"; REPO_ROOT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

[ -d "$REPO_ROOT" ] || die "--repo-root does not exist: $REPO_ROOT"
REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" <<'PY'
import json
import os
import re
import subprocess
import sys

repo_root = os.path.realpath(sys.argv[1])
state_dir = os.path.join(repo_root, ".autospec", "state")
reports_dir = os.path.join(repo_root, ".autospec", "reports")

LANG_EXTENSIONS = {
    ".js": "JavaScript",
    ".jsx": "JavaScript",
    ".mjs": "JavaScript",
    ".cjs": "JavaScript",
    ".ts": "TypeScript",
    ".tsx": "TypeScript",
    ".py": "Python",
    ".sh": "Shell",
    ".bash": "Shell",
    ".go": "Go",
    ".rs": "Rust",
    ".java": "Java",
    ".scala": "Scala",
    ".kt": "Kotlin",
    ".rb": "Ruby",
    ".php": "PHP",
    ".cs": "C#",
}

PACKAGE_FILES = {
    "package.json": "npm",
    "pnpm-lock.yaml": "pnpm",
    "yarn.lock": "yarn",
    "package-lock.json": "npm",
    "pyproject.toml": "python",
    "requirements.txt": "pip",
    "Pipfile": "pipenv",
    "poetry.lock": "poetry",
    "go.mod": "go",
    "Cargo.toml": "cargo",
    "pom.xml": "maven",
    "build.gradle": "gradle",
    "build.gradle.kts": "gradle",
    "Gemfile": "bundler",
}

EXCLUDE_DIRS = {".git", ".hg", ".svn", "node_modules", ".venv", "venv", "__pycache__", "dist", "build", "target"}
GENERATED_PREFIXES = {".autospec/state", ".autospec/reports"}


def ensure_dirs():
    os.makedirs(state_dir, exist_ok=True)
    os.makedirs(reports_dir, exist_ok=True)


def rel(path):
    return os.path.relpath(path, repo_root).replace(os.sep, "/")


def fact(value, confidence, evidence):
    return {"value": value, "confidence": confidence, "evidence": sorted(dict.fromkeys(evidence))}


def run_git(args):
    try:
        completed = subprocess.run(
            ["git"] + args,
            cwd=repo_root,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except OSError:
        return ""
    return completed.stdout.strip() if completed.returncode == 0 else ""


def is_generated(rel_path):
    return any(rel_path == prefix or rel_path.startswith(prefix + "/") for prefix in GENERATED_PREFIXES)


def walk_files():
    files = []
    for current, dirs, names in os.walk(repo_root):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDE_DIRS)
        for name in sorted(names):
            path = os.path.join(current, name)
            rel_path = rel(path)
            if is_generated(rel_path):
                continue
            files.append(rel_path)
    return files


def read_text(rel_path, limit=200000):
    try:
        with open(os.path.join(repo_root, rel_path), "r", encoding="utf-8", errors="ignore") as fh:
            return fh.read(limit)
    except OSError:
        return ""


def load_package_json(rel_path):
    try:
        with open(os.path.join(repo_root, rel_path), "r", encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def discover_default_branch():
    branch = run_git(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
    if branch.startswith("origin/"):
        return fact(branch.split("/", 1)[1], 0.8, ["git symbolic-ref refs/remotes/origin/HEAD"])
    branch = run_git(["branch", "--show-current"])
    if branch:
        return fact(branch, 0.5, ["git branch --show-current"])
    return fact("", 0.0, ["default branch unavailable from local git metadata"])


def discover_languages(files):
    evidence_by_lang = {}
    for rel_path in files:
        language = LANG_EXTENSIONS.get(os.path.splitext(rel_path)[1])
        if language:
            evidence_by_lang.setdefault(language, []).append(rel_path)
    languages = sorted(evidence_by_lang)
    evidence = [paths[0] for _, paths in sorted(evidence_by_lang.items())]
    return fact(languages, 0.9 if languages else 0.0, evidence or ["no recognized language extensions found"])


def discover_package_managers(files):
    managers = []
    evidence = []
    for marker, manager in sorted(PACKAGE_FILES.items()):
        if marker in files:
            managers.append(manager)
            evidence.append(marker)
    return fact(sorted(dict.fromkeys(managers)), 0.9 if managers else 0.0, evidence or ["no package manager manifests found"])


def discover_commands(files):
    commands = []
    evidence = []
    if "package.json" in files:
        package = load_package_json("package.json")
        scripts = package.get("scripts") if isinstance(package.get("scripts"), dict) else {}
        for name in sorted(scripts):
            if name in {"build", "test", "lint", "typecheck", "e2e", "start"} or "test" in name:
                commands.append(f"npm run {name}")
                evidence.append("package.json")
    if "Makefile" in files:
        text = read_text("Makefile")
        for target in ["build", "test", "lint", "validate"]:
            if re.search(rf"^{re.escape(target)}\s*:", text, re.M):
                commands.append(f"make {target}")
                evidence.append("Makefile")
    if "scripts/validate.sh" in files:
        commands.append("bash scripts/validate.sh")
        evidence.append("scripts/validate.sh")
    return fact(sorted(dict.fromkeys(commands)), 0.75 if commands else 0.0, evidence or ["no build/test commands inferred"])


def list_by_predicate(files, predicate):
    return sorted(path for path in files if predicate(path))


def discover_inventory(files):
    docs = list_by_predicate(files, lambda p: p.lower().startswith("docs/") or os.path.basename(p).lower().startswith("readme"))
    tests = list_by_predicate(files, lambda p: p.startswith("tests/") or "/test" in p.lower() or p.lower().endswith((".test.js", ".test.ts", ".spec.js", ".spec.ts", "_test.py")))
    sources = sorted(path for path in ["src", "lib", "app", "packages", "skills", "scripts"] if os.path.isdir(os.path.join(repo_root, path)))
    ci = list_by_predicate(files, lambda p: p.startswith(".github/workflows/") or p.startswith(".gitlab-ci"))
    scripts = list_by_predicate(files, lambda p: p.startswith("scripts/") or p.endswith(".sh"))
    config = list_by_predicate(files, lambda p: os.path.basename(p) in {".autospec.yml", "autospec.yml", "package.json", "pyproject.toml", "tsconfig.json", "vite.config.ts", "next.config.js", "docker-compose.yml"} or p.startswith(".autospec/"))
    containers = list_by_predicate(files, lambda p: os.path.basename(p).lower().startswith("dockerfile") or "docker-compose" in os.path.basename(p).lower() or os.path.basename(p) == "Containerfile")
    migrations = list_by_predicate(files, lambda p: "migration" in p.lower() or p.startswith(("db/", "database/")))
    return {
        "files_scanned": len(files),
        "docs_locations": docs,
        "test_locations": tests,
        "source_directories": sources,
        "ci_workflows": ci,
        "scripts": scripts,
        "config_files": config,
        "container_files": containers,
        "database_migration_indicators": migrations,
    }


def dependency_names():
    names = set()
    package = load_package_json("package.json")
    for key in ["dependencies", "devDependencies", "peerDependencies"]:
        section = package.get(key)
        if isinstance(section, dict):
            names.update(str(name).lower() for name in section)
    return names


def discover_api(files, deps):
    evidence = []
    openapi = list_by_predicate(files, lambda p: re.search(r"(openapi|swagger).*\.(ya?ml|json)$", p, re.I) is not None)
    evidence.extend(openapi)
    route_patterns = []
    for path in files:
        if os.path.splitext(path)[1] not in LANG_EXTENSIONS:
            continue
        text = read_text(path, 50000)
        if re.search(r"\b(app|router)\.(get|post|put|patch|delete)\s*\(", text) or "@app.route" in text:
            route_patterns.append(path)
    evidence.extend(route_patterns)
    api_deps = sorted(deps.intersection({"express", "fastify", "koa", "fastapi", "flask", "django", "@nestjs/core"}))
    evidence.extend([f"package dependency: {name}" for name in api_deps])
    return {
        "openapi_documents": openapi,
        "route_files": sorted(route_patterns),
        "dependencies": api_deps,
        "present": bool(evidence),
        "evidence": sorted(dict.fromkeys(evidence)) or ["no API indicators found"],
    }


def discover_ui(files, deps):
    evidence = []
    component_files = list_by_predicate(files, lambda p: os.path.splitext(p)[1] in {".jsx", ".tsx", ".vue", ".svelte"} or p.startswith(("src/pages/", "src/components/", "app/")))
    evidence.extend(component_files)
    ui_deps = sorted(deps.intersection({"react", "vue", "svelte", "next", "next.js", "@angular/core", "vite"}))
    evidence.extend([f"package dependency: {name}" for name in ui_deps])
    return {
        "component_files": component_files,
        "dependencies": ui_deps,
        "present": bool(evidence),
        "evidence": sorted(dict.fromkeys(evidence)) or ["no UI indicators found"],
    }


def discover_boolean_indicator(files, deps, keywords, path_predicate, absent_reason):
    evidence = [path for path in files if path_predicate(path)]
    evidence.extend([f"package dependency: {name}" for name in sorted(deps.intersection(keywords))])
    return fact(bool(evidence), 0.85 if evidence else 0.2, evidence or [absent_reason])


def first_readme_summary(files):
    for candidate in ["README.md", "readme.md", "README"]:
        if candidate in files:
            text = read_text(candidate, 20000)
            for line in text.splitlines():
                line = line.strip(" #\t")
                if line:
                    return line, candidate
    return os.path.basename(repo_root), "repository directory name"


def write_json(path, data):
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")


def write_yaml_registry(path, facts, indicators):
    lines = [
        "version: 1",
        "languages:",
    ]
    for language in facts["languages"]["value"]:
        lines.append(f"  - {language}")
    lines.append("package_managers:")
    for manager in facts["package_managers"]["value"]:
        lines.append(f"  - {manager}")
    lines.append("indicators:")
    for name in sorted(indicators):
        lines.append(f"  {name}: {str(indicators[name]['value']).lower()}")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))
        fh.write("\n")


ensure_dirs()
files = walk_files()
deps = dependency_names()
inventory = discover_inventory(files)
summary, summary_evidence = first_readme_summary(files)
facts = {
    "repo_name": fact(os.path.basename(repo_root), 1.0, ["repository root directory name"]),
    "default_branch": discover_default_branch(),
    "languages": discover_languages(files),
    "package_managers": discover_package_managers(files),
    "build_test_commands": discover_commands(files),
    "docs_locations": fact(inventory["docs_locations"], 0.9 if inventory["docs_locations"] else 0.0, inventory["docs_locations"] or ["no README or docs/ files found"]),
    "test_locations": fact(inventory["test_locations"], 0.9 if inventory["test_locations"] else 0.0, inventory["test_locations"] or ["no tests/ or test-named files found"]),
    "source_directories": fact(inventory["source_directories"], 0.85 if inventory["source_directories"] else 0.0, inventory["source_directories"] or ["no common source directories found"]),
    "ci_workflows": fact(inventory["ci_workflows"], 0.9 if inventory["ci_workflows"] else 0.0, inventory["ci_workflows"] or ["no CI workflow files found"]),
    "scripts": fact(inventory["scripts"], 0.8 if inventory["scripts"] else 0.0, inventory["scripts"] or ["no scripts found"]),
    "config_files": fact(inventory["config_files"], 0.8 if inventory["config_files"] else 0.0, inventory["config_files"] or ["no common config files found"]),
    "container_files": fact(inventory["container_files"], 0.9 if inventory["container_files"] else 0.0, inventory["container_files"] or ["no Docker/container files found"]),
    "database_migration_indicators": fact(inventory["database_migration_indicators"], 0.7 if inventory["database_migration_indicators"] else 0.1, inventory["database_migration_indicators"] or ["no database or migration indicators found"]),
    "product_purpose": fact(summary, 0.7 if summary_evidence != "repository directory name" else 0.3, [summary_evidence]),
}
api_surface = discover_api(files, deps)
ui_surface = discover_ui(files, deps)
indicators = {
    "api": fact(api_surface["present"], 0.85 if api_surface["present"] else 0.2, api_surface["evidence"]),
    "ui": fact(ui_surface["present"], 0.85 if ui_surface["present"] else 0.2, ui_surface["evidence"]),
    "cli": discover_boolean_indicator(files, deps, {"commander", "yargs", "click", "typer"}, lambda p: p.startswith("bin/") or p.endswith(".sh"), "no CLI indicators found"),
    "ai_rag": discover_boolean_indicator(files, deps, {"openai", "langchain", "llamaindex", "@langchain/core", "chromadb"}, lambda p: re.search(r"(rag|embedding|prompt|llm|vector)", p, re.I) is not None, "no AI/RAG indicators found"),
    "playwright_e2e": discover_boolean_indicator(files, deps, {"@playwright/test", "playwright"}, lambda p: "playwright" in p.lower() or p.startswith("tests/e2e/") or p.startswith("e2e/"), "no Playwright/e2e indicators found"),
}

docs_coverage = {
    "status": "present" if inventory["docs_locations"] else "missing",
    "locations": inventory["docs_locations"],
    "confidence": 0.9 if inventory["docs_locations"] else 0.0,
    "evidence": inventory["docs_locations"] or ["no README or docs/ files found"],
}
test_coverage = {
    "status": "present" if inventory["test_locations"] else "missing",
    "locations": inventory["test_locations"],
    "confidence": 0.9 if inventory["test_locations"] else 0.0,
    "evidence": inventory["test_locations"] or ["no tests/ or test-named files found"],
}
report = {
    "version": 1,
    "repo_root": repo_root,
    "facts": facts,
    "indicators": indicators,
    "inventory": inventory,
    "coverage": {"docs": docs_coverage, "tests": test_coverage},
    "api_surface": api_surface,
    "ui_surface": ui_surface,
}

write_json(os.path.join(state_dir, "repository-inventory.json"), inventory)
write_json(os.path.join(state_dir, "docs-coverage-map.json"), docs_coverage)
write_json(os.path.join(state_dir, "test-coverage-map.json"), test_coverage)
write_json(os.path.join(state_dir, "api-surface.json"), api_surface)
write_json(os.path.join(state_dir, "ui-surface.json"), ui_surface)
write_json(os.path.join(reports_dir, "metadata-discovery.json"), report)
write_yaml_registry(os.path.join(state_dir, "technology-registry.yml"), facts, indicators)

with open(os.path.join(state_dir, "product-purpose.md"), "w", encoding="utf-8") as fh:
    fh.write("# Product Purpose\n\n")
    fh.write(f"{facts['product_purpose']['value']}\n\n")
    fh.write(f"Confidence: {facts['product_purpose']['confidence']}\n")
    fh.write("Evidence:\n")
    for item in facts["product_purpose"]["evidence"]:
        fh.write(f"- {item}\n")

lines = [
    "# Metadata Discovery",
    "",
    "Status: **PASS**",
    "",
    f"- Repository: `{facts['repo_name']['value']}`",
    f"- Default branch: `{facts['default_branch']['value']}`",
    f"- Languages: {', '.join(facts['languages']['value']) if facts['languages']['value'] else 'unknown'}",
    f"- Package managers: {', '.join(facts['package_managers']['value']) if facts['package_managers']['value'] else 'none detected'}",
    f"- Documentation coverage: {docs_coverage['status']}",
    f"- Test coverage: {test_coverage['status']}",
    "",
    "## Indicators",
    "",
    "| Indicator | Present | Confidence | Evidence |",
    "| --- | --- | ---: | --- |",
]
for name in sorted(indicators):
    item = indicators[name]
    lines.append(f"| {name} | {str(item['value']).lower()} | {item['confidence']:.2f} | {'; '.join(item['evidence'][:4])} |")
lines.extend(["", "## Remediation Suggestions", ""])
if docs_coverage["status"] == "missing":
    lines.append("- Add a README or `docs/` entry point describing product purpose and operator workflows.")
if test_coverage["status"] == "missing":
    lines.append("- Add a `tests/` directory or recognizable test files so baseline checks can reason about coverage.")
if not indicators["playwright_e2e"]["value"]:
    lines.append("- Add Playwright/e2e evidence if UI flows are part of the baseline profile.")
if len(lines) >= 2 and lines[-1] == "## Remediation Suggestions":
    lines.append("- None.")
with open(os.path.join(reports_dir, "metadata-discovery.md"), "w", encoding="utf-8") as fh:
    fh.write("\n".join(lines))
    fh.write("\n")

print("metadata discovery: PASS")
print("reports: .autospec/reports/metadata-discovery.json, .autospec/reports/metadata-discovery.md")
PY
