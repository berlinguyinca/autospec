#!/usr/bin/env bats
# tests/repo-slug.bats — tests for scripts/repo-slug.sh
#
# Covers: canonical output, names containing '_', legacy read-compat,
#         resolve_slug_dir, and malformed-input rejection.
#
# Pure shell logic — no external tools needed (no gh stub required).

bats_require_minimum_version 1.5.0

SLUG_SCRIPT="${BATS_TEST_DIRNAME}/../scripts/repo-slug.sh"

# ── canonical_slug ────────────────────────────────────────────────────────────

@test "canonical_slug: normal owner/name → owner__name" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "berlinguyinca__autospec" ]
}

@test "canonical_slug: acceptance-criteria repo berlinguyinca/autospec" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "berlinguyinca__autospec" ]
}

@test "canonical_slug: name containing underscore is unambiguous" {
    # org/my_tool → org__my_tool (the __ separator cannot be confused with _)
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug org/my_tool"
    [ "$status" -eq 0 ]
    [ "$output" = "org__my_tool" ]
}

@test "canonical_slug: name containing hyphen passes through" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug my-org/my-repo"
    [ "$status" -eq 0 ]
    [ "$output" = "my-org__my-repo" ]
}

@test "canonical_slug: rejects input with no slash" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug nodomain"
    [ "$status" -ne 0 ]
}

@test "canonical_slug: rejects input with two slashes" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug a/b/c"
    [ "$status" -ne 0 ]
}

@test "canonical_slug: rejects empty input" {
    run bash -c "source '$SLUG_SCRIPT'; canonical_slug ''"
    [ "$status" -ne 0 ]
}

# ── repo-slug.sh as a standalone command ─────────────────────────────────────

@test "repo-slug.sh --canonical produces canonical slug" {
    run bash "$SLUG_SCRIPT" --canonical berlinguyinca/autospec
    [ "$status" -eq 0 ]
    [ "$output" = "berlinguyinca__autospec" ]
}

@test "repo-slug.sh without flag defaults to canonical" {
    run bash "$SLUG_SCRIPT" berlinguyinca/autospec
    [ "$status" -eq 0 ]
    [ "$output" = "berlinguyinca__autospec" ]
}

@test "repo-slug.sh exits non-zero on malformed input" {
    run bash "$SLUG_SCRIPT" "badslug"
    [ "$status" -ne 0 ]
}

# ── resolve_slug_dir (read-compat) ────────────────────────────────────────────

setup() {
    TEST_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TEST_DIR"
}

@test "resolve_slug_dir: canonical dir exists → returns canonical path" {
    mkdir -p "$TEST_DIR/berlinguyinca__autospec"
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/berlinguyinca__autospec" ]
}

@test "resolve_slug_dir: legacy underscore dir found when canonical absent" {
    mkdir -p "$TEST_DIR/berlinguyinca_autospec"
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/berlinguyinca_autospec" ]
}

@test "resolve_slug_dir: legacy hyphen dir found when canonical and _ absent" {
    mkdir -p "$TEST_DIR/berlinguyinca-autospec"
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/berlinguyinca-autospec" ]
}

@test "resolve_slug_dir: canonical takes precedence over legacy forms" {
    mkdir -p "$TEST_DIR/berlinguyinca__autospec"
    mkdir -p "$TEST_DIR/berlinguyinca_autospec"
    mkdir -p "$TEST_DIR/berlinguyinca-autospec"
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/berlinguyinca__autospec" ]
}

@test "resolve_slug_dir: returns canonical path even when no dir exists (new-repo path)" {
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' berlinguyinca/autospec"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/berlinguyinca__autospec" ]
}

@test "resolve_slug_dir: name with _ resolves to canonical __ form" {
    mkdir -p "$TEST_DIR/org__my_tool"
    run bash -c "source '$SLUG_SCRIPT'; resolve_slug_dir '$TEST_DIR' org/my_tool"
    [ "$status" -eq 0 ]
    [ "$output" = "$TEST_DIR/org__my_tool" ]
}
