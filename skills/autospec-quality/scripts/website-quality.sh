#!/usr/bin/env bash
# Generic, repository-owned website quality history and trend reports.
set -euo pipefail

usage() { printf '%s\n' 'website-quality.sh {record|validate|report} ...' >&2; }
die() { printf 'website-quality: %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; return; fi
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'; return; fi
  die 'sha256sum or shasum is required'
}
valid_file() {
  local file="$1"
  jq -e '
    type == "object" and .schema_version == "website-quality/v1" and
    (.site_id|type)=="string" and (.site_id|test("^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")) and
    (.run_id|type)=="string" and (.run_id|test("^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")) and
    (.git_sha|type)=="string" and (.git_sha|test("^[0-9a-f]{40}$")) and
    (.captured_at|type)=="string" and (.rubric_version|type)=="string" and (.rubric_version|length>0) and
    (.config_hash|type)=="string" and (.config_hash|length>0) and (.pages|type)=="array" and
    all(.pages[];
      (type=="object") and (.route_template|type)=="string" and (.route_template|length>0) and
      (.category_scores|type)=="object" and all(.category_scores[]; type=="number" and . >= 0 and . <= 1) and
      (.confidence|type)=="number" and .confidence >= 0 and .confidence <= 1 and
      (.coverage|type)=="number" and .coverage >= 0 and .coverage <= 1 and
      (.validity|IN("current","stale","incomplete","unverifiable")) and
      (.evidence|type)=="array" and all(.evidence[]; type=="string" and (startswith("/")|not)) and
      (.defects|type)=="array" and (.remediation|type)=="array" and (.acceptance_test|type)=="string"
    )' "$file" >/dev/null 2>&1
}
record() {
  local input='' history=''
  while [ "$#" -gt 0 ]; do case "$1" in --input) input=${2:-}; shift 2;; --history) history=${2:-}; shift 2;; *) die "unknown record option: $1";; esac; done
  need jq; [ -f "$input" ] || die "input not found: $input"; [ -n "$history" ] || die '--history required'
  valid_file "$input" || die 'capture does not satisfy website-quality/v1 schema'
  local site run target checksum tmp
  site=$(jq -r .site_id "$input"); run=$(jq -r .run_id "$input")
  target="$history/runs/$site/$run.json"; mkdir -p "$(dirname "$target")"
  [ ! -e "$target" ] || die "immutable run already exists: $target"
  checksum=$(sha256 "$input")
  tmp="$target.tmp.$$"
  jq --arg checksum "sha256:$checksum" --arg source_checksum "sha256:$checksum" \
    '. + {artifact_checksum:$checksum, source_checksum:$source_checksum}' "$input" > "$tmp"
  ln "$tmp" "$target" 2>/dev/null || { rm -f "$tmp"; die "cannot create immutable run: $target"; }
  rm -f "$tmp"
  printf '%s\n' "recorded $target"
}
validate() {
  local history=''; while [ "$#" -gt 0 ]; do case "$1" in --history) history=${2:-}; shift 2;; *) die "unknown validate option: $1";; esac; done
  need jq; [ -d "$history/runs" ] || die "history not found: $history"
  local file count=0; while IFS= read -r file; do
    valid_file "$file" || die "invalid history run: $file"
    jq -e '.artifact_checksum|type=="string" and startswith("sha256:")' "$file" >/dev/null || die "missing checksum: $file"
    count=$((count + 1))
  done < <(find "$history/runs" -type f -name '*.json' -print | sort)
  [ "$count" -gt 0 ] || die 'history contains no runs'; printf '%s\n' "validated $count run(s)"
}
report() {
  local history='' output=''; while [ "$#" -gt 0 ]; do case "$1" in --history) history=${2:-}; shift 2;; --output) output=${2:-}; shift 2;; *) die "unknown report option: $1";; esac; done
  need jq; [ -n "$output" ] || die '--output required'; validate --history "$history" >/dev/null
  local tmp; tmp=$(mktemp)
  find "$history/runs" -type f -name '*.json' -print | sort | while IFS= read -r f; do cat "$f"; printf '\n'; done > "$tmp"
  local rubric_count; rubric_count=$(jq -s 'map(.rubric_version)|unique|length' "$tmp")
  [ "$rubric_count" -eq 1 ] || { printf 'blocked: cross-rubric history cannot be compared\n' >&2; return 1; }
  jq -s '
    {schema_version:"website-quality-report/v1", rubric_version:.[0].rubric_version,
     generated_at:(map(.captured_at)|max), blockers:[], pages:(map(. as $run | .pages[] |
       {site_id:$run.site_id, route_template, status:(if .validity=="current" and .coverage==1 and .confidence>0 then "verified" else "blocked" end),
        validity, confidence, coverage, category_scores, evidence, defects, remediation, acceptance_test,
        run_id:$run.run_id, captured_at:$run.captured_at, git_sha:$run.git_sha}))}
  ' "$tmp" > "$output"
  rm -f "$tmp"
  printf '%s\n' "wrote $output"
}
main() { [ "$#" -gt 0 ] || { usage; exit 2; }; command=$1; shift; case "$command" in record) record "$@";; validate) validate "$@";; report) report "$@";; *) usage; exit 2;; esac; }
main "$@"
