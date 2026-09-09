#!/usr/bin/env bash
# Fixture: inline waiver on the line above the offending line.
# process-matcher:allow legacy integration reviewed in #3938
pkill -f "legacy daemon" || true
