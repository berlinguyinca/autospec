#!/usr/bin/env bats
# Regression coverage for issue #1859: batch claims must not count workers that
# never produced durable startup evidence (heartbeat or branch ref).

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    if [ ! -x "$AUTOSPEC" ]; then
        cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --bin autospec
    fi
    FIXTURE="$REPO_ROOT/tests/fixtures/autospec-run/issue-1858-run-state.json"
    TMPDIR_BATS="$(mktemp -d)"
    MOCK_BIN="$TMPDIR_BATS/bin"
    mkdir -p "$MOCK_BIN"
    : > "$TMPDIR_BATS/edit.log"
    : > "$TMPDIR_BATS/clear.log"
    mkdir -p "$TMPDIR_BATS/heartbeats/test__repo"

    write_issue_list > "$TMPDIR_BATS/auto.json"
    jq -n --arg body "$(safe_body)" '[{number:1858,title:"startup failed",body:$body,labels:[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]}]' > "$TMPDIR_BATS/active.json"
    cp "$TMPDIR_BATS/active.json" "$TMPDIR_BATS/active-original.json"

    write_git_mock
    write_gh_mock
}

write_git_mock() {
    REAL_GIT="$(command -v git)"
    cat > "$MOCK_BIN/git.new" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "ls-remote" ]; then
    # No remote branch ref exists for the issue-1858 startup-failure fixture.
    if [ "\${GIT_LS_REMOTE_FAIL:-0}" = "1" ]; then
        exit 2
    fi
    exit 0
fi
if [ "\$1" = "show-ref" ] && printf '%s\n' "\$*" | grep -q 'fix/issue-1858-startup-failed'; then
    # No local branch ref exists for the issue-1858 startup-failure fixture.
    exit 1
fi
exec "$REAL_GIT" "\$@"
MOCKEOF
    publish_mock "$MOCK_BIN/git.new" "$MOCK_BIN/git"
}

write_gh_mock() {
    cat > "$MOCK_BIN/gh.new" <<MOCKEOF
#!/usr/bin/env bash
set -eu
TMPDIR_BATS="$TMPDIR_BATS"
FIXTURE="$FIXTURE"
if [ "\${1:-}" = "api" ] && [ "\${2:-}" = "--method" ] && [ "\${3:-}" = "GET" ]; then
    case "\${4:-}" in
      *"labels=auto-implement"*)
        jq '{raw_count:length,items:map({number,title,body,labels:[.labels[].name],author:{login:"fixture-agent"}})}' "\$TMPDIR_BATS/auto.json"
        ;;
      *"labels=in-progress-by-bot"*)
        jq '{raw_count:length,items:map({number,title,body,labels:[.labels[].name],author:{login:"fixture-agent"}})}' "\$TMPDIR_BATS/active.json"
        ;;
      *) printf '{"raw_count":0,"items":[]}\n' ;;
    esac
    exit 0
fi
sub="\${1:-} \${2:-}"
case "\$sub" in
  "issue list")
    label=""
    while [ "\$#" -gt 0 ]; do
      if [ "\$1" = "--label" ]; then label="\${2:-}"; fi
      shift
    done
    case "\$label" in
      auto-implement) cat "\$TMPDIR_BATS/auto.json" ;;
      in-progress-by-bot) cat "\$TMPDIR_BATS/active.json" ;;
      *) printf '[]\n' ;;
    esac
    ;;
  "issue view")
    printf '{"state":"OPEN","body":"","labels":[]}\n'
    ;;
  "issue edit")
    printf '%s\n' "\$*" >> "\$TMPDIR_BATS/edit.log"
    if [ "\${ISSUE_EDIT_FAIL:-0}" = "1" ]; then
      exit 1
    fi
    case "\$*" in
      *"--remove-label in-progress-by-bot --add-label auto-implement"*) printf '[]\n' > "\$TMPDIR_BATS/active.json" ;;
      *"--remove-label auto-implement --add-label in-progress-by-bot"*) cp "\$TMPDIR_BATS/active-original.json" "\$TMPDIR_BATS/active.json" ;;
    esac
    exit 0
    ;;
  "repo view")
    printf 'test/repo\n'
    ;;
  *)
    if [ "\${FAIL_RUN_STATE_READ:-0}" = "1" ] && [ "\${1:-}" = "api" ] && [ "\${2:-}" = "repos/test/repo/issues/1858/comments" ]; then
      exit 1
    fi
    if [ "\${1:-}" = "api" ] && [ "\${2:-}" = "repos/test/repo/issues/1858/comments" ]; then
      state_json="\$(cat "\$FIXTURE")"
      if [ "\${FRESH_RUN_STATE:-0}" = "1" ]; then
        now="\$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
        state_json="\$(jq --arg now "\$now" '.claimed_at = \$now | .updated_at = \$now' "\$FIXTURE")"
      fi
      body="<!-- autospec-run-state:begin -->
\$state_json
<!-- autospec-run-state:end -->"
      jq -n --arg body "\$body" '[{id:10,body:\$body,updated_at:"2000-01-01T00:00:00Z"}]'
      exit 0
    fi
    if [ "\${1:-}" = "api" ] && [ "\${2:-}" = "repos/test/repo/issues/comments/10" ]; then
      printf '%s\n' "\$*" >> "\$TMPDIR_BATS/clear.log"
      if [ "\${RUN_STATE_CLEAR_FAIL:-0}" = "1" ]; then
        exit 1
      fi
      exit 0
    fi
    printf '[]\n'
    ;;
esac
MOCKEOF
    publish_mock "$MOCK_BIN/gh.new" "$MOCK_BIN/gh"
}

publish_mock() {
    source_path="$1"
    destination_path="$2"
    chmod +x "$source_path"
    mv "$source_path" "$destination_path"
    # Atomic rename prevents inode clashes; repeated validation on this ZFS
    # host still requires a short close-to-exec settle interval.
    sleep 0.01
}

teardown() {
    rm -rf "$TMPDIR_BATS"
}

safe_body() {
    cat <<'BODY'
## Safety review

<!-- autospec-safety:begin -->
- **decision:** `SAFETY_PASS`
<!-- autospec-safety:end -->

## Implementation outline

- edit `foo/bar.sh`
BODY
}

write_issue_list() {
    jq -n '[1859,1860,1861] | map({
      number:.,
      title:("ready " + tostring),
      body:("## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n\n## Implementation outline\n\n- edit `foo/" + tostring + ".sh`"),
      labels:[{"name":"auto-implement"},{"name":"safety:reviewed"}]
    })'
}

run_list_ready() {
    # The autospec-run orchestrator may export AUTOSPEC_RUN_ONLY_ISSUES for the
    # live queue; this fixture must exercise all synthetic issues regardless.
    unset AUTOSPEC_RUN_ONLY_ISSUES
    unset AUTOSPEC_CONFIG_FILE
    (
        cd "$TMPDIR_BATS"
        PATH="$MOCK_BIN:$PATH" \
        AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=3 \
        AUTOSPEC_HEARTBEAT_DIR="$TMPDIR_BATS/heartbeats" \
          "$AUTOSPEC" queue ready --repo test/repo --batch-size 3
    )
}

@test "batch size 3 ignores and requeues startup-failed second worker with no heartbeat or branch" {
    FAIL_RUN_STATE_READ=0 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | index(1858) == null')" = "true" ]
    [ "$(printf '%s\n' "$output" | jq -r '.worker_cap.active_count')" = "0" ]
    [ "$(printf '%s\n' "$output" | jq -r '.batch | map(.number) | join(",")')" = "1859,1860,1861" ] \
        || { printf 'queue output: %s\n' "$output" >&3; false; }
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$TMPDIR_BATS/edit.log"
    grep -q -- '^api repos/test/repo/issues/comments/10 -X DELETE' "$TMPDIR_BATS/clear.log"
}

@test "transient run-state read failures preserve claimed issue instead of requeueing it" {
    FAIL_RUN_STATE_READ=1 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | join(",")')" = "1858" ]
    [ "$(printf '%s\n' "$output" | jq -r '.worker_cap.active_count')" = "1" ]
    # A transient run-state read failure keeps the active worker counted for the
    # repo-wide cap; with cap=3 and one preserved claim, only two new workers fit.
    [ "$(printf '%s\n' "$output" | jq -r '.batch | map(.number) | join(",")')" = "1859,1860" ]
    [ ! -s "$TMPDIR_BATS/edit.log" ]
    [ ! -s "$TMPDIR_BATS/clear.log" ]
}

@test "young no-evidence startup claims are ignored for worker capacity until timeout" {
    FRESH_RUN_STATE=1 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | join(",")')" = "" ]
    [ "$(printf '%s\n' "$output" | jq -r '.worker_cap.active_count')" = "0" ]
    [ "$(printf '%s\n' "$output" | jq -r '.batch | map(.number) | join(",")')" = "1859,1860,1861" ]
    [ ! -s "$TMPDIR_BATS/edit.log" ]
    [ ! -s "$TMPDIR_BATS/clear.log" ]
}

@test "transient remote branch probe failures preserve claimed issue instead of requeueing it" {
    GIT_LS_REMOTE_FAIL=1 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | join(",")')" = "1858" ]
    [ "$(printf '%s\n' "$output" | jq -r '.worker_cap.active_count')" = "1" ]
    [ ! -s "$TMPDIR_BATS/edit.log" ]
    [ ! -s "$TMPDIR_BATS/clear.log" ]
}

@test "failed stale requeue label mutation preserves claimed issue and run-state" {
    ISSUE_EDIT_FAIL=1 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | join(",")')" = "1858" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$TMPDIR_BATS/edit.log"
    [ ! -s "$TMPDIR_BATS/clear.log" ]
}

@test "failed run-state clear rolls labels back and preserves claimed issue" {
    RUN_STATE_CLEAR_FAIL=1 AUTOSPEC_GH_API_RETRIES=1 run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | join(",")')" = "1858" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$TMPDIR_BATS/edit.log"
    grep -q -- '--remove-label auto-implement --add-label in-progress-by-bot' "$TMPDIR_BATS/edit.log"
    grep -q -- '^api repos/test/repo/issues/comments/10 -X DELETE' "$TMPDIR_BATS/clear.log"
}
