#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
V="$REPO_ROOT/skills/autospec-shared/scripts/validate-growth-config.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

valid_json() {
  cat > "$TMP/c.json" <<'JSON'
{"product":{"name":"Acme","one_liner":"x","value_props":["a"],"personas":["dev"],"competitors":["B"]},
 "site":{"url":"https://acme.dev","repo_path":".","framework":"astro","sitemap_url":"https://acme.dev/sitemap.xml"},
 "channels":{"technical_seo":true,"content":true,"outreach":true,"directories":true},
 "targets":{"keyword_seeds":["x"],"directories":[],"communities":[]},
 "measurement":{"gsc_property":"sc-domain:acme.dev","analytics":{"provider":"plausible","token_env":"PLAUSIBLE_API_TOKEN"},"github_repo":"acme/cli","rank_source":"manual"},
 "approval":{"control_repo":"acme/growth","cadence_caps":{"default_per_platform_per_week":2}},
 "guardrails":{"extra_blocks":[]}}
JSON
  echo "$TMP/c.json"
}

@test "script exists and is bash -n clean" {
  [ -f "$V" ]; run bash -n "$V"; [ "$status" -eq 0 ]
}

@test "accepts a complete valid config" {
  run bash "$V" "$(valid_json)"
  [ "$status" -eq 0 ]
}

@test "rejects config missing product.name" {
  f="$(valid_json)"; jq 'del(.product.name)' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"product.name"* ]]
}

@test "rejects an inline secret in measurement.analytics" {
  f="$(valid_json)"; jq '.measurement.analytics.token = "fixture"' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"secret"* || "$output" == *"token_env"* ]]
}

@test "rejects missing approval.control_repo" {
  f="$(valid_json)"; jq 'del(.approval.control_repo)' "$f" > "$TMP/bad.json"
  run bash "$V" "$TMP/bad.json"
  [ "$status" -ne 0 ]
}
