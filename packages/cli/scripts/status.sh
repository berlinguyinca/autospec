#!/usr/bin/env bash
# packages/cli/scripts/status.sh — list installed autospec skills + versions + cache-hit-rate
# Called by: autospec status [--help]
#
# Reads:
#   $AUTOSPEC_SKILLS_DIR or ~/.claude/skills/   — skill directories
#   ~/.autospec/telemetry.jsonl                  — optional telemetry for cache-hit-rate

set -eu

usage() {
  cat <<'EOF'
autospec status — list installed autospec skills and cache-hit-rate

Usage:
  autospec status [--help]

Options:
  --help    Show this help
EOF
}

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    *) echo "autospec status: unknown flag '$arg'" >&2; exit 1 ;;
  esac
done

SKILLS_DIR="${AUTOSPEC_SKILLS_DIR:-$HOME/.claude/skills}"

if [ ! -d "$SKILLS_DIR" ]; then
  echo "autospec status: no skills directory found at $SKILLS_DIR"
  echo "  Run 'autospec install' to install autospec skills."
  exit 0
fi

found=0
for skill_dir in "$SKILLS_DIR"/autospec-*/; do
  [ -d "$skill_dir" ] || continue
  skill_name="$(basename "$skill_dir")"
  skill_md="$skill_dir/SKILL.md"

  if [ -f "$skill_md" ]; then
    # Extract version from YAML frontmatter: version: X.Y.Z
    version="$(awk '/^---$/{c++; next} c==1 && /^version:/{gsub(/^version:[[:space:]]*/, ""); print; exit} c>=2{exit}' "$skill_md" 2>/dev/null || true)"
    version="${version:-unknown}"
  else
    version="unknown"
  fi

  printf '%-30s %s\n' "$skill_name" "$version"
  found=$((found + 1))
done

if [ "$found" -eq 0 ]; then
  echo "autospec status: no autospec-* skills found in $SKILLS_DIR"
  echo "  Run 'autospec install' to install autospec skills."
  exit 0
fi

# Cache-hit-rate from telemetry (optional)
TELEMETRY_FILE="${AUTOSPEC_TELEMETRY:-$HOME/.autospec/telemetry.jsonl}"
if [ -f "$TELEMETRY_FILE" ]; then
  # Compute cache-hit-rate: average of cache_hit_rate field across last 20 entries
  rate="$(tail -n 20 "$TELEMETRY_FILE" | \
    python3 -c "
import sys, json
rates = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        obj = json.loads(line)
        r = obj.get('cache_hit_rate')
        if r is not None:
            rates.append(float(r))
    except Exception:
        pass
if rates:
    avg = sum(rates) / len(rates)
    print(f'{avg:.1%}')
else:
    print('n/a')
" 2>/dev/null || echo "n/a")"
  echo ""
  echo "cache-hit-rate: $rate (last ≤20 runs)"
fi
