#!/usr/bin/env bash
set -u
autospec validate >/dev/null
state=$(autospec queue ready --json)
echo "$state"
