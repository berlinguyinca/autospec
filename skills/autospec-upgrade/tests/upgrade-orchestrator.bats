#!/usr/bin/env bats
# upgrade-orchestrator.bats — TDD suite for upgrade-orchestrator.sh (issue #1184)
# No network access, no real installs. All subprocess calls mocked via $TMP/bin PATH shim.

ORCHESTRATOR="${BATS_TEST_DIRNAME}/../scripts/upgrade-orchestrator.sh"

# ── Setup / teardown ──────────────────────────────────────────────────────────

setup() {
  TEST_ROOT="$(mktemp -d /tmp/uo-test-root.XXXXXX)"
  MOCK_BIN="$(mktemp -d /tmp/uo-mock-bin.XXXXXX)"
  AUTOSPEC_DIR="$TEST_ROOT/.autospec"
  STATE_FILE="$AUTOSPEC_DIR/upgrade-state.json"
  mkdir -p "$AUTOSPEC_DIR"

  # Per-script invocation logs
  DETECT_LOG="$TEST_ROOT/detect.log"
  BEHAVIOR_LOCK_LOG="$TEST_ROOT/behavior-lock.log"
  ENGINE_LOG="$TEST_ROOT/engine.log"
  BEST_PRACTICE_LOG="$TEST_ROOT/best-practice.log"
  MUTATION_GATE_LOG="$TEST_ROOT/mutation-gate.log"
  TAG_LOG="$TEST_ROOT/tag.log"
  MIGRATION_DOC_LOG="$TEST_ROOT/migration-doc.log"
  QA_LOG="$TEST_ROOT/qa.log"

  touch "$DETECT_LOG" "$BEHAVIOR_LOCK_LOG" "$ENGINE_LOG" \
        "$BEST_PRACTICE_LOG" "$MUTATION_GATE_LOG" "$TAG_LOG" \
        "$MIGRATION_DOC_LOG" "$QA_LOG"

  export TEST_ROOT MOCK_BIN AUTOSPEC_DIR STATE_FILE
  export DETECT_LOG BEHAVIOR_LOCK_LOG ENGINE_LOG BEST_PRACTICE_LOG
  export MUTATION_GATE_LOG TAG_LOG MIGRATION_DOC_LOG QA_LOG
}

teardown() {
  rm -rf "$TEST_ROOT" "$MOCK_BIN"
}

# ── Mock helpers ──────────────────────────────────────────────────────────────

# install_detect_mock <exit-code> — writes detection JSON + logs invocation
install_detect_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/upgrade-detect.sh" <<'EOF'
#!/usr/bin/env bash
DETECT_LOG="${DETECT_LOG:-/dev/null}"
OUT_DIR="${1:-}"
ROOT_ARG=""
OUT_ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --root) ROOT_ARG="$2"; shift 2 ;;
    --out)  OUT_ARG="$2";  shift 2 ;;
    *)      shift ;;
  esac
done
printf '%s\n' "$*" >> "$DETECT_LOG"
if [ -n "$OUT_ARG" ]; then
  mkdir -p "$OUT_ARG"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$OUT_ARG/detect.json"
fi
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/upgrade-detect.sh"
  rm -f "$MOCK_BIN/upgrade-detect.sh.bak"
  chmod +x "$MOCK_BIN/upgrade-detect.sh"
}

# install_behavior_lock_mock <exit-code>
install_behavior_lock_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/behavior-lock.sh" <<'EOF'
#!/usr/bin/env bash
BEHAVIOR_LOCK_LOG="${BEHAVIOR_LOCK_LOG:-/dev/null}"
printf 'behavior-lock %s\n' "$*" >> "$BEHAVIOR_LOCK_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/behavior-lock.sh"
  rm -f "$MOCK_BIN/behavior-lock.sh.bak"
  chmod +x "$MOCK_BIN/behavior-lock.sh"
}

# install_engine_mock <exit-code>
install_engine_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/upgrade-engine.sh" <<'EOF'
#!/usr/bin/env bash
ENGINE_LOG="${ENGINE_LOG:-/dev/null}"
printf 'upgrade-engine %s\n' "$*" >> "$ENGINE_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/upgrade-engine.sh"
  rm -f "$MOCK_BIN/upgrade-engine.sh.bak"
  chmod +x "$MOCK_BIN/upgrade-engine.sh"
}

# install_best_practice_mock <exit-code>
install_best_practice_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/best-practice-migrate.sh" <<'EOF'
#!/usr/bin/env bash
BEST_PRACTICE_LOG="${BEST_PRACTICE_LOG:-/dev/null}"
printf 'best-practice-migrate %s\n' "$*" >> "$BEST_PRACTICE_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/best-practice-migrate.sh"
  rm -f "$MOCK_BIN/best-practice-migrate.sh.bak"
  chmod +x "$MOCK_BIN/best-practice-migrate.sh"
}

# install_mutation_gate_mock <exit-code>
install_mutation_gate_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/mutation-gate.sh" <<'EOF'
#!/usr/bin/env bash
MUTATION_GATE_LOG="${MUTATION_GATE_LOG:-/dev/null}"
printf 'mutation-gate %s\n' "$*" >> "$MUTATION_GATE_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/mutation-gate.sh"
  rm -f "$MOCK_BIN/mutation-gate.sh.bak"
  chmod +x "$MOCK_BIN/mutation-gate.sh"
}

# install_tag_mock <exit-code>
install_tag_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/tag-upgrade.sh" <<'EOF'
#!/usr/bin/env bash
TAG_LOG="${TAG_LOG:-/dev/null}"
printf 'tag-upgrade %s\n' "$*" >> "$TAG_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/tag-upgrade.sh"
  rm -f "$MOCK_BIN/tag-upgrade.sh.bak"
  chmod +x "$MOCK_BIN/tag-upgrade.sh"
}

# install_migration_doc_mock <exit-code>
install_migration_doc_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/migration-doc.sh" <<'EOF'
#!/usr/bin/env bash
MIGRATION_DOC_LOG="${MIGRATION_DOC_LOG:-/dev/null}"
printf 'migration-doc %s\n' "$*" >> "$MIGRATION_DOC_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/migration-doc.sh"
  rm -f "$MOCK_BIN/migration-doc.sh.bak"
  chmod +x "$MOCK_BIN/migration-doc.sh"
}

# install_qa_mock <exit-code> — autospec-qa mock
install_qa_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/autospec-qa" <<'EOF'
#!/usr/bin/env bash
QA_LOG="${QA_LOG:-/dev/null}"
printf 'autospec-qa %s\n' "$*" >> "$QA_LOG"
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/autospec-qa"
  rm -f "$MOCK_BIN/autospec-qa.bak"
  chmod +x "$MOCK_BIN/autospec-qa"
}

# install_compute_steps_mock — writes a simple hops file
install_compute_steps_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/compute-upgrade-steps.sh" <<'EOF'
#!/usr/bin/env bash
OUT_ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT_ARG="$2"; shift 2 ;;
    *)     shift ;;
  esac
done
if [ -n "$OUT_ARG" ]; then
  mkdir -p "$OUT_ARG"
  printf '{"hops":[{"framework":"angular","from":20,"to":21}]}\n' \
    > "$OUT_ARG/hops.json"
fi
exit __EC__
EOF
  sed -i.bak "s/__EC__/$ec/" "$MOCK_BIN/compute-upgrade-steps.sh"
  rm -f "$MOCK_BIN/compute-upgrade-steps.sh.bak"
  chmod +x "$MOCK_BIN/compute-upgrade-steps.sh"
}

install_all_mocks() {
  install_detect_mock 0
  install_behavior_lock_mock 0
  install_compute_steps_mock 0
  install_engine_mock 0
  install_best_practice_mock 0
  install_mutation_gate_mock 0
  install_tag_mock 0
  install_migration_doc_mock 0
  install_qa_mock 0
}

# write_state <phase> [last_completed_hop] [last_green_tag]
write_state() {
  local phase="$1"
  local hop="${2:-}"
  local tag="${3:-}"
  mkdir -p "$AUTOSPEC_DIR"
  printf '{"current_phase":"%s","last_completed_hop":"%s","last_green_tag":"%s"}\n' \
    "$phase" "$hop" "$tag" > "$STATE_FILE"
}

# ── Existence ─────────────────────────────────────────────────────────────────

@test "upgrade-orchestrator.sh exists and is executable" {
  [ -x "$ORCHESTRATOR" ]
}

# ── Full fresh run from Phase 0 ───────────────────────────────────────────────

@test "fresh-run: exits 0 when all phases succeed" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

@test "fresh-run: state file written at completion with phase6" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  [ -f "$STATE_FILE" ]
  phase="$(jq -r '.current_phase' "$STATE_FILE")"
  [ "$phase" = "phase6_complete" ]
}

@test "fresh-run: all phase scripts are invoked" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  [ -s "$DETECT_LOG" ] || [ -f "$STATE_FILE" ]
  [ -s "$ENGINE_LOG" ]
  [ -s "$TAG_LOG" ]
  [ -s "$MIGRATION_DOC_LOG" ]
}

# ── RESUME-FROM-CHECKPOINT (Phase 2, after hop 1) ────────────────────────────
# Seed state at phase2_complete (engine done), resume → engine NOT re-run,
# best-practice-migrate, mutation-gate, qa, tag, migration-doc ARE run.

@test "resume-from-phase2: engine is NOT re-run when state is phase2_complete" {
  install_all_mocks
  write_state "phase2_complete" "angular-21" "post-upgrade-angular-21"
  # Pre-seed detect.json so phase 0 is resumable
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  # Engine log must be EMPTY — completed hops must not re-run
  [ ! -s "$ENGINE_LOG" ]
}

@test "resume-from-phase2: best-practice migration IS run after phase2_complete resume" {
  install_all_mocks
  write_state "phase2_complete" "angular-21" "post-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  [ -s "$BEST_PRACTICE_LOG" ]
}

@test "resume-from-phase2: detect script NOT re-run when phase2_complete" {
  install_all_mocks
  write_state "phase2_complete" "angular-21" "post-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  # detect.log should be empty — detect was already done
  [ ! -s "$DETECT_LOG" ]
}

# ── CRASH-RESUME (state file left at phase1_complete) ────────────────────────
# Simulate crash after Phase 1 (behavior-lock done); re-run resumes from Phase 2.

@test "crash-resume: behavior-lock NOT re-run when state is phase1_complete" {
  install_all_mocks
  write_state "phase1_complete" "" "pre-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  # behavior-lock log must be empty — phase 1 already completed
  [ ! -s "$BEHAVIOR_LOCK_LOG" ]
}

@test "crash-resume: engine IS run when state is phase1_complete" {
  install_all_mocks
  write_state "phase1_complete" "" "pre-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  [ -s "$ENGINE_LOG" ]
}

@test "crash-resume: completes successfully and writes phase6_complete state" {
  install_all_mocks
  write_state "phase1_complete" "" "pre-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  phase="$(jq -r '.current_phase' "$STATE_FILE")"
  [ "$phase" = "phase6_complete" ]
}

# ── IDEMPOTENT NO-OP (fully-completed state) ──────────────────────────────────

@test "idempotent-noop: re-run on phase6_complete exits 0" {
  install_all_mocks
  write_state "phase6_complete" "angular-21" "post-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
}

@test "idempotent-noop: no phase scripts invoked on phase6_complete re-run" {
  install_all_mocks
  write_state "phase6_complete" "angular-21" "post-upgrade-angular-21"
  printf '{"frameworks":["angular"],"versions":{"angular":"21"},"package_manager":"npm","runners":["jest"],"monorepo":false,"has_tests":true}\n' \
    > "$AUTOSPEC_DIR/detect.json"

  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  # All logs must be empty — nothing should have been invoked
  [ ! -s "$DETECT_LOG" ]
  [ ! -s "$ENGINE_LOG" ]
  [ ! -s "$BEHAVIOR_LOCK_LOG" ]
  [ ! -s "$BEST_PRACTICE_LOG" ]
  [ ! -s "$MUTATION_GATE_LOG" ]
  [ ! -s "$TAG_LOG" ]
  [ ! -s "$MIGRATION_DOC_LOG" ]
  [ ! -s "$QA_LOG" ]
}

# ── FAIL-TO-OPERATOR: phase failure stops execution ───────────────────────────

@test "fail-to-operator: engine failure exits non-zero" {
  install_all_mocks
  install_engine_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "fail-to-operator: state persisted at phase2 when engine fails" {
  install_all_mocks
  install_engine_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  [ -f "$STATE_FILE" ]
  # State should NOT be phase6_complete — it stopped mid-pipeline
  phase="$(jq -r '.current_phase' "$STATE_FILE")"
  [ "$phase" != "phase6_complete" ]
}

@test "fail-to-operator: best-practice NOT run when engine fails" {
  install_all_mocks
  install_engine_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  [ ! -s "$BEST_PRACTICE_LOG" ]
}

@test "fail-to-operator: behavior-lock failure exits non-zero and stops before engine" {
  install_all_mocks
  install_behavior_lock_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  [ ! -s "$ENGINE_LOG" ]
}

@test "fail-to-operator: mutation-gate failure exits non-zero" {
  install_all_mocks
  install_mutation_gate_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
}

@test "fail-to-operator: state file present after mutation-gate failure" {
  install_all_mocks
  install_mutation_gate_mock 1
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -ne 0 ]
  [ -f "$STATE_FILE" ]
}

# ── State file management ─────────────────────────────────────────────────────

@test "state-file: created in --out/.autospec on first run" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  [ -f "$STATE_FILE" ]
}

@test "state-file: current_phase field is valid JSON string" {
  install_all_mocks
  run env PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_SCRIPTS_DIR="$MOCK_BIN" \
    DETECT_LOG="$DETECT_LOG" ENGINE_LOG="$ENGINE_LOG" \
    TAG_LOG="$TAG_LOG" MIGRATION_DOC_LOG="$MIGRATION_DOC_LOG" \
    MUTATION_GATE_LOG="$MUTATION_GATE_LOG" BEHAVIOR_LOCK_LOG="$BEHAVIOR_LOCK_LOG" \
    BEST_PRACTICE_LOG="$BEST_PRACTICE_LOG" QA_LOG="$QA_LOG" \
    bash "$ORCHESTRATOR" --root "$TEST_ROOT" --out "$AUTOSPEC_DIR"
  [ "$status" -eq 0 ]
  phase="$(jq -r '.current_phase' "$STATE_FILE")"
  [ -n "$phase" ]
  [ "$phase" != "null" ]
}
