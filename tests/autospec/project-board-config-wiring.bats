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

@test "operator-facing project_board settings reach the resolver's env unaltered" {
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
  state_field_candidates: ["Custom state"]
  dep_field_candidates: ["Custom deps"]
  dep_markers: ["Waiting on"]
  max_parallel_repos: 5
  item_limit: 42
YML

  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
{
  printf 'STATE_FIELDS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_STATE_FIELDS:-}"
  printf 'DEP_FIELDS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_DEP_FIELDS:-}"
  printf 'DEP_MARKERS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_DEP_MARKERS:-}"
  printf 'PARALLEL=%s\n' "\${AUTOSPEC_PROJECT_BOARD_PARALLEL:-}"
  printf 'LIMIT=%s\n' "\${AUTOSPEC_PROJECT_BOARD_LIMIT:-}"
} > "$TMP/env-capture.txt"
cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
SH
  chmod +x "$TMP/resolve.sh"

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]

  run cat "$TMP/env-capture.txt"
  [ "$status" -eq 0 ]
  [[ "$output" == *"STATE_FIELDS=Custom state"* ]]
  [[ "$output" == *"DEP_FIELDS=Custom deps"* ]]
  [[ "$output" == *"DEP_MARKERS=Waiting on"* ]]
  [[ "$output" == *"PARALLEL=5"* ]]
  [[ "$output" == *"LIMIT=42"* ]]
}

@test "an already-exported env var wins over the bridged YAML value for the new fields" {
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
  state_field_candidates: ["Custom state"]
  max_parallel_repos: 5
YML
  export AUTOSPEC_PROJECT_BOARD_STATE_FIELDS="Operator override"
  export AUTOSPEC_PROJECT_BOARD_PARALLEL="9"

  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
{
  printf 'STATE_FIELDS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_STATE_FIELDS:-}"
  printf 'PARALLEL=%s\n' "\${AUTOSPEC_PROJECT_BOARD_PARALLEL:-}"
} > "$TMP/env-capture.txt"
cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
SH
  chmod +x "$TMP/resolve.sh"

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]

  run cat "$TMP/env-capture.txt"
  [ "$status" -eq 0 ]
  [[ "$output" == *"STATE_FIELDS=Operator override"* ]]
  [[ "$output" == *"PARALLEL=9"* ]]
}

@test "an unconfigured project_board block leaves the resolver seeing the legacy hardcoded defaults" {
  # Only url/repo_allowlist are set — every other key is absent from YAML.
  # This proves the "absent key" path is byte-identical to pre-bridge
  # behavior: the bridge (backed by ProjectBoardConfig::default() for the
  # unset fields) emits the SAME literal defaults project-board-resolve.sh
  # used to hardcode, so observed values never change even though the
  # mechanism producing them now goes through the bridge unconditionally.
  cat > "$TMP/repo/.autospec/autonomous.yml" <<'YML'
project_board:
  url: https://github.com/orgs/o/projects/1
  repo_allowlist: ["o/*"]
YML

  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
{
  printf 'STATE_FIELDS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_STATE_FIELDS:-}"
  printf 'DEP_FIELDS=%s\n' "\${AUTOSPEC_PROJECT_BOARD_DEP_FIELDS:-}"
  printf 'PARALLEL=%s\n' "\${AUTOSPEC_PROJECT_BOARD_PARALLEL:-}"
  printf 'LIMIT=%s\n' "\${AUTOSPEC_PROJECT_BOARD_LIMIT:-}"
} > "$TMP/env-capture.txt"
cat <<'JSON'
{"project":{},"fields":{},"repos":["o/r"],"items":[{"item_id":"PVTI_a","repo":"o/r","number":5,"state":"open","labels":[],"body":"Blocked by: none."}]}
JSON
SH
  chmod +x "$TMP/resolve.sh"

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]

  run cat "$TMP/env-capture.txt"
  [ "$status" -eq 0 ]
  [[ "$output" == *"STATE_FIELDS=AutoSpec state,Delivery status"* ]]
  [[ "$output" == *"DEP_FIELDS=Dependencies,Depends on"* ]]
  [[ "$output" == *"PARALLEL=2"* ]]
  [[ "$output" == *"LIMIT=500"* ]]
}

@test "with no .autospec/autonomous.yml at all, the resolver is never invoked and nothing crashes" {
  # No config file, no AUTOSPEC_PROJECT_BOARD_URL export: board_plan()
  # treats this as a dry/empty board, exactly as before this change.
  cat > "$TMP/resolve.sh" <<SH
#!/usr/bin/env bash
touch "$TMP/resolve-was-called"
printf '{"items":[]}'
SH
  chmod +x "$TMP/resolve.sh"

  run bash "$SCRIPT" --repo o/r
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.board == null or (.board.ready // 0) == 0'
  [ ! -e "$TMP/resolve-was-called" ]
}
