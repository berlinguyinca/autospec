#!/usr/bin/env bash
# docs-completeness-gaps.sh — emit gap objects for the Phase 5.5 docs-completeness dimension.
#
# After the autospec-run queue drains, Phase 5.5's broad gap-remediation review
# also checks documentation completeness (spec §D6 row 2): every feature shipped
# in the batch window has a page for every configured audience, and there are no
# outstanding `visual_stale` / `example_stale` drift signals. Each shortfall is
# emitted as a gap object (the SAME gap-json contract `/autospec-review
# --emit-gaps` uses) so it flows through the EXISTING gap-remediation loop
# (gap-remediation-loop.sh) — there is no parallel filing/dedupe/round machinery.
#
# Emitted gaps carry `dimension: "docs-completeness"`; the gap-remediation loop
# labels them `gap-remediation` like every other survivor so later rounds do not
# re-flag freshly-fixed work.
#
# Failures of the docs check itself (missing config, drift-script error, missing
# node/jq) log a WARN to stderr and emit an empty array — this dimension NEVER
# blocks run completion (mirrors /autospec-review failure semantics).
#
# Usage:
#   docs-completeness-gaps.sh [--repo-root <dir>] [--config <autospec.yml>]
#                             [--drift-script <path>] [--no-drift] [--help]
#
# Output (stdout): a JSON array of gap objects (possibly empty).
# Exit codes: 0 always (best-effort; never blocks the run).
#
# Requires: bash 3.2+, jq, node (for audience config); check-doc-drift.sh for drift.

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHARED_DIR="$(cd "$SCRIPT_DIR/../../autospec-shared/scripts" 2>/dev/null && pwd || true)"

REPO_ROOT="$(pwd)"
CONFIG_PATH=""
DRIFT_SCRIPT=""
RUN_DRIFT=true

warn() { printf 'docs-completeness: WARN %s\n' "$*" >&2; }
emit_empty() { printf '[]\n'; exit 0; }

while [ $# -gt 0 ]; do
  case "$1" in
    --repo-root)    REPO_ROOT="${2:-}"; shift 2 ;;
    --config)       CONFIG_PATH="${2:-}"; shift 2 ;;
    --drift-script) DRIFT_SCRIPT="${2:-}"; shift 2 ;;
    --no-drift)     RUN_DRIFT=false; shift ;;
    --help|-h)
      sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) warn "unknown argument: $1"; shift ;;
  esac
done

command -v jq >/dev/null 2>&1 || { warn "jq not found"; emit_empty; }

[ -n "$CONFIG_PATH" ] || CONFIG_PATH="$REPO_ROOT/.autospec/autospec.yml"
[ -n "$DRIFT_SCRIPT" ] || DRIFT_SCRIPT="${SHARED_DIR:-}/check-doc-drift.sh"

# Shared gap-json helpers (gap_title_hash). Source best-effort.
if [ -n "$SHARED_DIR" ] && [ -f "$SHARED_DIR/gap-json-lib.sh" ]; then
  # shellcheck disable=SC1091
  . "$SHARED_DIR/gap-json-lib.sh"
fi
if ! command -v gap_title_hash >/dev/null 2>&1; then
  gap_title_hash() {
    local sum
    if command -v shasum >/dev/null 2>&1; then
      sum="$(printf '%s' "$1" | shasum | awk '{print $1}')"
    else
      sum="$(printf '%s' "$1" | sha1sum | awk '{print $1}')"
    fi
    printf '%s' "${sum:0:10}"
  }
fi

# ── 1. Resolve configured audiences (name + path) via doc-config.mjs ─────────
# Best-effort: on failure, emit no audience gaps (never block).
AUD_JSON="[]"
DOC_CONFIG="$SCRIPT_DIR/../../autospec-doc/scripts/doc-config.mjs"
if command -v node >/dev/null 2>&1 && [ -f "$DOC_CONFIG" ]; then
  AUD_JSON="$(node -e '
    import(process.argv[1]).then(m => {
      const cfg = m.loadConfig(process.argv[2]);
      const out = (cfg.audiences || []).map(a => ({ name: a.name || a.id || a.label || "", path: a.path || "" }))
        .filter(a => a.name && a.path);
      process.stdout.write(JSON.stringify(out));
    }).catch(e => { process.stderr.write(String(e)); process.stdout.write("[]"); });
  ' "$DOC_CONFIG" "$CONFIG_PATH" 2>/dev/null)" || AUD_JSON="[]"
  [ -n "$AUD_JSON" ] || AUD_JSON="[]"
else
  warn "node or doc-config.mjs unavailable — skipping audience-completeness check"
fi

# ── 2. Build the batch-window feature set: union of features documented across
#       any audience (docs/<audience>/features/<feature>.md). A feature present
#       under any audience but missing under another is an incompleteness gap.
GAPS_TMP="$(mktemp)"
trap 'rm -f "$GAPS_TMP"' EXIT

# Collect audience name/path pairs into parallel arrays (bash 3.2 safe).
aud_count="$(printf '%s' "$AUD_JSON" | jq 'length' 2>/dev/null || echo 0)"
if [ "${aud_count:-0}" -gt 0 ]; then
  # Gather the union of feature basenames across all audience feature dirs.
  feat_union="$(mktemp)"
  i=0
  while [ "$i" -lt "$aud_count" ]; do
    apath="$(printf '%s' "$AUD_JSON" | jq -r ".[$i].path")"
    fdir="$REPO_ROOT/$apath/features"
    if [ -d "$fdir" ]; then
      find "$fdir" -maxdepth 1 -name '*.md' -type f 2>/dev/null \
        | while IFS= read -r f; do basename "$f" .md; done >> "$feat_union"
    fi
    i=$((i + 1))
  done
  sort -u "$feat_union" -o "$feat_union" 2>/dev/null || true

  # For each (feature, audience) pair, flag a gap when the page is absent.
  while IFS= read -r feat; do
    [ -z "$feat" ] && continue
    i=0
    while [ "$i" -lt "$aud_count" ]; do
      aname="$(printf '%s' "$AUD_JSON" | jq -r ".[$i].name")"
      apath="$(printf '%s' "$AUD_JSON" | jq -r ".[$i].path")"
      page="$REPO_ROOT/$apath/features/$feat.md"
      if [ ! -f "$page" ]; then
        rel="$apath/features/$feat.md"
        title="docs-completeness: feature '$feat' missing $aname page ($rel)"
        body="The feature '$feat' ships pages for some audiences but not '$aname'. Generate the missing audience page via \`/autospec-doc --audience $aname\` (or \`--full\`) so every configured audience covers every shipped feature. Expected path: \`$rel\`."
        dk="docs-completeness-missing-$aname-$feat"
        jq -n \
          --arg dim "docs-completeness" \
          --arg sev "medium" \
          --arg file "$rel" \
          --arg title "$title" \
          --arg body "$body" \
          --arg dk "$dk" \
          '{dimension:$dim, severity:$sev, file:$file, line:1, title:$title, body:$body, dedupe_key:$dk}' \
          >> "$GAPS_TMP"
      fi
      i=$((i + 1))
    done
  done < "$feat_union"
  rm -f "$feat_union"
fi

# ── 3. Drift signals: visual_stale / example_stale from check-doc-drift.sh ───
if $RUN_DRIFT && [ -n "$DRIFT_SCRIPT" ] && [ -x "$DRIFT_SCRIPT" ]; then
  DRIFT_OUT="$(cd "$REPO_ROOT" && bash "$DRIFT_SCRIPT" --working-tree 2>/dev/null)"
  drift_rc=$?
  # check-doc-drift exits 1/2 on drift/missing-scope — those are valid outputs,
  # not failures. Only a non-JSON body (parse failure) is treated as an error.
  if printf '%s' "$DRIFT_OUT" | jq -e 'type == "object"' >/dev/null 2>&1; then
    for kind in visual_stale example_stale; do
      n="$(printf '%s' "$DRIFT_OUT" | jq --arg k "$kind" '(.[$k] // []) | length' 2>/dev/null || echo 0)"
      j=0
      while [ "$j" -lt "${n:-0}" ]; do
        entry="$(printf '%s' "$DRIFT_OUT" | jq -c --arg k "$kind" ".[\$k][$j]")"
        dfile="$(printf '%s' "$entry" | jq -r '.doc_file // ""')"
        head="$(printf '%s' "$entry" | jq -r '.heading // ""')"
        title="docs-completeness: $kind in $dfile ($head)"
        body="\`check-doc-drift.sh\` reports an outstanding \`$kind\` signal for \`$dfile\` (\`$head\`). Regenerate the affected scope via \`/autospec-doc\` and re-verify so no stale visual/example content survives the run. Entry: $entry"
        dk="docs-completeness-$kind-$(gap_title_hash "$dfile$head")"
        jq -n \
          --arg dim "docs-completeness" \
          --arg sev "low" \
          --arg file "$dfile" \
          --arg title "$title" \
          --arg body "$body" \
          --arg dk "$dk" \
          '{dimension:$dim, severity:$sev, file:$file, line:1, title:$title, body:$body, dedupe_key:$dk}' \
          >> "$GAPS_TMP"
        j=$((j + 1))
      done
    done
  else
    warn "check-doc-drift.sh produced no parseable JSON (rc=$drift_rc) — skipping drift signals"
  fi
fi

# ── 4. Assemble the gap array, assigning stable G<n> gap_ids in order. ───────
if [ -s "$GAPS_TMP" ]; then
  jq -s 'to_entries | map(.value + {gap_id: ("G" + ((.key + 1) | tostring))})' "$GAPS_TMP"
else
  printf '[]\n'
fi

exit 0
