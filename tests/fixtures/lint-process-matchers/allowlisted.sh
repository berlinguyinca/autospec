#!/usr/bin/env bash
# Fixture: contains a -f matcher but is listed in allowlist.txt, so the
# lint must not report it.
pgrep -f "legacy daemon" >/dev/null 2>&1 || true
