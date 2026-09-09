#!/usr/bin/env bash
# Fixture: parent-PID matching is pid-based and safe; the lint must not
# flag it.
while pgrep -P "$child_pid" >/dev/null 2>&1; do
    sleep 1
done
