#!/usr/bin/env bash
# reverse-engineer.sh — orchestrate the 7-step reverse-engineer pipeline.
#
# Usage:
#   bash reverse-engineer.sh --repo-root <dir> [--docs-dir <dir>] [--date <YYYY-MM-DD>]
#
# Steps:
#   1. Inventory source files (inventory.mjs)
#   2. Per-file tree-sitter walk (walker.mjs, max 8 concurrent)
#   3. Cluster into significant units (cluster.mjs)
#   4. Emit specs to docs/specs/ (emit-spec.mjs)
#   5. Write manifest to stdout
#
# Out of scope: doc generators (Phase 5), AI summary fill-in (Phase 8).
#
# Exit codes:
#   0 — success
#   1 — usage error or pipeline failure

set -euo pipefail

# ── Locate script directory ────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RE_DIR="$SCRIPT_DIR/reverse-engineer"
WALKER="$SCRIPT_DIR/tree-sitter-walk/walker.mjs"
INVENTORY="$RE_DIR/inventory.mjs"
CLUSTER="$RE_DIR/cluster.mjs"
EMIT_SPEC="$RE_DIR/emit-spec.mjs"

# ── Parse arguments ────────────────────────────────────────────────────────────
REPO_ROOT=""
DOCS_DIR=""
DATE_OVERRIDE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)  REPO_ROOT="${2:?'--repo-root requires a value'}"; shift 2 ;;
    --docs-dir)   DOCS_DIR="${2:?'--docs-dir requires a value'}"; shift 2 ;;
    --date)       DATE_OVERRIDE="${2:?'--date requires a value'}"; shift 2 ;;
    -h|--help)
      echo "Usage: bash reverse-engineer.sh --repo-root <dir> [--docs-dir <dir>] [--date <YYYY-MM-DD>]"
      exit 0
      ;;
    *) echo "reverse-engineer.sh: unknown option '$1'" >&2; exit 1 ;;
  esac
done

if [[ -z "$REPO_ROOT" ]]; then
  echo "reverse-engineer.sh: --repo-root is required" >&2
  exit 1
fi

REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"
DOCS_DIR="${DOCS_DIR:-$REPO_ROOT/docs/specs}"
DATE="${DATE_OVERRIDE:-$(date +%Y-%m-%d)}"

# ── Verify prerequisites ───────────────────────────────────────────────────────
for f in "$INVENTORY" "$CLUSTER" "$EMIT_SPEC" "$WALKER"; do
  if [[ ! -f "$f" ]]; then
    echo "reverse-engineer.sh: required file not found: $f" >&2
    exit 1
  fi
done

if ! command -v node >/dev/null 2>&1; then
  echo "reverse-engineer.sh: node is required but not found in PATH" >&2
  exit 1
fi

# ── Ensure node_modules are installed in the shared package ───────────────────
SHARED_PKG_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if [[ -f "$SHARED_PKG_DIR/package.json" ]] && [[ ! -d "$SHARED_PKG_DIR/node_modules" ]]; then
  echo "[reverse-engineer] Installing node dependencies in $SHARED_PKG_DIR ..." >&2
  (cd "$SHARED_PKG_DIR" && npm install --silent 2>&1) || {
    echo "reverse-engineer.sh: npm install failed" >&2
    exit 1
  }
fi

echo "[reverse-engineer] repo-root: $REPO_ROOT" >&2
echo "[reverse-engineer] docs-dir:  $DOCS_DIR" >&2
echo "[reverse-engineer] date:      $DATE" >&2

# ── Single Node.js orchestrator (avoids shell/argv issues with paths) ──────────
# We pass all paths via environment variables; the script imports the modules
# directly rather than relying on CLI guards.

RESULT="$(node --input-type=module <<JSEOF
import { inventory }  from '${INVENTORY}';
import { walk }       from '${WALKER}';
import { cluster }    from '${CLUSTER}';
import { emitSpecs }  from '${EMIT_SPEC}';

const REPO_ROOT = '${REPO_ROOT}';
const DOCS_DIR  = '${DOCS_DIR}';
const DATE      = '${DATE}';
const MAX_CONCURRENT = 8;

// Step 1: Inventory
process.stderr.write('[reverse-engineer] step 1/5: inventory ...\n');
const entries = await inventory(REPO_ROOT);
process.stderr.write(\`[reverse-engineer] step 1/5: found \${entries.length} source files\n\`);

if (entries.length === 0) {
  process.stderr.write('[reverse-engineer] no source files found; emitting empty manifest\n');
  process.stdout.write(JSON.stringify({ written: [], skipped: [], manifest: [] }) + '\n');
  process.exit(0);
}

// Step 2: Walk (max 8 concurrent)
process.stderr.write('[reverse-engineer] step 2/5: tree-sitter walk (max 8 concurrent) ...\n');
const walkerOutputs = [];
for (let i = 0; i < entries.length; i += MAX_CONCURRENT) {
  const batch = entries.slice(i, i + MAX_CONCURRENT);
  const results = await Promise.all(batch.map(e =>
    walk(e.filePath).catch(() => ({
      language: 'unknown', exports: [], entry_points: [], imports: [], file_path: e.filePath
    }))
  ));
  walkerOutputs.push(...results);
}
process.stderr.write(\`[reverse-engineer] step 2/5: walked \${walkerOutputs.length} files\n\`);

// Step 3: Cluster
process.stderr.write('[reverse-engineer] step 3/5: clustering ...\n');
const clusterResult = cluster(walkerOutputs);
process.stderr.write(\`[reverse-engineer] step 3/5: significant=\${clusterResult.significant.length} trivial=\${clusterResult.trivial.length}\n\`);

// Step 4: Emit specs
process.stderr.write(\`[reverse-engineer] step 4/5: emitting specs to \${DOCS_DIR} ...\n\`);
const emitResult = await emitSpecs(clusterResult, { docsDir: DOCS_DIR, repoRoot: REPO_ROOT, date: DATE });
process.stderr.write(\`[reverse-engineer] step 4/5: written=\${emitResult.written.length} skipped=\${emitResult.skipped.length}\n\`);

// Step 5: Manifest to stdout
process.stderr.write('[reverse-engineer] step 5/5: done\n');
process.stdout.write(JSON.stringify(emitResult, null, 2) + '\n');
JSEOF
)"

echo "$RESULT"
