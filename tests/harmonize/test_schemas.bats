#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PROFILE_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-token-profile.schema.json"
  VARIANT_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-variant.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-harmonize-schemas-XXXXXX)"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "token-profile schema accepts a minimal valid profile" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/profile-ok.json" <<'JSON'
{
  "source": "source",
  "palette": [{ "hex": "#1a2b3c", "role": "primary", "count": 12 }],
  "type_scale": [{ "px": 16 }],
  "spacing": [{ "px": 8 }],
  "radii": [{ "px": 4 }],
  "shadows": [{ "value": "0 1px 2px rgba(0,0,0,0.1)" }],
  "components": {}
}
JSON

  run ajv validate -s "$PROFILE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/profile-ok.json"
  [ "$status" -eq 0 ]
}

@test "token-profile schema rejects a palette-less profile" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/profile-bad.json" <<'JSON'
{
  "source": "source",
  "type_scale": [],
  "spacing": [],
  "radii": [],
  "shadows": [],
  "components": {}
}
JSON

  run ajv validate -s "$PROFILE_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/profile-bad.json"
  [ "$status" -ne 0 ]
}

@test "variant schema accepts a baseline variant" {
  command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available (install ajv-cli to run this test)"

  cat > "$TEST_TMPDIR/variant-ok.json" <<'JSON'
{
  "id": "baseline",
  "label": "Baseline",
  "axis": "baseline",
  "tokens": {
    "palette": [{ "hex": "#1a2b3c" }],
    "type_scale": [],
    "spacing": [],
    "radii": [],
    "shadows": []
  },
  "design_md": "# Baseline\nDerived design language."
}
JSON

  run ajv validate -s "$VARIANT_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/variant-ok.json"
  [ "$status" -eq 0 ]
}
