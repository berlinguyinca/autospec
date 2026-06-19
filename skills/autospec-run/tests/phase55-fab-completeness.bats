#!/usr/bin/env bats
# skills/autospec-run/tests/phase55-fab-completeness.bats — TDD for the Phase 5.5
# fab-completeness dimension (issue #1236).
#
# The helper fab-completeness.sh asserts, for each printable model, that
#   (a) its 16-view contact sheet exists,
#   (b) its release-gate.json exists, is GREEN (no stage status=fail), and
#   (c) is FRESH (its geometry_hash matches the model's current STL sha256).
# Each failed assertion emits one structured GAP line (and a non-zero exit).
# A complete model emits nothing and the helper exits 0. Real fixtures, no mocks.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/fab-completeness.sh"

# stl_hash <file> — sha256 hex of a file (mirrors stl-release-gate.py's hash).
stl_hash() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

# Build a fab tree: a model with an STL, a contact sheet, and a gate whose
# status + geometry_hash are caller-controlled.
#   make_model <fab_dir> <stl_dir> <model> <sheet:yes|no> <gate:green|fail|none> <hash:fresh|stale>
make_model() {
  fab_dir="$1"; stl_dir="$2"; model="$3"; sheet="$4"; gate="$5"; hashmode="$6"
  mkdir -p "$stl_dir"
  printf 'solid %s\nendsolid %s\n' "$model" "$model" > "$stl_dir/$model.stl"

  if [ "$sheet" = "yes" ]; then
    mkdir -p "$fab_dir/renders/$model"
    printf '<html><body>contact sheet for %s</body></html>\n' "$model" \
      > "$fab_dir/renders/$model/contact-sheet.html"
  fi

  if [ "$gate" != "none" ]; then
    mkdir -p "$fab_dir/gates/$model"
    if [ "$hashmode" = "fresh" ]; then
      ghash="$(stl_hash "$stl_dir/$model.stl")"
    else
      ghash="deadbeefstalehash"
    fi
    if [ "$gate" = "green" ]; then
      geomstatus="pass"
    else
      geomstatus="fail"
    fi
    cat > "$fab_dir/gates/$model/release-gate.json" <<JSON
{
  "schema_version": 1,
  "name": "$model",
  "geometry_hash": "$ghash",
  "stages": [
    {"stage": "geometry", "status": "$geomstatus", "detail": "", "findings": []},
    {"stage": "render", "status": "pass", "detail": "", "findings": []},
    {"stage": "vision", "status": "warn", "detail": "", "findings": []}
  ],
  "vision_findings": []
}
JSON
  fi
}

setup() {
  WORK="$(mktemp -d)"
  FAB="$WORK/.autospec/fab"
  STL="$WORK/build/stls"
  mkdir -p "$FAB"
}

teardown() {
  rm -rf "$WORK"
}

# ── Helper hygiene ──────────────────────────────────────────────────────────

@test "fab-completeness.sh exists and is executable" {
  [ -f "$SCRIPT" ]
}

@test "fab-completeness.sh passes bash -n" {
  run bash -n "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "fab-completeness.sh --help exits 0" {
  run bash "$SCRIPT" --help
  [ "$status" -eq 0 ]
}

# ── Complete model → pass, no gaps ──────────────────────────────────────────

@test "a model with a contact sheet + green + fresh gate passes with no gap" {
  make_model "$FAB" "$STL" widget yes green fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models widget
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

# ── Missing gate → gap ──────────────────────────────────────────────────────

@test "a model missing its release-gate.json files a gap" {
  make_model "$FAB" "$STL" widget yes none fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models widget
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "widget"
  echo "$output" | grep -qi "gate"
}

# ── Missing contact sheet → gap ─────────────────────────────────────────────

@test "a model missing its contact sheet files a gap" {
  make_model "$FAB" "$STL" widget no green fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models widget
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "widget"
  echo "$output" | grep -qi "contact"
}

# ── Failing (non-green) gate → gap ──────────────────────────────────────────

@test "a model with a fail stage in its gate files a gap" {
  make_model "$FAB" "$STL" widget yes fail fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models widget
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "widget"
  echo "$output" | grep -qi "green"
}

# ── Stale gate (geometry_hash mismatch) → gap ───────────────────────────────

@test "a model with a stale geometry_hash files a gap" {
  make_model "$FAB" "$STL" widget yes green stale
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models widget
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "widget"
  echo "$output" | grep -qi "stale"
}

# ── Multi-model: one complete, one incomplete → only the incomplete is a gap ─

@test "with two models, only the incomplete one emits a gap" {
  make_model "$FAB" "$STL" alpha yes green fresh
  make_model "$FAB" "$STL" beta  yes none  fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL" --models alpha,beta
  [ "$status" -ne 0 ]
  echo "$output" | grep -q "beta"
  ! echo "$output" | grep -q "alpha"
}

# ── Scan mode: discover models from the gates dir when --models omitted ──────

@test "scan mode discovers models under the gates dir" {
  make_model "$FAB" "$STL" gamma yes green fresh
  run bash "$SCRIPT" --fab-dir "$FAB" --stl-dir "$STL"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}
