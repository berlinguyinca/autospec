#!/usr/bin/env bash
# scripts/performance-workstream.sh — continuous benchmark/performance workstream helper.
# Tracks per-commit benchmark baselines, gates statistically significant
# regressions, emits auto-implement issues, and writes reproducible optimization
# before/after reports for issue #1536.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  performance-workstream.sh record-benchmark --ledger FILE --benchmark NAME --commit SHA --p50-ms N --p99-ms N --allocations N --samples N [--stddev-ms N] [--timestamp ISO]
  performance-workstream.sh gate --ledger FILE --baseline-commit SHA --candidate-commit SHA [--max-regression-pct N] [--min-z-score N] [--regressions-out FILE]
  performance-workstream.sh propose-regression-issue --regressions FILE --out DIR
  performance-workstream.sh optimization-report --before FILE --after FILE --out FILE [--max-regression-pct N]
  performance-workstream.sh fast-path-guard --metric NAME --p99-ms N [--max-ms N]
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
import math
import os
import re
import sys
from pathlib import Path


def now_iso():
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def ensure_parent(path):
    parent = Path(path).parent
    if str(parent):
        parent.mkdir(parents=True, exist_ok=True)


def append_jsonl(path, obj):
    ensure_parent(path)
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(obj, sort_keys=True, separators=(",", ":")) + "\n")


def write_jsonl(path, rows):
    ensure_parent(path)
    with open(path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def read_jsonl(path):
    rows = []
    if not path or not os.path.exists(path):
        return rows
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def nonneg_float(value, flag):
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        raise SystemExit(f"{flag} must be a non-negative number")
    if parsed < 0:
        raise SystemExit(f"{flag} must be a non-negative number")
    return parsed


def positive_float(value, flag):
    parsed = nonneg_float(value, flag)
    if parsed <= 0:
        raise SystemExit(f"{flag} must be greater than zero")
    return parsed


def nonneg_int(value, flag):
    try:
        parsed = int(str(value))
    except (TypeError, ValueError):
        raise SystemExit(f"{flag} must be a non-negative integer")
    if parsed < 0:
        raise SystemExit(f"{flag} must be a non-negative integer")
    return parsed


def positive_int(value, flag):
    parsed = nonneg_int(value, flag)
    if parsed <= 0:
        raise SystemExit(f"{flag} must be greater than zero")
    return parsed


def slug(value):
    return re.sub(r"[^A-Za-z0-9]+", "-", str(value)).strip("-").lower() or "item"


def fmt1(value):
    return f"{float(value):.1f}"


def pct_delta(base, candidate):
    base = float(base)
    candidate = float(candidate)
    if base == 0:
        return math.inf if candidate > 0 else 0.0
    return ((candidate - base) / base) * 100.0


def z_score(base, candidate):
    delta = float(candidate.get("p99_ms", 0)) - float(base.get("p99_ms", 0))
    base_sd = float(base.get("stddev_ms", 0) or 0)
    cand_sd = float(candidate.get("stddev_ms", 0) or 0)
    base_n = max(int(base.get("samples", 1) or 1), 1)
    cand_n = max(int(candidate.get("samples", 1) or 1), 1)
    stderr = math.sqrt((base_sd ** 2 / base_n) + (cand_sd ** 2 / cand_n))
    if stderr == 0:
        return math.inf if delta > 0 else 0.0
    return delta / stderr


def index_by_commit(rows, commit):
    out = {}
    for row in rows:
        if row.get("commit") == commit:
            out[row["benchmark"]] = row
    return out


def cmd_record(args):
    row = {
        "timestamp": args.timestamp or now_iso(),
        "benchmark": args.benchmark,
        "commit": args.commit,
        "p50_ms": nonneg_float(args.p50_ms, "--p50-ms"),
        "p99_ms": nonneg_float(args.p99_ms, "--p99-ms"),
        "allocations": nonneg_int(args.allocations, "--allocations"),
        "samples": positive_int(args.samples, "--samples"),
        "stddev_ms": nonneg_float(args.stddev_ms, "--stddev-ms"),
    }
    append_jsonl(args.ledger, row)
    print(
        f"recorded {row['benchmark']} commit={row['commit']} "
        f"p50={fmt1(row['p50_ms'])}ms p99={fmt1(row['p99_ms'])}ms allocations={row['allocations']}"
    )
    return 0


def regression_rows(base, cand, max_pct, min_z):
    findings = []
    benchmark = cand["benchmark"]
    for metric in ["p50_ms", "p99_ms"]:
        delta = pct_delta(base[metric], cand[metric])
        if delta > max_pct:
            z = z_score(base, cand) if metric == "p99_ms" else math.inf
            if metric != "p99_ms" or z >= min_z:
                tag = "P99_REGRESSION_SIGNIFICANT" if metric == "p99_ms" else "P50_REGRESSION"
                findings.append({
                    "tag": tag,
                    "benchmark": benchmark,
                    "metric": metric,
                    "baseline": float(base[metric]),
                    "candidate": float(cand[metric]),
                    "delta_pct": round(delta, 1),
                    "z_score": round(z, 1) if math.isfinite(z) else "inf",
                    "commit": cand.get("commit", ""),
                    "fitness": "<50ms fast-path guard" if benchmark == "execution_fast_path" else "benchmark regression gate",
                })
    alloc_delta = pct_delta(base["allocations"], cand["allocations"])
    if alloc_delta > max_pct:
        findings.append({
            "tag": "ALLOCATION_REGRESSION",
            "benchmark": benchmark,
            "metric": "allocations",
            "baseline": int(base["allocations"]),
            "candidate": int(cand["allocations"]),
            "delta_pct": round(alloc_delta, 1),
            "z_score": "n/a",
            "commit": cand.get("commit", ""),
            "fitness": "allocation regression gate",
        })
    if benchmark == "execution_fast_path" and float(cand["p99_ms"]) > 50.0:
        findings.append({
            "tag": "FAST_PATH_BUDGET_BREACH",
            "benchmark": benchmark,
            "metric": "p99_ms",
            "baseline": float(base["p99_ms"]),
            "candidate": float(cand["p99_ms"]),
            "delta_pct": round(pct_delta(base["p99_ms"], cand["p99_ms"]), 1),
            "z_score": round(z_score(base, cand), 1),
            "commit": cand.get("commit", ""),
            "fitness": "<50ms fast-path guard",
        })
    return findings


def finding_line(finding):
    metric = finding["metric"]
    if metric == "allocations":
        return (
            f"{finding['tag']}:{finding['benchmark']}:"
            f"{int(finding['baseline'])}->{int(finding['candidate'])} (+{fmt1(finding['delta_pct'])}%)"
        )
    unit = "ms"
    extra = f", z={finding['z_score']}" if finding.get("z_score") != "n/a" else ""
    return (
        f"{finding['tag']}:{finding['benchmark']}:"
        f"{fmt1(finding['baseline'])}{unit}->{fmt1(finding['candidate'])}{unit} "
        f"(+{fmt1(finding['delta_pct'])}%{extra})"
    )


def cmd_gate(args):
    rows = read_jsonl(args.ledger)
    if not rows:
        print("performance gate: no benchmark records", file=sys.stderr)
        return 2
    baseline = index_by_commit(rows, args.baseline_commit)
    candidate = index_by_commit(rows, args.candidate_commit)
    if not baseline:
        print(f"performance gate: baseline commit not found: {args.baseline_commit}", file=sys.stderr)
        return 2
    if not candidate:
        print(f"performance gate: candidate commit not found: {args.candidate_commit}", file=sys.stderr)
        return 2
    max_pct = positive_float(args.max_regression_pct, "--max-regression-pct")
    min_z = nonneg_float(args.min_z_score, "--min-z-score")
    findings = []
    for benchmark, cand in sorted(candidate.items()):
        base = baseline.get(benchmark)
        if not base:
            continue
        findings.extend(regression_rows(base, cand, max_pct, min_z))
    if args.regressions_out:
        write_jsonl(args.regressions_out, findings)
    if findings:
        for finding in findings:
            print(finding_line(finding))
        print("performance gate failed")
        return 1
    print(f"performance gate passed ({len(candidate)} benchmarks vs {args.baseline_commit})")
    return 0


def issue_header(title, labels):
    return f"# {title}\n\nLabels: {labels}\n\n"


def regression_issue(row):
    benchmark = row.get("benchmark") or "benchmark"
    metric = row.get("metric") or "metric"
    commit = row.get("commit") or "candidate"
    test_cmd = row.get("test_cmd") or "bash scripts/performance-workstream.sh gate --ledger .autospec/benchmarks/performance.jsonl --baseline-commit BASE --candidate-commit CANDIDATE"
    candidate = row.get("candidate")
    baseline = row.get("baseline")
    delta = row.get("delta_pct")
    z = row.get("z_score")
    fitness = row.get("fitness") or "benchmark regression gate"
    return issue_header(
        f"Restore `{benchmark}` `{metric}` performance regression",
        "auto-implement, performance, benchmark-regression",
    ) + f"""## Goal

Restore benchmark `{benchmark}` metric `{metric}` below regression threshold for commit `{commit}`.

## Files to read first

- `scripts/performance-workstream.sh`
- `.autospec/benchmarks/performance.jsonl`

## Dependencies

none

## Context

Offending metric: `{metric}` regressed from `{baseline}` to `{candidate}` with delta `{delta}%` and significance `{z}`. Fitness guard: {fitness}.

## Implementation outline

- Reproduce the benchmark regression for `{benchmark}` before editing code.
- Optimize only the hot path responsible for `{metric}` and keep unrelated benchmarks green.
- Include a reproducible before/after benchmark report in the PR body.

## Tests required

- [ ] `performance-workstream.sh` gate fails before the fix and passes after the fix.

## Acceptance criteria

- [ ] `{benchmark}` `{metric}` is at or below baseline `{baseline}` or under the approved regression threshold.
- [ ] Candidate commit `{commit}` passes `performance-workstream.sh` after the optimization.
- [ ] The PR body includes before/after p50, p99, and allocation counts for `{benchmark}`.

## Smoke test

### Primary smoke test (inner loop)

```bash
{test_cmd}
```
"""


def cmd_propose_issue(args):
    rows = read_jsonl(args.regressions)
    if not rows:
        print(f"{args.regressions}: no regressions", file=sys.stderr)
        return 2
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    count = 0
    used = set()
    for row in rows:
        base_name = f"{slug(row.get('benchmark', 'benchmark'))}-{slug(row.get('metric', 'metric'))}-regression"
        name = base_name
        i = 2
        while name in used:
            name = f"{base_name}-{i}"
            i += 1
        used.add(name)
        (out_dir / f"{name}.md").write_text(regression_issue(row), encoding="utf-8")
        count += 1
    print(f"wrote {count} performance regression issues into {args.out}")
    return 0


def compare_for_report(before, after, max_pct):
    findings = []
    for metric in ["p50_ms", "p99_ms", "allocations"]:
        delta = pct_delta(before[metric], after[metric])
        if delta > max_pct:
            tag_metric = metric.upper().replace("_MS", "")
            findings.append(f"{tag_metric}_REGRESSION:{after['benchmark']}:{before[metric]}->{after[metric]} (+{fmt1(delta)}%)")
    return findings


def cmd_optimization_report(args):
    before_rows = {r["benchmark"]: r for r in read_jsonl(args.before)}
    after_rows = {r["benchmark"]: r for r in read_jsonl(args.after)}
    if not before_rows or not after_rows:
        print("optimization-report: before and after inputs must be non-empty", file=sys.stderr)
        return 2
    max_pct = positive_float(args.max_regression_pct, "--max-regression-pct")
    findings = []
    for benchmark, after in sorted(after_rows.items()):
        before = before_rows.get(benchmark)
        if before:
            findings.extend(compare_for_report(before, after, max_pct))
    if findings:
        for finding in findings:
            print(finding)
        print("optimization report rejected: collateral regression detected")
        return 1
    ensure_parent(args.out)
    lines = ["# Performance optimization before/after report", "", "| Benchmark | p50_ms | p99_ms | allocations |", "|---|---:|---:|---:|"]
    for benchmark in sorted(after_rows):
        before = before_rows.get(benchmark)
        after = after_rows[benchmark]
        if not before:
            continue
        lines.append(
            f"| `{benchmark}` | {fmt1(before['p50_ms'])} -> {fmt1(after['p50_ms'])} | "
            f"{fmt1(before['p99_ms'])} -> {fmt1(after['p99_ms'])} | "
            f"{int(before['allocations'])} -> {int(after['allocations'])} |"
        )
        lines.append(f"p99_ms: {fmt1(before['p99_ms'])} -> {fmt1(after['p99_ms'])}")
    lines.extend(["", "No collateral benchmark regressions exceeded the configured threshold."])
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote optimization report {args.out}")
    return 0


def cmd_fast_path(args):
    p99 = nonneg_float(args.p99_ms, "--p99-ms")
    max_ms = positive_float(args.max_ms, "--max-ms")
    if p99 > max_ms:
        print(f"FAST_PATH_BUDGET_BREACH:{args.metric}:{fmt1(p99)}ms>{fmt1(max_ms)}ms")
        return 1
    print(f"fast-path guard passed: {args.metric} p99={fmt1(p99)}ms <= {fmt1(max_ms)}ms")
    return 0


parser = argparse.ArgumentParser(prog="performance-workstream.sh")
sub = parser.add_subparsers(dest="cmd", required=True)

p = sub.add_parser("record-benchmark")
p.add_argument("--ledger", required=True)
p.add_argument("--benchmark", required=True)
p.add_argument("--commit", required=True)
p.add_argument("--p50-ms", required=True)
p.add_argument("--p99-ms", required=True)
p.add_argument("--allocations", required=True)
p.add_argument("--samples", required=True)
p.add_argument("--stddev-ms", default="0")
p.add_argument("--timestamp")
p.set_defaults(func=cmd_record)

p = sub.add_parser("gate")
p.add_argument("--ledger", required=True)
p.add_argument("--baseline-commit", required=True)
p.add_argument("--candidate-commit", required=True)
p.add_argument("--max-regression-pct", default="10")
p.add_argument("--min-z-score", default="2")
p.add_argument("--regressions-out")
p.set_defaults(func=cmd_gate)

p = sub.add_parser("propose-regression-issue")
p.add_argument("--regressions", required=True)
p.add_argument("--out", required=True)
p.set_defaults(func=cmd_propose_issue)

p = sub.add_parser("optimization-report")
p.add_argument("--before", required=True)
p.add_argument("--after", required=True)
p.add_argument("--out", required=True)
p.add_argument("--max-regression-pct", default="10")
p.set_defaults(func=cmd_optimization_report)

p = sub.add_parser("fast-path-guard")
p.add_argument("--metric", required=True)
p.add_argument("--p99-ms", required=True)
p.add_argument("--max-ms", default="50")
p.set_defaults(func=cmd_fast_path)

args = parser.parse_args()
raise SystemExit(args.func(args))
PY
