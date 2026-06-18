# Mock-installation helpers for upgrade-orchestrator.bats.
# Extracted to keep the bats file under the file-LOC guardrail; sourced via bats load.
# ── Mock helpers ──────────────────────────────────────────────────────────────

# install_detect_mock <exit-code> — writes detection JSON + logs invocation
install_detect_mock() {
  local ec="${1:-0}"
  cat > "$MOCK_BIN/upgrade-detect.sh" <<'EOF'
#!/usr/bin/env bash
DETECT_LOG="${DETECT_LOG:-/dev/null}"
# Record invocation BEFORE consuming args, so the recorder reliably proves the
# mock ran (a post-parse "$*" is empty -> a blank line, an ambiguous signal).
printf 'invoked %s\n' "$*" >> "$DETECT_LOG"
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

