#!/usr/bin/env bash
# scripts/test-quality-workstream.sh — continuous test-quality workstream helper.
# Tracks coverage, mutation score, flakes, survivor-mutant issue proposals, and
# read-only test-file enforcement for issue #1534.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  test-quality-workstream.sh record-metric --ledger FILE --crate NAME --coverage PCT --mutation PCT --flakes N [--timestamp ISO]
  test-quality-workstream.sh gate --ledger FILE [--min-coverage PCT] [--min-mutation PCT] [--max-flake-rate N]
  test-quality-workstream.sh propose-mutant-issue --mutants FILE --out DIR
  test-quality-workstream.sh quarantine-flake --ledger FILE --quarantine FILE --issues-dir DIR --crate NAME --test NAME --reason TEXT [--timestamp ISO]
  test-quality-workstream.sh lock-tests --repo-root DIR --paths PATH[,PATH...]
  test-quality-workstream.sh check-readonly --repo-root DIR --paths PATH[,PATH...]
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
import stat
import sys
from pathlib import Path


def now_iso():
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def ensure_parent(path):
    Path(path).parent.mkdir(parents=True, exist_ok=True)


def append_jsonl(path, obj):
    ensure_parent(path)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(obj, sort_keys=True, separators=(",", ":")) + "\n")


def read_jsonl(path):
    rows = []
    if not path or not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
    return rows


def pct(value, flag):
    try:
        parsed = int(str(value).rstrip("%"))
    except ValueError:
        raise SystemExit(f"{flag} must be an integer percentage")
    if parsed < 0 or parsed > 100:
        raise SystemExit(f"{flag} must be between 0 and 100")
    return parsed


def nonneg_int(value, flag):
    try:
        parsed = int(str(value))
    except ValueError:
        raise SystemExit(f"{flag} must be a non-negative integer")
    if parsed < 0:
        raise SystemExit(f"{flag} must be a non-negative integer")
    return parsed


def slug(value):
    value = re.sub(r"[^A-Za-z0-9]+", "-", value).strip("-").lower()
    return value or "item"


def cmd_record(args):
    row = {
        "timestamp": args.timestamp or now_iso(),
        "crate": args.crate,
        "coverage": pct(args.coverage, "--coverage"),
        "mutation": pct(args.mutation, "--mutation"),
        "flakes": nonneg_int(args.flakes, "--flakes"),
    }
    append_jsonl(args.ledger, row)
    print(f"recorded {row['crate']} coverage={row['coverage']}% mutation={row['mutation']}% flakes={row['flakes']}")
    return 0


def cmd_gate(args):
    rows = read_jsonl(args.ledger)
    if not rows:
        print("test-quality gate: no metrics recorded", file=sys.stderr)
        return 2
    min_cov = pct(args.min_coverage, "--min-coverage")
    min_mut = pct(args.min_mutation, "--min-mutation")
    max_flakes = nonneg_int(args.max_flake_rate, "--max-flake-rate")
    by_crate = {}
    for row in rows:
        by_crate.setdefault(row["crate"], []).append(row)
    findings = []
    for crate, crate_rows in sorted(by_crate.items()):
        crate_rows.sort(key=lambda r: r.get("timestamp", ""))
        quality_rows = [r for r in crate_rows if isinstance(r.get("coverage"), int) and isinstance(r.get("mutation"), int)]
        flake_rows = [r for r in crate_rows if isinstance(r.get("flakes"), int)]
        latest_quality = quality_rows[-1] if quality_rows else None
        previous_quality = quality_rows[-2] if len(quality_rows) > 1 else None
        latest_flake = flake_rows[-1] if flake_rows else None
        previous_flake = flake_rows[-2] if len(flake_rows) > 1 else None
        if latest_quality:
            if latest_quality["coverage"] < min_cov:
                findings.append(f"COVERAGE_BELOW_FLOOR:{crate}:{latest_quality['coverage']}%<{min_cov}%")
            if latest_quality["mutation"] < min_mut:
                findings.append(f"MUTATION_BELOW_FLOOR:{crate}:{latest_quality['mutation']}%<{min_mut}%")
        if latest_flake and latest_flake["flakes"] > max_flakes:
            findings.append(f"FLAKE_RATE_ABOVE_FLOOR:{crate}:{latest_flake['flakes']}>{max_flakes}")
        if previous_quality and latest_quality:
            if latest_quality["coverage"] < previous_quality["coverage"]:
                findings.append(f"COVERAGE_REGRESSION:{crate}:{previous_quality['coverage']}%->{latest_quality['coverage']}%")
            if latest_quality["mutation"] < previous_quality["mutation"]:
                findings.append(f"MUTATION_REGRESSION:{crate}:{previous_quality['mutation']}%->{latest_quality['mutation']}%")
        if previous_flake and latest_flake and latest_flake["flakes"] > previous_flake["flakes"]:
            findings.append(f"FLAKE_RATE_REGRESSION:{crate}:{previous_flake['flakes']}->{latest_flake['flakes']}")
    if findings:
        for finding in findings:
            print(finding)
        print("test-quality gate failed")
        return 1
    print(f"test-quality gate passed ({len(by_crate)} crates)")
    return 0


def read_mutants(path):
    rows = []
    for row in read_jsonl(path):
        rows.append(row)
    return rows


def mutant_issue_body(row):
    crate = row.get("crate", "unknown-crate")
    file_path = row.get("file", "unknown-file")
    mutant = row.get("mutant", row.get("description", "surviving mutant"))
    test_cmd = row.get("test", "<add focused regression test command>")
    return f"""# Kill surviving mutant in `{crate}`

Labels: auto-implement, test-quality, mutation-testing

## Goal

Add a regression test that kills the surviving mutant in `{file_path}`.

## Files to read first

- `{file_path}`
- The nearest existing test file for `{crate}`.

## Dependencies

none

## Implementation outline

- Surviving mutant: `{mutant}`.
- Add or strengthen a focused test for the surviving mutant `{mutant}`.
- Verified red: run `{test_cmd}` with the mutant present and confirm failure.
- Verified green: restore the implementation and confirm `{test_cmd}` passes.
- Keep existing assertions intact; do not weaken or delete test expectations.
- Restore the intended implementation after proving the mutant-specific test fails.

## Tests required

- [ ] `{test_cmd}` verifies the red/green mutant-killing loop.

## Acceptance criteria

- [ ] `{file_path}` has a focused regression test for surviving mutant `{mutant}`.
- [ ] `{test_cmd}` fails while the mutant is present and passes after restore.
- [ ] No existing assertion is weakened or deleted under `tests/`.

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""


def cmd_propose(args):
    rows = read_mutants(args.mutants)
    if not rows:
        print("propose-mutant-issue: no surviving mutants found", file=sys.stderr)
        return 2
    Path(args.out).mkdir(parents=True, exist_ok=True)
    seen = {}
    for index, row in enumerate(rows, start=1):
        base = slug(f"{row.get('crate','crate')}-{row.get('file','file')}")
        seen[base] = seen.get(base, 0) + 1
        unique = base if seen[base] == 1 else f"{base}-{seen[base]}"
        if row.get("id") or row.get("line"):
            unique = slug(f"{base}-{row.get('id', row.get('line'))}")
        file_name = unique + ".md"
        out = Path(args.out) / file_name
        out.write_text(mutant_issue_body(row), encoding="utf-8")
        print(f"wrote {out}")
    return 0


def cmd_quarantine(args):
    timestamp = args.timestamp or now_iso()
    metric = {"timestamp": timestamp, "crate": args.crate, "type": "flake", "flakes": 1}
    append_jsonl(args.ledger, metric)
    qrow = {"timestamp": timestamp, "crate": args.crate, "test": args.test, "reason": args.reason}
    append_jsonl(args.quarantine, qrow)
    Path(args.issues_dir).mkdir(parents=True, exist_ok=True)
    issue_path = Path(args.issues_dir) / (slug(f"{args.crate}-{args.test}") + ".md")
    issue_path.write_text(f"""# Harden flaky test `{args.test}`

Labels: auto-implement, hardening, test-quality, flake

## Goal

Stabilize `{args.test}` without weakening its assertions.

## Files to read first

- The test file containing `{args.test}`.
- The production code exercised by `{args.test}`.

## Dependencies

none

## Context

Quarantine reason: {args.reason}

## Implementation outline

- Reproduce the flake with a retry loop before changing code.
- Fix nondeterminism in production code or test setup while preserving assertions.
- Remove the quarantine entry after the retry loop is stable.

## Tests required

- [ ] `cargo test -p {args.crate} {args.test}` passes repeatedly after the fix.

## Acceptance criteria

- [ ] `{args.test}` is reproduced as flaky before implementation work starts.
- [ ] `{args.test}` passes repeatedly after the nondeterministic path is fixed.
- [ ] No assertion in `{args.test}` is weakened or deleted.

## Smoke test

### Primary smoke test (inner loop)

```bash
cargo test -p {args.crate} {args.test}
```
""", encoding="utf-8")
    print(f"quarantined {args.test}; wrote {issue_path}")
    return 0


def iter_test_files(repo_root, paths_csv):
    root = Path(repo_root)
    for raw in paths_csv.split(','):
        raw = raw.strip()
        if not raw:
            continue
        start = root / raw
        if start.is_file():
            yield start
        elif start.is_dir():
            for path in start.rglob('*'):
                if path.is_file():
                    yield path


def rel(path, root):
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def cmd_lock_tests(args):
    root = Path(args.repo_root)
    count = 0
    for path in iter_test_files(args.repo_root, args.paths):
        mode = path.stat().st_mode
        path.chmod(mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))
        count += 1
    print(f"locked {count} test files read-only")
    return 0


def cmd_check_readonly(args):
    root = Path(args.repo_root)
    findings = []
    count = 0
    for path in iter_test_files(args.repo_root, args.paths):
        count += 1
        mode = path.stat().st_mode
        if mode & (stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH):
            findings.append(f"TEST_FILE_WRITABLE:{rel(path, root)}")
    if findings:
        for finding in findings:
            print(finding)
        print("test read-only gate failed")
        return 1
    print(f"test files read-only ({count} files)")
    return 0


def parser():
    p = argparse.ArgumentParser(prog="test-quality-workstream.sh")
    sub = p.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("record-metric")
    r.add_argument("--ledger", required=True)
    r.add_argument("--crate", required=True)
    r.add_argument("--coverage", required=True)
    r.add_argument("--mutation", required=True)
    r.add_argument("--flakes", required=True)
    r.add_argument("--timestamp")
    r.set_defaults(fn=cmd_record)

    g = sub.add_parser("gate")
    g.add_argument("--ledger", required=True)
    g.add_argument("--min-coverage", default="90")
    g.add_argument("--min-mutation", default="80")
    g.add_argument("--max-flake-rate", default="0")
    g.set_defaults(fn=cmd_gate)

    m = sub.add_parser("propose-mutant-issue")
    m.add_argument("--mutants", required=True)
    m.add_argument("--out", required=True)
    m.set_defaults(fn=cmd_propose)

    q = sub.add_parser("quarantine-flake")
    q.add_argument("--ledger", required=True)
    q.add_argument("--quarantine", required=True)
    q.add_argument("--issues-dir", required=True)
    q.add_argument("--crate", required=True)
    q.add_argument("--test", required=True)
    q.add_argument("--reason", required=True)
    q.add_argument("--timestamp")
    q.set_defaults(fn=cmd_quarantine)

    for name, fn in (("lock-tests", cmd_lock_tests), ("check-readonly", cmd_check_readonly)):
        s = sub.add_parser(name)
        s.add_argument("--repo-root", required=True)
        s.add_argument("--paths", required=True)
        s.set_defaults(fn=fn)
    return p


def main(argv):
    args = parser().parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
PY
