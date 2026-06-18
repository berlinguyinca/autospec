#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  VARIANTS="$REPO_ROOT/skills/autospec-harmonize/scripts/design-variants.mjs"
  FETCH_VENDOR="$REPO_ROOT/skills/autospec-harmonize/scripts/fetch-vendor.sh"
  VARIANT_SCHEMA="$REPO_ROOT/schemas/autospec-harmonize-variant.schema.json"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-variants-XXXXXX)"

  # Write a minimal baseline variant JSON fixture
  cat > "$TEST_TMPDIR/baseline.json" <<'EOF'
{
  "id": "baseline",
  "label": "Baseline — faithful consolidation",
  "axis": "baseline",
  "wcag_min_ratio": 3.5,
  "tokens": {
    "palette": [
      {"hex": "#ffffff", "role": "bg"},
      {"hex": "#111111", "role": "text"},
      {"hex": "#1a73e8", "role": "primary"},
      {"hex": "#d93025", "role": "accent"}
    ],
    "type_scale": [
      {"px": 12}, {"px": 14}, {"px": 16}, {"px": 20}, {"px": 32}
    ],
    "spacing": [
      {"px": 8}, {"px": 16}, {"px": 24}
    ],
    "radii": [
      {"px": 4}, {"px": 8}
    ],
    "shadows": [
      {"value": "0 1px 3px rgba(0,0,0,0.2)"}
    ]
  },
  "design_md": "# Baseline\n\nBaseline design system."
}
EOF

  # Vendor fixture with clearly different palette (orange/green tones)
  cat > "$TEST_TMPDIR/vendor.json" <<'EOF'
{
  "palette": [
    {"hex": "#ff6600", "role": "primary"},
    {"hex": "#00aa44", "role": "accent"},
    {"hex": "#f5f5f5", "role": "bg"},
    {"hex": "#222222", "role": "text"}
  ]
}
EOF
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "design-variants exits 0 with baseline only (no axes)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes ""
  [ "$status" -eq 0 ]
}

@test "design-variants output is a JSON array with baseline at index 0" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  run node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "minimal"
  [ "$status" -eq 0 ]
  echo "$output" > "$TEST_TMPDIR/variants.json"
  run jq 'type' "$TEST_TMPDIR/variants.json"
  [ "$output" = '"array"' ]
  run jq '.[0].axis' "$TEST_TMPDIR/variants.json"
  [ "$output" = '"baseline"' ]
}

@test "high-contrast variant wcag_min_ratio exceeds baseline" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "high-contrast" \
    > "$TEST_TMPDIR/hc.json"
  # baseline index 0 has wcag_min_ratio; high-contrast is index 1
  BASELINE_WCAG=$(jq '.[0].wcag_min_ratio' "$TEST_TMPDIR/hc.json")
  HC_WCAG=$(jq '.[1].wcag_min_ratio' "$TEST_TMPDIR/hc.json")
  # Use node to compare floats
  run node -e "process.exit(($HC_WCAG > $BASELINE_WCAG) ? 0 : 1)"
  [ "$status" -eq 0 ]
}

@test "high-contrast variant has axis == high-contrast" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "high-contrast" \
    > "$TEST_TMPDIR/hc.json"
  run jq -r '.[1].axis' "$TEST_TMPDIR/hc.json"
  [ "$output" = "high-contrast" ]
}

@test "dense variant last spacing element is strictly smaller than baseline last spacing" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "dense" \
    > "$TEST_TMPDIR/dense.json"
  BASELINE_LAST=$(jq '.[0].tokens.spacing[-1].px' "$TEST_TMPDIR/dense.json")
  DENSE_LAST=$(jq '.[1].tokens.spacing[-1].px' "$TEST_TMPDIR/dense.json")
  run node -e "process.exit(($DENSE_LAST < $BASELINE_LAST) ? 0 : 1)"
  [ "$status" -eq 0 ]
}

@test "vendor-blend palette hex channels lie strictly between baseline and vendor" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "vendor-blend" \
    --vendor-file "$TEST_TMPDIR/vendor.json" \
    > "$TEST_TMPDIR/vb.json"
  # Check that the blended primary hex (#1a73e8 blended with #ff6600)
  # Each channel must be strictly between baseline and vendor channels
  # baseline primary: #1a73e8 = R=26,G=115,B=232
  # vendor primary:   #ff6600 = R=255,G=102,B=0
  # blended: R must be between 26 and 255, G between 102 and 115, B between 0 and 232
  BLENDED_PRIMARY=$(jq -r '[.[1].tokens.palette[] | select(.role=="primary")][0].hex' "$TEST_TMPDIR/vb.json")
  run node -e "
    const hex = '$BLENDED_PRIMARY';
    const r = parseInt(hex.slice(1,3),16);
    const g = parseInt(hex.slice(3,5),16);
    const b = parseInt(hex.slice(5,7),16);
    // baseline primary R=26,G=115,B=232  vendor primary R=255,G=102,B=0
    const ok = r > 26 && r < 255 && g > 102 && g < 115 && b > 0 && b < 232;
    process.exit(ok ? 0 : 1);
  "
  [ "$status" -eq 0 ]
}

@test "vendor-blend axis == vendor-blend" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "vendor-blend" \
    --vendor-file "$TEST_TMPDIR/vendor.json" \
    > "$TEST_TMPDIR/vb.json"
  run jq -r '.[1].axis' "$TEST_TMPDIR/vb.json"
  [ "$output" = "vendor-blend" ]
}

@test "vendor fetch failure prints code_health signal and others survive, exit 0" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"
  # No --vendor-file, fetch-vendor.sh will fail → vendor-blend is dropped
  # Request minimal + vendor-blend; minimal should still appear
  run node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" --axes "minimal,vendor-blend"
  [ "$status" -eq 0 ]
  # stderr should contain code_health signal
  [[ "$output $stderr" == *"harmonize_vendor_fetch_failed"* ]] || \
    echo "$output" | grep -q "harmonize_vendor_fetch_failed" || \
    { run bash -c "node '$VARIANTS' --baseline '$TEST_TMPDIR/baseline.json' --axes 'minimal,vendor-blend' 2>&1"; \
      [[ "$output" == *"harmonize_vendor_fetch_failed"* ]]; }
  # The array must still be emitted with baseline + minimal (no vendor-blend)
  run bash -c "node '$VARIANTS' --baseline '$TEST_TMPDIR/baseline.json' --axes 'minimal,vendor-blend' 2>/dev/null"
  [ "$status" -eq 0 ]
  echo "$output" > "$TEST_TMPDIR/fail.json"
  run jq 'type' "$TEST_TMPDIR/fail.json"
  [ "$output" = '"array"' ]
  # minimal should be present
  run jq '[.[] | select(.axis=="minimal")] | length' "$TEST_TMPDIR/fail.json"
  [ "$output" -ge 1 ]
  # vendor-blend should NOT be present
  run jq '[.[] | select(.axis=="vendor-blend")] | length' "$TEST_TMPDIR/fail.json"
  [ "$output" -eq 0 ]
}

@test "all variants validate against schema (ajv)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v ajv  >/dev/null 2>&1 || skip "ajv CLI not available"
  node "$VARIANTS" --baseline "$TEST_TMPDIR/baseline.json" \
    --axes "minimal,high-contrast,dense,bold" \
    > "$TEST_TMPDIR/all.json"
  # Validate each variant individually
  COUNT=$(jq 'length' "$TEST_TMPDIR/all.json")
  for i in $(seq 0 $((COUNT - 1))); do
    jq ".[$i]" "$TEST_TMPDIR/all.json" > "$TEST_TMPDIR/variant_$i.json"
    run ajv validate -s "$VARIANT_SCHEMA" --spec=draft2020 -d "$TEST_TMPDIR/variant_$i.json"
    [ "$status" -eq 0 ]
  done
}

@test "minimal variant recomputes wcag_min_ratio from its own palette (audit #1147 F1)" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  command -v jq   >/dev/null 2>&1 || skip "jq not available"

  # A baseline whose chromatic accent is the binding (minimum-contrast) foreground.
  # 'minimal' fully desaturates the accent, which changes its luminance and thus
  # the minimum foreground-vs-bg contrast — so the variant's wcag_min_ratio MUST
  # differ from the baseline's. Before the fix, minimal inherited the baseline's
  # stale value (clone), so this asserted inequality fails.
  cat > "$TEST_TMPDIR/binding.json" <<'EOF'
{
  "id": "baseline", "label": "B", "axis": "baseline",
  "tokens": {
    "palette": [
      {"hex": "#ffffff", "role": "bg"},
      {"hex": "#595959", "role": "text"},
      {"hex": "#1aa3a3", "role": "accent"},
      {"hex": "#0044cc", "role": "primary"}
    ],
    "type_scale": [{"px": 16}], "spacing": [{"px": 8}], "radii": [{"px": 4}],
    "shadows": [{"value": "x"}]
  },
  "design_md": "# B"
}
EOF
  node "$VARIANTS" --baseline "$TEST_TMPDIR/binding.json" --axes minimal > "$TEST_TMPDIR/out.json"
  BASE=$(jq -r '.[] | select(.axis=="baseline") | .wcag_min_ratio' "$TEST_TMPDIR/out.json")
  MIN=$(jq -r  '.[] | select(.axis=="minimal")  | .wcag_min_ratio' "$TEST_TMPDIR/out.json")
  # Both present and numeric
  [ -n "$BASE" ] && [ "$BASE" != "null" ]
  [ -n "$MIN" ]  && [ "$MIN"  != "null" ]
  # Recomputed, not inherited: the minimal palette's contrast differs from baseline's.
  [ "$(node -e "process.stdout.write(String($MIN !== $BASE))")" = "true" ]
}
