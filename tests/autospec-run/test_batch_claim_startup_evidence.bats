#!/usr/bin/env bats
# Regression coverage for issue #1859: batch claims must not count workers that
# never produced durable startup evidence (heartbeat or branch ref).

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/list-ready-issues.sh"
    FIXTURE="$REPO_ROOT/tests/fixtures/autospec-run/issue-1858-run-state.json"
    TMPDIR_BATS="$(mktemp -d)"
    MOCK_BIN="$TMPDIR_BATS/bin"
    mkdir -p "$MOCK_BIN"
    : > "$TMPDIR_BATS/edit.log"
    mkdir -p "$TMPDIR_BATS/heartbeats/test__repo"

    write_issue_list > "$TMPDIR_BATS/auto.json"
    jq -n --arg body "$(safe_body)" '[{number:1858,title:"startup failed",body:$body,labels:[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]}]' > "$TMPDIR_BATS/active.json"

    REAL_GIT="$(command -v git)"
    cat > "$MOCK_BIN/git" <<MOCKEOF
#!/usr/bin/env bash
if [ "\$1" = "ls-remote" ]; then
    # No remote branch ref exists for the issue-1858 startup-failure fixture.
    exit 0
fi
exec "$REAL_GIT" "\$@"
MOCKEOF
    chmod +x "$MOCK_BIN/git"

    cat > "$MOCK_BIN/gh" <<MOCKEOF
#!/usr/bin/env bash
set -eu
TMPDIR_BATS="$TMPDIR_BATS"
FIXTURE="$FIXTURE"
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
    exit 0
    ;;
  "repo view")
    printf 'test/repo\n'
    ;;
  *)
    if [ "\${1:-}" = "api" ] && [ "\${2:-}" = "repos/test/repo/issues/1858/comments" ]; then
      body="<!-- autospec-run-state:begin -->
\$(cat "\$FIXTURE")
<!-- autospec-run-state:end -->"
      jq -n --arg body "\$body" '[{id:10,body:$body,updated_at:"2000-01-01T00:00:00Z"}]'
      exit 0
    fi
    if [ "\${1:-}" = "api" ] && [ "\${2:-}" = "repos/test/repo/issues/comments/10" ]; then
      exit 0
    fi
    printf '[]\n'
    ;;
esac
MOCKEOF
    chmod +x "$MOCK_BIN/gh"
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
    PATH="$MOCK_BIN:$PATH" \
    AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS=3 \
    AUTOSPEC_HEARTBEAT_DIR="$TMPDIR_BATS/heartbeats" \
    AUTOSPEC_STARTUP_EVIDENCE_TIMEOUT_SECS=300 \
      bash "$SCRIPT" --repo test/repo --batch-size 3
}

@test "batch size 3 ignores and requeues startup-failed second worker with no heartbeat or branch" {
    run run_list_ready

    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | jq -r '.claimed | map(.number) | index(1858) == null')" = "true" ]
    [ "$(printf '%s\n' "$output" | jq -r '.worker_cap.active_count')" = "0" ]
    [ "$(printf '%s\n' "$output" | jq -r '.batch | map(.number) | join(",")')" = "1859,1860,1861" ]
    grep -q -- '--remove-label in-progress-by-bot --add-label auto-implement' "$TMPDIR_BATS/edit.log"
}
