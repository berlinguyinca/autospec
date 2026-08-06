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
# first offending line and are cleared by a guard anywhere in the same file.
#
# UI_NO_REDUCED_MOTION is additionally cleared for every file in one invocation when any
# of them carries a global reset — a prefers-reduced-motion block targeting the universal
# selector — since such a reset guards every element on the page. A block scoped to one
# class does not count, and linting a single file still reports, because one file cannot
# carry evidence of a reset living in another.
#
# UI_HOVER_ONLY_AFFORDANCE has no equivalent: focus styling is per component, and a
# global focus rule is not the common pattern that a global motion reset is. A repo
# whose focus styles live in a separate stylesheet will still see it fire per file.
#
# Token/theme source files (basename matching design|token|theme|palette, or DESIGN.md)
# are skipped — they legitimately declare raw values.
#
# Comments are stripped before any rule runs, so prose about a rule is not read as a
# violation of it. That cut both ways before: a comment naming a banned viewport
# directive was reported, and a comment naming the reduced-motion media feature cleared
# the guard flag and silenced a real finding. CSS/JS block comments and HTML comments are
# handled, including multi-line ones. `//` line comments are left alone, because they are
# indistinguishable from the middle of a URL at this level of parsing.
#
# UI_FIXED_VIEWPORT judges markup rather than text: the directives are only read inside a
# <meta> tag whose name is viewport, tracked across lines so a formatted multi-line tag
# still reports. Prose and scripts naming the directive are ignored.
#
# Exit code = number of findings (0 = clean), capped at 200.
# Exit 99 = the linter could not run: invocation error, or the awk interpreter failed
# (never a silent clean pass). It is deliberately outside the finding-count range — the
# previous value of 2 was indistinguishable from a file with two findings, which is most
# of what the fail-loud check exists to prevent.
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
    --*) echo "lint-ui: unknown option: $1" >&2; exit 99 ;;
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

# ── global reduced-motion reset ───────────────────────────────────────────────
# UI_NO_REDUCED_MOTION is decided per file, so a project that keeps its reset in one
# global stylesheet and animates in components — the ordinary way to organise CSS — saw
# a finding on every component. A reset targeting the universal selector genuinely
# guards every element on the page, so when one appears in this invocation the rule is
# satisfied for the whole set.
#
# The universal selector is what makes it safe: a `prefers-reduced-motion` block scoped
# to `.panel` guards `.panel` and nothing else, and must not clear the rule elsewhere.
# Linting a single file still reports, because one file cannot show evidence of a reset
# living in another — the rule is answered by evidence, not by assumption.
GLOBAL_RM=0
for f in $FILES; do
  [ -f "$f" ] || continue
  if "$AWK" '
      {
        line = $0
        # Block comments are removed here for the same reason the main pass removes
        # them, and the need is sharper: a comment continuation line begins with `*`,
        # which is exactly the universal selector this scan looks for. Reading comments
        # would let a note about a missing guard masquerade as a global reset — the
        # silencing bug, rebuilt in a second place.
        if (in_block) {
          i = index(line, "*/")
          if (i == 0) next
          line = substr(line, i + 2); in_block = 0
        }
        b = index(line, "/*")
        if (b > 0) {
          rest = substr(line, b + 2); i = index(rest, "*/")
          if (i == 0) { in_block = 1; line = substr(line, 1, b - 1) }
          else { line = substr(line, 1, b - 1) substr(rest, i + 2) }
        }
        # A short window after the media query: the selector list follows within a line
        # or two in practice, and a bounded window cannot mistake a later unrelated `*`
        # rule for part of this block.
        if (line ~ /prefers-reduced-motion/) window = 8
        if (window > 0) { if (line ~ /^[[:space:]]*\*/) found = 1; window-- }
      }
      END { exit(found ? 0 : 1) }
    ' "$f" 2>/dev/null; then
    GLOBAL_RM=1
    break
  fi
done

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
  recs="$("$AWK" -v globalrm="$GLOBAL_RM" '
    # Remove comment spans so no rule reads prose about itself as code. in_block and
    # in_html persist across lines to carry a multi-line comment.
    function strip_comments(s,   out, i, b, h) {
      while (1) {
        if (in_block) {
          i = index(s, "*/")
          if (i == 0) return ""
          s = substr(s, i + 2); in_block = 0
        } else if (in_html) {
          i = index(s, "-->")
          if (i == 0) return ""
          s = substr(s, i + 3); in_html = 0
        } else break
      }
      out = ""
      while (1) {
        b = index(s, "/*"); h = index(s, "<!--")
        if (b == 0 && h == 0) return out s
        if (b != 0 && (h == 0 || b < h)) {
          out = out substr(s, 1, b - 1); s = substr(s, b + 2)
          i = index(s, "*/")
          if (i == 0) { in_block = 1; return out }
          s = substr(s, i + 2)
        } else {
          out = out substr(s, 1, h - 1); s = substr(s, h + 4)
          i = index(s, "-->")
          if (i == 0) { in_html = 1; return out }
          s = substr(s, i + 3)
        }
      }
    }
    {
      n=NR; line=strip_comments($0)
      # UI_RAW_HEX: hex anywhere in a declaration value, or a JS/TS string literal.
      # The value side is matched up to the next ; { or }, so shorthand, gradient stops
      # and var() fallbacks count. Requiring the hex to follow the colon directly caught
      # only `color: #abc` and missed `border: 1px solid #ccc`, which is where colours
      # more often sit. Any run of three or more hex digits counts, so the four- and
      # eight-digit alpha forms (#RGBA, #RRGGBBAA) are caught as well; matching exactly
      # three or six let `#00000022` through. Intervals stay out of the pattern: mawk
      # aborts on an interval combined with a group, and did so silently here until
      # 2026-08-04.
      # A custom-property declaration IS the token definition, so its hex is the source
      # of truth rather than a violation of it: `--accent: #0f766e` cannot be rewritten
      # to use a token without becoming circular. Measured on berlinguyinca/autospec-gui,
      # where 11 of its 21 findings were the :root block defining its palette.
      #
      # Only the declaration is stripped, not the whole line, so a real usage sharing a
      # line is still caught. A `var(--x, #fff)` fallback is a usage rather than a
      # declaration and stays flagged.
      hexline = line
      gsub(/--[A-Za-z0-9_-]+[[:space:]]*:[^;]*/, "", hexline)
      if (hexline ~ /:[^;{}]*#[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]*([^0-9a-fA-F]|$)/ \
          || hexline ~ /["'"'"']#[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]*["'"'"']/) {
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
      # Only markup counts. The tag is tracked across lines, because a formatter puts
      # name= and content= on separate lines and the directives then share a line with
      # neither `<meta` nor `viewport`. Without this, the rule matched the directive
      # names anywhere — in prose, in a script string, in a style guide documenting the
      # anti-pattern.
      if (line ~ /<meta/) { in_meta = 1; meta_vp = 0 }
      if (in_meta && line ~ /viewport/) meta_vp = 1
      if (in_meta && meta_vp) {
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
      if (in_meta && line ~ />/) { in_meta = 0; meta_vp = 0 }
    }
    END {
      if (anim_line && !has_rm && globalrm != 1) {
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
    exit 99
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
