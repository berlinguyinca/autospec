#!/usr/bin/env bash
# scripts/technical-debt-workstream.sh — continuous debt/dead-code/CVE workstream helper.
# Ranks churn×complexity hotspots, proposes safe dead-code removals, and records
# dependency advisory scans for issue #1535.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  technical-debt-workstream.sh rank-hotspots --churn FILE --complexity FILE [--duplicates FILE] --out FILE [--limit N]
  technical-debt-workstream.sh propose-refactor-issue --hotspots FILE --out DIR --test-cmd CMD
  technical-debt-workstream.sh propose-dead-code-removal --symbols FILE --out DIR --test-cmd CMD
  technical-debt-workstream.sh scan-advisories --advisories FILE --ledger FILE --out DIR [--timestamp ISO]
USAGE
}

if [ "${1:-}" = "--help" ] || [ $# -eq 0 ]; then
    usage
    exit 0
fi

python3 - "$@" <<'PY'
import argparse
import datetime as dt
import json
import os
import re
import sys
from pathlib import Path


def now_iso():
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def ensure_parent(path):
    Path(path).parent.mkdir(parents=True, exist_ok=True)


def read_jsonl(path):
    rows = []
    if not path or not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        for line_no, line in enumerate(fh, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as exc:
                raise SystemExit(f"{path}:{line_no}: invalid JSONL: {exc}")
    return rows


def write_jsonl(path, rows):
    ensure_parent(path)
    with open(path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def append_jsonl(path, row):
    ensure_parent(path)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def slug(value):
    value = re.sub(r"[^A-Za-z0-9]+", "-", value).strip("-").lower()
    return value or "item"


def num(value, default=0):
    if value is None or value == "":
        return default
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return default
    if parsed.is_integer():
        return int(parsed)
    return parsed


def path_token(path):
    return path or "unknown-file"


def cmd_rank_hotspots(args):
    churn = {}
    for row in read_jsonl(args.churn):
        file_path = row.get("file") or row.get("path")
        if file_path:
            churn[file_path] = num(row.get("commits", row.get("churn", row.get("changes"))), 0)
    complexity = {}
    for row in read_jsonl(args.complexity):
        file_path = row.get("file") or row.get("path")
        if file_path:
            complexity[file_path] = num(row.get("complexity", row.get("cyclomatic", row.get("score"))), 0)
    duplicates = {}
    dup_cluster = {}
    for row in read_jsonl(args.duplicates or ""):
        file_path = row.get("file") or row.get("path")
        if file_path:
            duplicates[file_path] = num(row.get("duplicate_lines", row.get("lines", row.get("duplicates"))), 0)
            if row.get("cluster"):
                dup_cluster[file_path] = str(row["cluster"])

    rows = []
    for file_path in sorted(set(churn) | set(complexity)):
        c = churn.get(file_path, 0)
        cx = complexity.get(file_path, 0)
        dup = duplicates.get(file_path, 0)
        score = c * cx
        evidence = f"churn×complexity={score} (churn={c}, complexity={cx})"
        if dup:
            evidence += f"; duplicate_lines={dup}"
            if dup_cluster.get(file_path):
                evidence += f" cluster={dup_cluster[file_path]}"
        rows.append({
            "file": file_path,
            "score": score,
            "churn": c,
            "complexity": cx,
            "duplicate_lines": dup,
            "evidence": evidence,
        })
    rows.sort(key=lambda r: (-r["score"], -r["duplicate_lines"], r["file"]))
    limit = int(args.limit)
    rows = rows[:limit]
    write_jsonl(args.out, rows)
    print(f"ranked {len(rows)} hotspots into {args.out}")
    return 0


def read_first_jsonl(path):
    rows = read_jsonl(path)
    if not rows:
        raise SystemExit(f"{path}: no rows")
    return rows[0]


def issue_header(title, labels):
    return f"# {title}\n\nLabels: {labels}\n\n"


def refactor_issue(row, test_cmd):
    file_path = path_token(row.get("file"))
    score = row.get("score", 0)
    churn = row.get("churn", 0)
    complexity = row.get("complexity", 0)
    evidence = row.get("evidence") or f"churn×complexity={score}"
    return issue_header(f"Refactor churn×complexity hotspot `{file_path}`", "auto-implement, technical-debt, refactor") + f"""## Goal

Reduce the churn×complexity hotspot in `{file_path}` without changing behavior.

## Files to read first

- `{file_path}`
- The nearest tests that exercise `{file_path}`.

## Dependencies

none

## Context

Debt ranking evidence: {evidence}. The top item becomes a verified refactor PR, not a blind rewrite.

## Implementation outline

- Preserve public behavior while reducing local complexity or duplication in `{file_path}`.
- Prefer deleting duplicate branches and reusing existing helpers over adding abstractions.
- Keep the diff scoped to `{file_path}` and directly related tests unless evidence requires otherwise.

## Tests required

- [ ] `{test_cmd}` proves the refactor keeps behavior unchanged.

## Acceptance criteria

- [ ] `{file_path}` has lower local debt than score `{score}` with churn `{churn}` and complexity `{complexity}`.
- [ ] `{test_cmd}` passes after the verified refactor PR.
- [ ] The PR body cites the before/after debt evidence for `{file_path}`.

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""


def cmd_propose_refactor(args):
    row = read_first_jsonl(args.hotspots)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f"{slug(row.get('file', 'hotspot'))}-refactor.md"
    out.write_text(refactor_issue(row, args.test_cmd), encoding="utf-8")
    print(f"wrote {out}")
    return 0


def production_refs(refs):
    prod = []
    for ref in refs or []:
        ref = str(ref)
        normalized = ref.lstrip("./")
        if normalized.startswith(("tests/", "test/")) or "/tests/" in normalized or normalized.endswith("_test.rs"):
            continue
        prod.append(ref)
    return prod


def dead_code_issue(row, test_cmd):
    file_path = path_token(row.get("file"))
    symbol = row.get("symbol") or row.get("name") or "unused_symbol"
    refs = row.get("referenced_by") or []
    refs_text = ", ".join(f"`{r}`" for r in refs) if refs else "no references"
    return issue_header(f"Remove test-only dead code `{symbol}`", "auto-implement, technical-debt, dead-code") + f"""## Goal

Remove test-only dead code `{symbol}` from `{file_path}` after proving no production references remain.

## Files to read first

- `{file_path}`
- `tests/`

## Dependencies

none

## Context

Dead-code analysis found `{symbol}` referenced only from tests: {refs_text}. Analysis proposes; removal requires this verified PR.

## Implementation outline

- Confirm `{symbol}` has no production references before deleting it from `{file_path}`.
- Remove or update tests that referenced `{symbol}` only as dead-code scaffolding.
- Do not delete runtime code without a green verification command.

## Tests required

- [ ] `{test_cmd}` proves the removal keeps the workspace green.

## Acceptance criteria

- [ ] `{symbol}` is absent from `{file_path}` after the safe removal PR.
- [ ] `{test_cmd}` passes after removing the test-only reference from `tests/`.
- [ ] The PR cites the dead-code report evidence for `{symbol}`.

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""


def cmd_dead_code(args):
    rows = read_jsonl(args.symbols)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    wrote = 0
    for row in rows:
        refs = row.get("referenced_by") or []
        if production_refs(refs):
            continue
        symbol = row.get("symbol") or row.get("name") or "unused_symbol"
        file_path = row.get("file") or "unknown-file"
        out = out_dir / f"{slug(file_path)}-{slug(symbol)}-removal.md"
        out.write_text(dead_code_issue(row, args.test_cmd), encoding="utf-8")
        wrote += 1
    print(f"wrote {wrote} dead-code removal issues")
    return 0


def advisory_priority(row):
    active = bool(row.get("active", False))
    fixed = bool(row.get("fixed") or row.get("fixed_version") or row.get("patched"))
    cvss = float(num(row.get("cvss", row.get("score")), 0))
    if active and fixed and cvss >= 9:
        return 1
    if fixed and cvss >= 7:
        return 2
    return 3


def advisory_issue(row, priority):
    cve = row.get("id") or row.get("advisory") or "CVE-UNKNOWN"
    pkg = row.get("package") or row.get("pkg") or "unknown-package"
    current = row.get("current") or row.get("version") or "unknown"
    fixed = row.get("fixed") or row.get("fixed_version") or "latest patched version"
    cvss = row.get("cvss", row.get("score", 0))
    active = bool(row.get("active", False))
    manifest = row.get("manifest") or row.get("file") or "dependency manifest"
    title = row.get("title") or "dependency advisory"
    active_text = "active exploit" if active else "no active exploit flag"
    test_cmd = row.get("test_cmd") or "autospec validate"
    return issue_header(f"Patch {cve} in `{pkg}`", "auto-implement, dependency, cve, security") + f"""## Goal

Patch `{pkg}` for `{cve}` in `{manifest}` to the fixed version `{fixed}`.

## Files to read first

- `{manifest}`
- `Cargo.lock`
- `package-lock.json`

## Dependencies

none

## Context

Advisory scan priority P{priority}: {title}; CVSS {cvss}; {active_text}; current `{current}`; fixed `{fixed}`.

## Implementation outline

- Update `{pkg}` from `{current}` to `{fixed}` or the nearest compatible patched version.
- Keep lockfile changes limited to the dependency solver output for `{pkg}`.
- If no compatible patch exists, document the blocker and file a mitigation follow-up.

## Tests required

- [ ] `{test_cmd}` proves the dependency patch keeps fitness functions green.

## Acceptance criteria

- [ ] `{manifest}` no longer resolves `{pkg}` at vulnerable version `{current}`.
- [ ] `{test_cmd}` passes after the `{cve}` dependency patch.
- [ ] The PR cites CVSS `{cvss}` and the advisory id `{cve}`.

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""


def cmd_scan_advisories(args):
    timestamp = args.timestamp or now_iso()
    rows = read_jsonl(args.advisories)
    prioritized = []
    for row in rows:
        item = dict(row)
        item["priority"] = advisory_priority(item)
        prioritized.append(item)
    prioritized.sort(key=lambda r: (r["priority"], -float(num(r.get("cvss", r.get("score")), 0)), str(r.get("id", ""))))
    append_jsonl(args.ledger, {"timestamp": timestamp, "findings": len(rows), "prioritized": len(prioritized)})
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    for row in prioritized:
        cve = row.get("id") or row.get("advisory") or "CVE-UNKNOWN"
        pkg = row.get("package") or row.get("pkg") or "unknown-package"
        out = out_dir / f"p{row['priority']}-{slug(cve)}-{slug(pkg)}.md"
        out.write_text(advisory_issue(row, row["priority"]), encoding="utf-8")
    print(f"recorded advisory scan findings={len(rows)} issues={len(prioritized)}")
    return 0


def parser():
    p = argparse.ArgumentParser(prog="technical-debt-workstream.sh")
    sub = p.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("rank-hotspots")
    r.add_argument("--churn", required=True)
    r.add_argument("--complexity", required=True)
    r.add_argument("--duplicates")
    r.add_argument("--out", required=True)
    r.add_argument("--limit", default="10")
    r.set_defaults(fn=cmd_rank_hotspots)

    pr = sub.add_parser("propose-refactor-issue")
    pr.add_argument("--hotspots", required=True)
    pr.add_argument("--out", required=True)
    pr.add_argument("--test-cmd", required=True)
    pr.set_defaults(fn=cmd_propose_refactor)

    dc = sub.add_parser("propose-dead-code-removal")
    dc.add_argument("--symbols", required=True)
    dc.add_argument("--out", required=True)
    dc.add_argument("--test-cmd", required=True)
    dc.set_defaults(fn=cmd_dead_code)

    adv = sub.add_parser("scan-advisories")
    adv.add_argument("--advisories", required=True)
    adv.add_argument("--ledger", required=True)
    adv.add_argument("--out", required=True)
    adv.add_argument("--timestamp")
    adv.set_defaults(fn=cmd_scan_advisories)
    return p


def main(argv):
    args = parser().parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
PY
