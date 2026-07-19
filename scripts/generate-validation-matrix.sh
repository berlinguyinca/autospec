#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  set -- .
fi

python3 - "$@" <<'PY'
import json
import re
import sys
from pathlib import Path

CATEGORIES = [
    "syntax",
    "unit",
    "integration",
    "lint-typecheck",
    "build",
    "docs",
    "smoke",
]

SCRIPT_ALIASES = {
    "syntax": ["syntax", "check:syntax"],
    "unit": ["test:unit", "unit", "test", "tests"],
    "integration": ["test:integration", "integration", "itest", "test:e2e"],
    "lint-typecheck": ["typecheck", "lint:typecheck", "lint", "check"],
    "build": ["build", "compile", "package"],
    "docs": ["docs", "docs:build", "doc", "documentation"],
    "smoke": ["smoke", "test:smoke", "smoke:test"],
}

MAKE_ALIASES = {
    "syntax": ["syntax", "check-syntax"],
    "unit": ["unit", "test", "tests"],
    "integration": ["integration", "itest"],
    "lint-typecheck": ["lint", "typecheck", "check"],
    "build": ["build", "compile", "package"],
    "docs": ["docs", "doc", "documentation"],
    "smoke": ["smoke", "smoke-test"],
}

KEYWORDS = {
    "syntax": ["syntax", "shellcheck", "bash -n", "node --check"],
    "unit": ["unit", "pytest", "go test", "mvn test", "sbt test", "testthat"],
    "integration": ["integration", "failsafe", "verify", "e2e"],
    "lint-typecheck": ["lint", "typecheck", "ruff", "mypy", "eslint", "tsc", "checkstyle"],
    "build": ["build", "compile", "package", "docker build"],
    "docs": ["docs", "documentation", "mdbook", "sphinx", "pkgdown", "roxygen"],
    "smoke": ["smoke"],
}

class RepoScan:
    def __init__(self, root: Path):
        self.root = root
        self.candidates = {category: [] for category in CATEGORIES}
        self.sequence = 0

    def add(self, category, command, source_path, source_ref, priority):
        if category not in self.candidates or not command:
            return
        self.sequence += 1
        self.candidates[category].append(
            {
                "command": command,
                "source_path": source_path,
                "source_ref": source_ref,
                "priority": priority,
                "sequence": self.sequence,
            }
        )

    def rel(self, path: Path) -> str:
        return path.relative_to(self.root).as_posix()

    def scan_package_json(self):
        path = self.root / "package.json"
        if not path.is_file():
            return
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return
        scripts = data.get("scripts") if isinstance(data, dict) else None
        if not isinstance(scripts, dict):
            return
        for category, aliases in SCRIPT_ALIASES.items():
            for alias in aliases:
                if alias in scripts:
                    self.add(category, f"npm run {alias}", "package.json", f"script:{alias}", 10)
                    break

    def scan_makefile(self):
        for name in ["Makefile", "makefile"]:
            path = self.root / name
            if not path.is_file():
                continue
            targets = set()
            for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
                match = re.match(r"^([A-Za-z0-9_.:-]+)\s*:(?![=])", line)
                if match:
                    targets.add(match.group(1))
            for category, aliases in MAKE_ALIASES.items():
                for alias in aliases:
                    if alias in targets:
                        self.add(category, f"make {alias}", name, f"target:{alias}", 20)
                        break
            break

    def scan_taskfile(self):
        for name in ["Taskfile.yml", "Taskfile.yaml"]:
            path = self.root / name
            if not path.is_file():
                continue
            text = path.read_text(encoding="utf-8", errors="ignore")
            targets = set(re.findall(r"(?m)^  ([A-Za-z0-9_.:-]+):\s*$", text))
            for category, aliases in MAKE_ALIASES.items():
                for alias in aliases:
                    if alias in targets:
                        self.add(category, f"task {alias}", name, f"target:{alias}", 15)
                        break
            break

    def scan_workflows(self):
        workflow_dir = self.root / ".github" / "workflows"
        if not workflow_dir.is_dir():
            return
        for path in sorted(workflow_dir.glob("*.y*ml")):
            last_name = None
            for line in path.read_text(encoding="utf-8", errors="ignore").splitlines():
                name_match = re.match(r"\s*-?\s*name:\s*(.+?)\s*$", line)
                if name_match:
                    last_name = name_match.group(1).strip().strip('"\'')
                run_match = re.match(r"\s*run:\s*(.+?)\s*$", line)
                if not run_match:
                    continue
                command = run_match.group(1).strip().strip('"\'')
                haystack = f"{last_name or ''} {command}".lower()
                for category, words in KEYWORDS.items():
                    if any(word in haystack for word in words):
                        self.add(
                            category,
                            command,
                            self.rel(path),
                            f"step:{last_name or command}",
                            5,
                        )

    def scan_language_manifests(self):
        if (self.root / "go.mod").is_file():
            self.add("syntax", "go test ./...", "go.mod", "manifest:module", 40)
        if (self.root / "pyproject.toml").is_file():
            text = (self.root / "pyproject.toml").read_text(encoding="utf-8", errors="ignore").lower()
            if "pytest" in text or "[tool.pytest" in text:
                self.add("unit", "python -m pytest", "pyproject.toml", "manifest:tool.pytest", 40)
            if "mypy" in text:
                self.add("lint-typecheck", "python -m mypy .", "pyproject.toml", "manifest:dependency:mypy", 40)
        if (self.root / "requirements.txt").is_file():
            text = (self.root / "requirements.txt").read_text(encoding="utf-8", errors="ignore").lower()
            if re.search(r"(^|\n)\s*ruff([=<>~!]|\s|$)", text):
                self.add("lint-typecheck", "python -m ruff check .", "requirements.txt", "manifest:dependency:ruff", 35)
            elif re.search(r"(^|\n)\s*pytest([=<>~!]|\s|$)", text):
                self.add("unit", "python -m pytest", "requirements.txt", "manifest:dependency:pytest", 45)
        if (self.root / "pom.xml").is_file():
            self.add("integration", "mvn verify", "pom.xml", "manifest:project", 40)
        if (self.root / "build.sbt").is_file():
            self.add("build", "sbt compile", "build.sbt", "manifest:ThisBuild", 40)
        if (self.root / "DESCRIPTION").is_file():
            self.add("docs", "R CMD check .", "DESCRIPTION", "manifest:Package", 40)
        if (self.root / "Dockerfile").is_file():
            self.add("smoke", "docker build .", "Dockerfile", "manifest:dockerfile", 40)

    def scan(self):
        self.scan_package_json()
        self.scan_makefile()
        self.scan_taskfile()
        self.scan_workflows()
        self.scan_language_manifests()

    def rows(self):
        repo_name = self.root.name or self.root.resolve().name
        output = []
        for rank, category in enumerate(CATEGORIES, start=1):
            choices = sorted(
                self.candidates[category],
                key=lambda item: (item["priority"], item["sequence"]),
            )
            if choices:
                chosen = choices[0]
                output.append(
                    {
                        "repo": repo_name,
                        "category": category,
                        "rank": rank,
                        "status": "found",
                        "severity": "covered",
                        "command": chosen["command"],
                        "source_path": chosen["source_path"],
                        "source_ref": chosen["source_ref"],
                    }
                )
            else:
                output.append(
                    {
                        "repo": repo_name,
                        "category": category,
                        "rank": rank,
                        "status": "missing",
                        "severity": "validation-gap",
                        "command": None,
                        "source_path": None,
                        "source_ref": None,
                    }
                )
        return output

for raw_root in sys.argv[1:]:
    root = Path(raw_root).resolve()
    scanner = RepoScan(root)
    scanner.scan()
    for row in scanner.rows():
        print(json.dumps(row, sort_keys=True))
PY
