#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  PREVIEW="$REPO_ROOT/skills/autospec-harmonize/scripts/design-preview.mjs"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-preview-XXXXXX)"

  # Write a 3-variant array fixture JSON
  cat > "$TEST_TMPDIR/variants.json" <<'EOF'
[
  {
    "id": "baseline",
    "label": "Baseline — faithful consolidation",
    "axis": "baseline",
    "wcag_min_ratio": 5.21,
    "tokens": {
      "palette": [
        {"hex": "#ffffff", "role": "bg"},
        {"hex": "#111111", "role": "text"},
        {"hex": "#1a73e8", "role": "primary"}
      ],
      "type_scale": [{"px": 14}, {"px": 16}],
      "spacing": [{"px": 8}, {"px": 16}],
      "radii": [{"px": 4}],
      "shadows": [{"value": "0 1px 3px rgba(0,0,0,0.2)"}]
    },
    "design_md": "# Baseline\n\nBase design system."
  },
  {
    "id": "minimal",
    "label": "Minimal — reduced decoration",
    "axis": "minimal",
    "wcag_min_ratio": 4.80,
    "tokens": {
      "palette": [
        {"hex": "#ffffff", "role": "bg"},
        {"hex": "#333333", "role": "text"},
        {"hex": "#888888", "role": "primary"}
      ],
      "type_scale": [{"px": 14}, {"px": 16}],
      "spacing": [{"px": 8}, {"px": 16}],
      "radii": [{"px": 4}],
      "shadows": []
    },
    "design_md": "# Minimal\n\nShadows removed."
  },
  {
    "id": "high-contrast",
    "label": "High-Contrast — WCAG-AA+ accessibility",
    "axis": "high-contrast",
    "wcag_min_ratio": 9.29,
    "tokens": {
      "palette": [
        {"hex": "#ffffff", "role": "bg"},
        {"hex": "#000000", "role": "text"},
        {"hex": "#0044cc", "role": "primary"}
      ],
      "type_scale": [{"px": 14}, {"px": 16}],
      "spacing": [{"px": 8}, {"px": 16}],
      "radii": [{"px": 4}],
      "shadows": [{"value": "0 2px 6px rgba(0,0,0,0.4)"}]
    },
    "design_md": "# High-Contrast\n\nPalette pushed apart."
  }
]
EOF
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

@test "design-preview exits 0 with NO_PLAYWRIGHT=1 fallback" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  [ "$status" -eq 0 ]
}

@test "design-preview creates index.html with NO_PLAYWRIGHT=1" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  [ -f "$TEST_TMPDIR/preview/index.html" ]
}

@test "design-preview index.html has exactly 3 data-variant= sections" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  COUNT=$(grep -c 'data-variant=' "$TEST_TMPDIR/preview/index.html")
  [ "$COUNT" -eq 3 ]
}

@test "design-preview index.html has exactly 3 WCAG annotations" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  # Count the annotation ELEMENT, not a bare "WCAG" substring: a realistic
  # variant label (e.g. "High-Contrast — WCAG-AA+") legitimately contains the
  # word WCAG, so substring counting would over-count on real input.
  COUNT=$(grep -c 'class="contrast"' "$TEST_TMPDIR/preview/index.html")
  [ "$COUNT" -eq 3 ]
}

@test "design-preview prints code_health:harmonize_preview_no_render with NO_PLAYWRIGHT=1" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  run bash -c "AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 node '$PREVIEW' \
    --variants '$TEST_TMPDIR/variants.json' --out '$TEST_TMPDIR' 2>&1"
  [ "$status" -eq 0 ]
  [[ "$output" == *"harmonize_preview_no_render"* ]]
}

@test "design-preview index.html contains each variant id" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  grep -q 'baseline' "$TEST_TMPDIR/preview/index.html"
  grep -q 'minimal' "$TEST_TMPDIR/preview/index.html"
  grep -q 'high-contrast' "$TEST_TMPDIR/preview/index.html"
}

@test "design-preview index.html is non-empty" {
  command -v node >/dev/null 2>&1 || skip "node not available"
  env AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 \
    node "$PREVIEW" --variants "$TEST_TMPDIR/variants.json" --out "$TEST_TMPDIR"
  [ -s "$TEST_TMPDIR/preview/index.html" ]
}
