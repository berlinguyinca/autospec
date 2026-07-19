#!/usr/bin/env bash
# Compatibility entrypoint for issue smoke commands that resolve shared scripts
# under skills/autospec-shared/scripts/.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

exec bash "$REPO_ROOT/scripts/gen-skill-goldens.sh" "$@"
