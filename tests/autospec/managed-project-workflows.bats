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
