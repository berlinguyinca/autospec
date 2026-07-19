#!/usr/bin/env bash
# Unit guard for issue #1488: autospec-explore documents metabolomics/lab-ops specialists.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SKILL="$ROOT/skills/autospec-explore/SKILL.md"
for required in \
  'repo names' 'dependency manifests' 'docs' 'code paths' \
  'ms-data-specialist' 'chemical-ids-specialist' 'lc-binbase-specialist' \
  'mona-sirius-specialist' 'hpc-reliability-specialist' \
  '`evidence`' '`severity`' '`consumer`' '`gap_check`' \
  'gap-confirm' 'verify' 'ROI' 'pattern-synthesis' 'severity-first rank'; do
  grep -Fq -- "$required" "$SKILL"
done
