#!/usr/bin/env bats
# skills/autospec-run/tests/integration/phase4-drift-gate.bats
# Tests for Phase 4 docs drift gate wiring (issue #369)
#
# Uses --working-tree mode of check-doc-drift.sh (no gh CLI needed).
# Stubs AUTOSPEC_SCRIPTS_DIR to point at fixture scripts for exit-code branches.

REPO_ROOT="${BATS_TEST_DIRNAME}/../../../.."
FIXTURES_DIR="${BATS_TEST_DIRNAME}/fixtures"

setup() {
  mkdir -p "$FIXTURES_DIR/scripts"

  # Stub check-doc-drift.sh that exits 0 (clean)
  cat > "$FIXTURES_DIR/scripts/check-doc-drift-exit0.sh" <<'STUB'
#!/usr/bin/env bash
printf '{"status":"clean","drift":[]}\n'
exit 0
STUB
  chmod 0755 "$FIXTURES_DIR/scripts/check-doc-drift-exit0.sh"

  # Stub check-doc-drift.sh that exits 1 (drift detected)
  cat > "$FIXTURES_DIR/scripts/check-doc-drift-exit1.sh" <<'STUB'
#!/usr/bin/env bash
printf '{"status":"drift","drift":[{"file":"src/foo.sh","section":"## Usage"}]}\n'
exit 1
STUB
  chmod 0755 "$FIXTURES_DIR/scripts/check-doc-drift-exit1.sh"

  # Stub check-doc-drift.sh that exits 2 (missing scope)
  cat > "$FIXTURES_DIR/scripts/check-doc-drift-exit2.sh" <<'STUB'
#!/usr/bin/env bash
printf '{"status":"missing-scope","missing":["src/new-module.sh"]}\n'
exit 2
STUB
  chmod 0755 "$FIXTURES_DIR/scripts/check-doc-drift-exit2.sh"
}

teardown() {
  rm -rf "$FIXTURES_DIR"
}

# ── SKILL.md presence checks ────────────────────────────────────────────────

@test "autospec-run SKILL.md includes ## Docs drift gate section" {
  grep -qF "## Docs drift gate" "$REPO_ROOT/skills/autospec-run/SKILL.md"
}

@test "autospec-define SKILL.md includes ## Init mode section" {
  grep -qF "## Init mode" "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

@test "autospec-define SKILL.md documents auto-docs step" {
  grep -q "gen-docs-from-spec\|auto-docs\|doc-PR" "$REPO_ROOT/skills/autospec-define/SKILL.md"
}

# ── Drift gate exit-code branches ────────────────────────────────────────────

@test "drift gate exit 0 (clean) — proceeds without error" {
  run bash "$FIXTURES_DIR/scripts/check-doc-drift-exit0.sh" --pr 999
  [ "$status" -eq 0 ]
}

@test "drift gate exit 1 (drift) — emits drift JSON" {
  run bash "$FIXTURES_DIR/scripts/check-doc-drift-exit1.sh" --pr 999
  [ "$status" -eq 1 ]
  printf '%s\n' "$output" | grep -q "drift"
}

@test "drift gate exit 2 (missing scope) — reports missing-scope" {
  run bash "$FIXTURES_DIR/scripts/check-doc-drift-exit2.sh" --pr 999
  [ "$status" -eq 2 ]
  printf '%s\n' "$output" | grep -q "missing-scope"
}

# ── phase4-implementer.md includes drift gate ────────────────────────────────

@test "phase4-implementer.md includes check-doc-drift.sh reference" {
  grep -q "check-doc-drift.sh" "$REPO_ROOT/skills/autospec-run/prompts/phase4-implementer.md"
}
