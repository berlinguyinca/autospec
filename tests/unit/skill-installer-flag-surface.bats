#!/usr/bin/env bats
# tests/unit/skill-installer-flag-surface.bats — every per-skill installer must accept the
# flags the top-level install.sh passes through.
#
# install.sh appends --update (and --dry-run) to `bash skills/<skill>/install.sh` for every
# selected skill x harness pair. An installer whose argument parser lacks the flag exits 2 on
# "unknown argument", so a single drifted installer fails all three of its harness pairs and
# makes `install.sh --update` exit non-zero. Four installers had drifted this way
# (autospec-monitor, autospec-quality, autospec-rollover-status, autospec-test), which is 12
# failed pairs on every update run.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "install.sh passes --update through to every per-skill installer" {
    run grep -q -- '--update' "$REPO_ROOT/install.sh"
    [ "$status" -eq 0 ]
}

@test "every per-skill installer accepts --dry-run and --update" {
    rejected=""
    for installer in "$REPO_ROOT"/skills/*/install.sh; do
        skill="$(basename "$(dirname "$installer")")"
        if ! bash "$installer" --harness claude --dry-run --update >/dev/null 2>&1; then
            rejected="$rejected $skill"
        fi
    done
    [ -z "$rejected" ] || printf 'installers rejecting --dry-run --update:%s\n' "$rejected" >&2
    [ -z "$rejected" ]
}

@test "a skill installer that rejects --update is caught" {
    # Guards the check above against silently passing: an installer missing the flag must fail.
    probe="$BATS_TEST_TMPDIR/probe-install.sh"
    cat > "$probe" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
while [ $# -gt 0 ]; do
    case "$1" in
        --harness) shift ;;
        --dry-run) ;;
        *) printf 'error: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
    shift
done
SH
    run bash "$probe" --harness claude --dry-run --update
    [ "$status" -eq 2 ]
}
