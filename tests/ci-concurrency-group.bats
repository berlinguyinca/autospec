#!/usr/bin/env bats
# tests/ci-concurrency-group.bats — issue #4199: enumerate the CI
# concurrency-group defect population.
#
# The defect class: a shared concurrency group + cancel-in-progress on a
# push-triggered workflow silently cancels the previous merge's verification
# run. The detector must parse each workflow as YAML — the fixed per-commit
# group in rust.yml carries 8-12 comment lines between `concurrency:` and
# `group:`, so a 3-line grep context window never reaches it, and a detector
# that under-matches reports the population as clean.
# (Suite lives at the tests/ root, so no catalog registration is needed.)

# Use BATS_TEST_TMPDIR, not mktemp+trap: an EXIT trap installed in setup()
# overwrites bats' own EXIT trap, which is what emits the `not ok` TAP line —
# a failing test then vanishes from the output instead of being reported.
setup() {
    WORKFLOWS="$BATS_TEST_TMPDIR/workflows"
    mkdir -p "$WORKFLOWS"
    SCRIPT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)/scripts/check-ci-concurrency-group.sh"
}

# rust.yml shape: per-commit group. The ten comment lines between
# `concurrency:` and `group:` are the regression: a `grep -A3` detector
# never reaches `group:` and reports this workflow as having no
# concurrency block at all.
write_per_commit_workflow() {
    cat >"$WORKFLOWS/rust.yml" <<'EOF'
name: rust-suites

on:
  pull_request:
  push:
    branches: [main]

concurrency:
  # Main gets a per-commit group so a run can never be cancelled by the next
  # merge; branches keep the shared group where a superseded run is worthless.
  #
  # The previous attempt used `cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}`
  # and did NOT work: six consecutive commits on main all carried that expression
  # and all six runs were still cancelled. Expressions are reliable in `group`,
  # so the distinction is expressed there instead of in the boolean.
  group: rust-suites-${{ github.ref }}-${{ github.ref == 'refs/heads/main' && github.sha || 'shared' }}
  cancel-in-progress: true

jobs:
  build-test:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
EOF
}

# InferWeave/inferweave ci.yml shape: the defect — shared group,
# cancel-in-progress, and a push trigger.
write_defect_workflow() {
    cat >"$WORKFLOWS/ci.yml" <<'EOF'
name: ci

on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
EOF
}

# autospec-doc-drift.yml shape: shared + cancel but pull-request-only —
# a superseded PR run genuinely is worthless, which is the case the shared
# group exists for. Correct by design, not a defect.
write_pr_only_workflow() {
    cat >"$WORKFLOWS/autospec-doc-drift.yml" <<'EOF'
name: autospec-doc-drift

on:
  pull_request:

concurrency:
  group: autospec-doc-drift-${{ github.ref }}
  cancel-in-progress: true

jobs:
  doc-drift:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
EOF
}

# pages.yml shape: shared group, cancel-in-progress false.
write_shared_no_cancel_workflow() {
    cat >"$WORKFLOWS/pages.yml" <<'EOF'
name: Deploy GitHub Pages

on:
  push:
    branches:
      - main

concurrency:
  group: "pages"
  cancel-in-progress: false

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
EOF
}

write_no_concurrency_workflow() {
    cat >"$WORKFLOWS/release-cli.yml" <<'EOF'
name: Release CLI

on:
  push:
    tags:
      - 'v*'

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
EOF
}

write_unparseable_workflow() {
    printf 'concurrency:\n  group: [unclosed\n  cancel-in-progress: true\n' >"$WORKFLOWS/broken.yml"
}

@test "clean population: per-commit, PR-only, shared-no-cancel and none all pass (exit 0)" {
    write_per_commit_workflow
    write_pr_only_workflow
    write_shared_no_cancel_workflow
    write_no_concurrency_workflow
    run bash "$SCRIPT" "$WORKFLOWS"
    [ "$status" -eq 0 ]
    [[ "$output" == *"rust.yml: class=per-commit cancel=yes push=yes verdict=ok"* ]]
    [[ "$output" == *"autospec-doc-drift.yml: class=shared cancel=yes push=no verdict=ok"* ]]
    [[ "$output" == *"pages.yml: class=shared cancel=no push=yes verdict=ok"* ]]
    [[ "$output" == *"release-cli.yml: class=none cancel=absent push=yes verdict=na"* ]]
}

@test "shared group + cancel-in-progress + push trigger is a DEFECT-CANDIDATE (exit 1)" {
    write_per_commit_workflow
    write_defect_workflow
    run bash "$SCRIPT" "$WORKFLOWS"
    [ "$status" -eq 1 ]
    [[ "$output" == *"ci.yml: class=shared cancel=yes push=yes verdict=DEFECT-CANDIDATE"* ]]
    # the per-commit workflow in the same population still classifies ok
    [[ "$output" == *"rust.yml: class=per-commit cancel=yes push=yes verdict=ok"* ]]
}

@test "comment lines between concurrency: and group: do not hide the per-commit group" {
    # The regression from issue #4199: the fixed rust.yml carries 8-12 comment
    # lines in that gap; a 3-line grep window reports it as no concurrency.
    write_per_commit_workflow
    run bash "$SCRIPT" "$WORKFLOWS" --expect rust.yml=True
    [ "$status" -eq 0 ]
    [[ "$output" == *"rust.yml: class=per-commit"* ]]
    [[ "$output" == *"rust.yml: per-commit=True expected=True -> OK"* ]]
}

@test "known-answer assertions fail when the parsed population contradicts them" {
    write_defect_workflow
    run bash "$SCRIPT" "$WORKFLOWS" --expect ci.yml=True
    [ "$status" -eq 1 ]
    [[ "$output" == *"ci.yml: per-commit=False expected=True -> FAIL"* ]]
}

@test "a missing or unparseable workflow fails an assertion; it is never read as clean" {
    run bash "$SCRIPT" "$WORKFLOWS" --expect ghost.yml=True
    [ "$status" -eq 1 ]
    [[ "$output" == *"ghost.yml: per-commit=? expected=True -> FAIL"* ]]

    write_unparseable_workflow
    run bash "$SCRIPT" "$WORKFLOWS"
    [ "$status" -eq 1 ]
    [[ "$output" == *"broken.yml: class=unparseable"* ]]
    [[ "$output" == *"verdict=ERROR"* ]]

    run bash "$SCRIPT" "$WORKFLOWS" --expect broken.yml=True
    [ "$status" -eq 1 ]
    [[ "$output" == *"broken.yml: per-commit=? expected=True -> FAIL (workflow could not be parsed)"* ]]
}

@test "--list is an audit: it emits the sweep and always exits 0" {
    write_defect_workflow
    write_unparseable_workflow
    run bash "$SCRIPT" "$WORKFLOWS" --list
    [ "$status" -eq 0 ]
    [[ "$output" == *"ci.yml: class=shared cancel=yes push=yes verdict=DEFECT-CANDIDATE"* ]]
    [[ "$output" == *"broken.yml: class=unparseable"* ]]
}

@test "missing directory and unknown option are usage errors (exit 2)" {
    run bash "$SCRIPT" "$BATS_TEST_TMPDIR/no-such-dir"
    [ "$status" -eq 2 ]
    [[ "$output" == *"workflow directory not found"* ]]

    run bash "$SCRIPT" --bogus
    [ "$status" -eq 2 ]
    [[ "$output" == *"unknown option"* ]]

    run bash "$SCRIPT" --expect rust.yml
    [ "$status" -eq 2 ]
    [[ "$output" == *"--expect value must be REL=True|False"* ]]
}
