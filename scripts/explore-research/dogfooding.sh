#!/usr/bin/env bash
# scripts/explore-research/dogfooding.sh — discovery researcher (dogfooding).
#
# Reads live ~/.autospec/ run-state, failure ledgers, heartbeats,
# explore-loop.json, run-summary.md; git log churn + revert archaeology;
# never-invoked skills/flags (dead surface).
#
# Degradation: if ${AUTOSPEC_STATE_DIR:-$HOME/.autospec} is absent or any
# artifact is missing, emits {"source":"dogfooding","proposals":[]} exit 0 —
# never hard-fails.
#
# Safety: all host-specific absolute paths are redacted to ~/‑relative form
# before any value reaches a proposal. No live-state value is interpolated
# into a jq test() regex (uses string equality / capture() instead).
#
# Cap: last 20 runs / 200 commits.
# Default weight: 0.9.
#
# Output: JSON to stdout matching schemas/autospec-explore-proposal.schema.json
# (extended contract with severity + named_consumer).

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"
MAX_RUNS=20
MAX_COMMITS=200
MAX_PROPOSALS=20   # last 20 runs window per spec

# Graceful degradation: missing state dir → empty output, exit 0.
if [ ! -d "$STATE_DIR" ]; then
    echo '{"source":"dogfooding","proposals":[]}'
    exit 0
fi

cd "$REPO_ROOT" || { echo '{"source":"dogfooding","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"dogfooding","proposals":[]}'
    exit 0
fi

# ── Gather artifacts (best-effort; missing files produce empty strings) ─────

run_summary=""
if [ -f "$STATE_DIR/run-summary.md" ]; then
    run_summary="$(head -n 100 "$STATE_DIR/run-summary.md" 2>/dev/null || true)"
fi

explore_loop=""
if [ -f "$STATE_DIR/explore-loop.json" ]; then
    explore_loop="$(cat "$STATE_DIR/explore-loop.json" 2>/dev/null || true)"
fi

# Failure ledger: collect last MAX_RUNS entries from any ledger files.
failure_tmp="$(mktemp -t dogfood-failures.XXXXXX)"
trap 'rm -f "$failure_tmp"' EXIT

for ledger_file in "$STATE_DIR"/failure-ledger*.json "$STATE_DIR"/failures*.json; do
    if [ -f "$ledger_file" ]; then
        python3 -c "
import json, sys
try:
    data = json.load(open('$ledger_file', encoding='utf-8', errors='ignore'))
    if isinstance(data, list):
        for e in data[:$MAX_RUNS]:
            print(json.dumps(e))
    elif isinstance(data, dict):
        entries = data.get('entries', data.get('failures', []))
        for e in entries[:$MAX_RUNS]:
            print(json.dumps(e))
except Exception:
    pass
" 2>/dev/null >> "$failure_tmp" || true
    fi
done

# Git churn + revert signals (capped).
git_log_tmp="$(mktemp -t dogfood-gitlog.XXXXXX)"
trap 'rm -f "$failure_tmp" "$git_log_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git log --oneline -n "$MAX_COMMITS" 2>/dev/null \
        | grep -E -i '(revert|fix.*again|re-fix|broken|rollback|undo)' \
        | head -n 40 > "$git_log_tmp" || true
fi

# Redact helper: replace STATE_DIR and HOME with ~/
HOME_DIR="$HOME"
STATE_DIR_REL="${STATE_DIR#"$HOME_DIR/"}"

export AUTOSPEC_STATE_DIR_VAL="$STATE_DIR"
export AUTOSPEC_HOME_VAL="$HOME_DIR"
export AUTOSPEC_RUN_SUMMARY="$run_summary"
export AUTOSPEC_EXPLORE_LOOP="$explore_loop"
export AUTOSPEC_MAX_PROPOSALS="$MAX_PROPOSALS"

python3 - "$failure_tmp" "$git_log_tmp" <<'PY'
import json, os, re, sys

failure_path = sys.argv[1]
git_log_path = sys.argv[2]

state_dir  = os.environ.get("AUTOSPEC_STATE_DIR_VAL", "")
home_dir   = os.environ.get("AUTOSPEC_HOME_VAL", "")
run_summary = os.environ.get("AUTOSPEC_RUN_SUMMARY", "")
explore_raw = os.environ.get("AUTOSPEC_EXPLORE_LOOP", "")
cap         = int(os.environ.get("AUTOSPEC_MAX_PROPOSALS", "20"))

proposals = []

def redact(text):
    """Replace absolute host paths with ~/‑relative equivalents."""
    if home_dir and home_dir != "~":
        text = text.replace(home_dir + "/", "~/")
        text = text.replace(home_dir, "~")
    if state_dir and state_dir != "~/.autospec":
        text = text.replace(state_dir + "/", "~/.autospec/")
        text = text.replace(state_dir, "~/.autospec")
    return text

def add(title, evidence, complexity="small", confidence=0.75,
        severity="operability", named_consumer=""):
    if len(proposals) >= cap:
        return
    proposals.append({
        "title": redact(title),
        "evidence": redact(evidence),
        "estimated_complexity": complexity,
        "confidence": confidence,
        "severity": severity,
        "named_consumer": named_consumer,
    })

# ── Failure ledger analysis ────────────────────────────────────────────────
failure_counts = {}
try:
    with open(failure_path, "r", encoding="utf-8", errors="ignore") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                entry = json.loads(line)
            except Exception:
                continue
            # Normalize: look for a 'type', 'error', 'phase', or 'message' key.
            key = (entry.get("type") or entry.get("phase") or
                   entry.get("error", "")[:60] or "unknown")
            failure_counts[key] = failure_counts.get(key, 0) + 1
except FileNotFoundError:
    pass

for key, count in sorted(failure_counts.items(), key=lambda x: -x[1]):
    if count < 2:
        continue
    add(
        f"fix: recurring dogfooding failure '{key}' seen {count}x in run ledger",
        f"Failure ledger at ~/.autospec/ records '{key}' occurring {count} times in recent runs — a systematic gap, not a one-off.",
        complexity="medium",
        confidence=0.8,
        severity="stability",
        named_consumer="/autospec-run recovery path; /autospec-loop monitor",
    )

# ── explore-loop.json: stalled or zero-proposal rounds ────────────────────
if explore_raw.strip():
    try:
        loop_data = json.loads(explore_raw)
        rounds = loop_data if isinstance(loop_data, list) else loop_data.get("rounds", [])
        zero_rounds = [r for r in rounds if isinstance(r, dict) and
                       len(r.get("proposals", [])) == 0]
        if len(zero_rounds) >= 3:
            add(
                "fix(explore): loop producing zero proposals for 3+ consecutive rounds",
                f"explore-loop.json shows {len(zero_rounds)} zero-proposal rounds — researchers may be returning empty output or the constitution gate is over-filtering.",
                complexity="medium",
                confidence=0.75,
                severity="operability",
                named_consumer="/autospec-explore loop driver; constitution gate",
            )
    except Exception:
        pass

# ── run-summary.md: repeated manual relaunch notes ────────────────────────
relaunch_count = len(re.findall(r'(relaunch|re-launch|restarted|manually resumed)',
                                run_summary, re.IGNORECASE))
if relaunch_count >= 2:
    add(
        f"fix(resilience): autospec required {relaunch_count} manual relaunches — improve silent-exit recovery",
        f"run-summary.md mentions relaunch/restart {relaunch_count} times. Cf. feedback_monitor_silent_exit memory: Phase 4 monitor exits silently on complex work; automatic relaunch is needed.",
        complexity="medium",
        confidence=0.7,
        severity="operability",
        named_consumer="/autospec-run Phase 4 monitor; /autospec-loop",
    )

# ── Git log: revert / fix-again churn ─────────────────────────────────────
try:
    with open(git_log_path, "r", encoding="utf-8", errors="ignore") as fh:
        churn_lines = [l.strip() for l in fh if l.strip()]
except FileNotFoundError:
    churn_lines = []

if len(churn_lines) >= 3:
    # Cluster by repeated keyword tokens.
    from collections import Counter
    tokens = []
    for line in churn_lines:
        # Strip commit hash prefix.
        parts = line.split(None, 1)
        msg = parts[1] if len(parts) > 1 else line
        tokens.extend(re.findall(r'[a-z][a-z0-9\-]{3,}', msg.lower()))
    common = Counter(tokens).most_common(3)
    top_token = common[0][0] if common else "unknown"
    add(
        f"fix: high revert/churn rate around '{top_token}' in git history",
        f"git log shows {len(churn_lines)} revert/fix-again commits; top recurring token: '{top_token}'. Recurring churn signals a missing invariant test or a flaky harness path.",
        complexity="medium",
        confidence=0.65,
        severity="stability",
        named_consumer="/autospec-run convergence check; mutation gate",
    )

# ── Dead surface: skills or flags that appear never invoked ───────────────
# Heuristic: skills listed in AGENTS.md but not referenced in git log last 200.
agents_path = "AGENTS.md"
if os.path.isfile(agents_path):
    try:
        agents_content = open(agents_path, encoding="utf-8", errors="ignore").read()
        skill_refs = re.findall(r'/autospec-([a-z][a-z0-9\-]+)', agents_content)
        # Read git log once (reuse churn file content is partial; use a broader check).
        import subprocess
        log_out = ""
        try:
            r = subprocess.run(
                ["git", "log", "--oneline", f"-n{200}"],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10,
            )
            log_out = r.stdout.decode("utf-8", "ignore")
        except Exception:
            pass
        for skill in set(skill_refs):
            if skill not in log_out and len(proposals) < cap:
                # Don't report "common" skills that appear everywhere.
                if skill in ("run", "define", "explore", "review", "stop"):
                    continue
                add(
                    f"chore: verify /autospec-{skill} is still reachable / not dead surface",
                    f"AGENTS.md documents /autospec-{skill} but it does not appear in the last 200 commit messages — may be dead surface or under-documented.",
                    complexity="small",
                    confidence=0.5,
                    severity="nicety",
                    named_consumer="/autospec-explore dead-surface scan",
                )
    except Exception:
        pass

print(json.dumps({"source": "dogfooding", "proposals": proposals}))
PY
