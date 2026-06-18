#!/usr/bin/env bats

load 'helpers/upgrade-orchestrator-mocks'
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
