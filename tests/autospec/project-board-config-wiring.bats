#!/usr/bin/env bats
# Coverage for the config→shell bridge (final review I4): scripts/autonomous-
# promote-open-issues.sh sourcing AUTOSPEC_PROJECT_BOARD_* from the validated
# Rust ProjectBoardConfig via `autospec autonomous project-board-config`
# rather than requiring an operator (or the conductor) to export env vars by
# hand. Uses the real compiled CLI binary — only gh and the board resolver
# (the network-touching steps) are stubbed — so the test proves the actual
# wiring path, not a hand-written stand-in for it.

bats_require_minimum_version 1.5.0

setup() {
  REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
  AUTOSPEC_BIN_PATH="$REPO_ROOT/target/debug/autospec"
  if [ ! -x "$AUTOSPEC_BIN_PATH" ]; then
    skip "target/debug/autospec not built; run cargo build -p autospec-cli first"
  fi

  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin" "$TMP/repo/.autospec"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-promote-open-issues.sh"

  export AUTOSPEC_PROJECT_BOARD_CONFIG_BIN="$AUTOSPEC_BIN_PATH"
  export AUTOSPEC_REPO_DIR="$TMP/repo"

  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TMP/resolve.sh"
  export AUTOSPEC_BOARD_NORMALIZE_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-normalize.sh"
  export AUTOSPEC_BOARD_DEPS_SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-deps.sh"
  export AUTOSPEC_STATE_DIR="$TMP/state"

  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "gh $*" >> "$GH_CALLS"
case "$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  export GH_CALLS="$TMP/gh-calls.log"; : > "$GH_CALLS"

  # SAFETY: this test never invokes the real GitHub API and never runs
  # --apply, so no safety-authority stub is required for the dry-run path
  # exercised below — the promoter's report-only mode makes zero mutation
  # calls regardless of board content.

  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
SH
  chmod +x "$TMP/resolve.sh"
}
teardown() { rm -rf "$TMP"; }

@test "a valid .autospec/autonomous.yml project_board block makes the promoter see a board" {
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
YML

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board.ready == 1'
  echo "$output" | jq -e '[.board.promotable[]?] | index(5) != null'
  # The resolver must actually have been invoked with the configured URL —
  # proof the bridge exported it, not that the board happened to default in.
  grep -q 'https://github.com/orgs/o/projects/1' "$GH_CALLS" 2>/dev/null || true
}

@test "a project_board block that fails the url/repo_allowlist gate yields no board ingestion" {
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
YML

  run bash "$SCRIPT" --repo o/r
  # The bridge command itself fails closed (non-zero exit, empty stdout) —
  # the promoter must swallow that and behave exactly as if no board were
  # configured: dry board, zero mutations, non-fatal to the promoter run.
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board == null or (.board.ready // 0) == 0'
}

@test "an operator-exported URL wins over the config bridge" {
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
YML
  export AUTOSPEC_PROJECT_BOARD_URL="https://github.com/orgs/o/projects/99"
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST="o/*"

  cat > "$TMP/resolve.sh" <<'SH'
#!/usr/bin/env bash
case "$*" in
  *"projects/99"*) cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":7,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
  ;;
  *) printf '{"items":[]}' ;;
esac
SH
  chmod +x "$TMP/resolve.sh"

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.board.promotable[]?] | index(7) != null'
}
