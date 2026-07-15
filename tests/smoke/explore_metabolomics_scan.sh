#!/usr/bin/env bash
# Smoke guard for issue #1488: Rust explore specialists command proposes five metabolomics/lab-ops specialists.
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
cargo run -q -p autospec-cli -- explore specialists --repo-dir "$TMP" --num-specialists 6 --force \
  | python3 -c 'import json, sys; d=json.load(sys.stdin); slugs={s["slug"] for s in d["suggested_specialists"]}; required={"ms-data-specialist","chemical-ids-specialist","lc-binbase-specialist","mona-sirius-specialist","hpc-reliability-specialist"}; missing=required-slugs; assert not missing, (missing, d)'
