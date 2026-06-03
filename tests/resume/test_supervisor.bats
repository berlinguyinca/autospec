#!/usr/bin/env bats
# tests/resume/test_supervisor.bats — external boot supervisor (issue #882,
# child 2 of docs/specs/2026-06-03-crash-resume-design.md §Supervisor safety).
#
# Covers:
#   - supervisor_thrash: no open run in registry -> supervisor exits 0 and the
#     relaunch (/autospec-resume) path is NOT called.
#   - supervisor_two_repo: a two-repo registry resumes exactly the repos with a
#     confirmed open in-progress run-state and skips the other.
#   - supervisor security: never execs the registry's raw resume_command; only
#     delegates to /autospec-resume for an independently-confirmed open run.
#   - install_idempotent: install twice -> one launchd/systemd/cron entry;
#     uninstall removes it; status reports correctly on each mocked platform.
#
# All GitHub / launchctl / systemctl / crontab access is via PATH-shadow mocks.

ROOT="${BATS_TEST_DIRNAME}/../.."
SUPERVISOR="$ROOT/scripts/autospec-supervisor.sh"
INSTALL="$ROOT/scripts/autospec-supervisor-install.sh"
REGISTRY="$ROOT/scripts/autospec-run-registry.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP/state"
    export AUTOSPEC_ACTIVE_RUNS_DIR="$TEST_TMP/active-runs"
    export AUTOSPEC_RUN_REGISTRY_SH="$REGISTRY"
    mkdir -p "$AUTOSPEC_STATE_DIR"

    export MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"

    # Repos that GitHub will report as having an OPEN in-progress run (space-sep).
    export GH_OPEN_REPOS=""
    # Log of which repos /autospec-resume (the mock resume-scan) was invoked for.
    export RESUME_LOG="$TEST_TMP/resume.log"
    : > "$RESUME_LOG"
    # Log of any attempt to exec a raw registry resume_command (security probe).
    export POISON_LOG="$TEST_TMP/poison.log"
    : > "$POISON_LOG"

    write_gh_mock
    write_resume_scan_mock
    export AUTOSPEC_RESUME_SCAN_SH="$MOCK_DIR/resume-scan-mock.sh"
}
teardown() { rm -rf "$TEST_TMP"; }

# gh mock: `gh issue list --repo R --label in-progress-by-bot ... --jq length`
# returns 1 when R is in GH_OPEN_REPOS, else 0.
write_gh_mock() {
    cat > "$MOCK_DIR/gh" <<'EOF'
#!/usr/bin/env bash
args="$*"
repo=""
prev=""
for a in "$@"; do
    [ "$prev" = "--repo" ] && repo="$a"
    prev="$a"
done
case "$args" in
    *"issue list"*"--label in-progress-by-bot"*)
        for r in $GH_OPEN_REPOS; do
            if [ "$r" = "$repo" ]; then echo "1"; exit 0; fi
        done
        echo "0"; exit 0 ;;
esac
exit 0
EOF
    chmod +x "$MOCK_DIR/gh"
}

# resume-scan mock: stands in for `/autospec-resume --repo R`. Records the repo
# and (security) records if it was ever asked to exec a raw poisoned command.
write_resume_scan_mock() {
    cat > "$MOCK_DIR/resume-scan-mock.sh" <<'EOF'
#!/usr/bin/env bash
repo=""
prev=""
for a in "$@"; do
    [ "$prev" = "--repo" ] && repo="$a"
    prev="$a"
done
printf '%s\n' "$repo" >> "$RESUME_LOG"
echo "resuming $repo: 1 issue(s)"
exit 0
EOF
    chmod +x "$MOCK_DIR/resume-scan-mock.sh"
}

register() {
    # register --repo R --command C
    bash "$REGISTRY" write --repo "$1" --repo-dir /tmp/x --harness claude \
        --command "${2:-echo RELAUNCHED}" --host h >/dev/null
}

# ── supervisor_thrash ──────────────────────────────────────────────────────────

@test "supervisor_thrash: no open run -> exit 0, resume path NOT called" {
    register o/n "echo SHOULD-NOT-RUN"
    export GH_OPEN_REPOS=""    # GitHub confirms NO open in-progress run
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no open run"* ]]
    [[ "$output" == *"backing off"* ]]
    # resume-scan must never have been invoked.
    [ ! -s "$RESUME_LOG" ]
}

@test "supervisor_thrash: empty registry -> exit 0, nothing resumed" {
    export GH_OPEN_REPOS=""
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    [[ "$output" == *"no open run"* ]]
    [ ! -s "$RESUME_LOG" ]
}

# ── supervisor_two_repo ────────────────────────────────────────────────────────

@test "supervisor_two_repo: resumes only the confirmed-open repo, skips the other" {
    register a/one "echo A"
    register b/two "echo B"
    export GH_OPEN_REPOS="a/one"          # only a/one has an open in-progress run
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    [[ "$output" == *"resuming a/one"* ]]
    [[ "$output" != *"resuming b/two"* ]]
    grep -qx "a/one" "$RESUME_LOG"
    ! grep -qx "b/two" "$RESUME_LOG"
}

@test "supervisor_two_repo: both confirmed -> both resumed" {
    register a/one "echo A"
    register b/two "echo B"
    export GH_OPEN_REPOS="a/one b/two"
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    grep -qx "a/one" "$RESUME_LOG"
    grep -qx "b/two" "$RESUME_LOG"
}

# ── security: never exec a poisoned registry command ───────────────────────────

@test "security: supervisor delegates to /autospec-resume, never execs raw registry command" {
    # A poisoned resume_command that would touch POISON_LOG if ever eval'd.
    register a/one "touch $POISON_LOG.hit"
    export GH_OPEN_REPOS="a/one"
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    # The supervisor handed the repo to /autospec-resume (the confirmer/relauncher).
    grep -qx "a/one" "$RESUME_LOG"
    # It must NOT have eval'd the raw registry command itself.
    [ ! -e "$POISON_LOG.hit" ]
}

@test "security: unconfirmed repo with poisoned command is never relaunched" {
    register evil/repo "touch $POISON_LOG.hit"
    export GH_OPEN_REPOS=""    # GitHub does NOT confirm an open run
    run bash "$SUPERVISOR"
    [ "$status" -eq 0 ]
    [ ! -s "$RESUME_LOG" ]
    [ ! -e "$POISON_LOG.hit" ]
}

# ── install_idempotent: launchd ────────────────────────────────────────────────

@test "install_idempotent (launchd): install twice -> one plist; uninstall removes; status reports" {
    export AUTOSPEC_PLATFORM="darwin"
    export AUTOSPEC_LAUNCH_AGENTS_DIR="$TEST_TMP/LaunchAgents"
    export AUTOSPEC_SUPERVISOR_SH="$SUPERVISOR"
    # launchctl mock (no-op success).
    printf '#!/usr/bin/env bash\nexit 0\n' > "$MOCK_DIR/launchctl"; chmod +x "$MOCK_DIR/launchctl"

    run bash "$INSTALL" status
    [ "$status" -ne 0 ]
    [[ "$output" == *"not installed"* ]]

    bash "$INSTALL" install
    bash "$INSTALL" install            # idempotent second install
    count="$(ls "$AUTOSPEC_LAUNCH_AGENTS_DIR"/*.plist 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "1" ]

    run bash "$INSTALL" status
    [ "$status" -eq 0 ]
    [[ "$output" == *"installed (launchd)"* ]]

    bash "$INSTALL" uninstall
    count="$(ls "$AUTOSPEC_LAUNCH_AGENTS_DIR"/*.plist 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "0" ]
    run bash "$INSTALL" status
    [ "$status" -ne 0 ]
}

# ── install_idempotent: systemd ────────────────────────────────────────────────

@test "install_idempotent (systemd): install twice -> one unit; uninstall removes; status reports" {
    export AUTOSPEC_PLATFORM="linux"
    export AUTOSPEC_SYSTEMD_USER_DIR="$TEST_TMP/systemd"
    export AUTOSPEC_SUPERVISOR_SH="$SUPERVISOR"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$MOCK_DIR/systemctl"; chmod +x "$MOCK_DIR/systemctl"

    bash "$INSTALL" install
    bash "$INSTALL" install
    count="$(ls "$AUTOSPEC_SYSTEMD_USER_DIR"/*.service 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "1" ]

    run bash "$INSTALL" status
    [ "$status" -eq 0 ]
    [[ "$output" == *"installed (systemd)"* ]]

    bash "$INSTALL" uninstall
    count="$(ls "$AUTOSPEC_SYSTEMD_USER_DIR"/*.service 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "0" ]
    run bash "$INSTALL" status
    [ "$status" -ne 0 ]
}

# ── install_idempotent: @reboot cron fallback ──────────────────────────────────

@test "install_idempotent (cron): install twice -> one @reboot entry; uninstall removes; status reports" {
    export AUTOSPEC_PLATFORM="cron"
    export AUTOSPEC_SUPERVISOR_SH="$SUPERVISOR"
    # crontab mock backed by a file.
    export CRON_FILE="$TEST_TMP/crontab.txt"
    : > "$CRON_FILE"
    cat > "$MOCK_DIR/crontab" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
    -l) [ -s "$CRON_FILE" ] && cat "$CRON_FILE"; [ -s "$CRON_FILE" ] || exit 1; exit 0 ;;
    -r) : > "$CRON_FILE"; exit 0 ;;
    -)  cat > "$CRON_FILE"; exit 0 ;;
    *)  cat > "$CRON_FILE"; exit 0 ;;
esac
EOF
    chmod +x "$MOCK_DIR/crontab"

    run bash "$INSTALL" status
    [ "$status" -ne 0 ]

    bash "$INSTALL" install
    bash "$INSTALL" install
    count="$(grep -F 'autospec-supervisor' "$CRON_FILE" 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "1" ]

    run bash "$INSTALL" status
    [ "$status" -eq 0 ]
    [[ "$output" == *"installed (cron)"* ]]

    bash "$INSTALL" uninstall
    count="$(grep -F 'autospec-supervisor' "$CRON_FILE" 2>/dev/null | wc -l | tr -d ' ')"
    [ "$count" = "0" ]
    run bash "$INSTALL" status
    [ "$status" -ne 0 ]
}

# ── platform selection ─────────────────────────────────────────────────────────

@test "platform selection: darwin->launchd, linux->systemd, else->cron" {
    export AUTOSPEC_SUPERVISOR_SH="$SUPERVISOR"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$MOCK_DIR/launchctl"; chmod +x "$MOCK_DIR/launchctl"
    printf '#!/usr/bin/env bash\nexit 0\n' > "$MOCK_DIR/systemctl"; chmod +x "$MOCK_DIR/systemctl"

    export AUTOSPEC_PLATFORM="darwin"
    export AUTOSPEC_LAUNCH_AGENTS_DIR="$TEST_TMP/la"
    run bash "$INSTALL" install
    [[ "$output" == *"launchd"* ]]

    export AUTOSPEC_PLATFORM="linux"
    export AUTOSPEC_SYSTEMD_USER_DIR="$TEST_TMP/sd"
    run bash "$INSTALL" install
    [[ "$output" == *"systemd"* ]]

    export AUTOSPEC_PLATFORM="cron"
    export CRON_FILE="$TEST_TMP/cron.txt"; : > "$CRON_FILE"
    cat > "$MOCK_DIR/crontab" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
    -l) [ -s "$CRON_FILE" ] && cat "$CRON_FILE"; exit 0 ;;
    *)  cat > "$CRON_FILE"; exit 0 ;;
esac
EOF
    chmod +x "$MOCK_DIR/crontab"
    run bash "$INSTALL" install
    [[ "$output" == *"cron"* ]]
}
