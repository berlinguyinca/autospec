#!/usr/bin/env bats
# tests/docs/test_quickstart.bats
#
# Validates docs/QUICKSTART.md, docs/quickstart.cast, docs/site/, and
# .github/workflows/pages.yml for the D4 distribution milestone.
#
# Run: bats tests/docs/test_quickstart.bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
QUICKSTART="$REPO_ROOT/docs/QUICKSTART.md"
CAST="$REPO_ROOT/docs/quickstart.cast"
SITE_INDEX="$REPO_ROOT/docs/site/index.html"
SITE_CSS="$REPO_ROOT/docs/site/style.css"
PAGES_WORKFLOW="$REPO_ROOT/.github/workflows/pages.yml"

# ── File existence ─────────────────────────────────────────────────────────

@test "QUICKSTART.md exists" {
  [ -f "$QUICKSTART" ]
}

@test "quickstart.cast exists and is non-empty" {
  [ -f "$CAST" ]
  [ -s "$CAST" ]
}

@test "docs/site/index.html exists" {
  [ -f "$SITE_INDEX" ]
}

@test "docs/site/style.css exists" {
  [ -f "$SITE_CSS" ]
}

@test "pages.yml workflow exists" {
  [ -f "$PAGES_WORKFLOW" ]
}

# ── QUICKSTART structure ───────────────────────────────────────────────────

@test "QUICKSTART.md has 5 numbered steps" {
  # Spec §6 requires exactly 5 numbered-step headings or list items
  local count
  count=$(grep -cE '^## Step [1-5]' "$QUICKSTART")
  [ "$count" -eq 5 ]
}

@test "QUICKSTART.md Step 1 references npx autospec init" {
  grep -q 'npx autospec init' "$QUICKSTART"
}

@test "QUICKSTART.md Step 3 references autospec define" {
  grep -q 'autospec define' "$QUICKSTART"
}

@test "QUICKSTART.md Step 4 references autospec run" {
  grep -q 'autospec run' "$QUICKSTART"
}

@test "QUICKSTART.md references brew install" {
  grep -q 'brew install' "$QUICKSTART"
}

# ── QUICKSTART bash blocks parse cleanly ───────────────────────────────────

@test "QUICKSTART.md: all bash fenced blocks parse via bash -n" {
  local tmp_script fail=0
  tmp_script="$(mktemp -t qs-block-XXXXXX.sh)"

  # Extract each ```bash ... ``` block and check syntax
  awk '/^```bash$/{in_block=1; next} /^```$/{if(in_block){in_block=0}} in_block{print}' \
    "$QUICKSTART" > "$tmp_script"

  if [ -s "$tmp_script" ]; then
    bash -n "$tmp_script" || fail=1
  fi
  rm -f "$tmp_script"
  [ "$fail" -eq 0 ]
}

# ── site/index.html content ────────────────────────────────────────────────

@test "index.html references quickstart.cast" {
  grep -q 'quickstart.cast' "$SITE_INDEX"
}

@test "index.html references style.css" {
  grep -q 'style.css' "$SITE_INDEX"
}

@test "index.html has hero section" {
  grep -q 'hero' "$SITE_INDEX"
}

@test "index.html links to USER_MANUAL.md" {
  grep -q 'USER_MANUAL' "$SITE_INDEX"
}

# ── pages.yml content ──────────────────────────────────────────────────────

@test "pages.yml triggers on push to main" {
  grep -q 'main' "$PAGES_WORKFLOW"
  grep -q 'push' "$PAGES_WORKFLOW"
}

@test "pages.yml deploys docs/site path" {
  grep -q 'docs/site' "$PAGES_WORKFLOW"
}

@test "pages.yml uses deploy-pages action" {
  grep -q 'deploy-pages' "$PAGES_WORKFLOW"
}

@test "pages.yml is valid YAML" {
  if command -v python3 >/dev/null 2>&1 && python3 -c "import yaml" 2>/dev/null; then
    python3 - "$PAGES_WORKFLOW" << 'PYEOF'
import yaml, sys
yaml.safe_load(open(sys.argv[1]))
PYEOF
  elif command -v yamllint >/dev/null 2>&1; then
    yamllint "$PAGES_WORKFLOW"
  else
    skip "no yaml validator available"
  fi
}
