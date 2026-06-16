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
# Token/theme source files (basename matching design|token|theme|palette, or DESIGN.md)
# are skipped — they legitimately declare raw values.
#
# Exit code = number of findings (0 = clean), capped at 200.
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

emit() { # emit RULE_ID file line desc
  if [ "$DIRECTIVES" -eq 1 ]; then
    case "$1" in
      UI_RAW_HEX)          printf 'Fix %s: replace the raw hex with a DESIGN.md color token / CSS variable.\n' "$1" ;;
      UI_OFF_GRID_SPACING) printf 'Fix %s: snap spacing to the 4/8px grid (use a spacing token).\n' "$1" ;;
      UI_AD_HOC_ZINDEX)    printf 'Fix %s: use a z-index from the declared scale (0/10/20/30/40/50/100/1000).\n' "$1" ;;
      UI_BANNED_FONT)      printf 'Fix %s: use the DESIGN.md font tokens, not a banned/generic font.\n' "$1" ;;
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
  recs="$(awk '
    {
      line=$0; n=NR
      # UI_RAW_HEX: hex as a CSS value (": #abc") or a JS/TS string literal.
      if (line ~ /:[[:space:]]*#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?([^0-9a-fA-F]|$)/ \
          || line ~ /["'"'"']#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?["'"'"']/) {
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
    }
  ' "$f")"
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

[ -n "$out" ] && printf '%s' "$out"
[ "$count" -gt 200 ] && count=200
exit "$count"
