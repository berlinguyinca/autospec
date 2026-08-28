#!/usr/bin/env bash
# doc-freshness-tier.sh — documentation generation & freshness merge gate (#1540).
#
# Composes the existing autospec-doc primitives into one deterministic CI tier:
# - check-doc-drift.sh detects public API/config/flag drift and missing scopes;
# - example_stale and failing docs examples block the gate;
# - changed docs trigger llms.txt / llms-full.txt regeneration, with stale exports
#   surfaced as a non-zero gate result;
# - drift/missing-scope findings auto-file (or --dry-run propose) a doc-update
#   auto-implement issue so stale docs converge through the normal queue.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${AUTOSPEC_REPO_ROOT:-}"
MODE=""
MODE_ARG=""
DRY_RUN=0
DOC_UPDATE_LABELS="${AUTOSPEC_DOC_UPDATE_LABELS:-auto-implement,origin:self}"

usage() {
  cat <<'USAGE'
Usage: doc-freshness-tier.sh (--pr <N> | --diff <file> | --working-tree) [--repo-root <dir>] [--dry-run]

Runs doc-drift, docs-as-tests examples, doc-update issue filing, and llms export freshness.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --pr)
      [ $# -ge 2 ] || { echo "doc-freshness-tier: --pr requires a value" >&2; exit 2; }
      MODE="pr"; MODE_ARG="$2"; shift 2 ;;
    --diff)
      [ $# -ge 2 ] || { echo "doc-freshness-tier: --diff requires a value" >&2; exit 2; }
      MODE="diff"; MODE_ARG="$2"; shift 2 ;;
    --working-tree)
      MODE="working-tree"; MODE_ARG=""; shift ;;
    --repo-root)
      [ $# -ge 2 ] || { echo "doc-freshness-tier: --repo-root requires a value" >&2; exit 2; }
      REPO_ROOT="$2"; shift 2 ;;
    --dry-run)
      DRY_RUN=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "doc-freshness-tier: unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ -n "$MODE" ] || { echo "doc-freshness-tier: one of --pr, --diff, --working-tree is required" >&2; exit 2; }
if [ -z "$REPO_ROOT" ]; then
  REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

CHECK_DRIFT_SH="${AUTOSPEC_CHECK_DRIFT_SH:-${SCRIPT_DIR}/check-doc-drift.sh}"
if [ ! -f "$CHECK_DRIFT_SH" ]; then
  CHECK_DRIFT_SH="${SCRIPT_DIR}/../../../skills/autospec-shared/scripts/check-doc-drift.sh"
fi
VERIFY_EXAMPLES_MJS="${AUTOSPEC_VERIFY_EXAMPLES_MJS:-${SCRIPT_DIR}/../../autospec-doc/scripts/verify-examples.mjs}"
if [ ! -f "$VERIFY_EXAMPLES_MJS" ]; then
  VERIFY_EXAMPLES_MJS="${SCRIPT_DIR}/../skills/autospec-doc/scripts/verify-examples.mjs"
fi
GEN_LLMS_TXT_SH="${AUTOSPEC_GEN_LLMS_TXT_SH:-${SCRIPT_DIR}/gen-llms-txt.sh}"
if [ ! -f "$GEN_LLMS_TXT_SH" ]; then
  GEN_LLMS_TXT_SH="${SCRIPT_DIR}/../../../skills/autospec-shared/scripts/gen-llms-txt.sh"
fi

WORK_DIR="$(mktemp -d /tmp/autospec-doc-freshness-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT
DIFF_FILE="$WORK_DIR/diff.txt"

collect_diff() {
  case "$MODE" in
    diff)
      [ -f "$MODE_ARG" ] || { echo "doc-freshness-tier: diff file not found: $MODE_ARG" >&2; exit 2; }
      cp "$MODE_ARG" "$DIFF_FILE" ;;
    pr)
      if ! command -v gh >/dev/null 2>&1; then
        echo "doc-freshness-tier: gh CLI required for --pr" >&2; exit 2
      fi
      gh pr diff "$MODE_ARG" > "$DIFF_FILE" ;;
    working-tree)
      git -C "$REPO_ROOT" diff HEAD > "$DIFF_FILE" 2>/dev/null || true
      if [ ! -s "$DIFF_FILE" ]; then
        git -C "$REPO_ROOT" diff HEAD --cached > "$DIFF_FILE" 2>/dev/null || true
      fi ;;
  esac
  touch "$DIFF_FILE"
}

json_count() {
  local key="$1"
  node -e '
const fs = require("fs");
const key = process.argv[1];
let j = {};
try { j = JSON.parse(fs.readFileSync(0, "utf8")); } catch {}
const v = j[key];
process.stdout.write(String(Array.isArray(v) ? v.length : 0));
' "$key"
}

json_sources() {
  node -e '
const fs = require("fs");
let j = {};
try { j = JSON.parse(fs.readFileSync(0, "utf8")); } catch {}
const out = new Set();
for (const d of (j.drift || [])) for (const f of (d.matching_source_files || [])) out.add(f);
for (const d of (j.missing_scope || [])) if (d.source_file) out.add(d.source_file);
for (const d of (j.example_stale || [])) for (const f of (d.source_changed || [])) out.add(f);
process.stdout.write([...out].sort().join("\n"));
'
}

json_summary() {
  node -e '
const fs = require("fs");
let j = {};
try { j = JSON.parse(fs.readFileSync(0, "utf8")); } catch {}
const pick = (k) => Array.isArray(j[k]) ? j[k] : [];
const lines = [];
for (const e of pick("drift")) lines.push(`drift: ${e.doc_file || "<doc>"} ${e.heading || ""} sources=${(e.matching_source_files || []).join(",")}`);
for (const e of pick("missing_scope")) lines.push(`missing_scope: ${e.source_file || "<source>"}`);
for (const e of pick("example_stale")) lines.push(`example_stale: ${e.doc_file || "<doc>"} ${e.heading || ""}`);
process.stdout.write(lines.join("\n"));
'
}

changed_doc_files() {
  grep -E '^\+\+\+ ' "$DIFF_FILE" 2>/dev/null \
    | sed 's|^+++ b/||; s|^+++ /dev/null||' \
    | grep -E '^docs/.*\.md$' \
    | sort -u || true
}

run_drift_gate() {
  set +e
  case "$MODE" in
    diff) DRIFT_JSON="$(AUTOSPEC_REPO_ROOT="$REPO_ROOT" bash "$CHECK_DRIFT_SH" --diff "$DIFF_FILE" 2>"$WORK_DIR/drift.err")"; DRIFT_RC=$? ;;
    pr) DRIFT_JSON="$(AUTOSPEC_REPO_ROOT="$REPO_ROOT" bash "$CHECK_DRIFT_SH" --pr "$MODE_ARG" 2>"$WORK_DIR/drift.err")"; DRIFT_RC=$? ;;
    working-tree) DRIFT_JSON="$(AUTOSPEC_REPO_ROOT="$REPO_ROOT" bash "$CHECK_DRIFT_SH" --working-tree 2>"$WORK_DIR/drift.err")"; DRIFT_RC=$? ;;
  esac
  set -e
  if [ -s "$WORK_DIR/drift.err" ]; then cat "$WORK_DIR/drift.err" >&2; fi
  printf '%s\n' "$DRIFT_JSON"
}

file_doc_update_issue() {
  local sources summary title body labels_args=()
  sources="$(printf '%s' "$DRIFT_JSON" | json_sources)"
  summary="$(printf '%s' "$DRIFT_JSON" | json_summary)"
  title="docs: update stale documentation for public surface changes"
  body="## Goal
Update documentation scopes that drifted from public API/config/flag changes.

## Drift findings
\`\`\`
${summary:-drift detected}
\`\`\`

## Source files
\`\`\`
${sources:-<none>}
\`\`\`

## Acceptance criteria
- [ ] Update the affected docs/*.md scopes for the listed public surface changes.
- [ ] Run \`doc-freshness-tier.sh --working-tree\` and commit regenerated \`llms.txt\` / \`llms-full.txt\` if changed.
"
  echo "doc-update issue proposal: $title"
  echo "labels: $DOC_UPDATE_LABELS"
  printf '%s\n' "$summary"
  if [ "$DRY_RUN" -eq 1 ]; then
    return 0
  fi
  if ! command -v gh >/dev/null 2>&1; then
    echo "doc-freshness-tier: gh CLI unavailable; cannot file doc-update issue" >&2
    return 1
  fi
  # origin:self provenance (issue #1785): idempotent, best-effort label
  gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
  IFS=',' read -r -a _labels <<< "$DOC_UPDATE_LABELS"
  for label in "${_labels[@]}"; do
    [ -n "$label" ] && labels_args+=(--label "$label")
  done
  local issue_url
  issue_url="$(gh issue create --title "$title" --body "$body" "${labels_args[@]}")" || return 1
  bash "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/project-sync-issue.sh" "$issue_url" "$REPO_ROOT"
  printf '%s\n' "$issue_url"
}

run_examples_and_llms_for_doc_changes() {
  local docs=()
  while IFS= read -r _doc; do
    [ -n "$_doc" ] && docs+=("$_doc")
  done < <(changed_doc_files)
  if [ "${#docs[@]}" -eq 0 ]; then
    echo "doc-freshness-tier: no changed docs; llms export regeneration not required"
    return 0
  fi

  local full_docs=()
  for d in "${docs[@]}"; do
    [ -f "$REPO_ROOT/$d" ] && full_docs+=("$REPO_ROOT/$d")
  done
  if [ "${#full_docs[@]}" -gt 0 ]; then
    [ -f "$VERIFY_EXAMPLES_MJS" ] || { echo "doc-freshness-tier: verify-examples.mjs not found" >&2; return 2; }
    if ! node "$VERIFY_EXAMPLES_MJS" "${full_docs[@]}"; then
      echo "doc-freshness-tier: docs-as-tests example verification failed" >&2
      return 1
    fi
  fi

  [ -f "$GEN_LLMS_TXT_SH" ] || { echo "doc-freshness-tier: gen-llms-txt.sh not found" >&2; return 2; }
  if ! bash "$GEN_LLMS_TXT_SH" --repo-root "$REPO_ROOT"; then
    echo "doc-freshness-tier: llms export regeneration failed" >&2
    return 1
  fi

  if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    if ! git -C "$REPO_ROOT" diff --quiet -- llms.txt llms-full.txt docs/.llm-manifest.json 2>/dev/null; then
      echo "doc-freshness-tier: llms exports changed after regeneration; commit llms.txt/llms-full.txt with the doc change" >&2
      return 1
    fi
  fi
  echo "doc-freshness-tier: docs examples verified and llms exports current (${#full_docs[@]} doc file(s))"
}

collect_diff
run_drift_gate >/dev/null

DRIFT_COUNT="$(printf '%s' "$DRIFT_JSON" | json_count drift)"
MISSING_COUNT="$(printf '%s' "$DRIFT_JSON" | json_count missing_scope)"
EXAMPLE_STALE_COUNT="$(printf '%s' "$DRIFT_JSON" | json_count example_stale)"

STATUS=0
if [ "${DRIFT_COUNT:-0}" -gt 0 ] || [ "${MISSING_COUNT:-0}" -gt 0 ]; then
  file_doc_update_issue || true
  STATUS=1
fi
if [ "${EXAMPLE_STALE_COUNT:-0}" -gt 0 ]; then
  echo "doc-freshness-tier: example_stale blocks merge"
  printf '%s' "$DRIFT_JSON" | json_summary
  STATUS=1
fi
if ! run_examples_and_llms_for_doc_changes; then
  STATUS=1
fi

# Preserve check-doc-drift's hard failure when it failed for a reason not already
# covered above (for example malformed input or missing scope under old JSON).
if [ "${DRIFT_RC:-0}" -ne 0 ] && [ "$STATUS" -eq 0 ]; then
  STATUS="$DRIFT_RC"
fi

exit "$STATUS"
