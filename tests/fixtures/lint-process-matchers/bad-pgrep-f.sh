#!/usr/bin/env bash
# Fixture: process-matcher violation (pgrep -f).
while pgrep -f "autospec worker" >/dev/null 2>&1; do
    sleep 1
done
