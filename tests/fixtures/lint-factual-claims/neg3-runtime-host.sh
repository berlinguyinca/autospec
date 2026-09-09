#!/usr/bin/env bash
# Negative fixture: bare host-capability claim, but the same file probes for
# the tool (command -v), so the claim is executed, not asserted.
# The worker has no buildx; fall back to plain docker build.
set -eu
if command -v buildx >/dev/null 2>&1; then
    echo buildx
else
    echo fallback
fi
