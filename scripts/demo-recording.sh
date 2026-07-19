#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

cat <<EOF
AutoSpec demo outline
=====================

Safe mode: demo only. No network, GitHub mutation, branch push, merge, release upload, or destructive filesystem operation is required.

1. Start at README.md and read the one-line pitch.
2. Open examples/hello-autospec/spec.md.
3. Open examples/hello-autospec/sample-issue.md.
4. Open examples/hello-autospec/expected-closeout.md.
5. Show docs/assets/architecture.mmd.
6. Run the launch readiness check:

   bash scripts/validate-launch-readiness.sh

This demo is read-only. It does not create GitHub issues, push branches, or mutate a target repository.

Repository: $ROOT
EOF
