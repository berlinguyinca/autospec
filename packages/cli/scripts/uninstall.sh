#!/usr/bin/env bash
# packages/cli/scripts/uninstall.sh — remove autospec skills from harness paths
# Called by: autospec uninstall [--yes] [--help]
#
# Removes:
#   ~/.claude/skills/autospec-*         (Claude Code)
#   ~/.config/opencode/skills/autospec-* (OpenCode, if present)
#   ~/.codex/skills/autospec-*          (Codex CLI, if present)
#
# Preserves:
#   ~/.autospec/     — config, telemetry, memory
#   ~/.claude/       — everything except autospec skills

set -eu

usage() {
  cat <<'EOF'
autospec uninstall — remove autospec skills from your harness paths

Removes autospec-* directories from harness skill paths.
Preserves ~/.autospec/ (config, telemetry, memory) and all other harness config.

Usage:
  autospec uninstall [--yes] [--help]

Options:
  --yes     Skip confirmation prompt
  --help    Show this help
EOF
}

YES=0

for arg in "$@"; do
  case "$arg" in
    --help|-h) usage; exit 0 ;;
    --yes|-y) YES=1 ;;
    *) echo "autospec uninstall: unknown flag '$arg'" >&2; exit 1 ;;
  esac
done

# Allow test override of skills directory
CLAUDE_SKILLS="${AUTOSPEC_SKILLS_DIR:-$HOME/.claude/skills}"
OPENCODE_SKILLS="${OPENCODE_SKILLS_DIR:-$HOME/.config/opencode/skills}"
CODEX_SKILLS="${CODEX_SKILLS_DIR:-$HOME/.codex/skills}"

# Collect what would be removed
to_remove=()
for base in "$CLAUDE_SKILLS" "$OPENCODE_SKILLS" "$CODEX_SKILLS"; do
  [ -d "$base" ] || continue
  for skill_dir in "$base"/autospec-*/; do
    [ -d "$skill_dir" ] || continue
    to_remove+=("$skill_dir")
  done
done

if [ "${#to_remove[@]}" -eq 0 ]; then
  echo "autospec uninstall: no autospec-* skills found to remove."
  exit 0
fi

echo "autospec uninstall: the following skill directories will be removed:"
for d in "${to_remove[@]}"; do
  echo "  $d"
done
echo ""
echo "Preserved: ~/.autospec/ (config, telemetry, memory)"

if [ "$YES" -eq 0 ]; then
  printf 'Continue? [y/N] '
  read -r answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) echo "Aborted."; exit 0 ;;
  esac
fi

removed=0
for d in "${to_remove[@]}"; do
  rm -rf "$d"
  echo "removed: $d"
  removed=$((removed + 1))
done

echo ""
echo "autospec uninstall: removed $removed skill director$([ "$removed" -eq 1 ] && echo y || echo ies)."
echo "  Your ~/.autospec/ configuration has been preserved."
echo "  Re-install with: autospec install"
