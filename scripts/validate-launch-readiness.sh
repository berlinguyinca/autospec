#!/usr/bin/env bash
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

failures=0

fail() {
  failures=$((failures + 1))
  printf 'launch-readiness: FAIL: %s\n' "$*"
}

require_file() {
  local path="$1"
  [ -f "$path" ] || fail "missing $path"
}

require_readme_quickstart() {
  if [ ! -f README.md ]; then
    fail "missing README.md"
    return
  fi
  if ! grep -qi 'quickstart' README.md; then
    fail "README.md missing quickstart"
  fi
}

require_root_roadmap_delegates() {
  if [ ! -f ROADMAP.md ]; then
    fail "missing ROADMAP.md"
    return
  fi
  if ! grep -Eq '\[`?docs/roadmap\.md`?\]\(docs/roadmap\.md\)' ROADMAP.md; then
    fail "ROADMAP.md must delegate to docs/roadmap.md"
  fi
  if grep -Eq '^## (Near Term|Medium Term|After V74|Later)' ROADMAP.md; then
    fail "ROADMAP.md must not duplicate roadmap sections from docs/roadmap.md"
  fi
}

require_group() {
  local group="$1"
  shift
  local path
  for path in "$@"; do
    require_file "$path"
  done
  printf 'launch-readiness: checked %s\n' "$group"
}

require_readme_quickstart
require_root_roadmap_delegates

require_group "docs" \
  docs/index.md \
  docs/quickstart.md \
  docs/concepts.md \
  docs/architecture.md \
  docs/workflows.md \
  docs/faq.md \
  docs/roadmap.md

require_group "community files" \
  CONTRIBUTING.md \
  SECURITY.md \
  SAFETY.md \
  CHANGELOG.md \
  ROADMAP.md \
  .github/ISSUE_TEMPLATE/bug_report.md \
  .github/ISSUE_TEMPLATE/feature_request.md \
  .github/ISSUE_TEMPLATE/docs_feedback.md \
  .github/pull_request_template.md

require_group "demo materials" \
  examples/hello-autospec/README.md \
  examples/hello-autospec/spec.md \
  examples/hello-autospec/sample-issue.md \
  examples/hello-autospec/expected-closeout.md \
  scripts/demo-recording.sh \
  docs/assets/architecture.mmd \
  docs/assets/demo-placeholder.md

require_group "launch kit" \
  marketing/launch-post-github.md \
  marketing/launch-post-reddit.md \
  marketing/launch-post-hackernews.md \
  marketing/launch-post-linkedin.md \
  marketing/launch-post-x.md \
  marketing/demo-video-script.md \
  marketing/faq-for-comments.md

require_group "roadmap and changelog" \
  ROADMAP.md \
  CHANGELOG.md \
  docs/roadmap.md

require_group "security and safety docs" \
  SECURITY.md \
  SAFETY.md

if [ "$failures" -ne 0 ]; then
  printf 'launch-readiness: %s failure(s)\n' "$failures"
  exit 1
fi

printf 'AUTOSPEC_V61_LAUNCH_READY=true\n'
