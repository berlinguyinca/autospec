#!/usr/bin/env bash
# repo-quality-audit.sh — read-only repository quality audit artifact writer.
#
# Probes repository hygiene, verification surfaces, route/test coverage hints,
# dependency/security risks, and maintainability hotspots. The script writes a
# machine-readable JSON artifact plus a Markdown summary, and can optionally file
# deduplicated follow-up issues when operator policy permits it.

set -eu

usage() {
  cat <<'EOF'
repo-quality-audit.sh — read-only repository quality audit

Usage:
  repo-quality-audit.sh --repo <path> --json <path> --markdown <path> [--file-issues]

Environment:
  AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES=1  permit GitHub issue creation
  AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS=1  run configured npm verification/audit scripts

Exit: 0=artifact written, 1=usage/tool error
EOF
}

REPO="."
JSON_OUT=""
MD_OUT=""
FILE_ISSUES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --repo) REPO="${2:-}"; shift 2 ;;
    --json) JSON_OUT="${2:-}"; shift 2 ;;
    --markdown) MD_OUT="${2:-}"; shift 2 ;;
    --file-issues) FILE_ISSUES=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) printf 'repo-quality-audit: unknown option: %s\n' "$1" >&2; usage >&2; exit 1 ;;
  esac
done

[ -n "$JSON_OUT" ] || { printf 'repo-quality-audit: --json required\n' >&2; exit 1; }
[ -n "$MD_OUT" ] || { printf 'repo-quality-audit: --markdown required\n' >&2; exit 1; }
[ -d "$REPO" ] || { printf 'repo-quality-audit: repo not found: %s\n' "$REPO" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { printf 'repo-quality-audit: jq required\n' >&2; exit 1; }

REPO="$(cd "$REPO" && pwd)"
TMP_DIR="$(mktemp -d)"
FINDINGS_ND="$TMP_DIR/findings.ndjson"
SUPPRESSED_ND="$TMP_DIR/suppressed.ndjson"
ISSUES_ND="$TMP_DIR/issues.ndjson"
RISKS_ND="$TMP_DIR/risks.ndjson"
VERIFICATION_ND="$TMP_DIR/verification-lanes.ndjson"
RUNTIME_JSON="$TMP_DIR/runtime.json"
touch "$FINDINGS_ND" "$SUPPRESSED_ND" "$ISSUES_ND" "$RISKS_ND" "$VERIFICATION_ND"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ACCEPTED_FILE="$REPO/.autospec/quality-audit-accepted.json"

is_accepted() {
  key="$1"
  [ -f "$ACCEPTED_FILE" ] || return 1
  jq -e --arg key "$key" '(.accepted_debt // []) | index($key)' "$ACCEPTED_FILE" >/dev/null 2>&1
}

json_append() {
  out="$1"; shift
  jq -cn "$@" >> "$out"
}

add_finding() {
  probe="$1"; classification="$2"; severity="$3"; file="$4"; line="$5"; title="$6"; body="$7"; key="$8"
  target="$FINDINGS_ND"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  json_append "$target" \
    --arg probe "$probe" \
    --arg classification "$classification" \
    --arg severity "$severity" \
    --arg file "$file" \
    --argjson line "$line" \
    --arg title "$title" \
    --arg body "$body" \
    --arg key "$key" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key}'
}

add_guard_finding() {
  guard_script="$1"; file="$2"; line="$3"; rule="$4"; class_name="$5"; excerpt="$6"
  key="design-template-guard:$guard_script:$file"
  target="$FINDINGS_ND"
  classification="design-template-contract"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  title="design/template guard failure in $file"
  if [ -n "$class_name" ]; then
    body="Repo-specific $guard_script reported design-system class '$class_name' in $file."
  elif [ -n "$rule" ]; then
    body="Repo-specific $guard_script reported template/design contract rule '$rule' in $file."
  else
    body="Repo-specific $guard_script reported a design/template contract violation in $file."
  fi
  [ -z "$excerpt" ] || body="$body Output: $excerpt"
  json_append "$target" \
    --arg probe "design-template-guard" \
    --arg classification "$classification" \
    --arg severity "high" \
    --arg file "$file" \
    --argjson line "$line" \
    --arg title "$title" \
    --arg body "$body" \
    --arg key "$key" \
    --arg guard_script "$guard_script" \
    --arg rule "$rule" \
    --arg class_name "$class_name" \
    --arg excerpt "$excerpt" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,guard_script:$guard_script,rule:(if $rule == "" then null else $rule end),class:(if $class_name == "" then null else $class_name end),excerpt:(if $excerpt == "" then null else $excerpt end)}'
}

rel_path() {
  case "$1" in
    "$REPO"/*) printf '%s' "${1#"$REPO"/}" ;;
    *) printf '%s' "$1" ;;
  esac
}

has_package_script() {
  script="$1"
  [ -f "$REPO/package.json" ] || return 1
  jq -e --arg s "$script" '.scripts[$s] // empty' "$REPO/package.json" >/dev/null 2>&1
}

run_probe_commands_enabled() {
  [ "${AUTOSPEC_QUALITY_AUDIT_RUN_COMMANDS:-0}" = "1" ] && command -v npm >/dev/null 2>&1
}

summarize_command_output() {
  file="$1"
  tr '\n' ' ' < "$file" | cut -c1-500
}

record_verification_lane() {
  lane="$1"; status="$2"; command_text="$3"; detail="$4"
  json_append "$VERIFICATION_ND" \
    --arg lane "$lane" \
    --arg status "$status" \
    --arg command "$command_text" \
    --arg detail "$detail" \
    '{lane:$lane,status:$status,command:(if $command == "" then null else $command end),detail:(if $detail == "" then null else $detail end)}'
}

extract_guard_file() {
  line="$1"
  printf '%s\n' "$line" | grep -Eo '[[:alnum:]_./-]+\.(html|vue|svelte|css|scss|sass|less)' | head -1 || true
}

extract_guard_line_number() {
  line="$1"; file="$2"
  [ -n "$file" ] || { printf '0'; return 0; }
  printf '%s\n' "$line" | sed -nE "s#.*${file//\//\\/}[:( ]([0-9]+).*#\\1#p" | head -1
}

extract_guard_rule() {
  line="$1"
  rule="$(printf '%s\n' "$line" | grep -Eo '\[[A-Za-z0-9_-]+\]' | head -1 | tr -d '[]' || true)"
  if [ -z "$rule" ]; then
    rule="$(printf '%s\n' "$line" | sed -nE 's/.*rule[[:space:]:=]+["'"'"'`]?([A-Za-z0-9_-]+).*/\1/p' | head -1)"
  fi
  printf '%s' "$rule"
}

extract_guard_class() {
  line="$1"
  printf '%s\n' "$line" \
    | grep -Eo '(^|[^A-Za-z0-9_-])(m[trblxyse]?-[0-9]+|p[trblxyse]?-[0-9]+|alert(-[A-Za-z0-9_-]+)?|btn(-[A-Za-z0-9_-]+)?|badge(-[A-Za-z0-9_-]+)?|text-[A-Za-z0-9_-]+|bg-[A-Za-z0-9_-]+|border(-[A-Za-z0-9_-]+)?|d-[A-Za-z0-9_-]+|row|col(-[A-Za-z0-9_-]+)?)([^A-Za-z0-9_-]|$)' \
    | sed -E 's/^[^A-Za-z0-9_-]*//; s/[^A-Za-z0-9_-]*$//' \
    | head -1 || true
}

parse_design_template_guard_output() {
  script="$1"; out="$2"
  parsed=0
  while IFS= read -r line_text; do
    [ -n "$line_text" ] || continue
    file="$(extract_guard_file "$line_text")"
    [ -n "$file" ] || continue
    file="$(rel_path "$REPO/$file")"
    line_no="$(extract_guard_line_number "$line_text" "$file")"
    case "$line_no" in ''|*[!0-9]*) line_no=0 ;; esac
    rule="$(extract_guard_rule "$line_text")"
    class_name="$(extract_guard_class "$line_text")"
    excerpt="$(printf '%s' "$line_text" | cut -c1-240)"
    add_guard_finding "$script" "$file" "$line_no" "$rule" "$class_name" "$excerpt"
    parsed=$((parsed + 1))
  done < "$out"
  [ "$parsed" -gt 0 ]
}

probe_design_template_guard_script() {
  script="$1"
  has_package_script "$script" || return 0
  command_text="npm run $script"
  if ! run_probe_commands_enabled; then
    record_verification_lane "$script" "not run" "$command_text" "command probe disabled"
    return 0
  fi
  safe_script="$(printf '%s' "$script" | tr ':/' '--')"
  out="$TMP_DIR/npm-$safe_script.log"
  if ! (cd "$REPO" && npm run -s "$script") >"$out" 2>&1; then
    summary="$(summarize_command_output "$out")"
    if ! parse_design_template_guard_output "$script" "$out"; then
      add_finding "design-template-guard" "design-template-contract" "high" "package.json" 0 \
        "npm $script design/template guard fails" \
        "Repo-specific npm $script guard exited non-zero during the opt-in audit probe. First output: ${summary:-<empty>}" \
        "design-template-guard:$script"
    fi
    record_verification_lane "$script" "configured but failing" "$command_text" "${summary:-<empty>}"
  else
    record_verification_lane "$script" "passed" "$command_text" "command probe exited 0"
  fi
}

probe_npm_verification_script() {
  script="$1"
  if ! has_package_script "$script"; then
    add_finding "package-manager-scripts" "verification-contract-drift" "medium" "package.json" 0 \
      "missing npm $script script" \
      "package.json does not declare a $script script, so autospec cannot discover a standard $script gate." \
      "package-script-missing:$script"
    record_verification_lane "$script" "not configured" "" "package.json does not declare this script"
    return 0
  fi
  command_text="npm run $script"
  if ! run_probe_commands_enabled; then
    record_verification_lane "$script" "not run" "$command_text" "command probe disabled"
    return 0
  fi
  out="$TMP_DIR/npm-$script.log"
  if ! (cd "$REPO" && npm run -s "$script") >"$out" 2>&1; then
    summary="$(summarize_command_output "$out")"
    add_finding "verification-command" "verification-contract-drift" "high" "package.json" 0 \
      "npm $script script fails" \
      "Configured npm $script script exited non-zero during the opt-in audit probe. First output: ${summary:-<empty>}" \
      "verification-command:$script"
    record_verification_lane "$script" "configured but failing" "$command_text" "${summary:-<empty>}"
  else
    record_verification_lane "$script" "passed" "$command_text" "command probe exited 0"
  fi
}

probe_npm_audit_script() {
  if ! has_package_script audit; then
    dep_count="$(jq '((.dependencies // {}) + (.devDependencies // {})) | length' "$REPO/package.json" 2>/dev/null || echo 0)"
    if [ "$dep_count" -gt 0 ]; then
      add_finding "dependency-audit" "verification-contract-drift" "medium" "package.json" 0 \
        "missing dependency audit script" \
        "Dependencies are declared but no package-manager audit script is available for release/readiness checks." \
        "dependency-audit:missing-script"
    fi
    record_verification_lane "audit" "not configured" "" "package.json does not declare an audit script"
    return 0
  fi
  command_text="npm run audit"
  if ! run_probe_commands_enabled; then
    record_verification_lane "audit" "not run" "$command_text" "command probe disabled"
    return 0
  fi
  out="$TMP_DIR/npm-audit.json"
  audit_rc=0
  (cd "$REPO" && npm run -s audit -- --json) >"$out" 2>&1 || audit_rc=$?
  if jq -e '((.metadata.vulnerabilities.total // 0) > 0) or (((.vulnerabilities // {}) | length) > 0)' "$out" >/dev/null 2>&1; then
    total="$(jq -r '.metadata.vulnerabilities.total // ((.vulnerabilities // {}) | length)' "$out" 2>/dev/null || echo "unknown")"
    add_finding "dependency-audit-advisories" "current-branch-regression" "high" "package.json" 0 \
      "dependency audit reports advisories" \
      "The opt-in dependency audit probe reported ${total} vulnerability/advisory record(s)." \
      "dependency-audit:advisories"
    record_verification_lane "audit" "configured but failing" "$command_text" "audit reported ${total} vulnerability/advisory record(s)"
    return 0
  fi
  if [ "$audit_rc" -ne 0 ]; then
    summary="$(summarize_command_output "$out")"
    add_finding "dependency-audit-command" "verification-contract-drift" "high" "package.json" 0 \
      "npm audit script fails" \
      "Configured npm audit script exited non-zero during the opt-in audit probe. First output: ${summary:-<empty>}" \
      "dependency-audit:command-failed"
    record_verification_lane "audit" "configured but failing" "$command_text" "${summary:-<empty>}"
    return 0
  fi
  record_verification_lane "audit" "passed" "$command_text" "command probe exited 0"
}

version_compare_key() {
  printf '%s' "$1" | sed 's/^v//; s/[^0-9.].*$//' | awk -F. '{printf "%06d%06d%06d", $1+0, $2+0, $3+0}'
}

version_major() {
  printf '%s' "$1" | sed 's/^v//; s/[^0-9.].*$//' | awk -F. '{print $1+0}'
}

version_satisfies_atom() {
  version="$1"; atom="$2"
  atom="$(printf '%s' "$atom" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
  case "$atom" in
    \>=*)
      required="$(printf '%s' "${atom#>=}" | sed 's/^[[:space:]]*//')"
      [ "$(version_compare_key "$version")" -ge "$(version_compare_key "$required")" ]
      ;;
    \<*)
      required="$(printf '%s' "${atom#<}" | sed 's/^[[:space:]]*//')"
      [ "$(version_compare_key "$version")" -lt "$(version_compare_key "$required")" ]
      ;;
    \^*)
      required="${atom#^}"
      [ "$(version_major "$version")" -eq "$(version_major "$required")" ] \
        && [ "$(version_compare_key "$version")" -ge "$(version_compare_key "$required")" ]
      ;;
    [0-9]*)
      required="$(printf '%s' "$atom" | sed 's/[[:space:]].*$//')"
      [ "$(version_compare_key "$version")" -eq "$(version_compare_key "$required")" ]
      ;;
    *)
      return 2
      ;;
  esac
}

version_satisfies_constraint() {
  version="$1"; constraint="$2"
  remaining="$constraint"
  saw_supported=0
  while :; do
    alternative="${remaining%%||*}"
    [ "$alternative" != "$remaining" ] && remaining="${remaining#*||}" || remaining=""
    alt_supported=0
    alt_failed=0
    for atom in $alternative; do
      if version_satisfies_atom "$version" "$atom"; then
        alt_supported=1
      else
        rc=$?
        if [ "$rc" -eq 2 ]; then
          alt_failed=2
          break
        fi
        alt_supported=1
        alt_failed=1
      fi
    done
    [ "$alt_supported" -eq 1 ] && saw_supported=1
    [ "$alt_supported" -eq 1 ] && [ "$alt_failed" -eq 0 ] && return 0
    [ -n "$remaining" ] || break
  done
  [ "$saw_supported" -eq 1 ] && return 1
  return 2
}

engine_status() {
  tool="$1"; version="$2"; constraint="$3"
  if [ -z "$constraint" ]; then
    printf 'not configured'
    return 0
  fi
  if [ -z "$version" ]; then
    printf 'configured but failing'
    return 0
  fi
  if version_satisfies_constraint "$version" "$constraint"; then
    printf 'passed'
  else
    rc=$?
    if [ "$rc" -eq 2 ]; then
      printf 'not run'
    else
      printf 'configured but failing'
    fi
  fi
}

tool_version() {
  tool="$1"
  command -v "$tool" >/dev/null 2>&1 || return 0
  if [ "$tool" = "node" ]; then
    "$tool" -v 2>/dev/null | sed 's/^v//' | head -1
  else
    "$tool" --version 2>/dev/null | head -1
  fi
}

write_runtime_json() {
  if [ -f "$REPO/package.json" ]; then
    node_engine="$(jq -r '.engines.node // ""' "$REPO/package.json")"
    npm_engine="$(jq -r '.engines.npm // ""' "$REPO/package.json")"
    pnpm_engine="$(jq -r '.engines.pnpm // ""' "$REPO/package.json")"
    yarn_engine="$(jq -r '.engines.yarn // ""' "$REPO/package.json")"
  else
    node_engine=""; npm_engine=""; pnpm_engine=""; yarn_engine=""
  fi
  node_version="$(tool_version node)"
  npm_version="$(tool_version npm)"
  pnpm_version="$(tool_version pnpm)"
  yarn_version="$(tool_version yarn)"
  node_status="$(engine_status node "$node_version" "$node_engine")"
  npm_status="$(engine_status npm "$npm_version" "$npm_engine")"
  pnpm_status="$(engine_status pnpm "$pnpm_version" "$pnpm_engine")"
  yarn_status="$(engine_status yarn "$yarn_version" "$yarn_engine")"
  jq -n \
    --arg node_version "$node_version" --arg node_engine "$node_engine" --arg node_status "$node_status" \
    --arg npm_version "$npm_version" --arg npm_engine "$npm_engine" --arg npm_status "$npm_status" \
    --arg pnpm_version "$pnpm_version" --arg pnpm_engine "$pnpm_engine" --arg pnpm_status "$pnpm_status" \
    --arg yarn_version "$yarn_version" --arg yarn_engine "$yarn_engine" --arg yarn_status "$yarn_status" \
    '{
      node:{version:(if $node_version == "" then null else $node_version end),engine:(if $node_engine == "" then null else $node_engine end),status:$node_status},
      package_managers:{
        npm:{version:(if $npm_version == "" then null else $npm_version end),engine:(if $npm_engine == "" then null else $npm_engine end),status:$npm_status},
        pnpm:{version:(if $pnpm_version == "" then null else $pnpm_version end),engine:(if $pnpm_engine == "" then null else $pnpm_engine end),status:$pnpm_status},
        yarn:{version:(if $yarn_version == "" then null else $yarn_version end),engine:(if $yarn_engine == "" then null else $yarn_engine end),status:$yarn_status}
      }
    }' > "$RUNTIME_JSON"

  if [ "$node_status" = "configured but failing" ]; then
    add_finding "runtime-engine-compatibility" "verification-contract-drift" "high" "package.json" 0 \
      "local Node runtime does not satisfy engines.node" \
      "Current Node ${node_version:-<missing>} does not satisfy engines.node ${node_engine}." \
      "runtime-engine:node-version"
  fi
  for manager in npm pnpm yarn; do
    status="$(jq -r --arg manager "$manager" '.package_managers[$manager].status' "$RUNTIME_JSON")"
    engine="$(jq -r --arg manager "$manager" '.package_managers[$manager].engine // ""' "$RUNTIME_JSON")"
    version="$(jq -r --arg manager "$manager" '.package_managers[$manager].version // ""' "$RUNTIME_JSON")"
    if [ "$status" = "configured but failing" ]; then
      add_finding "runtime-engine-compatibility" "verification-contract-drift" "high" "package.json" 0 \
        "local $manager runtime does not satisfy engines.$manager" \
        "Current $manager ${version:-<missing>} does not satisfy engines.$manager ${engine}." \
        "runtime-engine:$manager-version"
    fi
  done
}

scan_text_files() {
  find "$REPO" \
    \( -path "$REPO/.git" -o -path "$REPO/node_modules" -o -path "$REPO/.autospec" \) -prune -o \
    -type f \( -name '*.js' -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.html' -o -name '*.vue' -o -name '*.svelte' -o -name '*.py' -o -name '*.sh' \) \
    -print
}

# Probe: dirty git status.
if (cd "$REPO" && git rev-parse --is-inside-work-tree >/dev/null 2>&1); then
  dirty="$(cd "$REPO" && git status --porcelain 2>/dev/null || true)"
  if [ -n "$dirty" ]; then
    add_finding "dirty-git-status" "current-branch-regression" "medium" "." 0 \
      "working tree has uncommitted changes" \
      "The audit observed uncommitted files. Treat these as current-branch risk until reviewed or committed." \
      "dirty-git-status"
  fi
fi

# Probe: package-manager scripts, runtime engine, typecheck/lint/test/audit.
write_runtime_json
if [ -f "$REPO/package.json" ]; then
  for script in test lint typecheck; do
    probe_npm_verification_script "$script"
  done
  for script in lint:styles lint:templates; do
    probe_design_template_guard_script "$script"
  done
  probe_npm_audit_script
  if ! jq -e '.engines.node // empty' "$REPO/package.json" >/dev/null 2>&1; then
    add_finding "runtime-engine-compatibility" "autospec-process-gap" "low" "package.json" 0 \
      "missing Node engine declaration" \
      "package.json has no engines.node constraint, so autospec cannot compare local and expected runtime versions." \
      "runtime-engine:node-missing"
  fi
else
  add_finding "package-manager-scripts" "autospec-process-gap" "medium" "." 0 \
    "missing package manager manifest" \
    "No package.json was found; JS/TS verification probes cannot discover scripts." \
    "package-manifest:missing"
  for script in test lint typecheck audit; do
    record_verification_lane "$script" "not configured" "" "package.json missing"
  done
fi

# Probe: route coverage.
route_files="$(find "$REPO" \( -path "$REPO/.git" -o -path "$REPO/node_modules" \) -prune -o -type f \( -iname '*route*' -o -iname '*router*' \) -print)"
if [ -n "$route_files" ]; then
  while IFS= read -r route_file; do
    [ -n "$route_file" ] || continue
    routes="$(grep -Eo "['\"][/][A-Za-z0-9_./:-]+['\"]" "$route_file" 2>/dev/null | tr -d "'\"" | sort -u || true)"
    while IFS= read -r route; do
      [ -n "$route" ] || continue
      if ! grep -R --exclude-dir=.git --exclude-dir=node_modules -F "$route" "$REPO/tests" "$REPO/src" 2>/dev/null | grep -v "$(rel_path "$route_file")" >/dev/null 2>&1; then
        add_finding "route-coverage" "app-follow-up" "medium" "$(rel_path "$route_file")" 0 \
          "route lacks nearby test coverage: $route" \
          "Route $route appears in the router but was not found in test or secondary source coverage scans." \
          "route-coverage:$route"
      fi
    done <<EOF
$routes
EOF
  done <<EOF
$route_files
EOF
fi

# Probe: design/template guards.
template_count="$(find "$REPO" \( -path "$REPO/.git" -o -path "$REPO/node_modules" \) -prune -o -type f \( -name '*.html' -o -name '*.vue' -o -name '*.svelte' \) -print | wc -l | tr -d ' ')"
if [ "$template_count" -gt 0 ] && [ ! -d "$REPO/tests" ]; then
  add_finding "design-template-guards" "autospec-process-gap" "medium" "." 0 \
    "templates exist without a tests directory" \
    "Template/design surfaces exist but no tests directory was found for guards or snapshots." \
    "design-template-guards:no-tests"
fi

# Probe: security-sensitive storage, focused/skipped tests, any usage, debug logging.
while IFS= read -r f; do
  rel="$(rel_path "$f")"
  security_hits="$(grep -nE '(localStorage|sessionStorage|document\.cookie).*(token|secret|password|auth)|((token|secret|password|auth).*(localStorage|sessionStorage|document\.cookie))' "$f" 2>/dev/null || true)"
  if [ -n "$security_hits" ]; then
    first_line="$(printf '%s\n' "$security_hits" | head -1 | cut -d: -f1)"
    add_finding "security-sensitive-storage" "current-branch-regression" "high" "$rel" "${first_line:-0}" \
      "security-sensitive browser storage" \
      "Security-sensitive token/secret/password storage pattern detected in browser storage or cookies." \
      "security-sensitive-storage:$rel"
  fi
  focus_hits="$(grep -nE '\b(describe|it|test)\.(only|skip)\b|@skip|\.skip\(' "$f" 2>/dev/null || true)"
  if [ -n "$focus_hits" ]; then
    first_line="$(printf '%s\n' "$focus_hits" | head -1 | cut -d: -f1)"
    add_finding "focused-skipped-tests" "app-follow-up" "medium" "$rel" "${first_line:-0}" \
      "focused test markers present" \
      "Focused or skipped tests can hide regressions from autospec verification." \
      "focused-skipped-tests:$rel"
  fi
  any_hits="$(grep -nE '\bas any\b|: *any\b|<any>' "$f" 2>/dev/null || true)"
  if [ -n "$any_hits" ]; then
    first_line="$(printf '%s\n' "$any_hits" | head -1 | cut -d: -f1)"
    add_finding "any-usage" "app-follow-up" "low" "$rel" "${first_line:-0}" \
      "TypeScript any usage" \
      "The audit found an explicit any usage that weakens static verification." \
      "any-usage:$rel"
  fi
  debug_hits="$(grep -nE '\b(console\.(log|debug|warn|error)|debugger)\b' "$f" 2>/dev/null || true)"
  if [ -n "$debug_hits" ]; then
    first_line="$(printf '%s\n' "$debug_hits" | head -1 | cut -d: -f1)"
    add_finding "debug-logging-hotspots" "app-follow-up" "low" "$rel" "${first_line:-0}" \
      "debug logging hotspot" \
      "Debug logging or debugger statements remain in application code." \
      "debug-logging-hotspots:$rel"
  fi
done <<EOF
$(scan_text_files)
EOF

# Probe: large files.
while IFS= read -r f; do
  [ -n "$f" ] || continue
  size="$(wc -c < "$f" | tr -d ' ')"
  if [ "$size" -gt 512000 ]; then
    rel="$(rel_path "$f")"
    add_finding "large-files" "app-follow-up" "low" "$rel" 0 \
      "large repository file" \
      "File is larger than 512 KiB and may indicate generated or bundled content checked into source." \
      "large-files:$rel"
  fi
done <<EOF
$(find "$REPO" \( -path "$REPO/.git" -o -path "$REPO/node_modules" \) -prune -o -type f -print)
EOF

ndjson_to_array() {
  file="$1"
  if [ -s "$file" ]; then
    jq -s '.' "$file"
  else
    printf '[]\n'
  fi
}

FINDINGS_JSON="$TMP_DIR/findings.json"
SUPPRESSED_JSON="$TMP_DIR/suppressed.json"
ISSUES_JSON="$TMP_DIR/issues.json"
RISKS_JSON="$TMP_DIR/risks.json"
VERIFICATION_JSON="$TMP_DIR/verification-lanes.json"
ndjson_to_array "$FINDINGS_ND" > "$FINDINGS_JSON"
ndjson_to_array "$SUPPRESSED_ND" > "$SUPPRESSED_JSON"
ndjson_to_array "$VERIFICATION_ND" > "$VERIFICATION_JSON"

issue_policy_permits=0
if [ "$FILE_ISSUES" -eq 1 ] && [ "${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-0}" = "1" ] && command -v gh >/dev/null 2>&1; then
  issue_policy_permits=1
fi

open_issues='[]'
if [ "$issue_policy_permits" -eq 1 ]; then
  open_issues="$(cd "$REPO" && gh issue list --state open --limit 500 --json number,title,body,labels,url 2>/dev/null || echo '[]')"
  printf '%s' "$open_issues" | jq -e 'type=="array"' >/dev/null 2>&1 || open_issues='[]'
fi

existing_issue_for_finding() {
  key="$1"; title="$2"
  printf '%s' "$open_issues" | jq -c --arg key "$key" --arg title "$title" \
    'first(.[] | select((.title == $title) or ((.body // "") | contains($key)))) // empty'
}

if [ "$issue_policy_permits" -eq 1 ]; then
  (cd "$REPO" && gh label create quality-audit --color d4c5f9 --force >/dev/null 2>&1) || true
  (cd "$REPO" && gh label create auto-implement --color 0e8a16 --force >/dev/null 2>&1) || true
  (cd "$REPO" && gh label create autospec:v2-flow --color 1d76db --force >/dev/null 2>&1) || true
  count="$(jq 'length' "$FINDINGS_JSON")"
  i=0
  created_issue_keys=""
  while [ "$i" -lt "$count" ]; do
    finding="$(jq -c ".[$i]" "$FINDINGS_JSON")"
    i=$((i + 1))
    key="$(printf '%s' "$finding" | jq -r '.dedupe_key')"
    if printf '%s\n' "$created_issue_keys" | grep -Fx "$key" >/dev/null 2>&1; then
      continue
    fi
    title="$(printf '%s' "$finding" | jq -r '"autospec audit: " + .title')"
    existing="$(existing_issue_for_finding "$key" "$title")"
    if [ -n "$existing" ]; then
      existing_title="$(printf '%s' "$existing" | jq -r --arg title "$title" '.title // $title')"
      existing_url="$(printf '%s' "$existing" | jq -r '.url // ""')"
      json_append "$ISSUES_ND" --arg title "$existing_title" --arg url "$existing_url" --arg key "$key" \
        '{title:$title,url:$url,dedupe_key:$key,existing:true}'
      created_issue_keys="${created_issue_keys}${key}
"
      continue
    fi
    body="$(printf '%s' "$finding" | jq -r '"## Goal\n" + .body + "\n\n## Acceptance criteria\n- [ ] Address `" + .dedupe_key + "` in `" + .file + "`.\n\n---\n- probe: " + .probe + "\n- classification: " + .classification + "\n- severity: " + .severity + "\n- dedupe_key: " + .dedupe_key')"
    url="$(cd "$REPO" && gh issue create --title "$title" --body "$body" --label "quality-audit" --label "auto-implement" --label "autospec:v2-flow" 2>/dev/null || true)"
    if [ -n "$url" ]; then
      json_append "$ISSUES_ND" --arg title "$title" --arg url "$url" --arg key "$key" \
        '{title:$title,url:$url,dedupe_key:$key}'
      created_issue_keys="${created_issue_keys}${key}
"
    fi
  done
fi
ndjson_to_array "$ISSUES_ND" > "$ISSUES_JSON"

issue_keys="$(jq -r '.[].dedupe_key' "$ISSUES_JSON" | sed 's/[.[\*^$()+?{}|]/\\&/g' | paste -sd'|' - || true)"
finding_count="$(jq 'length' "$FINDINGS_JSON")"
i=0
while [ "$i" -lt "$finding_count" ]; do
  finding="$(jq -c ".[$i]" "$FINDINGS_JSON")"
  i=$((i + 1))
  key="$(printf '%s' "$finding" | jq -r '.dedupe_key')"
  if [ -n "$issue_keys" ] && printf '%s\n' "$key" | grep -E "^($issue_keys)$" >/dev/null 2>&1; then
    continue
  fi
  risk="$(printf '%s' "$finding" | jq -r '"Unfiled " + .classification + ": " + .title + " (" + .dedupe_key + ")"')"
  jq -cn --arg risk "$risk" '$risk' >> "$RISKS_ND"
done
ndjson_to_array "$RISKS_ND" > "$RISKS_JSON"

total_findings="$(jq 'length' "$FINDINGS_JSON")"
suppressed_findings="$(jq 'length' "$SUPPRESSED_JSON")"
issue_count="$(jq 'length' "$ISSUES_JSON")"
risk_count="$(jq 'length' "$RISKS_JSON")"
status="pass"
[ "$total_findings" -eq 0 ] || status="fail"

mkdir -p "$(dirname "$JSON_OUT")" "$(dirname "$MD_OUT")"
jq -n \
  --arg status "$status" \
  --arg repo "$REPO" \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --argjson findings "$(cat "$FINDINGS_JSON")" \
  --argjson suppressed "$(cat "$SUPPRESSED_JSON")" \
  --argjson issue_links "$(cat "$ISSUES_JSON")" \
  --argjson residual_risks "$(cat "$RISKS_JSON")" \
  --argjson runtime "$(cat "$RUNTIME_JSON")" \
  --argjson verification_lanes "$(cat "$VERIFICATION_JSON")" \
  --argjson total_findings "$total_findings" \
  --argjson suppressed_findings "$suppressed_findings" \
  --argjson issue_links_count "$issue_count" \
  --argjson risk_count "$risk_count" \
  '{
    status:$status,
    repo:$repo,
    generated_at:$generated_at,
    summary:{
      total_findings:$total_findings,
      suppressed_findings:$suppressed_findings,
      issue_links:$issue_links_count,
      unfiled_residual_risks:$risk_count
    },
    runtime:$runtime,
    verification:{
      lanes:($verification_lanes | map({key:.lane,value:{status:.status,command:.command,detail:.detail}}) | from_entries)
    },
    findings:$findings,
    suppressed:$suppressed,
    issue_links:$issue_links,
    residual_risks:$residual_risks
  }' > "$JSON_OUT"

{
  printf '# autospec repo quality audit\n\n'
  printf -- '- Status: %s\n' "$status"
  printf -- '- Findings: %s\n' "$total_findings"
  printf -- '- Suppressed findings: %s\n' "$suppressed_findings"
  printf -- '- Filed issues: %s\n' "$issue_count"
  printf -- '- Unfiled residual risks: %s\n\n' "$risk_count"
  printf '## Runtime and engines\n\n'
  jq -r '
    "- node: " + (.node.version // "unavailable") + " (engine: " + (.node.engine // "not configured") + ", status: " + .node.status + ")",
    "- npm: " + (.package_managers.npm.version // "unavailable") + " (engine: " + (.package_managers.npm.engine // "not configured") + ", status: " + .package_managers.npm.status + ")",
    "- pnpm: " + (.package_managers.pnpm.version // "unavailable") + " (engine: " + (.package_managers.pnpm.engine // "not configured") + ", status: " + .package_managers.pnpm.status + ")",
    "- yarn: " + (.package_managers.yarn.version // "unavailable") + " (engine: " + (.package_managers.yarn.engine // "not configured") + ", status: " + .package_managers.yarn.status + ")"
  ' "$RUNTIME_JSON"
  printf '\n## Verification contract\n\n'
  if [ "$(jq 'length' "$VERIFICATION_JSON")" -eq 0 ]; then
    printf '(none)\n'
  else
    jq -r '.[] | "- " + .lane + ": " + .status + (if .command then " (`" + .command + "`)" else "" end)' "$VERIFICATION_JSON"
  fi
  printf '\n'
  printf '## Findings\n\n'
  if [ "$total_findings" -eq 0 ]; then
    printf '(none)\n'
  else
    jq -r '.[] | "- [" + .severity + "] " + .probe + " / " + .classification + " — " + .title + " (`" + .dedupe_key + "`)"' "$FINDINGS_JSON"
  fi
  printf '\n## Suppressed findings\n\n'
  if [ "$suppressed_findings" -eq 0 ]; then
    printf '(none)\n'
  else
    jq -r '.[] | "- " + .title + " (`" + .dedupe_key + "`)"' "$SUPPRESSED_JSON"
  fi
  printf '\n## Filed issues\n\n'
  if [ "$issue_count" -eq 0 ]; then
    printf '(none)\n'
  else
    jq -r '.[] | "- " + .title + " — " + .url' "$ISSUES_JSON"
  fi
  printf '\n## Residual risks\n\n'
  if [ "$risk_count" -eq 0 ]; then
    printf '(none)\n'
  else
    jq -r '.[] | "- " + .' "$RISKS_JSON"
  fi
} > "$MD_OUT"

printf 'repo-quality-audit: status=%s findings=%s suppressed=%s issues=%s residual=%s\n' \
  "$status" "$total_findings" "$suppressed_findings" "$issue_count" "$risk_count"
exit 0
