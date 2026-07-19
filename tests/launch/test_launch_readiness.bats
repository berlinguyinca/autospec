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
  printf '# Roadmap\n\nCanonical roadmap details live in [docs/roadmap.md](docs/roadmap.md).\n' > "$TMP_REPO/ROADMAP.md"
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

@test "launch readiness validator rejects divergent root roadmap content" {
  copy_required_launch_files
  printf '# Roadmap\n\nThis duplicate roadmap will drift from docs/roadmap.md.\n' > "$TMP_REPO/ROADMAP.md"

  run bash "$TMP_REPO/scripts/validate-launch-readiness.sh"

  [ "$status" -ne 0 ]
  [[ "$output" == *"ROADMAP.md must delegate to docs/roadmap.md"* ]]
}

copy_public_launch_files() {
  copy_required_launch_files

  mkdir -p "$TMP_REPO/.autospec/releases" \
    "$TMP_REPO/.autospec/reports" \
    "$TMP_REPO/.autospec/handoff" \
    "$TMP_REPO/docs/assets"

  printf 'AUTOSPEC_PUBLIC_LAUNCH_READY=true\n' > "$TMP_REPO/.autospec/releases/launch-candidate.md"
  printf 'final launch readiness\n' > "$TMP_REPO/.autospec/reports/final-launch-readiness.md"
  printf 'V74 Final Release Candidate\nAUTOSPEC_PUBLIC_LAUNCH_READY=true\n' > "$TMP_REPO/.autospec/releases/final-release-candidate.md"
  printf 'Final Platform Evidence\nautospec validate\n' > "$TMP_REPO/.autospec/reports/final-platform-evidence.md"
  printf 'handoff\n' > "$TMP_REPO/.autospec/handoff/codex-final-handoff.md"
  printf 'release checklist\n' > "$TMP_REPO/docs/release-checklist.md"
  printf 'public launch checklist\n' > "$TMP_REPO/docs/public-launch-checklist.md"
  printf 'good first issues\n' > "$TMP_REPO/docs/good-first-issues.md"
  printf 'screenshots placeholder\n' > "$TMP_REPO/docs/assets/screenshots-placeholder.md"
  printf 'social preview placeholder\n' > "$TMP_REPO/docs/assets/social-preview-placeholder.md"
  printf 'Comparison\nCurrent Maturity And Limitations\nbash scripts/demo-recording.sh\nquickstart\n' > "$TMP_REPO/README.md"

  for gate in validate-v25-baseline.sh validate-v60-release.sh validate-launch-readiness.sh; do
    cat > "$TMP_REPO/scripts/$gate" <<'EOF'
#!/usr/bin/env bash
set -eu
printf 'stub gate passed\n'
EOF
    chmod +x "$TMP_REPO/scripts/$gate"
  done
}

@test "public launch validator requires V74 final release candidate artifacts" {
  copy_public_launch_files
  rm "$TMP_REPO/.autospec/releases/final-release-candidate.md"

  run bash "$TMP_REPO/scripts/validate-public-launch-readiness.sh"

  [ "$status" -ne 0 ]
  [[ "$output" == *"missing .autospec/releases/final-release-candidate.md"* ]]
}

@test "public launch validator rejects stale failing QA verdict without supersession marker" {
  copy_public_launch_files
  cat > "$TMP_REPO/.autospec/qa-verdict.json" <<'EOF'
{
  "verdict": "FAIL",
  "head_sha": "older-sha"
}
EOF

  run bash "$TMP_REPO/scripts/validate-public-launch-readiness.sh"

  [ "$status" -ne 0 ]
  [[ "$output" == *"stale failing .autospec/qa-verdict.json lacks historical supersession marker"* ]]
}

@test "public launch validator accepts stale failing QA verdict when marked historical" {
  copy_public_launch_files
  cat > "$TMP_REPO/.autospec/qa-verdict.json" <<'EOF'
{
  "verdict": "FAIL",
  "head_sha": "older-sha",
  "evidence_status": "historical_stale_not_current_launch_evidence",
  "superseded_by": [
    "autospec validate --fast",
    "bash scripts/validate-public-launch-readiness.sh"
  ]
}
EOF

  run bash "$TMP_REPO/scripts/validate-public-launch-readiness.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"AUTOSPEC_PUBLIC_LAUNCH_READY=true"* ]]
}
