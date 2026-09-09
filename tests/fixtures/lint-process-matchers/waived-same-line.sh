#!/usr/bin/env bash
# Fixture: inline waiver on the offending line (same-line form).
pgrep -f "legacy daemon" >/dev/null 2>&1 || true  # process-matcher:allow legacy integration reviewed in #3938
