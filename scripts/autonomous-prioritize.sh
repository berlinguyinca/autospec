#!/usr/bin/env bash
# scripts/autonomous-prioritize.sh — value-gated cross-workstream scorer.
#
# Ranks candidate JSON/JSONL records from every autonomous workstream into one
# queue using the issue #1542 WSJF-derived formula:
#   priority = (severity * value * confidence * reversibility) / (effort * blast_radius)
# Recently touched files receive a deterministic decay multiplier to avoid
# A→B→A ping-pong, and fenced/high-blast-radius candidates route to a human gate.

set -eu

usage() {
    cat <<'USAGE'
Usage:
  autonomous-prioritize.sh score --candidates FILE [options]

Options:
  --candidates FILE              Candidate JSON array or JSONL file.
  --recent-touches FILE          JSON/JSONL paths recently touched by the agent.
  --value-floor N                Minimum runnable priority score (default: 1).
  --recent-decay N               Multiplier for recently touched candidates (default: 0.5).
  --human-gate-blast-radius N    Blast-radius threshold for human gate (default: 4).
  --out FILE                     Also write the priority queue JSON to FILE.
  -h, --help                     Print this help.

Candidate fields:
  id, workstream, title, severity, value, confidence, reversibility, effort,
  blast_radius, files[], fenced/high_risk/human_gate.
USAGE
}

if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ] || [ $# -eq 0 ]; then
    usage
    exit 0
fi

cmd="$1"; shift
case "$cmd" in
    score) ;;
    *) printf 'autonomous-prioritize: unknown command: %s\n' "$cmd" >&2; usage >&2; exit 2 ;;
esac

candidates=""
recent_touches=""
value_floor="${AUTOSPEC_VALUE_FLOOR:-1}"
recent_decay="${AUTOSPEC_RECENT_DECAY:-0.5}"
human_gate_blast_radius="${AUTOSPEC_HUMAN_GATE_BLAST_RADIUS:-4}"
out=""

while [ $# -gt 0 ]; do
    case "$1" in
        --candidates) candidates="${2:-}"; shift 2 ;;
        --recent-touches) recent_touches="${2:-}"; shift 2 ;;
        --value-floor) value_floor="${2:-}"; shift 2 ;;
        --recent-decay) recent_decay="${2:-}"; shift 2 ;;
        --human-gate-blast-radius) human_gate_blast_radius="${2:-}"; shift 2 ;;
        --out) out="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'autonomous-prioritize: unknown arg: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
done

[ -n "$candidates" ] || { printf 'autonomous-prioritize: --candidates is required\n' >&2; exit 2; }
[ -f "$candidates" ] || { printf 'autonomous-prioritize: candidates file not found: %s\n' "$candidates" >&2; exit 2; }

python3 - "$candidates" "$recent_touches" "$value_floor" "$recent_decay" "$human_gate_blast_radius" "$out" <<'PY'
import json
import math
import os
import sys
from pathlib import Path

cand_path, recent_path, floor_s, decay_s, gate_s, out_path = sys.argv[1:]


def number(value, default, minimum=None):
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        parsed = float(default)
    if not math.isfinite(parsed):
        parsed = float(default)
    if minimum is not None and parsed < minimum:
        parsed = minimum
    return parsed


value_floor = number(floor_s, 1.0, 0.0)
recent_decay = number(decay_s, 0.5, 0.0)
human_gate_blast_radius = number(gate_s, 4.0, 0.0)


def read_records(path):
    if not path:
        return []
    p = Path(path)
    if not p.exists():
        return []
    raw = p.read_text(encoding="utf-8").strip()
    if not raw:
        return []
    try:
        data = json.loads(raw)
        if isinstance(data, list):
            return data
        if isinstance(data, dict):
            for key in ("candidates", "ranked", "items", "recent", "touches"):
                if isinstance(data.get(key), list):
                    return data[key]
            return [data]
    except json.JSONDecodeError:
        pass
    rows = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        rows.append(json.loads(line))
    return rows


def paths_from(row):
    values = []
    for key in ("files", "paths", "touched_files"):
        raw = row.get(key)
        if isinstance(raw, list):
            values.extend(str(x) for x in raw if x)
        elif isinstance(raw, str) and raw:
            values.append(raw)
    for key in ("file", "path"):
        raw = row.get(key)
        if isinstance(raw, str) and raw:
            values.append(raw)
    return sorted(set(values))


recent_paths = set()
for row in read_records(recent_path):
    if isinstance(row, str):
        recent_paths.add(row)
    elif isinstance(row, dict):
        recent_paths.update(paths_from(row))

ranked = []
for idx, row in enumerate(read_records(cand_path)):
    if not isinstance(row, dict):
        continue
    severity = number(row.get("severity"), 1.0, 0.0)
    value = number(row.get("value"), 1.0, 0.0)
    confidence = number(row.get("confidence"), 1.0, 0.0)
    reversibility = number(row.get("reversibility"), 1.0, 0.0)
    effort = number(row.get("effort"), 1.0, 0.000001)
    blast_radius = number(row.get("blast_radius", row.get("blastRadius")), 1.0, 0.000001)
    raw_score = (severity * value * confidence * reversibility) / (effort * blast_radius)
    files = paths_from(row)
    touched = sorted(set(files) & recent_paths)
    decay_applied = bool(touched)
    score = raw_score * recent_decay if decay_applied else raw_score
    fenced = bool(row.get("fenced") or row.get("high_risk") or row.get("human_gate"))
    route = "human_gate" if fenced or blast_radius >= human_gate_blast_radius else "run"
    enriched = dict(row)
    enriched.update({
        "id": str(row.get("id") or row.get("number") or f"candidate-{idx + 1}"),
        "workstream": str(row.get("workstream") or row.get("source") or "unknown"),
        "files": files,
        "score": round(score, 6),
        "raw_score": round(raw_score, 6),
        "decay_applied": decay_applied,
        "decay_multiplier": recent_decay if decay_applied else 1.0,
        "recently_touched_files": touched,
        "route": route,
        "below_value_floor": score < value_floor,
        "blast_radius": blast_radius,
    })
    ranked.append(enriched)

ranked.sort(key=lambda r: (-float(r.get("score", 0)), str(r.get("workstream", "")), str(r.get("id", ""))))
top = ranked[0] if ranked else None
if top is None:
    decision = "idle"
elif top.get("route") == "human_gate":
    decision = "human_gate"
elif float(top.get("score", 0)) < value_floor:
    decision = "idle"
else:
    decision = "run"

considered_and_skipped = []
for row in ranked:
    reason = ""
    if row.get("route") == "human_gate":
        reason = "human_gate"
    elif row.get("below_value_floor"):
        reason = "below_value_floor"
    elif row is not top:
        reason = "lower_score"
    if reason:
        skipped = {"id": row.get("id"), "workstream": row.get("workstream"), "score": row.get("score"), "reason": reason}
        considered_and_skipped.append(skipped)

payload = {
    "schema": "autospec-priority-queue/v1",
    "formula": "(Severity × Value × Confidence × Reversibility) / (Effort × BlastRadius)",
    "value_floor": value_floor,
    "recent_decay": recent_decay,
    "human_gate_blast_radius": human_gate_blast_radius,
    "decision": decision,
    "top": top,
    "ranked": ranked,
    "considered_and_skipped": considered_and_skipped,
}
text = json.dumps(payload, indent=2, sort_keys=True)
if out_path:
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text + "\n", encoding="utf-8")
print(text)
PY
