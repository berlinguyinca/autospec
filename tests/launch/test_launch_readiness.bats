#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TMP_REPO="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$TMP_REPO"
  cp -R "$REPO_ROOT/scripts" "$TMP_REPO/scripts"
}

copy_required_launch_files() {
  local rel
  for rel in \
    README.md \
    docs/index.md \
    docs/quickstart.md \
    docs/concepts.md \
    docs/architecture.md \
    docs/workflows.md \
    docs/faq.md \
    docs/roadmap.md \
    CONTRIBUTING.md \
    SECURITY.md \
    SAFETY.md \
    CHANGELOG.md \
    ROADMAP.md \
    .github/ISSUE_TEMPLATE/bug_report.md \
    .github/ISSUE_TEMPLATE/feature_request.md \
    .github/ISSUE_TEMPLATE/docs_feedback.md \
    .github/pull_request_template.md \
    examples/hello-autospec/README.md \
    examples/hello-autospec/spec.md \
    examples/hello-autospec/sample-issue.md \
    examples/hello-autospec/expected-closeout.md \
    scripts/demo-recording.sh \
    docs/assets/architecture.mmd \
    docs/assets/demo-placeholder.md \
    marketing/launch-post-github.md \
    marketing/launch-post-reddit.md \
    marketing/launch-post-hackernews.md \
    marketing/launch-post-linkedin.md \
    marketing/launch-post-x.md \
    marketing/demo-video-script.md \
    marketing/faq-for-comments.md
  do
    mkdir -p "$TMP_REPO/$(dirname "$rel")"
    printf 'placeholder quickstart launch readiness\n' > "$TMP_REPO/$rel"
  done
}

@test "launch readiness validator prints success marker when required artifacts exist" {
  copy_required_launch_files

  run bash "$TMP_REPO/scripts/validate-launch-readiness.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"AUTOSPEC_V61_LAUNCH_READY=true"* ]]
}

@test "launch readiness validator fails when README lacks quickstart" {
  copy_required_launch_files
  printf 'AutoSpec launch docs without the required section\n' > "$TMP_REPO/README.md"

  run bash "$TMP_REPO/scripts/validate-launch-readiness.sh"

  [ "$status" -ne 0 ]
  [[ "$output" == *"README.md missing quickstart"* ]]
}
