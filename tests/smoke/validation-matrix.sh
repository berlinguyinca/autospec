#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

"$ROOT/scripts/generate-validation-matrix.sh" \
  "$ROOT/tests/fixtures/validation-matrix/js-app" \
  "$ROOT/tests/fixtures/validation-matrix/make-task" \
  "$ROOT/tests/fixtures/validation-matrix/polyglot" \
  "$ROOT/tests/fixtures/validation-matrix/empty" > "$OUT"

python3 - "$OUT" <<'PY'
import json
import sys
from collections import defaultdict

path = sys.argv[1]
rows = []
with open(path, encoding="utf-8") as fh:
    for line in fh:
        line=line.strip()
        if line:
            rows.append(json.loads(line))

required_categories = {"syntax", "unit", "integration", "lint-typecheck", "build", "docs", "smoke"}
by_repo = defaultdict(list)
for row in rows:
    by_repo[row["repo"]].append(row)

expected_repos = {"js-app", "make-task", "polyglot", "empty"}
assert set(by_repo) == expected_repos, set(by_repo)
for repo, repo_rows in by_repo.items():
    cats = [row["category"] for row in repo_rows]
    assert set(cats) == required_categories, (repo, cats)
    assert len(repo_rows) >= 7, (repo, len(repo_rows))
    assert [row["rank"] for row in repo_rows] == list(range(1, len(repo_rows) + 1)), repo
    for row in repo_rows:
        assert row["severity"] in {"covered", "validation-gap"}, row
        if row["status"] == "found":
            assert row["command"], row
            assert row["source_path"], row
            assert row["source_ref"], row
        else:
            assert row["status"] == "missing", row
            assert row["severity"] == "validation-gap", row
            assert row["command"] is None, row
            assert row["source_path"] is None, row
            assert "feature-gap" not in json.dumps(row), row

js = {row["category"]: row for row in by_repo["js-app"]}
assert js["unit"]["command"] == "npm run test:unit", js["unit"]
assert js["lint-typecheck"]["source_path"] == "package.json", js["lint-typecheck"]
assert js["smoke"]["source_path"] == ".github/workflows/ci.yml", js["smoke"]

make_task = {row["category"]: row for row in by_repo["make-task"]}
assert make_task["build"]["source_path"] == "Makefile", make_task["build"]
assert make_task["docs"]["source_path"] == "Taskfile.yml", make_task["docs"]

poly_sources = {row["source_path"] for row in by_repo["polyglot"] if row["status"] == "found"}
for required in {
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "pom.xml",
    "build.sbt",
    "DESCRIPTION",
    "Dockerfile",
}:
    assert required in poly_sources, (required, sorted(poly_sources))

empty = by_repo["empty"]
assert all(row["status"] == "missing" for row in empty), empty
assert all(row["severity"] == "validation-gap" for row in empty), empty
assert all(row["command"] is None for row in empty), empty
PY
