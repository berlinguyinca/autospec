#!/usr/bin/env bash
# scripts/autospec-language-table.sh — marker-file language table for stack
# detection (issue #3108). A refusal prints nothing and exits non-zero.
set -eu

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
    *.js) printf 'javascript\n' ;;
    *.java) printf 'java\n' ;;
    *.sh) printf 'bash\n' ;;
    *.rb) printf 'ruby\n' ;;
    *.cs) printf 'csharp\n' ;;
    *.md) printf 'markdown\n' ;;
    *) return 1 ;;
  esac
}

if [ "$#" -ne 2 ]; then
  printf 'usage: %s {marker_language|extension_language} <file>\n' "$0" >&2
  exit 2
fi

case "$1" in
  marker_language) marker_language "$2" ;;
  extension_language) extension_language "$2" ;;
  *) printf 'usage: %s {marker_language|extension_language} <file>\n' "$0" >&2; exit 2 ;;
esac
