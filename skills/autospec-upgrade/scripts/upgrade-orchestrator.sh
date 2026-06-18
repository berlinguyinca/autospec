#!/usr/bin/env bash
# upgrade-orchestrator.sh — resumable state machine for autospec-upgrade (issue #1184)
#
# Usage:
#   upgrade-orchestrator.sh [--root <dir>] [--out <autospec-dir>] [--max-fix <N>]
#
# Options:
#   --root <dir>     Project root (default: .).
#   --out <dir>      .autospec/ directory for state + artifacts (default: <root>/.autospec).
#   --max-fix <N>    Max fix-loop iterations passed to upgrade-engine.sh (default: 5).
#
# State file: <out>/upgrade-state.json
#   {"current_phase":"<phase>","last_completed_hop":"<hop>","last_green_tag":"<tag>"}
#
# Phase order (checkpoints):
#   phase0_complete  — detect done
#   phase1_complete  — behavior-lock done
#   phase2_complete  — upgrade engine (all hops) done
#   phase3_complete  — best-practice migration done
#   phase4_complete  — verify (mutation gate + qa) done
#   phase5_complete  — migration doc done
#   phase6_complete  — tags + report done (idempotent no-op on re-run)
#
# Exit codes:
#   0  — all phases complete (or already complete — idempotent)
#   1  — a phase failed; state persisted; operator action required
#   2  — argument / environment error

set -uo pipefail

# ── Argument parsing ──────────────────────────────────────────────────────────

ROOT="."
OUT_DIR=""
MAX_FIX=5

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root)
      ROOT="${2:-}"
      shift 2
      ;;
    --root=*)
      ROOT="${1#--root=}"
      shift
      ;;
    --out)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --out=*)
      OUT_DIR="${1#--out=}"
      shift
      ;;
    --max-fix)
      MAX_FIX="${2:-5}"
      shift 2
      ;;
    --max-fix=*)
      MAX_FIX="${1#--max-fix=}"
      shift
      ;;
    *)
      printf 'upgrade-orchestrator: unknown argument: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done

# ── Validate and resolve paths ────────────────────────────────────────────────

if [ ! -d "$ROOT" ]; then
  printf 'upgrade-orchestrator: root directory not found: %s\n' "$ROOT" >&2
  exit 2
fi
ROOT="$(cd "$ROOT" && pwd)"

if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$ROOT/.autospec"
fi
mkdir -p "$OUT_DIR"

STATE_FILE="$OUT_DIR/upgrade-state.json"
DETECT_JSON="$OUT_DIR/detect.json"
HOPS_JSON="$OUT_DIR/hops.json"

# ── Resolve sibling scripts ───────────────────────────────────────────────────
# AUTOSPEC_SCRIPTS_DIR env overrides; fallback to own directory.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

_resolve() {
  local name="$1"
  if [ -x "$SCRIPTS_DIR/$name" ]; then
    printf '%s' "$SCRIPTS_DIR/$name"
  elif command -v "$name" >/dev/null 2>&1; then
    printf '%s' "$name"
  else
    printf ''
  fi
}

UPGRADE_DETECT="$(_resolve upgrade-detect.sh)"
COMPUTE_STEPS="$(_resolve compute-upgrade-steps.sh)"
BEHAVIOR_LOCK="$(_resolve behavior-lock.sh)"
UPGRADE_ENGINE="$(_resolve upgrade-engine.sh)"
BEST_PRACTICE="$(_resolve best-practice-migrate.sh)"
MUTATION_GATE="$(_resolve mutation-gate.sh)"
TAG_UPGRADE="$(_resolve tag-upgrade.sh)"
MIGRATION_DOC="$(_resolve migration-doc.sh)"

# ── State helpers ─────────────────────────────────────────────────────────────

# read_phase: emit current_phase from state file, or empty string if absent/invalid
read_phase() {
  if [ ! -f "$STATE_FILE" ]; then
    printf ''
    return
  fi
  local phase
  phase="$(jq -r '.current_phase // ""' "$STATE_FILE" 2>/dev/null || printf '')"
  printf '%s' "$phase"
}

# write_state <phase> [last_completed_hop] [last_green_tag]
write_state() {
  local phase="$1"
  local hop="${2:-}"
  local tag="${3:-}"
  # Use printf to build JSON — avoids jq dependency for a simple write
  printf '{"current_phase":"%s","last_completed_hop":"%s","last_green_tag":"%s"}\n' \
    "$phase" "$hop" "$tag" > "$STATE_FILE"
}

# phase_index <phase_name>: print numeric index; "none" for unknown/absent
phase_index() {
  local p="$1"
  case "$p" in
    phase0_complete) printf '0' ;;
    phase1_complete) printf '1' ;;
    phase2_complete) printf '2' ;;
    phase3_complete) printf '3' ;;
    phase4_complete) printf '4' ;;
    phase5_complete) printf '5' ;;
    phase6_complete) printf '6' ;;
    *)               printf 'none' ;;
  esac
}

# completed_phase_index: index string of the last completed phase ("none" if not started)
completed_phase_index() {
  local current
  current="$(read_phase)"
  phase_index "$current"
}

# ── Phase runner helpers ──────────────────────────────────────────────────────

# run_script <label> <script-path> [args...]
# Runs the script; on non-zero exit prints an operator message and returns 1.
run_script() {
  local label="$1"
  shift
  local script="$1"
  shift
  if [ -z "$script" ]; then
    printf 'upgrade-orchestrator: %s — script not found (not installed?)\n' "$label" >&2
    return 1
  fi
  if ! "$script" "$@"; then
    printf 'upgrade-orchestrator: FAILED — %s\n' "$label" >&2
    printf 'upgrade-orchestrator: state preserved at %s — fix the issue and re-run to resume\n' "$STATE_FILE" >&2
    return 1
  fi
  return 0
}

# ── Read existing state ───────────────────────────────────────────────────────

DONE_IDX="$(completed_phase_index)"

# ── Idempotent no-op ─────────────────────────────────────────────────────────

if [ "$DONE_IDX" = "6" ]; then
  printf 'upgrade-orchestrator: already complete (phase6_complete) — nothing to do\n'
  exit 0
fi

# ── Phase 0: Detect ───────────────────────────────────────────────────────────

if [ "$DONE_IDX" = "none" ]; then
  printf 'upgrade-orchestrator: Phase 0 — detect\n'
  if ! run_script "upgrade-detect" "$UPGRADE_DETECT" --root "$ROOT" --out "$OUT_DIR"; then
    write_state "phase0_failed" "" ""
    exit 1
  fi
  # Compute hops while we have detection
  if ! run_script "compute-upgrade-steps" "$COMPUTE_STEPS" --root "$ROOT" --out "$OUT_DIR"; then
    write_state "phase0_failed" "" ""
    exit 1
  fi
  write_state "phase0_complete" "" ""
  DONE_IDX="0"
fi

# ── Phase 1: Behavior-lock ────────────────────────────────────────────────────

if [ "$DONE_IDX" = "0" ]; then
  printf 'upgrade-orchestrator: Phase 1 — behavior-lock\n'
  if ! run_script "behavior-lock" "$BEHAVIOR_LOCK" \
      --detect "$DETECT_JSON" --root "$ROOT" --out "$OUT_DIR"; then
    write_state "phase1_failed" "" ""
    exit 1
  fi
  write_state "phase1_complete" "" ""
  DONE_IDX="1"
fi

# ── Phase 2: Incremental upgrade loop ────────────────────────────────────────

if [ "$DONE_IDX" = "1" ]; then
  printf 'upgrade-orchestrator: Phase 2 — incremental upgrade loop\n'
  if ! run_script "upgrade-engine" "$UPGRADE_ENGINE" \
      --hops "$HOPS_JSON" --root "$ROOT" --max-fix "$MAX_FIX"; then
    write_state "phase2_failed" "" ""
    exit 1
  fi
  write_state "phase2_complete" "" ""
  DONE_IDX="2"
fi

# ── Phase 3: Best-practice migration ─────────────────────────────────────────

if [ "$DONE_IDX" = "2" ]; then
  printf 'upgrade-orchestrator: Phase 3 — best-practice migration\n'
  if ! run_script "best-practice-migrate" "$BEST_PRACTICE" \
      --detect "$DETECT_JSON" --root "$ROOT" --out "$OUT_DIR" --versions-green; then
    write_state "phase3_failed" "" ""
    exit 1
  fi
  write_state "phase3_complete" "" ""
  DONE_IDX="3"
fi

# ── Phase 4: Verify (mutation gate + qa) ─────────────────────────────────────

if [ "$DONE_IDX" = "3" ]; then
  printf 'upgrade-orchestrator: Phase 4 — verify\n'
  if ! run_script "mutation-gate" "$MUTATION_GATE" --gate --out "$OUT_DIR"; then
    printf 'upgrade-orchestrator: mutation gate failed — post-upgrade tags withheld\n' >&2
    write_state "phase4_failed" "" ""
    exit 1
  fi
  # autospec-qa --no-heal: invoked via PATH (mocked in tests; graceful skip if absent)
  if command -v autospec-qa >/dev/null 2>&1; then
    if ! autospec-qa --no-heal --root "$ROOT"; then
      printf 'upgrade-orchestrator: autospec-qa --no-heal failed\n' >&2
      write_state "phase4_failed" "" ""
      exit 1
    fi
  fi
  write_state "phase4_complete" "" ""
  DONE_IDX="4"
fi

# ── Phase 5: Document ─────────────────────────────────────────────────────────

if [ "$DONE_IDX" = "4" ]; then
  printf 'upgrade-orchestrator: Phase 5 — migration doc\n'
  if ! run_script "migration-doc" "$MIGRATION_DOC" --root "$ROOT" --out "$ROOT"; then
    write_state "phase5_failed" "" ""
    exit 1
  fi
  write_state "phase5_complete" "" ""
  DONE_IDX="5"
fi

# ── Phase 6: Tag + report ─────────────────────────────────────────────────────

if [ "$DONE_IDX" = "5" ]; then
  printf 'upgrade-orchestrator: Phase 6 — tag + report\n'
  # Determine framework from detect JSON if available
  FW="unknown"
  if [ -f "$DETECT_JSON" ]; then
    FW="$(jq -r '.frameworks[0] // "unknown"' "$DETECT_JSON" 2>/dev/null || printf 'unknown')"
  fi
  if ! run_script "tag-upgrade (report)" "$TAG_UPGRADE" report \
      --framework "$FW" --from "unknown" --to "latest" --out "$ROOT"; then
    write_state "phase6_failed" "" ""
    exit 1
  fi
  write_state "phase6_complete" "" ""
  DONE_IDX="6"
fi

printf 'upgrade-orchestrator: all phases complete\n'
exit 0
