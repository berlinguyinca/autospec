#!/usr/bin/env bash
# scripts/autospec-language-table.sh — marker-file language table for stack
# detection (issue #3108). A refusal prints nothing and exits non-zero.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

repo_dominant() {
  PYTHONPATH="$SCRIPT_DIR" python3 - "$1" <<'PYEOF'
import sys
from pathlib import Path
import autospec_autonomy_stack as stack

root = Path(sys.argv[1]).resolve()
detected = stack._detect_profiles(root)
langs = detected["languages"]
if not langs or langs[0]["id"] == "unknown":
    print("- 0.0")
    sys.exit(0)
winner = langs[0]
csv = ",".join(sorted(p["id"] for p in langs if p["id"] != "unknown"))
if winner["confidence"] > stack.CLAMPED_CONFIDENCE:
    print(f"{winner['id']} {winner['confidence']} {csv}")
else:
    print(f"- {winner['confidence']} {csv}")
PYEOF
}

marker_language() {
  local base dir
  base="$(basename "$1")"
  case "$base" in
    Cargo.toml) printf 'rust\n' ;;
    go.mod) printf 'go\n' ;;
    pyproject.toml) printf 'python\n' ;;
    package.json)
      dir="$(dirname "$1")"
      if [ -f "$dir/tsconfig.json" ]; then
        printf 'typescript\n'
      else
        printf 'javascript\n'
      fi
      ;;
    pom.xml | build.gradle) printf 'java\n' ;;
    Gemfile) printf 'ruby\n' ;;
    *.csproj) printf 'csharp\n' ;;
    *) return 1 ;;
  esac
}

extension_language() {
  local base lower
  base="$(basename "$1")"
  lower="$(printf '%s' "$base" | LC_ALL=C tr '[:upper:]' '[:lower:]')"
  case "$lower" in
    *.rs) printf 'rust\n' ;;
    *.go) printf 'go\n' ;;
    *.py) printf 'python\n' ;;
    *.ts) printf 'typescript\n' ;;
    *.tsx) printf 'typescript\n' ;;
    *.js) printf 'javascript\n' ;;
    *.jsx) printf 'javascript\n' ;;
    *.mjs) printf 'javascript\n' ;;
    *.cjs) printf 'javascript\n' ;;
    *.java) printf 'java\n' ;;
    *.sh) printf 'bash\n' ;;
    *.rb) printf 'ruby\n' ;;
    *.cs) printf 'csharp\n' ;;
    *.md) printf 'markdown\n' ;;
    *) return 1 ;;
  esac
}

if [ "$#" -ne 2 ]; then
  printf 'usage: %s {marker_language|extension_language|repo_dominant} <path>\n' "$0" >&2
  exit 2
fi

case "$1" in
  marker_language) marker_language "$2" ;;
  extension_language) extension_language "$2" ;;
  repo_dominant) repo_dominant "$2" ;;
  *) printf 'usage: %s {marker_language|extension_language|repo_dominant} <path>\n' "$0" >&2; exit 2 ;;
esac
