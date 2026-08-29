#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn autonomous_integration_base_sync() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let remote = fixture.root.join("github.com/test/repo.git");
    let advance = fixture.root.join("base-advance-clone");
    git_fixture(
        &fixture.root,
        &["clone", remote.to_str().unwrap(), advance.to_str().unwrap()],
    );
    git_fixture(&advance, &["config", "user.name", "Autospec Test"]);
    git_fixture(
        &advance,
        &["config", "user.email", "autospec@example.invalid"],
    );
    fs::write(advance.join("README.md"), "advanced integration base\n")
        .expect("write advanced base content");
    git_fixture(&advance, &["add", "README.md"]);
    git_fixture(&advance, &["commit", "-m", "advance the integration base"]);
    git_fixture(&advance, &["push", "origin", "HEAD:main"]);
    let remote_oid = git_fixture(&remote, &["rev-parse", "refs/heads/main"]);
    let local_before = git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]);
    assert_ne!(
        local_before, remote_oid,
        "the fixture must require a fast-forward synchronization"
    );

    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run foreground against an advanced integration base");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let local_after = git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]);
    assert_eq!(
        local_after, remote_oid,
        "the implementation base must be the fetched remote OID"
    );
    git_fixture(
        &fixture.repo_dir,
        &["merge-base", "--is-ancestor", &local_before, &local_after],
    );
    assert_eq!(
        git_fixture(&remote, &["rev-parse", "refs/heads/main"]),
        remote_oid,
        "synchronization must never push to the remote"
    );
}

#[test]
fn autonomous_integration_base_sync_conflict_precedes_selection() {
    let fixture = ForegroundFixture::new();
    fixture.initialize_git_remote();
    let remote = fixture.root.join("github.com/test/repo.git");
    fs::write(fixture.repo_dir.join("README.md"), "local divergence\n")
        .expect("write local divergence content");
    git_fixture(&fixture.repo_dir, &["add", "README.md"]);
    git_fixture(&fixture.repo_dir, &["commit", "-m", "local divergence"]);
    let local_oid = git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]);
    let diverge = fixture.root.join("base-divergence-clone");
    git_fixture(
        &fixture.root,
        &["clone", remote.to_str().unwrap(), diverge.to_str().unwrap()],
    );
    git_fixture(&diverge, &["config", "user.name", "Autospec Test"]);
    git_fixture(
        &diverge,
        &["config", "user.email", "autospec@example.invalid"],
    );
    fs::write(diverge.join("README.md"), "remote divergence\n")
        .expect("write remote divergence content");
    git_fixture(&diverge, &["add", "README.md"]);
    git_fixture(&diverge, &["commit", "-m", "remote divergence"]);
    git_fixture(&diverge, &["push", "origin", "HEAD:main"]);
    let remote_oid = git_fixture(&remote, &["rev-parse", "refs/heads/main"]);
    assert_ne!(
        local_oid, remote_oid,
        "the fixture must diverge the integration bases"
    );

    let output = fixture
        .command()
        .env("AUTOSPEC_FOREGROUND_EMPTY_QUEUE", "1")
        .output()
        .expect("run foreground against a diverged integration base");
    assert!(
        !output.status.success(),
        "a diverged integration base must fail closed before selection; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("diverged") || stderr.contains("integration base"),
        "the failure must name the synchronization conflict; stderr={stderr}"
    );
    assert_eq!(
        git_fixture(&fixture.repo_dir, &["rev-parse", "refs/heads/main"]),
        local_oid,
        "a synchronization conflict must leave the base unchanged"
    );
    let calls = fs::read_to_string(&fixture.calls).expect("read GitHub calls");
    assert!(
        !calls.contains("issue\nedit\n42") && !calls.contains("issue\nview\n42"),
        "no selection or admission may follow a synchronization conflict\ncalls={calls}"
    );
}

struct ForegroundFixture {
    root: PathBuf,
    repo_dir: PathBuf,
    bin: PathBuf,
    mode: PathBuf,
    comments: PathBuf,
    pull_requests: PathBuf,
    calls: PathBuf,
    accountability: PathBuf,
    operator: PathBuf,
    state: PathBuf,
    health: PathBuf,
    heartbeats: PathBuf,
    claim_repo: PathBuf,
    claim_remote: PathBuf,
    claim_state: PathBuf,
}

impl ForegroundFixture {
    fn new() -> Self {
        let root = temp_dir("autospec-foreground-conductor");
        let repo_dir = root.join("repo");
        let bin = root.join("bin");
        let mode = root.join("mode");
        let comments = root.join("comments.json");
        let pull_requests = root.join("pull-requests.json");
        let calls = root.join("gh.log");
        let accountability = root.join("accountability-epic.md");
        let operator = root.join("operator");
        let state = root.join("state");
        let health = root.join("health");
        let heartbeats = root.join("heartbeats");
        let claim_repo = root.join("claim-repo");
        let claim_remote = root.join("claim-remote.git");
        let claim_state = root.join("claim-state");
        fs::create_dir_all(&repo_dir).expect("create repo directory");
        fs::create_dir_all(&bin).expect("create fake bin");
        git_fixture(&root, &["init", "--bare", claim_remote.to_str().unwrap()]);
        git_fixture(&root, &["init", claim_repo.to_str().unwrap()]);
        git_fixture(
            &claim_repo,
            &["remote", "add", "origin", claim_remote.to_str().unwrap()],
        );
        fs::write(&mode, "unreviewed\n").expect("write mode");
        fs::write(&comments, "[]\n").expect("write comments");
        fs::write(&pull_requests, "[]\n").expect("write pull requests");
        write_executable(
            &bin.join("gh"),
            r####"#!/bin/sh
set -eu
printf '%s\n' "$@" >> "$AUTOSPEC_FOREGROUND_CALLS"
if [ -n "${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER:-}" ]; then . "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"; fi
if [ "${AUTOSPEC_FOREGROUND_BLOCK_GH:-0}" = 1 ]; then
  while :; do sleep 1; done
fi
mode="$(cat "$AUTOSPEC_FOREGROUND_MODE")"
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ -n "${AUTOSPEC_BRIDGE_TAKEOVER_OID:-}" ] && [ -e "${AUTOSPEC_BRIDGE_HARNESS_DONE:-/nonexistent}" ] && [ ! -e "${AUTOSPEC_BRIDGE_TAKEOVER_DONE:-/nonexistent}" ]; then
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref refs/autospec/claims/issue-42 "$AUTOSPEC_BRIDGE_TAKEOVER_OID"
  : > "$AUTOSPEC_BRIDGE_TAKEOVER_DONE"
fi
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ -n "${AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE:-}" ] && [ ! -e "$AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE" ] && { [ -z "${AUTOSPEC_BRIDGE_HARNESS_DONE:-}" ] || [ -e "$AUTOSPEC_BRIDGE_HARNESS_DONE" ]; } && { [ "${AUTOSPEC_BRIDGE_FAIL_GH_AFTER_BRANCH:-0}" != 1 ] || git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" show-ref --verify --quiet refs/heads/feat/autonomous-issue-42; }; then
  : > "$AUTOSPEC_BRIDGE_FAIL_GH_READ_ONCE"
  exit 42
fi
if [ "$1" = pr ] && [ "${2:-}" = list ] && [ "${AUTOSPEC_BRIDGE_FAIL_GH_AFTER_CREATE_ALWAYS:-0}" = 1 ] && [ "$(cat "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")" != "[]" ]; then
  exit 42
fi
issue() {
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    if [ "$mode" = claimed ]; then real_labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then real_labels='[]'; else real_labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi
    printf '%s\n' "{\"number\":42,\"title\":\"Ship the bridge fixture\",\"body\":\"## Goal\\n\\nAdd \`tests/smoke/generation.sh\` proving the native executor bridge runs.\\n\\n## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\\n\\n## Implementation outline\\n\\n- \`tests/smoke/generation.sh\`\\n\\n## Tests required\\n\\n- smoke\\n\\n### Primary smoke test (inner loop)\\n\\n\`\`\`bash\\n/bin/test -s tests/smoke/generation.sh\\n\`\`\`\\n\\n### Operator/full verification\\n\\n\`\`\`bash\\n/bin/test -s tests/smoke/generation.sh\\n\`\`\`\",\"labels\":$real_labels,\"author\":{\"login\":\"agent\"},\"state\":\"${FOREGROUND_ISSUE_STATE:-open}\"}"
  elif [ "$mode" = unreviewed ]; then
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Goal\n\nAdd the foreground adapter.","labels":[{"name":"auto-implement"}],"author":{"login":"agent"},"state":"'"${FOREGROUND_ISSUE_STATE:-open}"'"}'
  else
    printf '%s\n' '{"number":42,"title":"Add Rust foreground","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","labels":[{"name":"auto-implement"},{"name":"safety:reviewed"}],"author":{"login":"agent"},"state":"'"${FOREGROUND_ISSUE_STATE:-open}"'"}'
  fi
}
claim_issue() {
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    case " $* " in
      *"{labels: [.labels[] | {name: .name}]}"*) if [ "$mode" = claimed ]; then labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi ;;
      *" --jq "*) if [ "$mode" = claimed ]; then labels='["in-progress-by-bot","safety:reviewed"]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='["auto-implement","safety:reviewed"]'; fi ;;
      *) if [ "$mode" = claimed ]; then labels='[{"name":"in-progress-by-bot"},{"name":"safety:reviewed"}]'; elif [ "$mode" = terminal ]; then labels='[]'; else labels='[{"name":"auto-implement"},{"name":"safety:reviewed"}]'; fi ;;
    esac
    if [ "$mode" = terminal ] && [ -n "${AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE:-}" ] && [ ! -e "$AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE" ]; then
      : > "$AUTOSPEC_FOREGROUND_FAIL_TERMINAL_ONCE"
      exit 1
    fi
    printf '%s\n' "{\"labels\":$labels}"
    return
  fi
  if [ "$mode" = claimed ]; then labels='["in-progress-by-bot","safety:reviewed"]'; else labels='["auto-implement","safety:reviewed"]'; fi
  printf '%s\n' "{\"labels\":$labels,\"title\":\"Add Rust foreground\",\"body\":\"## Safety review\\n\\n<!-- autospec-safety:begin -->\\n- **decision:** \`SAFETY_PASS\`\\n<!-- autospec-safety:end -->\",\"author\":\"agent\"}"
}
steal_claim() {
  reference=refs/autospec/claims/issue-42
  current=$(git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" rev-parse "$reference")
  tree=$(git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" mktree </dev/null)
  message="$AUTOSPEC_FOREGROUND_COMMENTS.claim-message"
  cat > "$message" <<'EOF'
autospec-claim-ledger-v1
generation=foreign-generation

<!-- autospec-run-state:begin -->
{"schema":1,"repo":"test/repo","issue":42,"worker_id":"foreign-worker","state":"claimed","branch":"foreign/issue-42","pr":"","step":"claimed","paths":[],"claimed_at":"2030-07-15T00:00:00Z","updated_at":"2030-07-15T00:00:00Z","ttl_seconds":10800,"claim_id":"foreign-claim"}
<!-- autospec-run-state:end -->
EOF
  oid=$(GIT_AUTHOR_NAME='Autospec Claim Test' \
    GIT_AUTHOR_EMAIL='autospec-claim-test@localhost' \
    GIT_COMMITTER_NAME='Autospec Claim Test' \
    GIT_COMMITTER_EMAIL='autospec-claim-test@localhost' \
    git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" commit-tree "$tree" -p "$current" -F "$message")
  git --git-dir "$AUTOSPEC_CLAIM_GIT_REMOTE" update-ref "$reference" "$oid" "$current"
  rm -f "$message"
}
if [ "$1" = api ] && [ "$2" = graphql ]; then
  printf '%s\n' '{"items":[],"page_info":{"has_next_page":false,"end_cursor":null}}'
  exit 0
fi
if [ "$1" = repo ] && [ "$2" = view ]; then
  if [ "${AUTOSPEC_FOREGROUND_NO_DEFAULT_BRANCH:-0}" = 1 ]; then
    printf '\n'
  else
    printf '%s\n' main
  fi
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/main ]; then
  printf '%s\n' '{}'
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/master_ai ]; then
  printf '%s\n' '{}'
  exit 0
fi
if [ "$1" = api ] && [ "$2" = repos/test/repo/branches/missing ]; then
  exit 1
fi
if [ "$1" = api ] && { [ "$2" = repos/test/repo/commits/main/status ] || [ "$2" = repos/test/repo/commits/master_ai/status ]; }; then
  if [ "${AUTOSPEC_FOREGROUND_HEALTH_CASE:-success}" = ignored_failure ]; then
    printf '%s\n' '{"state":"failure","total_count":1,"statuses":[{"context":"Unit Tests","state":"failure"}]}'
  else
    printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'
  fi
  exit 0
fi
if [ "$1" = api ]; then
  endpoint=""
  for value in "$@"; do case "$value" in repos/*) endpoint="$value" ;; esac; done
  case "$endpoint" in
    repos/test/repo/issues\?*)
      case "$endpoint" in
        *labels=in-progress-by-bot*) printf '%s\n' '{"raw_count":0,"items":[]}' ;;
        *labels=auto-implement*)
          if [ "${AUTOSPEC_FOREGROUND_QUEUE_FAILURE:-0}" = 1 ]; then exit 1; fi
          if [ "${AUTOSPEC_FOREGROUND_CORRUPT_SPEND_AFTER_QUEUE:-0}" = 1 ]; then
            mkdir -p "$AUTOSPEC_AUTONOMOUS_SPEND_DIR/test_repo"
            printf '%s\n' '{malformed' > "$AUTOSPEC_AUTONOMOUS_SPEND_DIR/test_repo/spend.json"
          fi
          if [ "${AUTOSPEC_FOREGROUND_EMPTY_QUEUE:-0}" = 1 ]; then
            printf '%s\n' '{"raw_count":0,"items":[]}'
          else
            printf '%s' '{"raw_count":1,"items":['; issue; printf '%s\n' ']}'
          fi ;;
        *) printf '%s\n' '{"raw_count":0,"items":[]}' ;;
      esac
      exit 0 ;;
    repos/test/repo/issues/42/comments*)
      if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CONFIRM:-0}" = 1 ] && grep -q '<!-- autospec-executor-result:begin -->' "$AUTOSPEC_FOREGROUND_COMMENTS"; then
        exit 1
      fi
      cat "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
    repos/test/repo/pulls/17) printf '%s\n' '{}'; exit 0 ;;
    repos/test/repo/issues/17/comments*|repos/test/repo/pulls/17/reviews*|repos/test/repo/pulls/17/comments*)
      printf '%s\n' '[]'
      exit 0 ;;
    repos/test/repo/issues/42/labels) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; exit 0 ;;
    repos/test/repo/issues/42)
      if printf '%s\n' "$@" | grep -q PATCH; then printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE"; else issue; fi
      exit 0 ;;
    repos/test/repo/issues/comments/100)
      body=""
      for value in "$@"; do case "$value" in body=*) body="${value#body=}" ;; esac; done
      if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_pending; then
        steal_claim
      fi
      jq --arg body "$body" '.[0].body = $body | .[0].updated_at = "2026-07-15T00:00:00Z"' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
      mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
      exit 0 ;;
  esac
fi
if [ "$1" = issue ] && [ "$2" = view ]; then claim_issue "$@"; exit 0; fi
if [ "$1" = label ] && [ "$2" = create ]; then exit 0; fi
if [ "$1" = issue ] && [ "$2" = edit ]; then
  if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ]; then
    case " $* " in
      *" --remove-label in-progress-by-bot "*" --add-label auto-implement "*) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *" --remove-label in-progress-by-bot "*) printf 'terminal\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *" --add-label in-progress-by-bot "*) printf 'claimed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
    esac
    if [ -n "${FOREGROUND_STOP_ON_RETRYABLE_RELEASE:-}" ] && [ "$(cat "$AUTOSPEC_FOREGROUND_MODE")" = reviewed ]; then
      mkdir -p "$(dirname "$FOREGROUND_STOP_ON_RETRYABLE_RELEASE")"
      printf '%s\n' "${FOREGROUND_STOP_MODE_ON_RETRYABLE_RELEASE:-immediate}" '2026-07-31T00:00:00Z test@localhost' > "$FOREGROUND_STOP_ON_RETRYABLE_RELEASE"
    fi
  else
    last=""
    for value in "$@"; do last="$value"; done
    case "$last" in
      auto-implement) printf 'reviewed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
      *) printf 'claimed\n' > "$AUTOSPEC_FOREGROUND_MODE" ;;
    esac
  fi
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = comment ]; then
  body=""; shift 2
  while [ "$#" -gt 0 ]; do case "$1" in --body) body="$2"; shift 2 ;; *) shift ;; esac; done
  if [ "${AUTOSPEC_FOREGROUND_FAIL_EVIDENCE_CREATE:-0}" = 1 ] && printf '%s' "$body" | grep -q '<!-- autospec-executor-result:begin -->'; then
    exit 1
  fi
  if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_OUTCOME:-0}" = 1 ] && printf '%s' "$body" | grep -q executor_pending; then
    steal_claim
  fi
  jq --arg body "$body" '. + [{"id":((map(.id) | max // 99) + 1),"updated_at":"2026-07-15T00:00:00Z","body":$body}]' "$AUTOSPEC_FOREGROUND_COMMENTS" > "$AUTOSPEC_FOREGROUND_COMMENTS.tmp"
  mv "$AUTOSPEC_FOREGROUND_COMMENTS.tmp" "$AUTOSPEC_FOREGROUND_COMMENTS"
  exit 0
fi
if [ "$1" = pr ] && [ "$2" = list ]; then
  if [ "${AUTOSPEC_FOREGROUND_STEAL_ON_RESULT_VALIDATION:-0}" = 1 ]; then
    steal_claim
  fi
  cat "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = create ]; then
  head=$(git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" rev-parse refs/heads/feat/autonomous-issue-42)
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  body_file=""; previous=""
  for value in "$@"; do
    if [ "$previous" = --body-file ]; then body_file="$value"; fi
    previous="$value"
  done
  jq -n --rawfile body "$body_file" --arg head "$head" --arg base "$base" '[{"number":17,"body":$body,"headRefName":"feat/autonomous-issue-42","headRefOid":$head,"isDraft":true,"baseRefName":$base}]' > "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  printf '%s\n' 'https://example.invalid/test/repo/pull/17'
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = ready ]; then
  jq '.[0].isDraft = false' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS" > "$AUTOSPEC_FOREGROUND_PULL_REQUESTS.tmp"
  mv "$AUTOSPEC_FOREGROUND_PULL_REQUESTS.tmp" "$AUTOSPEC_FOREGROUND_PULL_REQUESTS"
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = view ]; then
  head=$(jq -r '.[0].headRefOid' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  body=$(jq -c '.[0].body' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  case " $* " in
    *" headRefOid,statusCheckRollup "*)
      printf '%s\n' "{\"headRefOid\":\"$head\",\"statusCheckRollup\":[{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}]}" ;;
    *)
      if [ -e "$AUTOSPEC_BRIDGE_MERGED" ]; then
        merge=$(cat "$AUTOSPEC_BRIDGE_MERGED")
        case " $* " in
          *" number,state,isDraft,headRefName,headRefOid,baseRefName,mergeCommit"*)
            printf '%s\n' "{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefName\":\"feat/autonomous-issue-42\",\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":{\"oid\":\"$merge\"},\"body\":$body}" ;;
          *)
            printf '%s\n' "{\"number\":17,\"state\":\"MERGED\",\"isDraft\":false,\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":{\"oid\":\"$merge\"},\"body\":$body}" ;;
        esac
      else
        case " $* " in
          *" number,state,isDraft,headRefName,headRefOid,baseRefName,mergeCommit"*)
            printf '%s\n' "{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefName\":\"feat/autonomous-issue-42\",\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":null,\"body\":$body}" ;;
          *)
            printf '%s\n' "{\"number\":17,\"state\":\"OPEN\",\"isDraft\":false,\"headRefOid\":\"$head\",\"baseRefName\":\"$base\",\"mergeCommit\":null,\"body\":$body}" ;;
        esac
      fi ;;
  esac
  exit 0
fi
if [ "${AUTOSPEC_FOREGROUND_REAL_BRIDGE:-0}" = 1 ] && [ "$1" = pr ] && [ "$2" = merge ]; then
  head=$(jq -r '.[0].headRefOid' "$AUTOSPEC_FOREGROUND_PULL_REQUESTS")
  base="${AUTOSPEC_BRIDGE_BASE_REF:-main}"
  case " $* " in *" --match-head-commit $head "*) ;; *) exit 74 ;; esac
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref "refs/heads/$base" "$head"
  git --git-dir "$AUTOSPEC_BRIDGE_REMOTE" update-ref -d refs/heads/feat/autonomous-issue-42
  printf '%s\n' "$head" > "$AUTOSPEC_BRIDGE_MERGED"
  exit 0
fi
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
"####,
        );
        Self {
            root,
            repo_dir,
            bin,
            mode,
            comments,
            pull_requests,
            calls,
            accountability,
            operator,
            state,
            health,
            heartbeats,
            claim_repo,
            claim_remote,
            claim_state,
        }
    }

    fn command(&self) -> Command {
        let mut command = self.configured_command();
        command.args([
            "autonomous",
            "run-foreground",
            "--repo",
            "test/repo",
            "--repo-dir",
            self.repo_dir.to_str().expect("repo path"),
            "--branch",
            "main",
        ]);
        command
    }

    fn configured_command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .current_dir(&self.repo_dir)
            .env("PATH", path_with(&self.bin))
            .env("AUTOSPEC_FOREGROUND_MODE", &self.mode)
            .env("AUTOSPEC_FOREGROUND_COMMENTS", &self.comments)
            .env("AUTOSPEC_FOREGROUND_PULL_REQUESTS", &self.pull_requests)
            .env("AUTOSPEC_FOREGROUND_CALLS", &self.calls)
            .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY", &self.accountability)
            .env(
                "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/foreground_accountability_gh.sh"
                ),
            )
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator)
            .env("AUTOSPEC_STATE_DIR", &self.state)
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", &self.health)
            .env("AUTOSPEC_HEARTBEAT_DIR", &self.heartbeats)
            .env("AUTOSPEC_CLAIM_GIT_REMOTE", &self.claim_remote)
            .env("AUTOSPEC_CLAIM_GIT_STATE_DIR", &self.claim_state)
            .env("AUTOSPEC_CLAIM_CONFIRM_READS", "1")
            .env("AUTOSPEC_CLAIM_SETTLE_MILLIS", "0")
            .env_remove("AUTOSPEC_CONFIG_FILE");
        command
    }

    fn initialize_git_remote(&self) {
        let remote = self.root.join("github.com/test/repo.git");
        fs::create_dir_all(remote.parent().expect("integration remote parent"))
            .expect("create integration remote parent");
        git_fixture(&self.root, &["init", "--bare", remote.to_str().unwrap()]);
        git_fixture(&self.repo_dir, &["init", "-b", "main"]);
        git_fixture(&self.repo_dir, &["config", "user.name", "Autospec Test"]);
        git_fixture(
            &self.repo_dir,
            &["config", "user.email", "autospec@example.invalid"],
        );
        fs::write(self.repo_dir.join("README.md"), "fixture\n").expect("write Git fixture");
        git_fixture(&self.repo_dir, &["add", "README.md"]);
        git_fixture(&self.repo_dir, &["commit", "-m", "fixture"]);
        git_fixture(
            &self.repo_dir,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git_fixture(&self.repo_dir, &["push", "-u", "origin", "main"]);
        git_fixture(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    }
}

static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_dir(prefix: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let sequence = TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{suffix}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create fixture directory");
    path
}

fn git_fixture(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make fake executable");
}

fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").expect("PATH is set")
    )
}
