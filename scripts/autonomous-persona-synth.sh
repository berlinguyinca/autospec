#!/usr/bin/env bash
# autonomous-persona-synth.sh — F2 persona synthesis (Tier-A LLM) + cache/cadence
#                               + edit-preservation + global-file lock.
#
# The conductor invokes a Tier-A agent that reads the F1 bundle
# (scripts/autonomous-persona-sources.sh) and synthesizes the GLOBAL operator
# persona to ~/.autospec/operator-persona.md, then applies the per-repo overlay
# to produce the effective persona, recording a per-dimension confidence note.
#
# Usage:
#   bash scripts/autonomous-persona-synth.sh [options]
#
# Options:
#   --repo-root DIR        Override repository root (default: parent of scripts/)
#   --autospec-home DIR    Override ~/.autospec home directory
#   --force                Re-synthesize even if the global persona is fresh
#   --dry-run              Report what would happen; never write
#   -h, --help             Show this help
#
# Cadence / staleness:
#   Refresh is governed by AUTOSPEC_PERSONA_REFRESH_DAYS (default 7). If the
#   global persona file exists and its mtime is younger than that window, the
#   script skips re-synthesis (exit 0) unless --force is given.
#
# Operator edits preserved:
#   Blocks wrapped in
#       <!-- operator-edited -->
#       ...
#       <!-- /operator-edited -->
#   in the existing global file are read as input and re-emitted byte-identically;
#   the synthesizer never overwrites them.
#
# Global-file lock:
#   A mkdir-based lock on ~/.autospec/.persona.lock.d is acquired before writing
#   the global file so two repos' conductors refreshing on cadence cannot corrupt
#   it. The lock is advisory and self-healing: a stale lock older than
#   AUTOSPEC_PERSONA_LOCK_STALE_MIN minutes (default 30) is reclaimed.
#
# LLM seam (mock-friendly):
#   The Tier-A synthesis is performed by persona_synth_llm(), which honors the
#   AUTOSPEC_PERSONA_SYNTH_CMD override (a subprocess that reads the global
#   sub-bundle JSON on stdin and writes synthesized markdown on stdout). Tests
#   override this seam; the deterministic scaffolding (cadence/lock/edit-preserve/
#   overlay) is asserted, the synthesized text is not.
#
# Engineering rules:
#   set -eu; if/then/fi (no one-sided && short-circuits); jq capture()/== not
#   test() with interpolated values; no RETURN traps (inline cleanup);
#   mkdir-based lock (not flock).
#
# Exit codes:
#   0  Synthesis completed, or skipped because the persona is fresh
#   1  Usage / dependency error
#   2  Lock held by a live (non-stale) holder — caller should retry later
#   3  Synthesis produced structurally invalid output

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
AUTOSPEC_HOME="${AUTOSPEC_HOME:-$HOME/.autospec}"

REFRESH_DAYS="${AUTOSPEC_PERSONA_REFRESH_DAYS:-7}"
LOCK_STALE_MIN="${AUTOSPEC_PERSONA_LOCK_STALE_MIN:-30}"

FORCE=0
DRY_RUN=0

# ---- argument parsing ----
while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    --autospec-home)
      AUTOSPEC_HOME="$2"
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      sed -n 's/^# \?//p' "$0" | head -55
      exit 0
      ;;
    *)
      echo "autonomous-persona-synth: unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# ---- dependency check ----
if ! command -v jq >/dev/null 2>&1; then
  echo "autonomous-persona-synth: jq not found — required for JSON handling" >&2
  exit 1
fi

GLOBAL_FILE="$AUTOSPEC_HOME/operator-persona.md"
EFFECTIVE_FILE="$REPO_ROOT/.autospec/operator-persona.effective.md"
LOCK_DIR="$AUTOSPEC_HOME/.persona.lock.d"
SOURCES_SH="${AUTOSPEC_PERSONA_SOURCES_CMD:-$SCRIPT_DIR/autonomous-persona-sources.sh}"

# ---------------------------------------------------------------------------
# file_mtime_epoch PATH — print mtime in epoch seconds (portable: BSD + GNU).
# ---------------------------------------------------------------------------
file_mtime_epoch() {
  local _p="$1"
  if stat -f %m "$_p" >/dev/null 2>&1; then
    stat -f %m "$_p"
  else
    stat -c %Y "$_p"
  fi
}

# ---------------------------------------------------------------------------
# is_fresh — return 0 if the global persona file is younger than REFRESH_DAYS.
# ---------------------------------------------------------------------------
is_fresh() {
  if [ ! -f "$GLOBAL_FILE" ]; then
    return 1
  fi
  local _now _mtime _age_days_sec _window_sec
  _now="$(date +%s)"
  _mtime="$(file_mtime_epoch "$GLOBAL_FILE" 2>/dev/null || echo 0)"
  _window_sec=$(( REFRESH_DAYS * 24 * 60 * 60 ))
  _age_days_sec=$(( _now - _mtime ))
  if [ "$_age_days_sec" -lt "$_window_sec" ]; then
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------------
# acquire_lock — mkdir-based lock with self-healing stale reclaim.
#   exit 2 if a live (non-stale) holder owns the lock.
# ---------------------------------------------------------------------------
acquire_lock() {
  mkdir -p "$AUTOSPEC_HOME" 2>/dev/null || true
  if mkdir "$LOCK_DIR" 2>/dev/null; then
    printf '%s\n' "$$" > "$LOCK_DIR/pid" 2>/dev/null || true
    return 0
  fi

  # Lock exists — check staleness.
  local _now _mtime _age_sec _stale_sec
  _now="$(date +%s)"
  _mtime="$(file_mtime_epoch "$LOCK_DIR" 2>/dev/null || echo 0)"
  _stale_sec=$(( LOCK_STALE_MIN * 60 ))
  _age_sec=$(( _now - _mtime ))

  if [ "$_age_sec" -ge "$_stale_sec" ]; then
    # Stale — reclaim it (advisory, self-healing).
    rm -rf "$LOCK_DIR" 2>/dev/null || true
    if mkdir "$LOCK_DIR" 2>/dev/null; then
      printf '%s\n' "$$" > "$LOCK_DIR/pid" 2>/dev/null || true
      return 0
    fi
  fi
  return 2
}

# ---------------------------------------------------------------------------
# release_lock — inline cleanup (NO RETURN trap; called explicitly).
# ---------------------------------------------------------------------------
release_lock() {
  rm -rf "$LOCK_DIR" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# extract_operator_edits OUT_FILE — copy each
#   <!-- operator-edited --> ... <!-- /operator-edited -->
# block (markers inclusive) from the existing GLOBAL_FILE to OUT_FILE, verbatim.
# Produces an empty file if none exist.
# ---------------------------------------------------------------------------
extract_operator_edits() {
  local _out="$1"
  : > "$_out"
  if [ ! -f "$GLOBAL_FILE" ]; then
    return 0
  fi
  awk '
    /<!-- operator-edited -->/      { keep=1 }
    keep                            { print }
    /<!-- \/operator-edited -->/    { keep=0 }
  ' "$GLOBAL_FILE" >> "$_out"
}

# ---------------------------------------------------------------------------
# persona_synth_llm — Tier-A synthesis seam.
#   Reads the global sub-bundle JSON on stdin; writes synthesized markdown to
#   stdout. Overridable for tests via AUTOSPEC_PERSONA_SYNTH_CMD.
# ---------------------------------------------------------------------------
persona_synth_llm() {
  if [ -n "${AUTOSPEC_PERSONA_SYNTH_CMD:-}" ]; then
    # shellcheck disable=SC2086
    eval "${AUTOSPEC_PERSONA_SYNTH_CMD}"
    return $?
  fi

  # Default real seam: hand the bundle to a Tier-A agent (claude). The agent is
  # asked to emit a structured operator-persona markdown document.
  local _bundle
  _bundle="$(cat)"
  local _prompt
  _prompt="$(cat <<EOF
You are a Tier-A operator-persona synthesizer for autospec. Read the JSON bundle
of persona source files below and produce a concise, hand-editable operator
persona as Markdown. The document MUST begin with a top-level heading
'# Operator persona' and use '## ' sections per dimension (e.g. Decision style,
Risk tolerance, Autonomy preferences, Quality bar, Communication). Be specific
and grounded only in the provided sources; do not invent facts.

Source bundle (JSON):
$_bundle
EOF
)"

  if command -v claude >/dev/null 2>&1; then
    claude -p "$_prompt"
  else
    echo "autonomous-persona-synth: no synthesis backend (claude not found and AUTOSPEC_PERSONA_SYNTH_CMD unset)" >&2
    return 1
  fi
}

# ---------------------------------------------------------------------------
# validate_shape FILE — structural presence check on synthesized output.
#   Asserts the document shape WITHOUT asserting exact LLM text.
# ---------------------------------------------------------------------------
validate_shape() {
  local _f="$1"
  if [ ! -s "$_f" ]; then
    return 1
  fi
  if ! grep -q '^# Operator persona' "$_f"; then
    return 1
  fi
  if ! grep -q '^## ' "$_f"; then
    return 1
  fi
  return 0
}

# ---------------------------------------------------------------------------
# confidence_note GLOBAL_BUNDLE_JSON OVERLAY_BUNDLE_JSON PERSONA_FILE
#   Emit a per-dimension confidence note derived deterministically from the
#   number of present sources. Dimensions are the '## ' headings of the
#   synthesized persona.
# ---------------------------------------------------------------------------
confidence_note() {
  local _global_json="$1"
  local _overlay_json="$2"
  local _persona_file="$3"

  local _global_n _overlay_n _total
  _global_n="$(printf '%s' "$_global_json" | jq 'length' 2>/dev/null || echo 0)"
  _overlay_n="$(printf '%s' "$_overlay_json" | jq 'length' 2>/dev/null || echo 0)"
  _total=$(( _global_n + _overlay_n ))

  local _level
  if [ "$_total" -ge 3 ]; then
    _level="high"
  elif [ "$_total" -ge 1 ]; then
    _level="medium"
  else
    _level="low"
  fi

  printf '## Confidence (per dimension)\n\n'
  printf '_Derived from %s source(s) (global=%s, overlay=%s)._\n\n' \
    "$_total" "$_global_n" "$_overlay_n"
  # One confidence line per synthesized dimension heading.
  grep '^## ' "$_persona_file" 2>/dev/null \
    | sed 's/^## //' \
    | while IFS= read -r _dim; do
        if [ "$_dim" = "Confidence (per dimension)" ]; then
          continue
        fi
        printf -- '- %s: %s\n' "$_dim" "$_level"
      done
}

issue_archetype_overlay() {
  local _id
  [ -f "$SCRIPT_DIR/persona-catalog.sh" ] || return 0
  _id="$(bash "$SCRIPT_DIR/persona-catalog.sh" select-overlay 2>/dev/null || true)"
  [ -n "$_id" ] || return 0

  printf '\n## Issue archetype overlay\n\n'
  printf '_Selected persona archetype: `%s`._\n\n' "$_id"
  bash "$SCRIPT_DIR/persona-catalog.sh" load "$_id" 2>/dev/null || true
  printf '\n'
}

write_effective_persona() {
  local _global_json="$1"
  local _overlay_json="$2"
  mkdir -p "$(dirname "$EFFECTIVE_FILE")" 2>/dev/null || true
  _eff_tmp="$EFFECTIVE_FILE.tmp.$$"
  {
    cat "$GLOBAL_FILE"
    printf '\n## Repo overlay\n\n'
    _overlay_count="$(printf '%s' "$_overlay_json" | jq 'length' 2>/dev/null || echo 0)"
    if [ "$_overlay_count" -gt 0 ]; then
      printf '_Effective persona refined by %s repo-local overlay source(s):_\n\n' "$_overlay_count"
      printf '%s' "$_overlay_json" \
        | jq -r '.[] | "- " + (.source) + " (" + (.path) + ")"' 2>/dev/null || true
      printf '\n'
    else
      printf '_No repo-local overlay sources present._\n\n'
    fi
    issue_archetype_overlay
    confidence_note "$_global_json" "$_overlay_json" "$GLOBAL_FILE"
  } > "$_eff_tmp"
  mv -f "$_eff_tmp" "$EFFECTIVE_FILE"
}

# ---------------------------------------------------------------------------
# Main flow.
# ---------------------------------------------------------------------------

if [ "$DRY_RUN" -eq 1 ]; then
  echo "autonomous-persona-synth: [dry-run] would synthesize $GLOBAL_FILE and apply overlay to $EFFECTIVE_FILE" >&2
  exit 0
fi

# ── F1 bundle ──────────────────────────────────────────────────────────────
BUNDLE_JSON="$(REPO_ROOT="$REPO_ROOT" AUTOSPEC_HOME="$AUTOSPEC_HOME" bash "$SOURCES_SH")"
GLOBAL_BUNDLE="$(printf '%s' "$BUNDLE_JSON" | jq -c '.global // []')"
OVERLAY_BUNDLE="$(printf '%s' "$BUNDLE_JSON" | jq -c '.overlay // []')"

# ── Cadence / staleness gate ───────────────────────────────────────────────
if [ "$FORCE" -ne 1 ] && is_fresh; then
  write_effective_persona "$GLOBAL_BUNDLE" "$OVERLAY_BUNDLE"
  echo "autonomous-persona-synth: global persona fresh (< ${REFRESH_DAYS}d) — refreshed effective persona" >&2
  exit 0
fi

# ── Acquire global-file lock ───────────────────────────────────────────────
if ! acquire_lock; then
  echo "autonomous-persona-synth: global persona lock held by a live holder — try again later" >&2
  exit 2
fi

# From here on, release the lock inline on every exit path (no RETURN trap).
_tmp_dir="$(mktemp -d -t persona-synth.XXXXXX)"
_edits="$_tmp_dir/operator-edits.md"
_synth="$_tmp_dir/synth.md"

# ── Preserve operator-edited blocks from the existing global file ───────────
extract_operator_edits "$_edits"

# ── Tier-A synthesis (mocked in tests) ─────────────────────────────────────
if ! printf '%s' "$GLOBAL_BUNDLE" | persona_synth_llm > "$_synth"; then
  echo "autonomous-persona-synth: synthesis seam failed" >&2
  rm -rf "$_tmp_dir"
  release_lock
  exit 1
fi

# ── Structural presence check (no exact-text assertion) ────────────────────
if ! validate_shape "$_synth"; then
  echo "autonomous-persona-synth: synthesized output failed structural shape check" >&2
  rm -rf "$_tmp_dir"
  release_lock
  exit 3
fi

# ── Merge preserved operator edits back verbatim ───────────────────────────
_merged="$_tmp_dir/global.md"
cat "$_synth" > "$_merged"
if [ -s "$_edits" ]; then
  {
    printf '\n<!-- preserved operator edits (never overwritten by synthesis) -->\n'
    cat "$_edits"
  } >> "$_merged"
fi

# ── Atomic write of the global persona file ────────────────────────────────
mkdir -p "$(dirname "$GLOBAL_FILE")" 2>/dev/null || true
_global_tmp="$GLOBAL_FILE.tmp.$$"
cat "$_merged" > "$_global_tmp"
mv -f "$_global_tmp" "$GLOBAL_FILE"

# ── Apply overlay → effective persona + per-dimension confidence ────────────
write_effective_persona "$GLOBAL_BUNDLE" "$OVERLAY_BUNDLE"

# ── Inline cleanup + lock release (no trap) ────────────────────────────────
rm -rf "$_tmp_dir"
release_lock

echo "autonomous-persona-synth: wrote $GLOBAL_FILE and $EFFECTIVE_FILE" >&2
exit 0
