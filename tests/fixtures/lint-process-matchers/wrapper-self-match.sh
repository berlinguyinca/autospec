#!/usr/bin/env bash
# Fixture: the self-matching wrapper case from issue #3938.
#
# This wrapper's own argv contains "worker.sh" (the worker path it runs),
# so a pattern wait for that string would match this wrapper itself and
# hang forever. The pid-based helper in
# scripts/lib/autospec-process-wait.sh does not.
while pgrep -f "worker.sh" >/dev/null 2>&1; do
    sleep 1
done
bash worker.sh
