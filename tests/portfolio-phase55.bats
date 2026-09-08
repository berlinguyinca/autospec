#!/usr/bin/env bats
#
# Phase 5.5 adversarial no-mock verification — automatic spec Projects (#3444).
#
# Drives the REAL `autospec project` CLI boundary (crates/autospec-cli) against
# recorded GitHub wire fixtures replayed by a stub `gh` program installed via
# the transport's AUTOSPEC_GH_PROGRAM hook. No network and no mocked domain
# state inside the CLI: every response below is a recorded GraphQL/`gh` JSON
# payload, and every mutation the CLI attempts is logged so the suite can
# assert exact mutation counts.
#
# Adversarial targets named by the issue:
#   - lost responses       create/marker-write ambiguity -> create_unknown,
#                          journaled resume, never a blind second mutation
#   - duplicate markers    two managed blocks in one README; two Projects
#                          bearing the same exact marker
#   - lease races          no lease or portfolio-transaction surface exists in
#                          this build: the CLI must refuse, not pretend
#   - blockers             mode, owner drift, identity conflict, 403 list
#   - completion gates     no command may claim managed_state done
#
# The opt-in live GitHub smoke (disposable artifacts + cleanup) is not part of
# default CI and is out of scope here; see
# reports/autospec-review/automatic-spec-projects-phase55.md for the evidence
# mapping and the follow-up defects this suite pins as fail-closed negatives.

# Runnable both as `bats tests/portfolio-phase55.bats` (repo convention) and
# as `bash tests/portfolio-phase55.bats` (self-reexecutes under bats).
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/.."
CLI_BIN="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/debug/autospec"
OWNER="phase55org"
PRODUCT_KEY="phase55"
PROJECT_URL="https://github.com/orgs/phase55org/projects/7"

# Exact managed marker block the CLI renders for this fixture identity, as a
# JSON string (literal \n escapes, no double quotes inside).
MARKER_BEGIN='<!-- autospec-managed-project:begin -->'
MARKER_END='<!-- autospec-managed-project:end -->'
MARKER_JSON="${MARKER_BEGIN}\\nschema: 2\\nkind: product\\nproduct-key: ${PRODUCT_KEY}\\nowner: ${OWNER}\\n${MARKER_END}"

setup_file() {
  if [ ! -x "$CLI_BIN" ]; then
    (cd "$REPO_ROOT" && cargo build -p autospec-cli) || {
      echo "cargo build -p autospec-cli failed" >&2
      return 1
    }
    CLI_BIN="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/debug/autospec"
  fi
}

setup() {
  TMP="$(mktemp -d)"
  STATE="$TMP/gh-state"
  mkdir -p "$STATE/counters" "$TMP/home"
  chmod 700 "$TMP/home"
  REPO="$TMP/repo"
  mkdir -p "$REPO/.autospec"
  cat > "$REPO/.autospec/autonomous.yml" <<EOF
project_board:
  mode: managed
  product_key: $PRODUCT_KEY
  owner: $OWNER
  repo_allowlist: ["$OWNER/*"]
  repository_seeds: ["$OWNER/autospec"]
  discovery_max_repos: 25
EOF
  cat > "$TMP/gh" <<'STUB'
#!/usr/bin/env bash
# Recorded-fixture gh replacer. Serves $GH_STATE_DIR/<name>.json for each call
# shape; appends every invocation to calls.log; honors <name>.fail files
# (message on stderr, exit 1) to simulate lost/ambiguous or definitive
# GitHub responses.
set -u
STATE="${GH_STATE_DIR:?GH_STATE_DIR is required}"
printf '%s\n' "$*" >> "$STATE/calls.log"

next_counter() {
  local file="$STATE/counters/$1"
  local n=0
  [ -f "$file" ] && n="$(cat "$file")"
  n=$((n + 1))
  printf '%s' "$n" > "$file"
  printf '%s' "$n"
}

serve() {
  if [ -f "$STATE/$1.fail" ]; then
    cat "$STATE/$1.fail" >&2
    exit 1
  fi
  if [ ! -f "$STATE/$1.json" ]; then
    echo "HTTP 404: not found (missing fixture $1.json)" >&2
    exit 1
  fi
  cat "$STATE/$1.json"
}

if [ "${1:-}" = "api" ]; then
  # The Project list is a re-query: serve the current recorded board state.
  serve "list"
  exit $?
fi

if [ "${1:-}" != "project" ]; then
  echo "gh-stub: unsupported command: $*" >&2
  exit 1
fi

sub="${2:-}"
case "$sub" in
  view)
    serve "view-${3}-$(next_counter "view-${3}")"
    ;;
  create)
    title=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --title) title="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    printf '%s\n' "$title" >> "$STATE/create.log"
    mode="ok"
    [ -f "$STATE/create-mode" ] && mode="$(cat "$STATE/create-mode")"
    case "$mode" in
      ambiguous) echo "gh: connection reset by peer" >&2; exit 1 ;;
      definitive) echo "gh: HTTP 403: forbidden (missing 'project' write scope)" >&2; exit 1 ;;
    esac
    serve "create-$(next_counter create)"
    ;;
  edit)
    project_number="$3"
    readme=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --readme) readme="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    printf '%s\n' "$readme" > "$STATE/edited-${project_number}.readme"
    serve "edit-${project_number}-$(next_counter "edit-${project_number}")"
    ;;
  item-list)
    serve "items-${3}"
    ;;
  item-add)
    printf '%s\n' "$*" >> "$STATE/item-add.log"
    echo '{}'
    ;;
  *)
    echo "gh-stub: unsupported project subcommand: $sub" >&2
    exit 1
    ;;
esac
STUB
  chmod +x "$TMP/gh"
}

teardown() {
  rm -rf "$TMP"
}

run_cli() {
  run env AUTOSPEC_HOME="$TMP/home" AUTOSPEC_GH_PROGRAM="$TMP/gh" GH_STATE_DIR="$STATE" \
    "$CLI_BIN" "$@"
}

# Count CLI transport invocations whose argv starts with the given prefix.
call_count() {
  local n
  n="$(grep -c "^$1 " "$STATE/calls.log" 2>/dev/null || true)"
  printf '%s' "${n:-0}"
}

# Recorded GraphQL --paginate --slurp pages: list of {number,title} nodes.
# Rewrites the current recorded board state (the CLI re-queries it per run).
write_list() {
  local nodes="" entry
  for entry in "$@"; do
    [ -n "$nodes" ] && nodes="$nodes,"
    nodes="$nodes{\"number\":${entry%%:*},\"title\":\"${entry#*:}\"}"
  done
  printf '[{"data":{"repositoryOwner":{"projectsV2":{"nodes":[%s],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}]' \
    "$nodes" > "$STATE/list.json"
}

# Record one `gh project view` response: write_project <name> <number> <title> <readme-json-string>
write_project() {
  printf '{"id":"MDc6ProSpec%s","number":%s,"title":"%s","url":"https://github.com/orgs/%s/projects/%s","owner":{"login":"%s"},"readme":"%s"}' \
    "$2" "$2" "$3" "$OWNER" "$2" "$OWNER" "$4" > "$STATE/$1.json"
}

# A README (already JSON-escaped) carrying the exact marker block.
MARKER_README_JSON="# Phase 55 board\\n\\n${MARKER_JSON}\\n"

@test "repeated resolve adopts the same marked Project and never creates again" {
  write_list "7:Phase 55 Board"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  write_project "view-7-2" 7 "Phase 55 Board" "$MARKER_README_JSON"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"\"url\":\"$PROJECT_URL\""* ]]

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"\"url\":\"$PROJECT_URL\""* ]]

  # Adopt, don't create: zero create mutations across both runs.
  [ "$(call_count 'project create')" -eq 0 ]
  [ ! -f "$STATE/create.log" ]
  # Durable binding records the adopted number and owner.
  grep -q '"project_number": 7' "$TMP/home/projects/$PRODUCT_KEY/binding.json"
  grep -q "\"owner\": \"$OWNER\"" "$TMP/home/projects/$PRODUCT_KEY/binding.json"
}

@test "fresh create verifies the marker before the local binding persists" {
  write_list
  write_project "create-1" 9 "$PRODUCT_KEY" ''
  write_project "view-9-1" 9 "$PRODUCT_KEY" ''
  write_project "edit-9-1" 9 "$PRODUCT_KEY" "$MARKER_JSON"
  write_project "view-9-2" 9 "$PRODUCT_KEY" "$MARKER_JSON"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"number":9'* ]]

  # Exactly one create, then the marker write, then post-edit verification.
  [ "$(call_count 'project create')" -eq 1 ]
  [ "$(call_count 'project edit')" -eq 1 ]
  [ "$(call_count 'project view 9')" -eq 2 ]
  # The installed README must carry the exact marker block.
  grep -q "kind: product" "$STATE/edited-9.readme"
  grep -q "$MARKER_BEGIN" "$STATE/edited-9.readme"
  # The binding persisted only after marker verification.
  grep -q '"project_number": 9' "$TMP/home/projects/$PRODUCT_KEY/binding.json"
}

@test "lost create response fails closed and never blindly re-creates" {
  # Run 1: empty board; the create call dies with an ambiguous transport
  # error (response lost). The CLI must journal the intent and fail closed.
  write_list
  echo ambiguous > "$STATE/create-mode"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"cannot create managed GitHub Project"* ]]
  [ "$(call_count 'project create')" -eq 1 ]
  # The create intent is durable in the event journal.
  grep -q "project:create:$PRODUCT_KEY" "$TMP/home/projects/$PRODUCT_KEY/events.jsonl"

  # Run 2: GitHub actually created the Project despite the lost response.
  # The CLI may bind it only with a verified identity; for this identity it
  # must stop with a resumable diagnostic, not a second create.
  rm -f "$STATE/create-mode"
  write_list "9:$PRODUCT_KEY"
  write_project "view-9-1" 9 "$PRODUCT_KEY" ''

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"no verified project identity"* ]]
  # Still exactly one create attempt in total: no blind re-create.
  [ "$(call_count 'project create')" -eq 1 ]
  [ "$(wc -l < "$STATE/create.log")" -eq 1 ]
}

@test "ambiguous marker write is journaled and resumes to the verified Project" {
  write_list
  write_project "create-1" 9 "$PRODUCT_KEY" ''
  write_project "view-9-1" 9 "$PRODUCT_KEY" ''
  # The marker edit dies with a lost response.
  echo "gh: connection reset by peer" > "$STATE/edit-9-1.fail"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"cannot write managed GitHub Project marker"* ]]
  [ "$(call_count 'project create')" -eq 1 ]

  # Resume: GitHub did apply the edit; the CLI adopts by verified identity,
  # acks the pending marker projection, and persists exactly one Project.
  write_project "view-9-2" 9 "$PRODUCT_KEY" "$MARKER_JSON"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"number":9'* ]]
  [ "$(call_count 'project create')" -eq 1 ]
  [ "$(call_count 'project edit')" -eq 1 ]
  grep -q '"project_number": 9' "$TMP/home/projects/$PRODUCT_KEY/binding.json"
}

@test "duplicate managed marker blocks fail closed" {
  write_list "7:Phase 55 Board"
  local double_marker
  double_marker="${MARKER_README_JSON}\\n\\n${MARKER_JSON}"
  write_project "view-7-1" 7 "Phase 55 Board" "$double_marker"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"must contain exactly one complete block"* ]]
  # No recovery mutation of any kind.
  [ "$(call_count 'project create')" -eq 0 ]
  [ "$(call_count 'project edit')" -eq 0 ]
  [ ! -f "$TMP/home/projects/$PRODUCT_KEY/binding.json" ] || \
    ! grep -q '"project_number": 7' "$TMP/home/projects/$PRODUCT_KEY/binding.json"
}

@test "two Projects bearing the exact marker are ambiguous and block resolution" {
  write_list "7:Phase 55 Board" "8:Phase 55 Board Copy"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  write_project "view-8-1" 8 "Phase 55 Board Copy" "$MARKER_README_JSON"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"multiple GitHub Projects have the managed marker"* ]]
  [ "$(call_count 'project create')" -eq 0 ]
  [ "$(call_count 'project edit')" -eq 0 ]
}

@test "marker owned by another organization is a hard identity conflict" {
  write_list "7:Phase 55 Board"
  local foreign
  foreign="${MARKER_JSON/owner: $OWNER/owner: otherorg}"
  write_project "view-7-1" 7 "Phase 55 Board" "$foreign"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"marker owner otherorg conflicts with approved owner"* ]]
  # The Project is not ours to adopt and not ours to mutate.
  [ "$(call_count 'project create')" -eq 0 ]
  [ "$(call_count 'project edit')" -eq 0 ]
}

@test "bound Project presenting a different marker fails closed without mutation" {
  write_list "7:Phase 55 Board"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]

  # Someone else rewrote the README with a marker for another product key.
  local other_key
  other_key="${MARKER_JSON/product-key: $PRODUCT_KEY/product-key: other-key}"
  write_project "view-7-2" 7 "Phase 55 Board" "$other_key"
  local before_edits
  before_edits="$(call_count 'project edit')"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"contains a different managed marker"* ]]
  [ "$(call_count 'project edit')" -eq "$before_edits" ]
}

@test "policy owner drift conflicts with the durable binding before any remote call" {
  write_list "7:Phase 55 Board"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  local calls_before
  calls_before="$(wc -l < "$STATE/calls.log" 2>/dev/null || echo 0)"

  sed -i "s/owner: $OWNER/owner: otherorg/" "$REPO/.autospec/autonomous.yml"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"binding owner conflicts with policy"* ]]
  # The conflict is caught locally: no further GitHub traffic.
  [ "$(wc -l < "$STATE/calls.log")" -eq "$calls_before" ]
}

@test "non-managed repository blocks before any GitHub call" {
  printf 'project_board:\n  mode: external\n' > "$REPO/.autospec/autonomous.yml"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"project_board.mode must be managed"* ]]
  # External mode never satisfies the primary binding: zero transport calls.
  [ ! -f "$STATE/calls.log" ]
}

@test "definitive 403 on Project listing blocks with a diagnostic and no mutation" {
  echo "gh: HTTP 403: forbidden (missing 'project' scope)" > "$STATE/list.fail"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"cannot list GitHub Projects"* ]]
  [ "$(call_count 'project create')" -eq 0 ]
  [ "$(call_count 'project edit')" -eq 0 ]
}

@test "lease and portfolio-transaction surfaces are refused, not faked" {
  # This build ships the managed-Project store (with portfolio identity and
  # lease-generation fields) but not the public portfolio transaction or the
  # coordination-ref lease. The CLI must refuse unknown surfaces instead of
  # claiming lease or portfolio guarantees that do not exist yet.
  run_cli project lease --repo-dir "$REPO"
  [ "$status" -ne 0 ]
  [[ "$output" == *"unknown autospec project subcommand: lease"* ]]

  run_cli portfolio validate --manifest /dev/null
  [ "$status" -ne 0 ]
  [[ "$output" == *"unknown autospec command: portfolio"* ]]

  # No lease state was invented by either attempt.
  [ ! -d "$TMP/home/projects/$PRODUCT_KEY" ] || \
    ! grep -rq '"lease_generation"' "$TMP/home/projects/$PRODUCT_KEY/" 2>/dev/null
}

@test "sync reconciles item membership through the journaled projection" {
  write_list "7:Phase 55 Board"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  printf '{"items":[]}' > "$STATE/items-7.json"

  run_cli project sync --repo-dir "$REPO" --issue-url "https://github.com/$OWNER/autospec/issues/42"
  [ "$status" -eq 0 ]
  [[ "$output" == *"\"outcome\":\"reconciled\""* ]]
  [[ "$output" == *"\"pending_projection\":0"* ]]
  [[ "$output" == *"\"project_url\":\"$PROJECT_URL\""* ]]
  # Exactly one item-add mutation for the requested issue.
  [ "$(wc -l < "$STATE/item-add.log")" -eq 1 ]
  grep -q "issues/42" "$STATE/item-add.log"
}

@test "no CLI surface claims completion: managed_state done is never written" {
  write_list "7:Phase 55 Board"
  write_project "view-7-1" 7 "Phase 55 Board" "$MARKER_README_JSON"
  write_project "view-7-2" 7 "Phase 55 Board" "$MARKER_README_JSON"
  printf '{"items":[]}' > "$STATE/items-7.json"

  run_cli project resolve --repo-dir "$REPO"
  [ "$status" -eq 0 ]
  run_cli project sync --repo-dir "$REPO" --issue-url "https://github.com/$OWNER/autospec/issues/42"
  [ "$status" -eq 0 ]

  # Completion is gated on terminal-success children, parent reconciliation,
  # and the Phase 5.5 audit item; no command in this build may write or emit
  # a done state. (if-forms, not mid-body `!` negations, per the
  # tests/lint/test_bats_negation_checker.bats ratchet.)
  if grep -rq "managed_state" "$TMP/home/projects/$PRODUCT_KEY/"; then
    echo "managed_state was written to the durable state root" >&2
    return 1
  fi
  if [[ "$output" == *"\"done\""* ]]; then
    echo "CLI emitted a done state: $output" >&2
    return 1
  fi
  [[ "$output" == *"\"outcome\":\"reconciled\""* ]]
}
