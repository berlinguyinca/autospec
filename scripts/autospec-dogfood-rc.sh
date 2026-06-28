#!/usr/bin/env bash
set -eu
DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$DIR/autospec-rc-closure-lib.py" --command dogfood-rc "$@"
