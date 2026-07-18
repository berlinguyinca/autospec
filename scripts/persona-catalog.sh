#!/usr/bin/env bash
# persona-catalog.sh — merged read-only bundled + writable user persona catalog.
#
# Usage:
#   bash scripts/persona-catalog.sh list
#   bash scripts/persona-catalog.sh load <id>
#
# Backends:
#   1. user-state: ~/.autospec/personas/ (or $AUTOSPEC_HOME/personas)
#   2. bundled:    personas/catalog/ inside this repository
#
# User-state entries shadow bundled entries with the same front-matter id.

set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BUNDLED_DIR="$REPO_ROOT/personas/catalog"
USER_DIR="$HOME/.autospec/personas"

usage() {
  cat >&2 <<'EOF'
Usage:
  persona-catalog.sh list
  persona-catalog.sh load <id>
EOF
}

warn() {
  printf 'persona-catalog: %s\n' "$*" >&2
}

front_matter_id() {
  awk '
    NR == 1 && $0 != "---" { exit 1 }
    NR == 1 { in_fm = 1; next }
    in_fm && $0 == "---" {
      if (id != "") {
        print id
        exit 0
      }
      exit 1
    }
    in_fm && $0 ~ /^id:[[:space:]]*/ {
      id = $0
      sub(/^id:[[:space:]]*/, "", id)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", id)
      if (id !~ /^[A-Za-z0-9._-]+$/) {
        exit 1
      }
    }
    END {
      if (!in_fm) {
        exit 1
      }
    }
  ' "$1"
}

scan_backend() {
  _backend="$1"
  _dir="$2"

  if [ ! -d "$_dir" ]; then
    return 0
  fi

  for _file in "$_dir"/*.md; do
    if [ ! -f "$_file" ]; then
      continue
    fi
    if _id="$(front_matter_id "$_file")"; then
      printf '%s\t%s\t%s\n' "$_id" "$_backend" "$_file"
    else
      warn "warning: malformed front matter: $_file"
    fi
  done
}

entry_path_for_id() {
  _entries="$1"
  _id="$2"
  awk -F '\t' -v id="$_id" '$1 == id { print $3; exit 0 }' "$_entries"
}

entry_has_id() {
  _entries="$1"
  _id="$2"
  awk -F '\t' -v id="$_id" '$1 == id { found = 1 } END { exit found ? 0 : 1 }' "$_entries"
}

with_entries() {
  _user_entries="$1"
  _bundled_entries="$2"
  scan_backend "user-state" "$USER_DIR" | sort -t "$(printf '\t')" -k1,1 -k3,3 > "$_user_entries"
  scan_backend "bundled" "$BUNDLED_DIR" | sort -t "$(printf '\t')" -k1,1 -k3,3 > "$_bundled_entries"
}

list_ids() {
  _tmp_dir="$(mktemp -d -t persona-catalog.XXXXXX)"
  _user_entries="$_tmp_dir/user.tsv"
  _bundled_entries="$_tmp_dir/bundled.tsv"
  _ids="$_tmp_dir/ids.txt"

  with_entries "$_user_entries" "$_bundled_entries"
  : > "$_ids"

  awk -F '\t' '{ print $1 }' "$_user_entries" >> "$_ids"
  while IFS="$(printf '\t')" read -r _id _backend _path; do
    if entry_has_id "$_user_entries" "$_id"; then
      warn "user-state shadows bundled id: $_id"
    else
      printf '%s\n' "$_id" >> "$_ids"
    fi
  done < "$_bundled_entries"

  sort -u "$_ids"
  rm -rf "$_tmp_dir"
}

load_id() {
  _id="$1"
  _tmp_dir="$(mktemp -d -t persona-catalog.XXXXXX)"
  _user_entries="$_tmp_dir/user.tsv"
  _bundled_entries="$_tmp_dir/bundled.tsv"

  with_entries "$_user_entries" "$_bundled_entries"

  _user_path="$(entry_path_for_id "$_user_entries" "$_id")"
  _bundled_path="$(entry_path_for_id "$_bundled_entries" "$_id")"

  if [ -n "$_user_path" ]; then
    if [ -n "$_bundled_path" ]; then
      warn "user-state shadows bundled id: $_id"
    fi
    cat "$_user_path"
    rm -rf "$_tmp_dir"
    return 0
  fi

  if [ -n "$_bundled_path" ]; then
    cat "$_bundled_path"
    rm -rf "$_tmp_dir"
    return 0
  fi

  warn "id not found: $_id"
  rm -rf "$_tmp_dir"
  return 1
}

if [ $# -lt 1 ]; then
  usage
  exit 1
fi

case "$1" in
  list)
    if [ $# -ne 1 ]; then
      usage
      exit 1
    fi
    list_ids
    ;;
  load)
    if [ $# -ne 2 ]; then
      usage
      exit 1
    fi
    load_id "$2"
    ;;
  -h|--help)
    usage
    ;;
  *)
    usage
    exit 1
    ;;
esac
