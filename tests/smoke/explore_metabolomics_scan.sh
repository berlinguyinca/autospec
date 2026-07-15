#!/usr/bin/env bash
# Smoke guard for issue #1488: deterministic scan proposes five metabolomics/lab-ops specialists.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP="$(mktemp -d -t metab-scan.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/src/mona_sirius" "$TMP/pipelines/slurm_jobs"
cat > "$TMP/pyproject.toml" <<'PYPROJECT'
[project]
name = "metabolomics-us"
dependencies = ["pyteomics", "rdkit"]
PYPROJECT
cat > "$TMP/README.md" <<'README'
# metabolomics-us
Processes mzML files, InChIKey annotations, BinBase retention index bins, MoNA/SIRIUS references, and Slurm jobs.
README
AUTOSPEC_REPO_ROOT="$TMP" AUTOSPEC_NUM_SPECIALISTS=6 bash "$ROOT/scripts/explore-specialist-scan.sh" --force \
  | python3 -c 'import json, sys; d=json.load(sys.stdin); slugs={s["slug"] for s in d["suggested_specialists"]}; required={"ms-data-specialist","chemical-ids-specialist","lc-binbase-specialist","mona-sirius-specialist","hpc-reliability-specialist"}; missing=required-slugs; assert not missing, (missing, d)'
