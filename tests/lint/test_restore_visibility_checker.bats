#!/usr/bin/env bats
# tests/lint/test_restore_visibility_checker.bats
#
# Self-enforcement for scripts/lint-restore-visibility.sh (issue #3878).
#
# Restoring a file is not the same as making the restoration visible to an
# mtime-based build system: `mv` (and `cp -p`, `install -p`, `tar -x`,
# `git stash pop`) put a file back with the backup's timestamp, so a rebuild
# that compares mtimes silently declines to run and the stale artefact is
# wrong rather than merely old. The checker makes that hazard blocking; these
# tests pin the detector's behavior and replay the stale-rebuild failure
# against a populated negative case to prove the artefact assertion fires when
# the rebuild is suppressed.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd -P)"
    CHECKER="${REPO_ROOT}/scripts/lint-restore-visibility.sh"
    TMP_ROOT="$(mktemp -d)"
    mkdir -p "${TMP_ROOT}/tests"
}

teardown() {
    rm -rf "${TMP_ROOT}"
}

# ── fixture harnesses ─────────────────────────────────────────────────────────
# Fixture bodies are assembled with printf, not heredocs, so no line in this
# file can be mistaken for one by a naive line scanner.

write_mv_restore_no_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'cp "$MANIFEST" "$MANIFEST.bak"' \
        'jq ".pin" "$MANIFEST" > "$MANIFEST"' \
        'cargo build' \
        'mv -f "$MANIFEST.bak" "$MANIFEST"' \
        'cargo build' > "$1"
}

write_mv_restore_with_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'cp "$MANIFEST" "$MANIFEST.bak"' \
        'jq ".pin" "$MANIFEST" > "$MANIFEST"' \
        'cargo build' \
        'mv -f "$MANIFEST.bak" "$MANIFEST"' \
        'touch "$MANIFEST"' \
        'cargo build' > "$1"
}

write_content_rewrite_restore() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'saved="$(cat "$MANIFEST")"' \
        'printf "%s" "$BAD" > "$MANIFEST"' \
        'printf "%s" "$saved" > "$MANIFEST"' \
        'cargo build' > "$1"
}

write_rewrite_after_mv() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'cp "$MANIFEST" "$MANIFEST.bak"' \
        'printf "%s" "$BAD" > "$MANIFEST"' \
        'mv -f "$MANIFEST.bak" "$MANIFEST"' \
        'printf "%s" "$REAL" > "$MANIFEST"' \
        'cargo build' > "$1"
}

write_cp_p_restore_no_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'cp "$f" "$f.bak"' \
        'corrupt "$f"' \
        'cp -p "$f.bak" "$f"' \
        'cargo build' > "$1"
}

write_plain_cp_restore() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'cp "$f" "$f.bak"' \
        'corrupt "$f"' \
        'cp "$f.bak" "$f"' \
        'cargo build' > "$1"
}

write_tar_extract_no_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'tar -czf backup.tar.gz src/' \
        'corrupt src/main.c' \
        'tar -xzf backup.tar.gz -C .' \
        'make' > "$1"
}

write_tar_extract_with_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'tar -czf backup.tar.gz src/' \
        'corrupt src/main.c' \
        'tar -xzf backup.tar.gz -C .' \
        'touch src/main.c' \
        'make' > "$1"
}

write_stash_pop_no_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'git stash' \
        'git stash pop' \
        'cargo build' > "$1"
}

write_stash_pop_with_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -eu' \
        'git stash' \
        'git stash pop' \
        'touch src/main.rs' \
        'cargo build' > "$1"
}

write_inline_trap_no_touch() {
    printf '%s\n' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'cp "$MANIFEST" "$MANIFEST.bak"' \
        "trap 'mv -f \"\$MANIFEST.bak\" \"\$MANIFEST\"' EXIT" > "$1"
}

write_inline_trap_with_touch() {
    printf '%s\n' \
        'set -eu' \
        'MANIFEST=/tmp/manifest' \
        'cp "$MANIFEST" "$MANIFEST.bak"' \
        "trap 'mv -f \"\$MANIFEST.bak\" \"\$MANIFEST\"; touch \"\$MANIFEST\"' EXIT" > "$1"
}

write_trap_function_main_path_touch_only() {
    printf '%s\n' \
        'set -eu' \
        'restore() {' \
        '  mv -f "$f.bak" "$f"' \
        '}' \
        'trap restore EXIT' \
        'corrupt_and_test' \
        'touch "$f"' > "$1"
}

write_trap_function_touch_inside() {
    printf '%s\n' \
        'set -eu' \
        'restore() {' \
        '  mv -f "$f.bak" "$f"' \
        '  touch "$f"' \
        '}' \
        'trap restore EXIT' \
        'corrupt_and_test' > "$1"
}

write_allowlisted_site() {
    printf '%s\n' \
        'set -eu' \
        'corrupt src/main.c' \
        '# restore-visibility:allow legacy tarball ships stored mtimes by design' \
        'tar -xzf backup.tar.gz -C .' \
        'make' > "$1"
}

write_allowlisted_site_without_reason() {
    printf '%s\n' \
        'set -eu' \
        '# restore-visibility:allow' \
        'tar -xzf backup.tar.gz -C .' > "$1"
}

write_bats_run_restore_no_touch() {
    printf '%s\n' \
        '#!/usr/bin/env bats' \
        'set -eu' \
        '@''test "restore the fixture" {' \
        '  cp "$f" "$f.bak"' \
        '  run mv -f "$f.bak" "$f"' \
        '  [ "$status" -eq 0 ]' \
        '}' > "$1"
}

write_heredoc_payload() {
    printf '%s\n' \
        'set -eu' \
        'cat > out.txt <<"PAYLOAD"' \
        'mv -f "$f.bak" "$f"' \
        'PAYLOAD' \
        '[ -s out.txt ]' > "$1"
}

# ── detector behavior ─────────────────────────────────────────────────────────

@test "the shipped repository scan is green" {
    run bash "${CHECKER}" --root "${REPO_ROOT}"
    [ "$status" -eq 0 ]
    [[ "$output" == *"OK"* ]]
}

@test "--help prints the usage and exits 0" {
    run bash "${CHECKER}" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"STALE_RESTORE"* ]]
    [[ "$output" == *"--root"* ]]
}

@test "an mv restore without a later touch is a blocking finding" {
    write_mv_restore_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:7"* ]]
    [[ "$output" == *'restore of "$MANIFEST" via mv'* ]]
}

@test "an mv restore followed by a touch is not a site" {
    write_mv_restore_with_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "restoring by rewriting content is not a site" {
    write_content_rewrite_restore "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a content rewrite after an mv restore observes the restoration" {
    write_rewrite_after_mv "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a cp -p restore without a later touch is a blocking finding" {
    write_cp_p_restore_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:5"* ]]
    [[ "$output" == *"via cp"* ]]
}

@test "a plain cp restore (fresh mtime) is not a site" {
    write_plain_cp_restore "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a tar extract without a later touch is a blocking finding" {
    write_tar_extract_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:5"* ]]
}

@test "a tar extract followed by a touch is not a site" {
    write_tar_extract_with_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a git stash pop without a later touch is a blocking finding" {
    write_stash_pop_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:4"* ]]
}

@test "a git stash pop followed by a touch is not a site" {
    write_stash_pop_with_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "an inline trap restore without a touch inside the body is a finding" {
    write_inline_trap_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"inline trap restore"* ]]
}

@test "an inline trap restore with a touch inside the body is not a site" {
    write_inline_trap_with_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a main-path touch does not satisfy a restore in a trap-referenced function" {
    write_trap_function_main_path_touch_only "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:3"* ]]
}

@test "a touch inside the trap-referenced function satisfies its restore" {
    write_trap_function_touch_inside "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a restore behind a bats run keyword is still a site" {
    write_bats_run_restore_no_touch "${TMP_ROOT}/tests/restore.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.bats"* ]]
}

@test "a heredoc payload that mentions a restore is data, not a site" {
    write_heredoc_payload "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "an allow marker with a reason suppresses the site" {
    write_allowlisted_site "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "an allow marker without a reason does not suppress the site" {
    write_allowlisted_site_without_reason "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 1 ]
}

@test "--list prints every site and exits 0" {
    write_mv_restore_no_touch "${TMP_ROOT}/tests/restore.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}" --list
    [ "$status" -eq 0 ]
    [[ "$output" == *"STALE_RESTORE:tests/restore.sh:7"* ]]
}

# ── the hazard itself, replayed (acceptance criterion 3) ─────────────────────

# A fake mtime-based build system with the include_str! shape: the artefact
# embeds the input's value and is only rebuilt when the input is strictly
# newer than the artefact. This is exactly the staleness rule that let a
# restored manifest outlive the binary built from its corrupted copy.

wait_newer() {
    local a="$1" b="$2" i=0
    while [ "$i" -lt 50 ]; do
        if find "$a" -newer "$b" 2>/dev/null | grep -q .; then
            return 0
        fi
        i=$((i + 1))
        sleep 0.1
    done
    find "$a" -newer "$b" 2>/dev/null | grep -q .
}

@test "the artefact assertion fires when the post-restore rebuild is suppressed" {
    local W INPUT ART REAL BAD i
    W="$(mktemp -d)"
    INPUT="${W}/manifest"
    ART="${W}/gateway-bin"
    REAL="sha256:deadbeefcafe"
    BAD="sha256:0000000000000000000000000000000000000000000000000000000000000000"

    touch -t 202001010000.00 "$ART"
    build() {
        if find "$INPUT" -newer "$ART" 2>/dev/null | grep -q .; then
            printf 'image %s\n' "$(cut -d= -f2 "$INPUT")" > "$ART"
        fi
    }

    # fresh build carries the real pin
    printf 'pin=%s\n' "$REAL" > "$INPUT"
    build
    run grep -q "$REAL" "$ART"
    [ "$status" -eq 0 ]

    # negative case: corrupt the input and prove the gate can fail
    cp "$INPUT" "$INPUT.bak"
    printf 'pin=%s\n' "$BAD" > "$INPUT"
    touch -t 202001010000.00 "$ART"
    build
    run grep -q "$BAD" "$ART"
    [ "$status" -eq 0 ]

    # restore via mv: the backup's mtime is older than the artefact, so the
    # mtime-based rebuild has nothing to do
    mv -f "$INPUT.bak" "$INPUT"
    build

    # the assertion the fix adds: the artefact still carries the corrupted pin
    run grep -q "$BAD" "$ART"
    [ "$status" -eq 0 ]
    run grep -q "$REAL" "$ART"
    [ "$status" -ne 0 ]

    # make the restoration visible and the rebuild lands the real pin
    touch "$INPUT"
    wait_newer "$INPUT" "$ART"
    build
    run grep -q "$REAL" "$ART"
    [ "$status" -eq 0 ]
    run grep -q "$BAD" "$ART"
    [ "$status" -ne 0 ]
    rm -rf "$W"
}

@test "the restored file is observably older than the artefact after an mv restore" {
    local W INPUT ART
    W="$(mktemp -d)"
    INPUT="${W}/manifest"
    ART="${W}/gateway-bin"

    touch -t 202001010000.00 "$ART"
    build() {
        if find "$INPUT" -newer "$ART" 2>/dev/null | grep -q .; then
            printf 'value=%s\n' "$(cat "$INPUT")" > "$ART"
        fi
    }

    printf 'value=real\n' > "$INPUT"
    build
    cp "$INPUT" "$INPUT.bak"
    printf 'value=bad\n' > "$INPUT"
    touch -t 202001010000.00 "$ART"
    build

    mv -f "$INPUT.bak" "$INPUT"
    # the restored input must NOT be newer than the artefact: that is the
    # condition under which the rebuild is (correctly, silently) skipped
    run bash -c "find '$INPUT' -newer '$ART' | grep -q ."
    [ "$status" -ne 0 ]

    touch "$INPUT"
    wait_newer "$INPUT" "$ART"
    rm -rf "$W"
}
