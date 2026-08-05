#!/usr/bin/env bash
# scripts/lint-ui.sh — deterministic design-token-drift linter for UI files.
#
# Complements the LLM-vision visual-fidelity judge (subjective fidelity) with
# objective, file:line rules that flag code bypassing a design system — the
# deterministic half of the implementer's DESIGN_DRIFT directive. Intended to run
# (by the autospec-qa visual cluster / implementer) only when the repo has a root
# DESIGN.md; this script itself just lints the files it is given.
#
# Usage:
#   scripts/lint-ui.sh <file> [<file> ...]   # lint the given UI files
#   scripts/lint-ui.sh --directives <file>…  # reformat findings as directive lines
#   scripts/lint-ui.sh --help
#
# Output (default): one finding per stdout line:
#   RULE_ID:<path>:<line>: <one-line description>
# With --directives:
#   Fix <RULE_ID>: <imperative action>
#
# Rules (token-drift focus):
#   UI_RAW_HEX          A raw hex color literal in a value/string (use a DESIGN.md token/CSS var).
#   UI_OFF_GRID_SPACING margin/padding/gap px value off the 4px grid (and > 2px).
#   UI_AD_HOC_ZINDEX    z-index outside the scale {0,10,20,30,40,50,100,1000}.
#   UI_BANNED_FONT      font-family using a banned/generic font (Inter/Roboto/Arial/Helvetica/system stacks).
#
# Rules (motion / input focus — no motion scale required):
#   UI_NO_REDUCED_MOTION      File declares motion (@keyframes, an animation property, or a
#                             transition of transform) with no prefers-reduced-motion fallback.
#                             Colour/opacity-only transitions are not motion and never trip it.
#   UI_INFINITE_ANIMATION     Infinite animation with no pause control (WCAG 2.2.2).
#   UI_FIXED_VIEWPORT         Viewport meta blocking zoom via user-scalable=no or
#                             maximum-scale<=1 (defeats WCAG 1.4.4).
#   UI_HOVER_ONLY_AFFORDANCE  File styles :hover but never :focus, so keyboard and touch
#                             users lose the affordance.
#
# UI_NO_REDUCED_MOTION and UI_HOVER_ONLY_AFFORDANCE are whole-file rules: they report the
# first offending line and are cleared by a guard anywhere in the same file. A repo whose
# reduced-motion reset or focus styles live in a separate global stylesheet will see them
# fire per file; scope the linter to changed files, or co-locate the guard.
#
# Token/theme source files (basename matching design|token|theme|palette, or DESIGN.md)
# are skipped — they legitimately declare raw values.
#
# Exit code = number of findings (0 = clean), capped at 200.
# Exit 2 = invocation error, or the awk interpreter failed (never a silent clean pass).
#
# Env:
#   AUTOSPEC_LINT_UI_AWK  awk binary to use (default: awk). Rules are written for
#                         mawk/gawk/BSD awk; set this to prefer a specific one.
#
# Requires: bash 3.2+, awk.

set +e

DIRECTIVES=0
FILES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --directives) DIRECTIVES=1 ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    --*) echo "lint-ui: unknown option: $1" >&2; exit 2 ;;
    *) FILES="$FILES $1" ;;
  esac
  shift
done

[ -n "$FILES" ] || { echo "Usage: scripts/lint-ui.sh <file> [<file> ...]" >&2; exit 0; }

# Rule patterns avoid POSIX interval quantifiers ({n}) — mawk 1.3.4 panics with
# "REcompile() - panic: values still on machine stack" when an interval is combined
# with a group, and the panic previously left every file reported clean.
AWK="${AUTOSPEC_LINT_UI_AWK:-awk}"
ERRFILE="$(mktemp)"

emit() { # emit RULE_ID file line desc
  if [ "$DIRECTIVES" -eq 1 ]; then
    case "$1" in
      UI_RAW_HEX)          printf 'Fix %s: replace the raw hex with a DESIGN.md color token / CSS variable.\n' "$1" ;;
      UI_OFF_GRID_SPACING) printf 'Fix %s: snap spacing to the 4/8px grid (use a spacing token).\n' "$1" ;;
      UI_AD_HOC_ZINDEX)    printf 'Fix %s: use a z-index from the declared scale (0/10/20/30/40/50/100/1000).\n' "$1" ;;
      UI_BANNED_FONT)      printf 'Fix %s: use the DESIGN.md font tokens, not a banned/generic font.\n' "$1" ;;
      UI_NO_REDUCED_MOTION)     printf 'Fix %s: add a prefers-reduced-motion fallback for the motion declared in this file.\n' "$1" ;;
      UI_INFINITE_ANIMATION)    printf 'Fix %s: give the infinite animation a pause/stop control, or make it finite (WCAG 2.2.2).\n' "$1" ;;
      UI_FIXED_VIEWPORT)        printf 'Fix %s: allow zoom — drop user-scalable=no and maximum-scale=1 (WCAG 1.4.4).\n' "$1" ;;
      UI_HOVER_ONLY_AFFORDANCE) printf 'Fix %s: mirror the :hover treatment on :focus-visible so keyboard and touch users get it.\n' "$1" ;;
    esac
  else
    printf '%s:%s:%s: %s\n' "$1" "$2" "$3" "$4"
  fi
}

count=0
out=""
for f in $FILES; do
  [ -f "$f" ] || continue
  base="$(basename "$f")"
  # Skip token/theme source files (they legitimately declare raw values).
  case "$(printf '%s' "$base" | tr '[:upper:]' '[:lower:]')" in
    *design*|*token*|*theme*|*palette*) continue ;;
  esac
  # One awk pass per file emits TAB-separated RULE\tline\tdesc records.
  recs="$("$AWK" '
    {
      line=$0; n=NR
      # UI_RAW_HEX: hex as a CSS value (": #abc") or a JS/TS string literal.
      if (line ~ /:[[:space:]]*#[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]([0-9a-fA-F][0-9a-fA-F][0-9a-fA-F])?([^0-9a-fA-F]|$)/ \
          || line ~ /["'"'"']#[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]([0-9a-fA-F][0-9a-fA-F][0-9a-fA-F])?["'"'"']/) {
        print "UI_RAW_HEX\t" n "\traw hex color literal — use a DESIGN.md token/CSS variable"
      }
      # UI_OFF_GRID_SPACING: margin/padding/gap px not a multiple of 4 (and > 2px).
      if (line ~ /(margin|padding|gap)[a-zA-Z-]*[[:space:]]*:/) {
        tmp=line
        while (match(tmp, /[0-9]+px/)) {
          numstr=substr(tmp, RSTART, RLENGTH-2); num=numstr+0
          if (num > 2 && (num % 4) != 0) {
            print "UI_OFF_GRID_SPACING\t" n "\tspacing " num "px is off the 4px grid — use a spacing token"
            break
          }
          tmp=substr(tmp, RSTART+RLENGTH)
        }
      }
      # UI_AD_HOC_ZINDEX: z-index outside the allowed scale.
      if (match(line, /z-index[[:space:]]*:[[:space:]]*[0-9]+/)) {
        zs=substr(line, RSTART, RLENGTH); sub(/.*:[[:space:]]*/, "", zs); z=zs+0
        if (z!=0 && z!=10 && z!=20 && z!=30 && z!=40 && z!=50 && z!=100 && z!=1000) {
          print "UI_AD_HOC_ZINDEX\t" n "\tz-index " z " is outside the scale {0,10,20,30,40,50,100,1000}"
        }
      }
      # UI_BANNED_FONT: font-family with a banned/generic font.
      if (line ~ /font-family/ && tolower(line) ~ /(inter|roboto|arial|helvetica|space grotesk|system-ui|-apple-system)/) {
        print "UI_BANNED_FONT\t" n "\tbanned/generic font in font-family — use DESIGN.md font tokens"
      }
      # Motion presence (file-level state for UI_NO_REDUCED_MOTION). Only real
      # motion counts: keyframes, an animation declaration, or a transition of
      # transform. A colour/opacity-only transition is not motion.
      if (line ~ /@keyframes/ || line ~ /animation[a-zA-Z-]*[[:space:]]*:/ \
          || (line ~ /transition/ && line ~ /transform/)) {
        if (!anim_line) anim_line = n
      }
      if (line ~ /prefers-reduced-motion/) has_rm = 1
      # Hover/focus parity (file-level state for UI_HOVER_ONLY_AFFORDANCE).
      # :focus also matches :focus-visible and :focus-within.
      if (line ~ /:hover/) { if (!hover_line) hover_line = n }
      if (line ~ /:focus/) has_focus = 1
      # UI_INFINITE_ANIMATION: animation that never stops (WCAG 2.2.2 needs a
      # pause/stop/hide control for motion lasting over 5 seconds).
      if (line ~ /animation-iteration-count[[:space:]]*:[[:space:]]*infinite/ \
          || (line ~ /animation[a-zA-Z-]*[[:space:]]*:/ && line ~ /infinite/)) {
        print "UI_INFINITE_ANIMATION\t" n "\tinfinite animation with no pause control — WCAG 2.2.2"
      }
      # UI_FIXED_VIEWPORT: a viewport meta that blocks zoom (defeats WCAG 1.4.4).
      fixed_vp = 0
      if (line ~ /user-scalable[[:space:]]*=[[:space:]]*["'"'"']?[[:space:]]*no/) fixed_vp = 1
      if (match(line, /maximum-scale[[:space:]]*=[[:space:]]*["'"'"']?[0-9.]+/)) {
        ms = substr(line, RSTART, RLENGTH)
        sub(/.*=[[:space:]]*["'"'"']?/, "", ms)
        if (ms + 0 <= 1) fixed_vp = 1
      }
      if (fixed_vp) {
        print "UI_FIXED_VIEWPORT\t" n "\tviewport blocks zoom — remove user-scalable=no / maximum-scale=1 (WCAG 1.4.4)"
      }
    }
    END {
      if (anim_line && !has_rm) {
        print "UI_NO_REDUCED_MOTION\t" anim_line "\tmotion declared with no prefers-reduced-motion fallback in this file"
      }
      if (hover_line && !has_focus) {
        print "UI_HOVER_ONLY_AFFORDANCE\t" hover_line "\t:hover with no :focus equivalent — keyboard and touch users lose the affordance"
      }
    }
  ' "$f" 2>"$ERRFILE")"
  awk_status=$?
  # Fail loud: a broken interpreter must never be reported as a clean file.
  if [ "$awk_status" -ne 0 ] || [ -s "$ERRFILE" ]; then
    printf 'lint-ui: awk failed on %s (exit %s): %s\n' \
      "$f" "$awk_status" "$(cat "$ERRFILE")" >&2
    rm -f "$ERRFILE"
    exit 2
  fi
  [ -n "$recs" ] || continue
  while IFS="$(printf '\t')" read -r rule ln desc; do
    [ -n "$rule" ] || continue
    out="${out}$(emit "$rule" "$f" "$ln" "$desc")
"
    count=$((count + 1))
    [ "$count" -ge 200 ] && break
  done <<EOF
$recs
EOF
  [ "$count" -ge 200 ] && break
done

rm -f "$ERRFILE"
[ -n "$out" ] && printf '%s' "$out"
[ "$count" -gt 200 ] && count=200
exit "$count"
