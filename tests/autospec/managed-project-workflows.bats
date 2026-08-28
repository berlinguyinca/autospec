#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
HELPER="$REPO_ROOT/skills/autospec-shared/scripts/project-sync-issue.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$AUTOSPEC_CALLS"
[ -z "${EVENTS:-}" ] || printf 'autospec:%s\n' "$*" >> "$EVENTS"
if [ -n "${AUTOSPEC_SYNC_FAIL:-}" ]; then
  echo "sync denied" >&2
  exit 9
fi
SH
  chmod +x "$TMP/bin/autospec"
  export AUTOSPEC_CALLS="$TMP/autospec.calls"
  export PATH="$TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_CALLS AUTOSPEC_SYNC_FAIL AUTOSPEC_DRY_RUN
}

@test "greenfield skill bootstraps verify before registering spawned repositories" {
  for skill in autospec autospec-define autospec-split; do
    body="$REPO_ROOT/skills/$skill/SKILL.md"
    expanded="$TMP/$skill.expanded"
    bash "$REPO_ROOT/scripts/expand-skill-blocks.sh" "$body" > "$expanded"
    create_line="$(grep -nF 'gh repo create <owner>/<name> --<private|public> --source=. --remote=origin --push' "$expanded" | head -1 | cut -d: -f1)"
    verify_line="$(grep -nF 'gh repo view <owner>/<name> --json url,defaultBranchRef' "$expanded" | head -1 | cut -d: -f1)"
    register_line="$(grep -nF 'project onboard --repo-dir "$PWD" --repo "$REPO" --spawned-from "$SPAWNED_FROM"' "$expanded" | head -1 | cut -d: -f1)"
    [ -n "$verify_line" ]
    [ -n "$register_line" ]
    [ "$verify_line" -lt "$register_line" ]
    if [ "$skill" != autospec ]; then
      [ -n "$create_line" ]
      [ "$create_line" -lt "$verify_line" ]
    fi
    grep -q 'journaled_projection_pending' "$expanded"
    grep -q 'ERROR.*registration failed before durable admission' "$expanded"
  done
}

@test "autospec-project exposes bounded onboard and managed sync arguments as data" {
  body="$REPO_ROOT/skills/autospec-project/SKILL.md"
  grep -Fq '/autospec-project onboard --repo owner/name' "$body"
  grep -Fq '/autospec-project onboard --workspace /absolute/path' "$body"
  grep -Fq '/autospec-project onboard --owner owner --allow owner/repo --allow owner/prefix-*' "$body"
  grep -Fxq '/autospec-project sync' "$body"
  grep -Fq 'requires at least one explicit `--allow` value' "$body"
  grep -Fq 'autospec project onboard --repo-dir "$PWD" --repo "owner/name"' "$body"
  grep -Fq 'autospec project onboard --repo-dir "$PWD" --workspace "/absolute/path"' "$body"
  grep -Fq 'autospec project onboard --repo-dir "$PWD" --owner "owner"' "$body"
  grep -Fq -- '--allow "pattern"' "$body"
  grep -Fq 'Forward `--dry-run` as a separate literal flag' "$body"
  grep -Fq 'never use' "$body"
  grep -Fq '`eval`' "$body"
  grep -Fq '`project_url`' "$body"
}

@test "control plane registers adopted and created repositories without fabricating spawned evidence" {
  project="$TMP/control-project"
  remotes="$TMP/remotes"
  events="$TMP/control.events"
  mkdir -p "$project/.autospec" "$remotes/acme" "$TMP/control-bin"
  git init --bare "$remotes/acme/adopted.git" >/dev/null
  cat > "$TMP/control-bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'gh:%s\n' "$*" >> "$EVENTS"
if [ "$1 $2" = "repo view" ]; then
  repo="$3"
  [ -d "$REMOTE_ROOT/${repo}.git" ] || exit 1
  printf 'file://%s/%s.git\n' "$REMOTE_ROOT" "$repo"
  exit 0
fi
if [ "$1 $2" = "repo create" ]; then
  repo="$3"
  mkdir -p "$REMOTE_ROOT/$(dirname "$repo")"
  git init --bare "$REMOTE_ROOT/${repo}.git" >/dev/null
  exit 0
fi
exit 99
SH
  cat > "$TMP/control-bin/autospec" <<'SH'
#!/usr/bin/env bash
printf 'autospec:%s\n' "$*" >> "$EVENTS"
printf '{"outcome":"reconciled","pending_projection":0}\n'
SH
  chmod +x "$TMP/control-bin/gh" "$TMP/control-bin/autospec"

  run env PATH="$TMP/control-bin:$PATH" EVENTS="$events" REMOTE_ROOT="$remotes" \
    AUTOSPEC_BIN="$TMP/control-bin/autospec" AUTOSPEC_RUN_ID='run:lower-precedence' \
    AUTOSPEC_CONTROL_PLANE_WORKDIR="$TMP/work" bash -c \
    'cd "$1" && bash "$2" bootstrap --confirm --owner acme --governance-repo created --observatory-repo adopted --source-spec "spec;touch should-not-exist"' \
    _ "$project" "$REPO_ROOT/scripts/autospec-control-plane.sh"
  [ "$status" -eq 0 ]
  grep -Fxq "autospec:project onboard --repo-dir $project --repo acme/adopted" "$events"
  grep -Fxq "autospec:project onboard --repo-dir $project --repo acme/created --spawned-from spec;touch should-not-exist" "$events"
  ! grep -F 'acme/adopted --spawned-from' "$events"
  [ ! -e "$project/should-not-exist" ]
  create_line="$(grep -n '^gh:repo create acme/created' "$events" | cut -d: -f1)"
  verify_line="$(grep -n '^gh:repo view acme/created --json url,defaultBranchRef' "$events" | tail -1 | cut -d: -f1)"
  register_line="$(grep -n '^autospec:project onboard .*acme/created' "$events" | cut -d: -f1)"
  [ "$create_line" -lt "$verify_line" ]
  [ "$verify_line" -lt "$register_line" ]
}

@test "control plane keeps a verified repository when registration remains pending" {
  project="$TMP/failing-project"
  remotes="$TMP/failing-remotes"
  events="$TMP/failing.events"
  mkdir -p "$project/.autospec" "$remotes" "$TMP/failing-bin"
  cat > "$TMP/failing-bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'gh:%s\n' "$*" >> "$EVENTS"
if [ "$1 $2" = "repo view" ]; then
  [ -d "$REMOTE_ROOT/$3.git" ] || exit 1
  printf 'file://%s/%s.git\n' "$REMOTE_ROOT" "$3"
  exit 0
fi
if [ "$1 $2" = "repo create" ]; then
  mkdir -p "$REMOTE_ROOT/$(dirname "$3")"
  git init --bare "$REMOTE_ROOT/$3.git" >/dev/null
  exit 0
fi
exit 99
SH
  cat > "$TMP/failing-bin/autospec" <<'SH'
#!/usr/bin/env bash
printf 'autospec:%s\n' "$*" >> "$EVENTS"
printf '{"outcome":"journaled_projection_pending","pending_projection":2,"error":"project lookup unavailable"}\n'
SH
  chmod +x "$TMP/failing-bin/gh" "$TMP/failing-bin/autospec"

  run env PATH="$TMP/failing-bin:$PATH" EVENTS="$events" REMOTE_ROOT="$remotes" \
    AUTOSPEC_BIN="$TMP/failing-bin/autospec" AUTOSPEC_CONTROL_PLANE_WORKDIR="$TMP/failing-work" \
    bash -c 'cd "$1" && bash "$2" bootstrap --confirm --owner acme --governance-repo gov --observatory-repo obs' \
    _ "$project" "$REPO_ROOT/scripts/autospec-control-plane.sh"
  [ "$status" -eq 0 ]
  warning='WARNING: managed Project repository registration journaled; projection remains pending (count=2)'
  [ "$(printf '%s\n' "$output" | grep -Fxc "$warning")" -eq 2 ]
  [ -d "$remotes/acme/gov.git" ]
  [ -d "$remotes/acme/obs.git" ]
  [ "$(grep -c '^gh:repo create acme/gov' "$events")" -eq 1 ]
  [ "$(grep -c '^gh:repo create acme/obs' "$events")" -eq 1 ]
}

@test "control plane propagates hard registration failures instead of treating them as pending" {
  project="$TMP/hard-failure-project"
  remotes="$TMP/hard-failure-remotes"
  events="$TMP/hard-failure.events"
  mkdir -p "$project/.autospec" "$remotes" "$TMP/hard-failure-bin"
  cat > "$TMP/hard-failure-bin/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf 'gh:%s\n' "$*" >> "$EVENTS"
if [ "$1 $2" = "repo view" ]; then
  [ -d "$REMOTE_ROOT/$3.git" ] || exit 1
  printf 'file://%s/%s.git\n' "$REMOTE_ROOT" "$3"
  exit 0
fi
if [ "$1 $2" = "repo create" ]; then
  mkdir -p "$REMOTE_ROOT/$(dirname "$3")"
  git init --bare "$REMOTE_ROOT/$3.git" >/dev/null
  exit 0
fi
exit 99
SH
  cat > "$TMP/hard-failure-bin/autospec" <<'SH'
#!/usr/bin/env bash
printf 'autospec:%s\n' "$*" >> "$EVENTS"
printf 'invalid managed project configuration\n' >&2
exit 9
SH
  chmod +x "$TMP/hard-failure-bin/gh" "$TMP/hard-failure-bin/autospec"

  run env PATH="$TMP/hard-failure-bin:$PATH" EVENTS="$events" REMOTE_ROOT="$remotes" \
    AUTOSPEC_BIN="$TMP/hard-failure-bin/autospec" AUTOSPEC_CONTROL_PLANE_WORKDIR="$TMP/hard-failure-work" \
    bash -c 'cd "$1" && bash "$2" bootstrap --confirm --owner acme --governance-repo gov --observatory-repo obs' \
    _ "$project" "$REPO_ROOT/scripts/autospec-control-plane.sh"

  [ "$status" -eq 9 ]
  [[ "$output" == *"invalid managed project configuration"* ]]
  [ -d "$remotes/acme/gov.git" ]
  [ ! -d "$remotes/acme/obs.git" ]
  [ "$(grep -c '^gh:repo create acme/gov' "$events")" -eq 1 ]
}

@test "shared issue projection invokes the managed sync boundary" {
  run bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [ "$output" = "" ]
  grep -Fxq "project sync --repo-dir $TMP/repo --issue-url https://github.com/acme/widgets/issues/42" "$AUTOSPEC_CALLS"
}

@test "shared issue projection skips dry-run and degrades on sync failure" {
  run env AUTOSPEC_DRY_RUN=1 bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [ ! -e "$AUTOSPEC_CALLS" ]

  run env AUTOSPEC_SYNC_FAIL=1 bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"WARNING: managed Project sync failed"* ]]
  [ "$(wc -l < "$AUTOSPEC_CALLS" | tr -d ' ')" -eq 1 ]
}

@test "standalone publisher skills install the shared projection helper" {
  skills=(
    autospec-autonomous
    autospec-explore
    autospec-grow-define
    autospec-qa
    autospec-release
    autospec-review
    autospec-run
  )
  for skill in "${skills[@]}"; do
    run env HOME="$TMP/home-$skill" sh "$REPO_ROOT/skills/$skill/install.sh" --harness codex --dry-run
    [ "$status" -eq 0 ] || {
      echo "$skill installer failed: $output" >&2
      return 1
    }
    [[ "$output" == *"project-sync-issue.sh"* ]] || {
      echo "$skill installer omitted project-sync-issue.sh" >&2
      return 1
    }
  done
}

@test "brute-force ambiguous create adopts and syncs the refreshed issue once" {
  repo="$TMP/qa-repo"
  mkdir -p "$repo/src"
  cat > "$repo/src/classify.py" <<'PY'
def classify(name):
    if "acid" in name:
        return "acid"
    if "base" in name:
        return "base"
    if "salt" in name:
        return "salt"
    if "ion" in name:
        return "ion"
    if "gas" in name:
        return "gas"
    return "unknown"
PY
  git -C "$repo" init -q
  cp "$HELPER" "$TMP/bin/project-sync-issue.sh"
  events="$TMP/events"
  body_file="$TMP/created-body"
  open_lists="$TMP/open-lists"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf 'gh:%s\n' "$*" >> "$EVENTS"
if [ "$1 $2" = "issue list" ]; then
  if [[ "$*" == *"--state closed"* ]]; then printf '[]\n'; exit 0; fi
  count=0; [ -f "$OPEN_LISTS" ] && count="$(cat "$OPEN_LISTS")"
  count=$((count + 1)); printf '%s\n' "$count" > "$OPEN_LISTS"
  if [ "$count" -eq 1 ]; then printf '[]\n'; else
    jq -cn --rawfile body "$BODY_FILE" '[{number:88,state:"OPEN",title:"adopted",body:$body,url:"https://github.com/acme/widgets/issues/88"}]'
  fi
  exit 0
fi
if [ "$1 $2" = "issue create" ]; then
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--body" ]; then printf '%s' "$2" > "$BODY_FILE"; break; fi
    shift
  done
  exit 1
fi
exit 0
SH
  chmod +x "$TMP/bin/gh" "$TMP/bin/project-sync-issue.sh"

  run env REPO_DIR="$repo" VERDICT_FILE="$repo/.autospec/qa-verdict.json" \
    AUTOSPEC_SCRIPTS_DIR="$TMP/bin" EVENTS="$events" BODY_FILE="$body_file" \
    OPEN_LISTS="$open_lists" bash "$REPO_ROOT/scripts/qa-brute-force-sweep.sh"
  [ "$status" -eq 0 ]
  [ "$(grep -c '^gh:issue create' "$events")" -eq 1 ] || {
    printf '%s\n' "$output" >&2
    cat "$events" >&2
    return 1
  }
  [ "$(wc -l < "$AUTOSPEC_CALLS" | tr -d ' ')" -eq 1 ]
  grep -Fq 'https://github.com/acme/widgets/issues/88' "$AUTOSPEC_CALLS"
}

@test "publisher creates once then syncs once, skips dry-run, and never recreates after sync failure" {
  cp "$HELPER" "$TMP/bin/project-sync-issue.sh"
  events="$TMP/publisher.events"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf 'gh:%s\n' "$*" >> "$EVENTS"
if [ "$1 $2" = "issue list" ]; then printf '0\n'; exit 0; fi
if [ "$1 $2" = "issue create" ]; then printf 'https://github.com/acme/widgets/issues/42\n'; fi
SH
  chmod +x "$TMP/bin/gh" "$TMP/bin/project-sync-issue.sh"
  finding='{"category":"behavior","summary":"checkout fails","file":"src/app.rs","status":"FAIL"}'

  run env EVENTS="$events" AUTOSPEC_SCRIPTS_DIR="$TMP/bin" REPO_DIR="$TMP/repo" \
    bash "$REPO_ROOT/scripts/qa-finding-to-issue.sh" --finding "$finding" --dedup-cache "$TMP/create.cache"
  [ "$status" -eq 0 ]
  [ "$(grep -c '^gh:issue create' "$events")" -eq 1 ]
  [ "$(grep -c '^autospec:project sync' "$events")" -eq 1 ]
  create_line="$(grep -n '^gh:issue create' "$events" | cut -d: -f1)"
  sync_line="$(grep -n '^autospec:project sync' "$events" | cut -d: -f1)"
  [ "$create_line" -lt "$sync_line" ]

  : > "$events"
  : > "$AUTOSPEC_CALLS"
  run env EVENTS="$events" AUTOSPEC_SCRIPTS_DIR="$TMP/bin" REPO_DIR="$TMP/repo" \
    bash "$REPO_ROOT/scripts/qa-finding-to-issue.sh" --finding "$finding" --dry-run --dedup-cache "$TMP/dry.cache"
  [ "$status" -eq 0 ]
  ! grep -q '^gh:issue create' "$events"
  [ ! -s "$AUTOSPEC_CALLS" ]

  : > "$events"
  run env EVENTS="$events" AUTOSPEC_SYNC_FAIL=1 AUTOSPEC_SCRIPTS_DIR="$TMP/bin" REPO_DIR="$TMP/repo" \
    bash "$REPO_ROOT/scripts/qa-finding-to-issue.sh" --finding "$finding" --dedup-cache "$TMP/fail.cache"
  [ "$status" -eq 0 ]
  [ "$(grep -c '^gh:issue create' "$events")" -eq 1 ]
  [ "$(grep -c '^autospec:project sync' "$events")" -eq 1 ]
}

@test "define split and classify require sync after successful non-dry-run mutation" {
  for skill in autospec-define autospec-split autospec-classify; do
    body="$REPO_ROOT/skills/$skill/SKILL.md"
    grep -Fq 'autospec project sync --repo-dir "$PWD" --issue-url "$ISSUE_URL"' "$body"
    grep -q 'dry-run' "$body"
    grep -q 'WARNING.*Project sync' "$body"
  done
}
