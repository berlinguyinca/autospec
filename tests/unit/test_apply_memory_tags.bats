#!/usr/bin/env bats
# tests/unit/test_apply_memory_tags.bats — Exercises scripts/apply-memory-tags.sh
# Tests: fresh-apply, re-apply (idempotency), missing-file (skip with warn)

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/apply-memory-tags.sh"
    MANIFEST="$REPO_ROOT/scripts/memory-tags.yml"

    # Create a temp memory dir for each test
    TMPDIR_MEM="$(mktemp -d)"
    # Create a minimal test manifest
    TMPDIR_MANIFEST="$(mktemp)"
}

teardown() {
    rm -rf "$TMPDIR_MEM"
    rm -f "$TMPDIR_MANIFEST"
}

# ── syntax check ─────────────────────────────────────────────────────────────

@test "apply-memory-tags: bash -n exits 0" {
    run bash -n "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "apply-memory-tags: --help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

# ── fresh apply ──────────────────────────────────────────────────────────────

@test "apply-memory-tags: inserts tags into untagged file" {
    # Create stub memory file with existing frontmatter (no tags:)
    cat > "$TMPDIR_MEM/feedback_test_stub.md" <<'EOF'
---
name: Test stub
description: A test memory file
---
Body content here.
EOF

    # Create minimal manifest
    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_test_stub.md:
  tags: [bash, testing, autospec-run]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]

    # Verify tags: line was added
    grep -q "^tags: \[bash, testing, autospec-run\]" "$TMPDIR_MEM/feedback_test_stub.md"
}

@test "apply-memory-tags: prints OK summary line for processed file" {
    cat > "$TMPDIR_MEM/feedback_test_stub.md" <<'EOF'
---
name: Test stub
---
Body content.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_test_stub.md:
  tags: [bash, testing]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OK.*feedback_test_stub.md"
}

@test "apply-memory-tags: prints summary counts" {
    cat > "$TMPDIR_MEM/feedback_test_stub.md" <<'EOF'
---
name: Test stub
---
Body content.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_test_stub.md:
  tags: [bash, testing]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "Summary:"
    echo "$output" | grep -q "processed"
}

# ── idempotency ───────────────────────────────────────────────────────────────

@test "apply-memory-tags: re-run on already-tagged file is no-op (exit 0)" {
    # Create file with tags: already in frontmatter
    cat > "$TMPDIR_MEM/feedback_already_tagged.md" <<'EOF'
---
name: Already tagged
tags: [bash, existing]
---
Body content here.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_already_tagged.md:
  tags: [bash, existing]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "SKIP.*feedback_already_tagged.md"
}

@test "apply-memory-tags: re-run does not duplicate frontmatter" {
    cat > "$TMPDIR_MEM/feedback_test_stub.md" <<'EOF'
---
name: Test stub
description: Test
---
Body content.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_test_stub.md:
  tags: [bash, testing, autospec-run]
EOF

    # First run
    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]

    # Second run
    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]

    # Count occurrences of 'tags:' — should be exactly 1
    count=$(grep -c "^tags:" "$TMPDIR_MEM/feedback_test_stub.md")
    [ "$count" -eq 1 ]
}

# ── missing file warning ──────────────────────────────────────────────────────

@test "apply-memory-tags: missing file in manifest warns to stderr and exits 0" {
    # manifest references a file that does NOT exist in memory dir
    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_does_not_exist.md:
  tags: [bash, missing]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    echo "$stderr" | grep -q "not found" || echo "$output" | grep -qi "warn"
}

@test "apply-memory-tags: missing file does not prevent processing other files" {
    # Create one valid and one missing file in manifest
    cat > "$TMPDIR_MEM/feedback_valid.md" <<'EOF'
---
name: Valid file
---
Body.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_does_not_exist.md:
  tags: [bash, missing]

feedback_valid.md:
  tags: [autospec-run, testing]
EOF

    run bash "$SCRIPT" --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    # Valid file was still processed
    grep -q "^tags:" "$TMPDIR_MEM/feedback_valid.md"
}

# ── dry-run mode ─────────────────────────────────────────────────────────────

@test "apply-memory-tags: --dry-run does not write files" {
    cat > "$TMPDIR_MEM/feedback_dryrun_test.md" <<'EOF'
---
name: Dry run test
---
Body content.
EOF

    cat > "$TMPDIR_MANIFEST" <<'EOF'
feedback_dryrun_test.md:
  tags: [bash, dry-run]
EOF

    run bash "$SCRIPT" --dry-run --memory-dir "$TMPDIR_MEM" --manifest "$TMPDIR_MANIFEST"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "DRY RUN\|PLAN"

    # File should NOT have tags: added
    run grep -c "^tags:" "$TMPDIR_MEM/feedback_dryrun_test.md"
    [ "$output" -eq 0 ]
}

# ── real manifest check ───────────────────────────────────────────────────────

@test "apply-memory-tags: real manifest has entries for all 14 feedback files" {
    count=$(grep -c "^feedback_.*\.md:$" "$MANIFEST")
    [ "$count" -eq 14 ]
}
