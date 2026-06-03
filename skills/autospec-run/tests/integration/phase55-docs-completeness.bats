#!/usr/bin/env bats
# skills/autospec-run/tests/integration/phase55-docs-completeness.bats
# Tests for the Phase 5.5 docs-completeness dimension (issue #923).
#
# The helper docs-completeness-gaps.sh emits gap objects (gap-json contract)
# for (a) batch-window features missing a page for a configured audience and
# (b) outstanding visual_stale / example_stale drift signals. Gaps flow through
# the EXISTING gap-remediation loop; the helper itself never blocks the run.

bats_require_minimum_version 1.5.0

REPO_ROOT="${BATS_TEST_DIRNAME}/../../../.."
HELPER="${REPO_ROOT}/skills/autospec-run/scripts/docs-completeness-gaps.sh"

setup() {
  WORK="$(mktemp -d)"
  # Minimal config with two audiences so "missing page" is deterministic.
  mkdir -p "$WORK/.autospec"
  cat > "$WORK/.autospec/autospec.yml" <<'YML'
documentation:
  audiences:
    - {name: user, path: docs/user, focus: "tasks", require_scope: true}
    - {name: admin, path: docs/admin, focus: "operate", require_scope: true}
YML
  # A feature documented for user but NOT admin → admin page is a gap.
  mkdir -p "$WORK/docs/user/features" "$WORK/docs/admin/features"
  printf '# Widgets\n' > "$WORK/docs/user/features/widgets.md"
}

teardown() {
  rm -rf "$WORK"
}

# ── Helper hygiene ──────────────────────────────────────────────────────────

@test "docs-completeness-gaps.sh passes bash -n" {
  run bash -n "$HELPER"
  [ "$status" -eq 0 ]
}

@test "docs-completeness-gaps.sh --help exits 0" {
  run bash "$HELPER" --help
  [ "$status" -eq 0 ]
}

# ── Missing audience page → gap ─────────────────────────────────────────────

@test "flags a feature missing a page for a configured audience" {
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --no-drift
  [ "$status" -eq 0 ]
  # admin/features/widgets.md is missing → one docs-completeness gap.
  echo "$output" | jq -e '[.[] | select(.dimension=="docs-completeness")] | length >= 1' >/dev/null
  echo "$output" | jq -e 'any(.[]; .title | test("admin"))' >/dev/null
}

@test "fully-documented batch yields zero docs-completeness gaps" {
  printf '# Widgets\n' > "$WORK/docs/admin/features/widgets.md"
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --no-drift
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'length == 0' >/dev/null
}

# ── Emitted gaps satisfy the gap-json contract ──────────────────────────────

@test "emitted gaps carry every required gap-json key" {
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --no-drift
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'all(.[]; has("gap_id") and has("dimension") and has("severity") and has("file") and has("line") and has("title") and has("body") and has("dedupe_key"))' >/dev/null
}

# ── Drift signals (visual_stale / example_stale) → gaps ─────────────────────

@test "flags an example_stale signal from drift JSON" {
  # Stub a check-doc-drift.sh that reports one example_stale entry.
  mkdir -p "$WORK/scripts"
  cat > "$WORK/scripts/check-doc-drift.sh" <<'STUB'
#!/usr/bin/env bash
printf '{"visual_stale":[],"example_stale":[{"doc_file":"docs/user/features/widgets.md","heading":"## Run it"}]}\n'
exit 0
STUB
  chmod 0755 "$WORK/scripts/check-doc-drift.sh"
  # Fully document features so the only gap comes from drift.
  printf '# Widgets\n' > "$WORK/docs/admin/features/widgets.md"
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --drift-script "$WORK/scripts/check-doc-drift.sh"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'any(.[]; .title | test("example_stale"))' >/dev/null
}

@test "flags a visual_stale signal from drift JSON" {
  mkdir -p "$WORK/scripts"
  cat > "$WORK/scripts/check-doc-drift.sh" <<'STUB'
#!/usr/bin/env bash
printf '{"visual_stale":[{"doc_file":"docs/user/features/widgets.md","heading":"## Diagram"}],"example_stale":[]}\n'
exit 0
STUB
  chmod 0755 "$WORK/scripts/check-doc-drift.sh"
  printf '# Widgets\n' > "$WORK/docs/admin/features/widgets.md"
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --drift-script "$WORK/scripts/check-doc-drift.sh"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'any(.[]; .title | test("visual_stale"))' >/dev/null
}

# ── Never blocks: a failing drift script only warns, exits 0, empty array ────

@test "a failing drift script warns but exits 0 with no drift gaps" {
  mkdir -p "$WORK/scripts"
  cat > "$WORK/scripts/check-doc-drift.sh" <<'STUB'
#!/usr/bin/env bash
echo "boom" >&2
exit 3
STUB
  chmod 0755 "$WORK/scripts/check-doc-drift.sh"
  printf '# Widgets\n' > "$WORK/docs/admin/features/widgets.md"
  run --separate-stderr bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/autospec.yml" --drift-script "$WORK/scripts/check-doc-drift.sh"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'length == 0' >/dev/null
}

# ── Missing config → graceful empty (non-doc repos never block) ─────────────

@test "missing config yields empty array and exit 0" {
  run bash "$HELPER" --repo-root "$WORK" --config "$WORK/.autospec/does-not-exist.yml" --no-drift
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'type == "array"' >/dev/null
}

# ── Phase 5.5 prose wiring (lock-step trio carries the dimension) ────────────

@test "SKILL.md Phase 5.5 references the docs-completeness dimension" {
  grep -q "docs-completeness" "$REPO_ROOT/skills/autospec-run/SKILL.md"
}

@test "SKILL.md Phase 5.5 invokes docs-completeness-gaps.sh" {
  grep -q "docs-completeness-gaps.sh" "$REPO_ROOT/skills/autospec-run/SKILL.md"
}

@test "Phase 5.5 docs-completeness routes through the existing gap-remediation loop" {
  grep -q "gap-remediation-loop.sh" "$REPO_ROOT/skills/autospec-run/SKILL.md"
}
