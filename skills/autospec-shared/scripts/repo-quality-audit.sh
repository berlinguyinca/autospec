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
ARTIFACTS_JSON="$TMP_DIR/artifacts.json"
STORAGE_KEYS="$TMP_DIR/storage-keys.txt"
HOTSPOTS_ND="$TMP_DIR/maintainability-hotspots.ndjson"
HOTSPOT_KEYS="$TMP_DIR/maintainability-hotspot-keys.txt"
F64_MODULE_KEYS="$TMP_DIR/f64-module-keys.txt"
FAILED_ISSUE_KEYS="$TMP_DIR/failed-issue-keys.txt"
touch "$FINDINGS_ND" "$SUPPRESSED_ND" "$ISSUES_ND" "$RISKS_ND" "$VERIFICATION_ND"
touch "$STORAGE_KEYS" "$HOTSPOTS_ND" "$HOTSPOT_KEYS" "$FAILED_ISSUE_KEYS"
touch "$F64_MODULE_KEYS"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

ACCEPTED_FILE="$REPO/.autospec/quality-audit-accepted.json"

is_accepted() {
  accepted_key="$1"
  [ -f "$ACCEPTED_FILE" ] || return 1
  jq -e --arg key "$accepted_key" '(.accepted_debt // []) | index($key)' "$ACCEPTED_FILE" >/dev/null 2>&1
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

probe_mock_policy() {
  policy_file="$REPO/AGENTS.md"
  [ -f "$policy_file" ] || return 0
  grep -Eiq 'real[- ]service|no mocks|without mocks|testcontainers|real DB|real database' "$policy_file" || return 0
  while IFS= read -r file; do
    [ -f "$file" ] || continue
    rel="${file#"$REPO"/}"
    case "$rel" in
      */unit/*|tests/unit/*) class="unit" ;;
      */integration/*|tests/integration/*|*/it/*) class="integration" ;;
      */contract/*|tests/contract/*) class="contract" ;;
      */e2e/*|tests/e2e/*|*/smoke/*|tests/smoke/*) class="e2e" ;;
      *) class="integration" ;;
    esac
    exception=0
    grep -Eq 'quality-audit:[[:space:]]*mock-exception[[:space:]]+[^[:space:]]' "$file" && exception=1 || true
    grep -Ein 'wiremock|mockito|mockall|(^|[^[:alnum:]_])(mock|stub)[[:space:]_]*(broker|db|database|postgres|mysql|redis|kafka|rabbit)' "$file" 2>/dev/null | while IFS=: read -r line excerpt; do
      [ "$exception" -eq 1 ] && continue
      framework="mock/stub"
      printf '%s' "$excerpt" | grep -Eiq 'wiremock' && framework="wiremock"
      printf '%s' "$excerpt" | grep -Eiq 'mockito' && framework="mockito"
      printf '%s' "$excerpt" | grep -Eiq 'mockall' && framework="mockall"
      target="broker/DB"
      classification="mock-policy-$class"
      if [ "$class" = "integration" ]; then
        kind="migrate-to-real-service"
      else
        kind="mock-exception-review"
      fi
      add_finding "mock-policy" "$classification" "high" "$rel" "${line:-0}" \
        "forbidden $framework mock targets $target ($class test)" \
        "Mock framework $framework targets forbidden service $target in a $class test; action: $kind." \
        "mock-policy:$rel:$line"
    done
  done < <(find "$REPO" \( -path '*/.git' -o -path '*/node_modules' \) -prune -o -type f \( -path '*/tests/*' -o -path '*/test/*' \) -print)
}

# Rust numeric invariant probe. It is opt-in: a repository must explicitly
# declare its numeric policy in AGENTS.md before f64 usage becomes actionable.
probe_rust_f64_invariants() {
  policy="$REPO/AGENTS.md"
  [ -f "$policy" ] || return 0
  grep -Eiq 'numeric invariant|numeric invariants|Decimal|f64[[:space:]].*(money|price|quantity|pnl|mean|std|sharpe|t_stat)|money.*Decimal' "$policy" || return 0
  while IFS= read -r file; do
    rel="${file#"$REPO"/}"
    class="harmless scalar"
    case "$rel" in
      tests/*|*/tests/*|test/*|*/test/*) class="test-only" ;;
    esac
    while IFS=: read -r line excerpt; do
      [ -n "$excerpt" ] || continue
      case "$excerpt" in
        *quality-audit:\ f64-bridge*) continue ;;
      esac
      lower="$(printf '%s' "$excerpt" | tr '[:upper:]' '[:lower:]')"
      if printf '%s' "$lower" | grep -Eq '(^|[^[:alnum:]_])(price|cost|money|amount|pnl)([^[:alnum:]_]|$)'; then
        class="money/price"
      elif printf '%s' "$lower" | grep -Eq '(^|[^[:alnum:]_])(qty|quantity|count|volume)([^[:alnum:]_]|$)'; then
        class="quantity"
      elif printf '%s' "$lower" | grep -Eq '(^|[^[:alnum:]_])(percent|percentage|ratio|rate)([^[:alnum:]_]|$)'; then
        class="percentage"
      elif printf '%s' "$lower" | grep -Eq '(^|[^[:alnum:]_])(mean|std|sharpe|t_stat|variance|stdev)([^[:alnum:]_]|$)'; then
        class="statistical metric"
      fi
      case "$class" in
        "money/price"|quantity|"statistical metric")
          module="${rel%%/src/*}"
          grep -Fqx "$module" "$F64_MODULE_KEYS" && continue
          printf '%s\n' "$module" >> "$F64_MODULE_KEYS"
          add_finding "f64-numeric-invariant" "$class" "medium" "$rel" "${line:-0}" \
            "f64 numeric invariant in $module" \
            "${class} value uses f64; prefer Decimal or document a quality-audit: f64-bridge annotation. classification=$class" \
            "f64-numeric-invariant:$module" ;;
      esac
    done < <(grep -Ein '(^|[^[:alnum:]_])f64([^[:alnum:]_]|$)' "$file" 2>/dev/null || true)
  done < <(source_scan_find -type f -name '*.rs' -print)
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

add_route_registry_drift_finding() {
  script="$1"; family="$2"; routes_json="$3"
  count="$(printf '%s' "$routes_json" | jq 'length')"
  sample="$(printf '%s' "$routes_json" | jq -r 'join(", ")')"
  key="route-registry-drift:$script:$family"
  target="$FINDINGS_ND"
  classification="app-follow-up"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  json_append "$target" \
    --arg probe "route-registry-drift" \
    --arg classification "$classification" \
    --arg severity "high" \
    --arg file "package.json" \
    --argjson line "0" \
    --arg title "route registry drift in $family routes" \
    --arg body "Route coverage meta test $script reported $count live route(s) missing from the curated route registry or smoke-test catalog for family $family: $sample." \
    --arg key "$key" \
    --arg script "$script" \
    --arg family "$family" \
    --argjson routes "$routes_json" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,route_coverage_script:$script,route_family:$family,missing_routes:$routes}'
}

add_route_compiler_warning_finding() {
  script="$1"; code="$2"; line_text="$3"; idx="$4"
  excerpt="$(printf '%s' "$line_text" | cut -c1-240)"
  key="route-coverage-warning:$script:$code:$idx"
  target="$FINDINGS_ND"
  classification="app-follow-up"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  json_append "$target" \
    --arg probe "route-coverage-compiler-warning" \
    --arg classification "$classification" \
    --arg severity "medium" \
    --arg file "package.json" \
    --argjson line "0" \
    --arg title "route coverage compiler warning $code" \
    --arg body "Route coverage meta test $script emitted Angular compiler warning $code. Output: $excerpt" \
    --arg key "$key" \
    --arg script "$script" \
    --arg code "$code" \
    --arg excerpt "$excerpt" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,route_coverage_script:$script,warning_code:$code,excerpt:$excerpt}'
}

add_route_setup_failure_finding() {
  script="$1"; summary="$2"
  key="route-coverage-setup:$script"
  target="$FINDINGS_ND"
  classification="verification-contract-drift"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  json_append "$target" \
    --arg probe "route-coverage-setup" \
    --arg classification "$classification" \
    --arg severity "high" \
    --arg file "package.json" \
    --argjson line "0" \
    --arg title "route coverage meta test setup failed" \
    --arg body "Route coverage meta test could not reach its required local server or setup dependency. First output: ${summary:-<empty>}" \
    --arg key "$key" \
    --arg script "$script" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,route_coverage_script:$script}'
}

add_dependency_advisory_finding() {
  manager="$1"; package_name="$2"; advisory_severity="$3"; dependency_type="$4"; fix_available="$5"; semver_major="$6"; artifact="$7"
  key="dependency-audit:$manager:$package_name"
  target="$FINDINGS_ND"
  classification="dependency-security"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  if [ "$fix_available" = "false" ]; then
    fix_text="No package-manager fix is available; human review or replacement is required."
  elif [ "$semver_major" = "true" ]; then
    fix_text="A fix is available, but it requires a semver-major upgrade."
  else
    fix_text="A package-manager fix is available."
  fi
  json_append "$target" \
    --arg probe "dependency-audit-advisory" \
    --arg classification "$classification" \
    --arg severity "$advisory_severity" \
    --arg file "package.json" \
    --argjson line "0" \
    --arg title "$manager audit reports $advisory_severity advisory in $package_name" \
    --arg body "$manager audit reports a $advisory_severity advisory in $package_name ($dependency_type dependency). $fix_text Raw audit JSON: $artifact." \
    --arg key "$key" \
    --arg manager "$manager" \
    --arg package_name "$package_name" \
    --arg advisory_severity "$advisory_severity" \
    --arg dependency_type "$dependency_type" \
    --argjson fix_available "$fix_available" \
    --argjson semver_major "$semver_major" \
    --arg artifact "$artifact" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,dependency_manager:$manager,package_name:$package_name,advisory_severity:$advisory_severity,dependency_type:$dependency_type,fix_available:$fix_available,semver_major_fix:$semver_major,raw_artifact:$artifact}'
}

add_sensitive_storage_finding() {
  file="$1"; line="$2"; storage_api="$3"; sensitive_term="$4"; storage_key="$5"; excerpt="$6"
  key="security-sensitive-storage:$file:$storage_api:$sensitive_term"
  [ -z "$storage_key" ] || key="$key:$storage_key"
  if grep -Fx "$key" "$STORAGE_KEYS" >/dev/null 2>&1; then
    return 0
  fi
  printf '%s\n' "$key" >> "$STORAGE_KEYS"
  target="$FINDINGS_ND"
  classification="client-credential-storage"
  if is_accepted "$key" || is_accepted "security-sensitive-storage:$file"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  json_append "$target" \
    --arg probe "security-sensitive-storage" \
    --arg classification "$classification" \
    --arg severity "high" \
    --arg file "$file" \
    --argjson line "$line" \
    --arg title "security-sensitive browser storage in $file" \
    --arg body "Detected $storage_api access involving sensitive term '$sensitive_term'${storage_key:+ and storage key '$storage_key'}. Browser storage for tokens, API keys, credentials, auth state, or group authorization state should be reviewed. Excerpt: $excerpt" \
    --arg key "$key" \
    --arg storage_api "$storage_api" \
    --arg sensitive_term "$sensitive_term" \
    --arg storage_key "$storage_key" \
    --arg excerpt "$excerpt" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,storage_api:$storage_api,sensitive_term:$sensitive_term,storage_key:(if $storage_key == "" then null else $storage_key end),excerpt:$excerpt}'
}

add_maintainability_hotspot_finding() {
  file="$1"; rank="$2"; score="$3"; lines="$4"; kind="$5"; any_count="$6"; debug_count="$7"; disabled_count="$8"; eslint_count="$9"; ts_ignore_count="${10}"; recent_touch="${11}"; any_density="${12}"; test_signal="${13}"
  key="maintainability-hotspot:$file"
  target="$FINDINGS_ND"
  classification="maintainability-hotspot"
  if is_accepted "$key"; then
    classification="inherited-accepted-debt"
    target="$SUPPRESSED_ND"
  fi
  title="ranked maintainability hotspot #$rank: $file"
  body="Rank #$rank maintainability hotspot ($kind) scored $score from $lines lines, any=$any_count, any_density=$any_density, debug_logging=$debug_count, disabled_tests=$disabled_count, eslint_disable=$eslint_count, ts_ignore=$ts_ignore_count, recent_touch=$recent_touch, test_signal=$test_signal. File a bounded refactor issue for this file/cluster only, and require behavior locks or regression tests before cleanup edits."
  json_append "$target" \
    --arg probe "maintainability-hotspot" \
    --arg classification "$classification" \
    --arg severity "medium" \
    --arg file "$file" \
    --argjson line "0" \
    --arg title "$title" \
    --arg body "$body" \
    --arg key "$key" \
    --argjson rank "$rank" \
    --argjson score "$score" \
    --argjson lines "$lines" \
    --arg kind "$kind" \
    --argjson any_count "$any_count" \
    --argjson debug_count "$debug_count" \
    --argjson disabled_count "$disabled_count" \
    --argjson eslint_count "$eslint_count" \
    --argjson ts_ignore_count "$ts_ignore_count" \
    --arg recent_touch "$recent_touch" \
    --argjson any_density "$any_density" \
    --arg test_signal "$test_signal" \
    '{probe:$probe,classification:$classification,severity:$severity,file:$file,line:$line,title:$title,body:$body,dedupe_key:$key,rank:$rank,score:$score,hotspot_kind:$kind,lines:$lines,any_count:$any_count,any_density:$any_density,debug_logging_count:$debug_count,disabled_test_count:$disabled_count,eslint_disable_count:$eslint_count,ts_ignore_count:$ts_ignore_count,recent_touch:$recent_touch,test_signal:$test_signal,remediation:"bounded refactor follow-up; behavior locks/regression tests required before cleanup edits"}'
}

external_path_identity() {
  path="$1"
  base="${path##*/}"
  [ -n "$base" ] || base="unknown"
  identity="$(printf '%s' "$path" | cksum | awk '{print $1}')"
  printf 'external/%s-%s' "$base" "$identity"
}

canonical_git_common_dir() {
  root="$1"
  common="$(git -C "$root" rev-parse --git-common-dir 2>/dev/null)" || return 1
  case "$common" in /*) : ;; *) common="$root/$common" ;; esac
  (cd "$common" 2>/dev/null && pwd -P)
}
normalize_repo_path() {
  path="$1"; repo_root="$(cd "$REPO" 2>/dev/null && pwd -P || printf '%s' "$REPO")"
  case "$path" in
    /*) case "/$path/" in */../*) external_path_identity "$path"; return 0 ;; esac ;;
  esac
  case "$path" in /*) canonical_dir="$(cd "$(dirname "$path")" 2>/dev/null && pwd -P || true)"; [ -z "$canonical_dir" ] || path="$canonical_dir/$(basename "$path")" ;; esac
  case "$path" in
    "$repo_root"/*) printf '%s' "${path#"$repo_root"/}"; return 0 ;;
    /*)
      container="$(dirname "$path")"
      foreign_root="$(git -C "$container" rev-parse --show-toplevel 2>/dev/null || true)"
      if [ -n "$foreign_root" ]; then
        foreign_root="$(cd "$foreign_root" 2>/dev/null && pwd -P || true)"
        repo_common="$(canonical_git_common_dir "$REPO" || true)"
        foreign_common="$(canonical_git_common_dir "$foreign_root" || true)"
        if [ -n "$repo_common" ] && [ "$repo_common" = "$foreign_common" ]; then
          case "$path" in
            "$foreign_root"/*)
              relative="${path#"$foreign_root"/}"
              if [ -e "$REPO/$relative" ]; then
                printf '%s' "$relative"
                return 0
              fi
              ;;
          esac
        fi
      fi
      external_path_identity "$path"
      ;;
    ./*) printf '%s' "${path#./}" ;;
    *) printf '%s' "$path" ;;
  esac
}

rel_path() {
  normalize_repo_path "$1"
}

normalize_title_identity() {
  printf '%s' "$1" \
    | LC_ALL=C tr '[:upper:]' '[:lower:]' \
    | LC_ALL=C sed 's/[^a-z0-9][^a-z0-9]*/-/g; s/^-//; s/-$//'
}

canonicalize_findings() {
  source_file="$1"
  canonical_file="$TMP_DIR/canonical-$(basename "$source_file")"
  : > "$canonical_file"
  while IFS= read -r finding; do
    [ -n "$finding" ] || continue
    raw_path="$(printf '%s' "$finding" | jq -r '.file // "."')"
    normalized_path="$(normalize_repo_path "$raw_path")"
    display_title="$(printf '%s' "$finding" | jq -r --arg raw "$raw_path" --arg normalized "$normalized_path" \
      '(.title // "") | split($raw) | join($normalized)')"
    normalized_title="$(normalize_title_identity "$display_title")"
    semantic_seed="$(printf '%s' "$finding" | jq -r --arg raw "$raw_path" --arg normalized "$normalized_path" \
      '.dedupe_key | split($raw) | join($normalized)')"
    canonical_key="${semantic_seed}|path=${normalized_path}|title=${normalized_title}"
    printf '%s' "$finding" | jq -c \
      --arg raw "$raw_path" \
      --arg path "$normalized_path" \
      --arg title "$display_title" \
      --arg normalized_title "$normalized_title" \
      --arg key "$canonical_key" '
        walk(if type == "string" then split($raw) | join($path) else . end)
        | .file = $path
        | .title = $title
        | .normalized_path = $path
        | .normalized_title = $normalized_title
        | .dedupe_key = $key
      ' >> "$canonical_file"
  done < "$source_file"
  mv "$canonical_file" "$source_file"
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

artifact_rel_path() {
  file="$1"
  case "$file" in
    "$REPO"/*) printf '%s' "${file#"$REPO"/}" ;;
    *) printf '%s' "$file" ;;
  esac
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
    reported_file="$file"
    line_no="$(extract_guard_line_number "$line_text" "$reported_file")"
    case "$file" in /*) : ;; *) file="$REPO/$file" ;; esac
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

route_coverage_scripts() {
  [ -f "$REPO/package.json" ] || return 0
  jq -r '
    (.scripts // {}) | to_entries[]
    | select(((.key + " " + .value) | ascii_downcase | test("route"))
        and ((.key + " " + .value) | ascii_downcase | test("coverage|registry|smoke|catalog|meta")))
    | .key
  ' "$REPO/package.json" 2>/dev/null | sort -u
}

route_coverage_targets() {
  route_coverage_scripts | while IFS= read -r script; do
    [ -n "$script" ] || continue
    printf 'npm-script\t%s\tnpm run %s\n' "$script" "$script"
  done
  for spec in \
    "e2e-playwright/route-coverage.meta.spec.ts" \
    "e2e-playwright/route-coverage.spec.ts" \
    "tests/route-coverage.meta.spec.ts" \
    "tests/route-coverage.spec.ts"
  do
    [ -f "$REPO/$spec" ] || continue
    printf 'playwright-spec\troute-coverage:%s\tCI=1 npx playwright test %s --project=chrome\n' "$spec" "$spec"
  done
}

route_family_for() {
  route="$1"
  first="${route%%/*}"
  case "$first" in
    ""|"."|".."|":"*) printf 'root' ;;
    *) printf '%s' "$first" | tr -c 'A-Za-z0-9_.-' '-' | sed 's/^-*//; s/-*$//; s/^$/root/' ;;
  esac
}

route_coverage_setup_failed() {
  out="$1"
  grep -Eiq 'ECONNREFUSED|ERR_CONNECTION_REFUSED|server[^[:alnum:]]+(not )?running|dev server|baseURL|base url|connection refused|unable to connect|could not connect|failed to connect|address already in use|EADDRINUSE' "$out"
}

extract_route_registry_missing_routes() {
  out="$1"
  awk '
    function clean(s) {
      gsub(/\r/, "", s)
      gsub(/^[[:space:]]*([-*]|•)[[:space:]]*/, "", s)
      gsub(/^[[:space:]]*(Missing|missing)[^:]*:[[:space:]]*/, "", s)
      gsub(/[`"'\'';,]/, "", s)
      sub(/[[:space:]]+$/, "", s)
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]].*$/, "", s)
      sub(/^\/+/, "", s)
      sub(/\/+$/, "", s)
      return s
    }
    function valid(s) {
      return s != "" && s !~ /^(ROUTE_REGISTRY|Route|route|routes|registry|catalog)$/ && s ~ /^[A-Za-z0-9_.:-]+(\/[A-Za-z0-9_.:-]+)*$/
    }
    {
      low = tolower($0)
      if (low ~ /(live routes missing|missing.*(route_registry|route registry|smoke-test catalog|smoke test catalog|route catalog|registry)|not registered|unregistered)/) {
        in_missing = 1
        if ($0 ~ /:/) {
          cand = $0
          sub(/^.*:/, "", cand)
          cand = clean(cand)
          if (valid(cand)) print cand
        }
        next
      }
      if (in_missing && $0 ~ /^[[:space:]]*([-*]|•)[[:space:]]*"?[A-Za-z0-9_.:-]+(\/[A-Za-z0-9_.:-]+)*/) {
        cand = clean($0)
        if (valid(cand)) print cand
        next
      }
      if (in_missing && $0 !~ /^[[:space:]]*$/ && $0 !~ /^[[:space:]]*([-*]|•)/) {
        in_missing = 0
      }
    }
  ' "$out" | sort -u
}

parse_route_coverage_output() {
  script="$1"; out="$2"
  routes_file="$TMP_DIR/route-coverage-routes.tsv"
  : > "$routes_file"
  while IFS= read -r route; do
    [ -n "$route" ] || continue
    family="$(route_family_for "$route")"
    printf '%s\t%s\n' "$family" "$route" >> "$routes_file"
  done <<EOF
$(extract_route_registry_missing_routes "$out")
EOF

  if [ -s "$routes_file" ]; then
    cut -f1 "$routes_file" | sort -u | while IFS= read -r family; do
      [ -n "$family" ] || continue
      routes_json="$(awk -F '\t' -v family="$family" '$1 == family {print $2}' "$routes_file" | sort -u | jq -R . | jq -s '.')"
      add_route_registry_drift_finding "$script" "$family" "$routes_json"
    done
  fi

  warning_idx=0
  while IFS= read -r warning_line; do
    [ -n "$warning_line" ] || continue
    code="$(printf '%s\n' "$warning_line" | grep -Eo 'NG[0-9]{4}' | head -1 || true)"
    [ -n "$code" ] || continue
    warning_idx=$((warning_idx + 1))
    add_route_compiler_warning_finding "$script" "$code" "$warning_line" "$warning_idx"
  done <<EOF
$(grep -E 'NG[0-9]{4}' "$out" 2>/dev/null || true)
EOF

  [ -s "$routes_file" ] || [ "$warning_idx" -gt 0 ]
}

probe_route_coverage_script() {
  kind="$1"; script="$2"; command_text="$3"
  if ! run_probe_commands_enabled; then
    record_verification_lane "$script" "not run" "$command_text" "command probe disabled"
    return 0
  fi
  safe_script="$(printf '%s' "$script" | tr ':/' '--')"
  out="$TMP_DIR/route-coverage-$safe_script.log"
  if [ "$kind" = "playwright-spec" ]; then
    spec="${script#route-coverage:}"
    if (cd "$REPO" && CI=1 npx playwright test "$spec" --project=chrome) >"$out" 2>&1; then
      status=0
    else
      status=$?
    fi
  else
    if (cd "$REPO" && npm run -s "$script") >"$out" 2>&1; then
      status=0
    else
      status=$?
    fi
  fi
  if [ "$status" -ne 0 ]; then
    summary="$(summarize_command_output "$out")"
    if parse_route_coverage_output "$script" "$out"; then
      :
    elif route_coverage_setup_failed "$out"; then
      add_route_setup_failure_finding "$script" "$summary"
      record_verification_lane "$script" "setup failure" "$command_text" "${summary:-<empty>}"
      return 0
    else
      add_finding "route-coverage-command" "verification-contract-drift" "high" "package.json" 0 \
        "route coverage meta test fails" \
        "Configured route coverage meta test $script exited non-zero, but autospec could not parse route registry drift from its output. First output: ${summary:-<empty>}" \
        "route-coverage-command:$script"
    fi
    record_verification_lane "$script" "configured but failing" "$command_text" "${summary:-<empty>}"
  else
    parse_route_coverage_output "$script" "$out" || true
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
    artifact_dir="$(dirname "$JSON_OUT")/artifacts"
    mkdir -p "$artifact_dir"
    artifact_path="$artifact_dir/npm-audit.json"
    cp "$out" "$artifact_path"
    artifact_rel="$(artifact_rel_path "$artifact_path")"
    total="$(jq -r '.metadata.vulnerabilities.total // ((.vulnerabilities // {}) | length)' "$out" 2>/dev/null || echo "unknown")"
    advisory_count="$(jq '((.vulnerabilities // {}) | length)' "$out" 2>/dev/null || echo 0)"
    if [ "$advisory_count" -gt 0 ]; then
      jq -r '
      (.vulnerabilities // {}) | to_entries[]
      | .key as $name
      | .value as $v
      | [
          $name,
          ($v.severity // "unknown"),
          (if ($v.isDirect // false) then "direct" else "transitive" end),
          (if (($v.fixAvailable // false) == false) then "false" else "true" end),
          (if (($v.fixAvailable | type) == "object" and ($v.fixAvailable.isSemVerMajor // false)) then "true" else "false" end)
        ] | @tsv
      ' "$out" | while IFS="$(printf '\t')" read -r package_name advisory_severity dependency_type fix_available semver_major; do
        [ -n "$package_name" ] || continue
        add_dependency_advisory_finding "npm" "$package_name" "$advisory_severity" "$dependency_type" "$fix_available" "$semver_major" "$artifact_rel"
      done
    else
      add_finding "dependency-audit-advisories" "dependency-security" "high" "package.json" 0 \
        "dependency audit reports advisories" \
        "The npm audit JSON reported ${total} vulnerability/advisory record(s), but did not expose per-package vulnerability metadata. Raw audit JSON: $artifact_rel." \
        "dependency-audit:advisories"
    fi
    jq -n --arg npm_audit "$artifact_rel" '{npm_audit:$npm_audit}' > "$ARTIFACTS_JSON"
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

has_non_npm_package_manifest() {
  for manifest in Cargo.toml pyproject.toml requirements.txt setup.py setup.cfg Gemfile go.mod pom.xml build.gradle build.gradle.kts build.sbt; do
    [ -f "$REPO/$manifest" ] && return 0
  done
  return 1
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

source_scan_find() {
  # -name prunes at any depth (per-crate target/, .claude worktrees are repo copies);
  # -path stays root-anchored for names real source also uses, e.g. src/build/.
  find "$REPO" \( -name .git -o -name node_modules -o -name .autospec -o -name .claude -o -name target \
    -o -name vendor -o -name .angular -o -name .next -o -path "$REPO/dist" -o -path "$REPO/build" \
    -o -path "$REPO/coverage" -o -path "$REPO/out" -o -path "$REPO/public/build" \) -prune -o "$@"
}

scan_text_files() {
  source_scan_find \
    -type f \( -name '*.js' -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.html' -o -name '*.vue' -o -name '*.svelte' -o -name '*.py' -o -name '*.sh' \) \
    -print
}

scan_text_file_metrics() {
  source_scan_find \
    -type f \( -name '*.js' -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' -o -name '*.mjs' -o -name '*.cjs' -o -name '*.html' -o -name '*.vue' -o -name '*.svelte' -o -name '*.py' -o -name '*.sh' \) \
    -exec awk '
      function emit_summary() {
        if (current != "") {
          print "summary\t" current "\t" (lines + 0) "\t" (focus_count + 0) "\t" (focus_line + 0) "\t" (any_count + 0) "\t" (any_line + 0) "\t" (debug_count + 0) "\t" (debug_line + 0) "\t" (eslint_count + 0) "\t" (eslint_line + 0) "\t" (ts_ignore_count + 0) "\t" (ts_ignore_line + 0)
        }
      }
      function reset_file() {
        lines=0; focus_count=0; focus_line=0; any_count=0; any_line=0; debug_count=0; debug_line=0; eslint_count=0; eslint_line=0; ts_ignore_count=0; ts_ignore_line=0
      }
      FILENAME != current {
        emit_summary()
        current=FILENAME
        reset_file()
      }
      {
        lines=FNR
        if ($0 ~ /localStorage|sessionStorage|document[.]cookie/) print "storage\t" FILENAME "\t" FNR "\t" $0
        if ($0 ~ /(^|[^[:alnum:]_])(describe|it|test)[.](only|skip)([^[:alnum:]_]|$)|@skip|[.]skip[(]/) { focus_count++; if (!focus_line) focus_line=FNR }
        if ($0 ~ /(^|[^[:alnum:]_])as any([^[:alnum:]_]|$)|: *any([^[:alnum:]_]|$)|<any>/) { any_count++; if (!any_line) any_line=FNR }
        if ($0 ~ /(^|[^[:alnum:]_])console[.](log|debug|warn|error)([^[:alnum:]_]|$)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)/) { debug_count++; if (!debug_line) debug_line=FNR }
        if ($0 ~ /eslint-disable/) { eslint_count++; if (!eslint_line) eslint_line=FNR }
        if ($0 ~ /@ts-ignore|@ts-expect-error/) { ts_ignore_count++; if (!ts_ignore_line) ts_ignore_line=FNR }
      }
      END { emit_summary() }
    ' {} +
}

sensitive_storage_term() {
  line="$1"
  printf '%s\n' "$line" | grep -Eio 'x-api-key|access[_-]?token|refresh[_-]?token|id[_-]?token|api[_-]?key|token|secret|password|credential|authorization|bearer|auth|user[_-]?groups?|groups?' | head -1 || true
}

storage_api_for_line() {
  line="$1"
  printf '%s\n' "$line" | grep -Eo 'localStorage|sessionStorage|document\.cookie' | head -1 || true
}

storage_key_for_line() {
  line="$1"
  key="$(printf '%s\n' "$line" | sed -nE "s/.*(localStorage|sessionStorage)\.(getItem|setItem|removeItem)\([[:space:]]*['\"]([^'\"]+)['\"].*/\3/p" | head -1)"
  if [ -z "$key" ]; then
    key="$(printf '%s\n' "$line" | sed -nE "s/.*(localStorage|sessionStorage)\[['\"]([^'\"]+)['\"]\].*/\2/p" | head -1)"
  fi
  printf '%s' "$key"
}

file_kind_for() {
  file="$1"
  case "$file" in
    *.spec.ts|*.spec.tsx|*.test.ts|*.test.tsx|*.spec.js|*.test.js) printf 'spec' ;;
    *.html|*.vue|*.svelte) printf 'template' ;;
    *.ts|*.tsx|*.js|*.jsx|*.mjs|*.cjs) printf 'source' ;;
    *) printf 'other' ;;
  esac
}

recent_touch_for() {
  file="$1"
  if (cd "$REPO" && git rev-parse --is-inside-work-tree >/dev/null 2>&1); then
    touched="$(cd "$REPO" && git log -1 --format=%cs -- "$file" 2>/dev/null || true)"
    [ -n "$touched" ] || touched="untracked"
    printf '%s' "$touched"
  else
    printf 'unknown'
  fi
}

recent_rank_for() {
  touch_date="$1"
  case "$touch_date" in
    [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) printf '%s' "$touch_date" | tr -d '-' ;;
    *) printf '0' ;;
  esac
}

json_config_value() {
  key="$1"
  config_file="$REPO/.autospec/quality-audit.json"
  [ -f "$config_file" ] || return 0
  jq -r --arg key "$key" '.maintainability[$key] // empty' "$config_file"
}

validate_quality_audit_config() {
  config_file="$REPO/.autospec/quality-audit.json"
  [ -f "$config_file" ] || return 0
  if ! jq -e 'type == "object"' "$config_file" >/dev/null 2>&1; then
    printf 'repo-quality-audit: .autospec/quality-audit.json must be valid JSON object\n' >&2
    exit 1
  fi
  if ! jq -e '(.maintainability // {}) | type == "object"' "$config_file" >/dev/null 2>&1; then
    printf 'repo-quality-audit: .autospec/quality-audit.json maintainability must be an object\n' >&2
    exit 1
  fi
}

non_negative_int_or_die() {
  name="$1"; value="$2"
  case "$value" in
    ''|*[!0-9]*)
      printf 'repo-quality-audit: %s must be a non-negative integer\n' "$name" >&2
      exit 1
      ;;
  esac
}

maintainability_threshold() {
  config_key="$1"; env_name="$2"; default_value="$3"
  env_value="$(eval "printf '%s' \"\${$env_name-}\"")"
  if [ -n "$env_value" ]; then
    value="$env_value"
  else
    value="$(json_config_value "$config_key")"
    [ -n "$value" ] || value="$default_value"
  fi
  non_negative_int_or_die "$config_key" "$value"
  printf '%s' "$value"
}

validate_quality_audit_config

THRESHOLD_LINES="$(maintainability_threshold "large_file_lines" "AUTOSPEC_QUALITY_AUDIT_LARGE_FILE_LINES" "800")"
THRESHOLD_ANY="$(maintainability_threshold "any_threshold" "AUTOSPEC_QUALITY_AUDIT_ANY_THRESHOLD" "10")"
THRESHOLD_DEBUG="$(maintainability_threshold "debug_threshold" "AUTOSPEC_QUALITY_AUDIT_DEBUG_THRESHOLD" "5")"
THRESHOLD_DISABLED="$(maintainability_threshold "disabled_test_threshold" "AUTOSPEC_QUALITY_AUDIT_DISABLED_TEST_THRESHOLD" "1")"
THRESHOLD_DISABLE="$(maintainability_threshold "disable_comment_threshold" "AUTOSPEC_QUALITY_AUDIT_DISABLE_COMMENT_THRESHOLD" "1")"

TEST_FILE_EXTENSIONS=".spec.ts .spec.tsx .test.ts .test.tsx .spec.js .test.js"

test_signal_for() {
  rel="$1"; kind="$2"
  case "$kind" in
    spec) printf 'self-spec'; return 0 ;;
  esac
  base="${rel%.*}"
  for ext in $TEST_FILE_EXTENSIONS; do
    if [ -f "$REPO/$base$ext" ]; then
      printf 'adjacent-spec'
      return 0
    fi
  done
  printf 'none'
}

cap_score_component() {
  value="$1"; cap="$2"
  if [ "$value" -gt "$cap" ]; then
    printf '%s' "$cap"
  else
    printf '%s' "$value"
  fi
}

record_hotspot_metrics() {
  rel="$1"; lines="$2"; kind="$3"; any_count="$4"; debug_count="$5"; disabled_count="$6"; eslint_count="$7"; ts_ignore_count="$8"; recent_touch="$9"; test_signal="${10}"
  if [ "$lines" -lt "$THRESHOLD_LINES" ] \
    && [ "$any_count" -lt "$THRESHOLD_ANY" ] \
    && [ "$debug_count" -lt "$THRESHOLD_DEBUG" ] \
    && [ "$disabled_count" -lt "$THRESHOLD_DISABLED" ] \
    && [ "$eslint_count" -lt "$THRESHOLD_DISABLE" ] \
    && [ "$ts_ignore_count" -lt "$THRESHOLD_DISABLE" ]; then
    return 0
  fi
  [ "$lines" -gt 0 ] || lines=1
  any_density_per_1000=$(( any_count * 1000 / lines ))
  any_density="$(jq -n --argjson density "$any_density_per_1000" '$density / 1000')"
  any_component="$(cap_score_component "$((any_count * 3))" 60)"
  debug_component="$(cap_score_component "$((debug_count * 2))" 40)"
  score=$(( lines / 100 + any_component + any_density_per_1000 / 10 + debug_component + disabled_count * 25 + eslint_count * 10 + ts_ignore_count * 15 ))
  case "$kind" in
    source) score=$((score + 10)) ;;
    template) score=$((score + 8)) ;;
    spec) score=$((score + 4)) ;;
  esac
  case "$test_signal" in
    adjacent-spec) score=$((score + 25)) ;;
    self-spec) score=$((score + 10)) ;;
  esac
  recent_rank="$(recent_rank_for "$recent_touch")"
  if [ "$recent_rank" -gt 0 ]; then
    score=$((score + 20))
  fi
  json_append "$HOTSPOTS_ND" \
    --arg file "$rel" \
    --arg kind "$kind" \
    --arg recent_touch "$recent_touch" \
    --arg test_signal "$test_signal" \
    --argjson score "$score" \
    --argjson recent_rank "$recent_rank" \
    --argjson lines "$lines" \
    --argjson any_density "$any_density" \
    --argjson any_count "$any_count" \
    --argjson debug_count "$debug_count" \
    --argjson disabled_count "$disabled_count" \
    --argjson eslint_count "$eslint_count" \
    --argjson ts_ignore_count "$ts_ignore_count" \
    '{file:$file,kind:$kind,recent_touch:$recent_touch,test_signal:$test_signal,score:$score,recent_rank:$recent_rank,lines:$lines,any_density:$any_density,any_count:$any_count,debug_count:$debug_count,disabled_count:$disabled_count,eslint_count:$eslint_count,ts_ignore_count:$ts_ignore_count}'
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

# Probe: forbidden broker/database mocks under real-service testing policy.
probe_mock_policy
probe_rust_f64_invariants

# Probe: package-manager scripts, runtime engine, typecheck/lint/test/audit.
write_runtime_json
if [ -f "$REPO/package.json" ]; then
  for script in test lint typecheck; do
    probe_npm_verification_script "$script"
  done
  for script in lint:styles lint:templates; do
    probe_design_template_guard_script "$script"
  done
  while IFS="$(printf '\t')" read -r kind script command_text; do
    [ -n "$script" ] || continue
    probe_route_coverage_script "$kind" "$script" "$command_text"
  done <<EOF
$(route_coverage_targets)
EOF
  probe_npm_audit_script
  if ! jq -e '.engines.node // empty' "$REPO/package.json" >/dev/null 2>&1; then
    add_finding "runtime-engine-compatibility" "autospec-process-gap" "low" "package.json" 0 \
      "missing Node engine declaration" \
      "package.json has no engines.node constraint, so autospec cannot compare local and expected runtime versions." \
      "runtime-engine:node-missing"
  fi
else
  if has_non_npm_package_manifest; then
    for script in test lint typecheck audit; do
      record_verification_lane "$script" "not applicable" "" "non-npm package manifest detected"
    done
  else
    add_finding "package-manager-scripts" "autospec-process-gap" "medium" "." 0 \
      "missing package manager manifest" \
      "No supported package manager manifest was found; JS/TS verification probes cannot discover scripts." \
      "package-manifest:missing"
    for script in test lint typecheck audit; do
      record_verification_lane "$script" "not configured" "" "package manifest missing"
    done
  fi
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
METRICS_TSV="$TMP_DIR/text-file-metrics.tsv"
scan_text_file_metrics > "$METRICS_TSV"
while IFS="$(printf '\t')" read -r tag file line_no line_text; do
  [ "$tag" = "storage" ] || continue
  rel="$(rel_path "$file")"
  storage_api="$(storage_api_for_line "$line_text")"
  sensitive_term="$(sensitive_storage_term "$line_text")"
  storage_key="$(storage_key_for_line "$line_text")"
  if [ -z "$sensitive_term" ] && [ -n "$storage_key" ]; then
    sensitive_term="$(sensitive_storage_term "$storage_key")"
  fi
  [ -n "$storage_api" ] || continue
  [ -n "$sensitive_term" ] || continue
  excerpt="$(printf '%s' "$line_text" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//' | cut -c1-240)"
  add_sensitive_storage_finding "$rel" "${line_no:-0}" "$storage_api" "$sensitive_term" "$storage_key" "$excerpt"
done < "$METRICS_TSV"

while IFS="$(printf '\t')" read -r tag file lines focus_count focus_first_line any_count any_first_line debug_count debug_first_line eslint_count eslint_first_line ts_ignore_count ts_ignore_first_line; do
  [ "$tag" = "summary" ] || continue
  [ -n "$file" ] || continue
  rel="$(rel_path "$file")"
  kind="$(file_kind_for "$rel")"
  disabled_count="$focus_count"
  if [ "$focus_count" -gt 0 ]; then
    add_finding "focused-skipped-tests" "app-follow-up" "medium" "$rel" "${focus_first_line:-0}" \
      "focused test markers present" \
      "Focused or skipped tests can hide regressions from autospec verification." \
      "focused-skipped-tests:$rel"
  fi
  if [ "$any_count" -gt 0 ]; then
    add_finding "any-usage" "app-follow-up" "low" "$rel" "${any_first_line:-0}" \
      "TypeScript any usage" \
      "The audit found an explicit any usage that weakens static verification." \
      "any-usage:$rel"
  fi
  if [ "$debug_count" -gt 0 ]; then
    add_finding "debug-logging-hotspots" "app-follow-up" "low" "$rel" "${debug_first_line:-0}" \
      "debug logging hotspot" \
      "Debug logging or debugger statements remain in application code." \
      "debug-logging-hotspots:$rel"
  fi
  if [ "$eslint_count" -gt 0 ]; then
    add_finding "eslint-disable-usage" "app-follow-up" "medium" "$rel" "${eslint_first_line:-0}" \
      "eslint-disable usage" \
      "eslint-disable comments can hide maintainability and correctness regressions from static checks." \
      "eslint-disable:$rel"
  fi
  if [ "$ts_ignore_count" -gt 0 ]; then
    add_finding "ts-ignore-usage" "app-follow-up" "medium" "$rel" "${ts_ignore_first_line:-0}" \
      "TypeScript suppression usage" \
      "@ts-ignore or @ts-expect-error suppressions should be justified and reduced during bounded cleanup." \
      "ts-ignore:$rel"
  fi
  if [ "$lines" -lt "$THRESHOLD_LINES" ] \
    && [ "$any_count" -lt "$THRESHOLD_ANY" ] \
    && [ "$debug_count" -lt "$THRESHOLD_DEBUG" ] \
    && [ "$disabled_count" -lt "$THRESHOLD_DISABLED" ] \
    && [ "$eslint_count" -lt "$THRESHOLD_DISABLE" ] \
    && [ "$ts_ignore_count" -lt "$THRESHOLD_DISABLE" ]; then
    continue
  fi
  recent_touch="$(recent_touch_for "$rel")"
  test_signal="$(test_signal_for "$rel" "$kind")"
  record_hotspot_metrics "$rel" "$lines" "$kind" "$any_count" "$debug_count" "$disabled_count" "$eslint_count" "$ts_ignore_count" "$recent_touch" "$test_signal"
done < "$METRICS_TSV"

if [ -s "$HOTSPOTS_ND" ]; then
  rank=0
  jq -c -s 'sort_by(-.score, -.recent_rank, -.any_density, -.lines, .file) | .[:10] | .[]' "$HOTSPOTS_ND" | while IFS= read -r hotspot; do
    [ -n "$hotspot" ] || continue
    rank=$((rank + 1))
    file="$(printf '%s' "$hotspot" | jq -r '.file')"
    printf 'maintainability-hotspot:%s\n' "$file" >> "$HOTSPOT_KEYS"
    score="$(printf '%s' "$hotspot" | jq -r '.score')"
    lines="$(printf '%s' "$hotspot" | jq -r '.lines')"
    kind="$(printf '%s' "$hotspot" | jq -r '.kind')"
    any_count="$(printf '%s' "$hotspot" | jq -r '.any_count')"
    any_density="$(printf '%s' "$hotspot" | jq -r '.any_density')"
    debug_count="$(printf '%s' "$hotspot" | jq -r '.debug_count')"
    disabled_count="$(printf '%s' "$hotspot" | jq -r '.disabled_count')"
    eslint_count="$(printf '%s' "$hotspot" | jq -r '.eslint_count')"
    ts_ignore_count="$(printf '%s' "$hotspot" | jq -r '.ts_ignore_count')"
    recent_touch="$(printf '%s' "$hotspot" | jq -r '.recent_touch')"
    test_signal="$(printf '%s' "$hotspot" | jq -r '.test_signal')"
    add_maintainability_hotspot_finding "$file" "$rank" "$score" "$lines" "$kind" "$any_count" "$debug_count" "$disabled_count" "$eslint_count" "$ts_ignore_count" "$recent_touch" "$any_density" "$test_signal"
  done
fi

# Probe: large files.
while IFS= read -r f; do
  [ -n "$f" ] || continue
  rel="$(rel_path "$f")"
  add_finding "large-files" "app-follow-up" "low" "$rel" 0 \
    "large repository file" \
    "File is larger than 512 KiB and may indicate generated or bundled content checked into source." \
    "large-files:$rel"
done <<EOF
$(source_scan_find -type f -size +512k -print)
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
canonicalize_findings "$FINDINGS_ND"
canonicalize_findings "$SUPPRESSED_ND"
ndjson_to_array "$FINDINGS_ND" > "$FINDINGS_JSON"

# Async-aware Rust lock probe: only report std locks in an async function or
# when the acquisition scope contains an await. Synchronous callbacks can
# explicitly document the boundary with quality-audit: sync-boundary.
while IFS= read -r rust_file; do
  rel="${rust_file#"$REPO"/}"
  test_only=0; case "$rel" in tests/*|*/tests/*) test_only=1;; esac
  line_no=0
  while IFS= read -r line; do
    line_no=$((line_no + 1))
    printf '%s' "$line" | grep -Eq 'std::sync::(Mutex|RwLock).*(lock|read|write)|\.((lock)|(read)|(write))\(\)' || continue
    context="synchronous"
    start=$((line_no - 20)); [ "$start" -lt 1 ] && start=1
    end=$((line_no + 20))
    snippet="$(sed -n "${start},${end}p" "$rust_file")"
    if printf '%s' "$snippet" | grep -Eq 'async[[:space:]]+fn|\.await'; then context="async-boundary"; fi
    if printf '%s' "$snippet" | grep -Fq 'quality-audit: sync-boundary'; then continue; fi
    [ "$context" = async-boundary ] || continue
    class="production-async-lock"; [ "$test_only" -eq 1 ] && class="test-only-async-lock"
    add_finding "sync-lock-async-aware" "$class" medium "$rel" "$line_no" \
      "std::sync lock crosses an async boundary" \
      "Replace with tokio::sync equivalent; evidence: $context" \
      "sync-lock-async-aware:$rel:$line_no"
  done < "$rust_file"
done < <(source_scan_find -type f -name '*.rs' -print)
ndjson_to_array "$FINDINGS_ND" > "$FINDINGS_JSON"
ndjson_to_array "$SUPPRESSED_ND" > "$SUPPRESSED_JSON"
ndjson_to_array "$VERIFICATION_ND" > "$VERIFICATION_JSON"
[ -f "$ARTIFACTS_JSON" ] || printf '{}\n' > "$ARTIFACTS_JSON"

issue_policy_permits=0
if [ "$FILE_ISSUES" -eq 1 ] && [ "${AUTOSPEC_QUALITY_AUDIT_FILE_ISSUES:-0}" = "1" ] && command -v gh >/dev/null 2>&1; then
  issue_policy_permits=1
fi

OPEN_ISSUES_JSON="$TMP_DIR/open-issues.json"
CLOSED_ISSUES_JSON="$TMP_DIR/closed-issues.json"
printf '[]\n' > "$OPEN_ISSUES_JSON"
printf '[]\n' > "$CLOSED_ISSUES_JSON"
issue_catalog_ok=1
if [ "$issue_policy_permits" -eq 1 ]; then
  if ! (cd "$REPO" && gh issue list --state open --limit 500 --json number,state,title,body,labels,url 2>/dev/null) > "$OPEN_ISSUES_JSON"; then
    issue_catalog_ok=0
    printf '[]\n' > "$OPEN_ISSUES_JSON"
  elif ! jq -e 'type=="array"' "$OPEN_ISSUES_JSON" >/dev/null 2>&1; then
    issue_catalog_ok=0
    printf '[]\n' > "$OPEN_ISSUES_JSON"
  fi
  if ! (cd "$REPO" && gh issue list --state closed --limit 500 --json number,state,title,body,labels,url 2>/dev/null) > "$CLOSED_ISSUES_JSON"; then
    issue_catalog_ok=0
    printf '[]\n' > "$CLOSED_ISSUES_JSON"
  elif ! jq -e 'type=="array"' "$CLOSED_ISSUES_JSON" >/dev/null 2>&1; then
    issue_catalog_ok=0
    printf '[]\n' > "$CLOSED_ISSUES_JSON"
  fi
fi

issue_catalog="$(jq -cn --slurpfile open "$OPEN_ISSUES_JSON" --slurpfile closed "$CLOSED_ISSUES_JSON" \
  '($open[0] | map(.state = "OPEN")) + ($closed[0] | map(.state = "CLOSED"))')"

existing_issue_for_finding() {
  key="$1"; title="$2"; semantic_seed="${key%%|path=*}"
  key_path="${key#*|path=}"
  key_path="${key_path%%|title=*}"
  key_title="${key##*|title=}"
  marker="<!-- autospec-quality-audit-dedupe:v2:$key -->"
  while IFS= read -r candidate; do
    [ -n "$candidate" ] || continue
    candidate_body="$(printf '%s' "$candidate" | jq -r '.body // ""')"
    if printf '%s\n' "$candidate_body" | grep -Fx "$marker" >/dev/null 2>&1; then
      printf '%s' "$candidate"
      return 0
    fi
    if printf '%s\n' "$candidate_body" | grep -F '<!-- autospec-quality-audit-dedupe:v2:' >/dev/null 2>&1; then
      continue
    fi
    legacy_key="$(printf '%s\n' "$candidate_body" | sed -n \
      -e 's/^[[:space:]]*-[[:space:]]*dedupe_key:[[:space:]]*//p' \
      -e 's/^[[:space:]]*dedupe_key:[[:space:]]*//p' | head -1)"
    [ -n "$legacy_key" ] || continue
    path_occurrences="$(jq -nr --arg seed "$semantic_seed" --arg path "$key_path" '$seed | split($path) | length - 1')"
    if [ "$path_occurrences" -ne 1 ]; then
      [ "$legacy_key" = "$semantic_seed" ] || continue
      legacy_path=""
    else
      seed_prefix="$(jq -nr --arg seed "$semantic_seed" --arg path "$key_path" '$seed | split($path)[0]')"
      seed_suffix="$(jq -nr --arg seed "$semantic_seed" --arg path "$key_path" '$seed | split($path)[1]')"
      legacy_path="$(jq -nr --arg legacy "$legacy_key" --arg prefix "$seed_prefix" --arg suffix "$seed_suffix" '
        if ($legacy | startswith($prefix)) and ($legacy | endswith($suffix))
        then $legacy | ltrimstr($prefix) | rtrimstr($suffix)
        else empty end
      ')"
      [ -n "$legacy_path" ] || continue
      if [ "$legacy_path" != "$key_path" ]; then
        case "$legacy_path" in /*/"$key_path") : ;; *) continue ;; esac
      fi
      normalized_seed="$(printf '%s' "$legacy_key" | jq -Rr --arg raw "$legacy_path" --arg path "$key_path" 'split($raw) | join($path)')"
      [ "$normalized_seed" = "$semantic_seed" ] || continue
    fi
    candidate_title="$(printf '%s' "$candidate" | jq -r '.title // ""')"
    if [ -n "$legacy_path" ]; then
      candidate_title="$(printf '%s' "$candidate_title" | jq -Rr --arg raw "$legacy_path" --arg path "$key_path" 'split($raw) | join($path)')"
    fi
    candidate_title="${candidate_title#autospec audit: }"
    [ "$(normalize_title_identity "$candidate_title")" = "$key_title" ] || continue
    printf '%s' "$candidate"
    return 0
  done <<EOF
$(printf '%s' "$issue_catalog" | jq -c '.[]')
EOF
}

coalesced_by_hotspot() {
  probe="$1"; file="$2"
  case "$probe" in
    any-usage|debug-logging-hotspots|eslint-disable-usage|ts-ignore-usage)
      grep -Fx "maintainability-hotspot:$file" "$HOTSPOT_KEYS" >/dev/null 2>&1
      ;;
    *) return 1 ;;
  esac
}

if [ "$issue_policy_permits" -eq 1 ] && [ "$issue_catalog_ok" -eq 1 ]; then
  (cd "$REPO" && gh label create quality-audit --color d4c5f9 --force >/dev/null 2>&1) || true
  (cd "$REPO" && gh label create auto-implement --color 0e8a16 --force >/dev/null 2>&1) || true
  (cd "$REPO" && gh label create autospec:v2-flow --color 1d76db --force >/dev/null 2>&1) || true
  # origin:self provenance (issue #1785): idempotent, best-effort label
  (cd "$REPO" && gh label create origin:self --color 8250df --force >/dev/null 2>&1) || true
  count="$(jq 'length' "$FINDINGS_JSON")"
  i=0
  created_issue_keys=""
  while [ "$i" -lt "$count" ]; do
    finding="$(jq -c ".[$i]" "$FINDINGS_JSON")"
    i=$((i + 1))
    key="$(printf '%s' "$finding" | jq -r '.dedupe_key')"
    probe="$(printf '%s' "$finding" | jq -r '.probe')"
    file="$(printf '%s' "$finding" | jq -r '.file')"
    if coalesced_by_hotspot "$probe" "$file"; then
      continue
    fi
    if printf '%s\n' "$created_issue_keys" | grep -Fx "$key" >/dev/null 2>&1; then
      continue
    fi
    title="$(printf '%s' "$finding" | jq -r '"autospec audit: " + .title')"
    existing="$(existing_issue_for_finding "$key" "$title")"
    if [ -n "$existing" ]; then
      existing_title="$(printf '%s' "$existing" | jq -r --arg title "$title" '.title // $title')"
      existing_url="$(printf '%s' "$existing" | jq -r '.url // ""')"
      existing_state="$(printf '%s' "$existing" | jq -r '.state // "OPEN"')"
      if [ "$existing_state" = "CLOSED" ]; then
        existing_number="$(printf '%s' "$existing" | jq -r '.number')"
        recurrence_body="$TMP_DIR/recurrence-$existing_number.md"
        printf 'Recurring autospec quality-audit evidence:\n\n- dedupe_key: `%s`\n- file: `%s`\n- title: %s\n' \
          "$key" "$file" "$title" > "$recurrence_body"
        if ! (cd "$REPO" && gh issue comment "$existing_number" --body-file "$recurrence_body" >/dev/null 2>&1); then
          printf '%s\n' "$key" >> "$FAILED_ISSUE_KEYS"
          continue
        fi
        if ! (cd "$REPO" && gh issue reopen "$existing_number" >/dev/null 2>&1); then
          printf '%s\n' "$key" >> "$FAILED_ISSUE_KEYS"
          continue
        fi
        json_append "$ISSUES_ND" --arg title "$existing_title" --arg url "$existing_url" --arg key "$key" \
          '{title:$title,url:$url,dedupe_key:$key,existing:true,reopened:true}'
      else
        json_append "$ISSUES_ND" --arg title "$existing_title" --arg url "$existing_url" --arg key "$key" \
          '{title:$title,url:$url,dedupe_key:$key,existing:true}'
      fi
      created_issue_keys="${created_issue_keys}${key}
"
      continue
    fi
    body="$(printf '%s' "$finding" | jq -r '"## Goal\n" + .body + "\n\n## Acceptance criteria\n- [ ] Address `" + .dedupe_key + "` in `" + .file + "`.\n\n---\n- probe: " + .probe + "\n- classification: " + .classification + "\n- severity: " + .severity + "\n- dedupe_key: " + .dedupe_key + "\n\n<!-- autospec-quality-audit-dedupe:v2:" + .dedupe_key + " -->"')"
    url="$(cd "$REPO" && gh issue create --title "$title" --body "$body" --label "quality-audit" --label "auto-implement" --label "autospec:v2-flow" --label "origin:self" 2>/dev/null || true)"
    if [ -n "$url" ]; then
      json_append "$ISSUES_ND" --arg title "$title" --arg url "$url" --arg key "$key" \
        '{title:$title,url:$url,dedupe_key:$key}'
      created_issue_keys="${created_issue_keys}${key}
"
    fi
  done
fi
ndjson_to_array "$ISSUES_ND" > "$ISSUES_JSON"

jq --slurpfile issues "$ISSUES_JSON" --rawfile hotspot_keys "$HOTSPOT_KEYS" --rawfile failed_issue_keys "$FAILED_ISSUE_KEYS" '
  ($issues[0] | map(.dedupe_key)) as $issue_keys
  | ($hotspot_keys | split("\n") | map(select(length > 0))) as $hotspot_keys
  | ($failed_issue_keys | split("\n") | map(select(length > 0))) as $failed_issue_keys
  | [ .[]
      | select((.dedupe_key as $key | ($issue_keys | index($key)) | not))
      | select(
          if (.dedupe_key as $key | ($failed_issue_keys | index($key)) != null) then
            true
          elif (.probe == "any-usage" or .probe == "debug-logging-hotspots" or .probe == "focused-skipped-tests" or .probe == "eslint-disable-usage" or .probe == "ts-ignore-usage") then
            (("maintainability-hotspot:" + .file + "|path=") as $hotspot_key | ($issue_keys | map(startswith($hotspot_key)) | any) | not)
          else
            true
          end
        )
      | "Unfiled " + .classification + ": " + .title + " (" + .dedupe_key + ")"
    ]
' "$FINDINGS_JSON" > "$RISKS_JSON"

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
  --slurpfile findings_input "$FINDINGS_JSON" \
  --slurpfile suppressed_input "$SUPPRESSED_JSON" \
  --slurpfile issue_links_input "$ISSUES_JSON" \
  --slurpfile residual_risks_input "$RISKS_JSON" \
  --slurpfile runtime_input "$RUNTIME_JSON" \
  --slurpfile artifacts_input "$ARTIFACTS_JSON" \
  --slurpfile verification_lanes_input "$VERIFICATION_JSON" \
  --argjson total_findings "$total_findings" \
  --argjson suppressed_findings "$suppressed_findings" \
  --argjson issue_links_count "$issue_count" \
  --argjson risk_count "$risk_count" \
  '($findings_input[0] // []) as $findings
  | ($suppressed_input[0] // []) as $suppressed
  | ($issue_links_input[0] // []) as $issue_links
  | ($residual_risks_input[0] // []) as $residual_risks
  | ($runtime_input[0] // {}) as $runtime
  | ($artifacts_input[0] // {}) as $artifacts
  | ($verification_lanes_input[0] // []) as $verification_lanes
  | {
    status:$status,
    repo:$repo,
    generated_at:$generated_at,
    summary:{
      total_findings:$total_findings,
      suppressed_findings:$suppressed_findings,
      issue_links:$issue_links_count,
      unfiled_residual_risks:$risk_count
    },
    artifacts:$artifacts,
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
  printf '## Artifacts\n\n'
  if [ "$(jq 'length' "$ARTIFACTS_JSON")" -eq 0 ]; then
    printf '(none)\n\n'
  else
    jq -r 'to_entries[] | "- " + .key + ": `" + (.value|tostring) + "`"' "$ARTIFACTS_JSON"
    printf '\n'
  fi
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
