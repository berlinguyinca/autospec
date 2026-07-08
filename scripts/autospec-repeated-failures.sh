#!/usr/bin/env bash
# scripts/autospec-repeated-failures.sh — detect repeated local autonomy failures.

set -eu

REPO_ROOT="$(pwd)"
THRESHOLD=2
while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --threshold) THRESHOLD="$2"; shift 2 ;;
        -h|--help) echo "Usage: autospec-repeated-failures.sh [--repo-root DIR] [--threshold N]"; exit 0 ;;
        *) printf 'autospec-repeated-failures: unknown arg: %s\n' "$1" >&2; exit 2 ;;
    esac
done

REPO_ROOT="$(cd "$REPO_ROOT" && pwd -P)"

python3 - "$REPO_ROOT" "$THRESHOLD" <<'PY'
import collections, glob, json, os, sys

root, threshold = os.path.realpath(sys.argv[1]), int(sys.argv[2])
reports = os.path.join(root, ".autospec", "reports")
state = os.path.join(root, ".autospec", "state")

def load(path, default):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return default

def write_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=True)
        fh.write("\n")

def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text.rstrip() + "\n")

counts = collections.Counter()
evidence = collections.defaultdict(list)
for path in sorted(glob.glob(os.path.join(state, "verifications", "*.json"))):
    data = load(path, {})
    for dim in data.get("dimensions", []):
        if dim.get("status") in {"fail", "warn"}:
            key = ("verifier_finding", dim.get("dimension", "unknown"), dim.get("summary", ""))
            counts[key] += 1
            evidence[key].append(os.path.relpath(path, root))

handover = load(os.path.join(state, "stuck-handovers.json"), {"handovers": []})
for item in handover.get("handovers", []):
    if item.get("state") in {"needs-guidance", "stuck"}:
        key = ("stuck_issue", str(item.get("source_issue_number") or item.get("work_item_id")), item.get("state"))
        counts[key] += 1
        evidence[key].append("state/stuck-handovers.json")

repeated = []
for key, count in sorted(counts.items(), key=lambda kv: (kv[0], kv[1])):
    if count >= threshold:
        repeated.append({"kind": key[0], "subject": key[1], "summary": key[2], "count": count, "threshold": threshold, "evidence": evidence[key]})

report = {"version": 1, "threshold": threshold, "has_repeated_failures": bool(repeated), "repeated_failures": repeated}
write_json(os.path.join(reports, "repeated-failures.json"), report)
md = ["# Repeated Failures", "", f"Repeated failures: **{str(bool(repeated)).lower()}**", "", "| Kind | Subject | Count | Evidence |", "| --- | --- | ---: | --- |"]
if repeated:
    for item in repeated:
        md.append(f"| {item['kind']} | {item['subject']} | {item['count']} | {', '.join(item['evidence'])} |")
else:
    md.append("| none | none | 0 | none |")
write_text(os.path.join(reports, "repeated-failures.md"), "\n".join(md))
print("repeated failures: FOUND" if repeated else "repeated failures: PASS")
sys.exit(1 if repeated else 0)
PY
