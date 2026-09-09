#!/usr/bin/env bash
# autospec-startup-self-update.sh — startup self-update preflight.
# See docs/specs/2026-05-01-autospec-startup-self-update-design.md
#
# WHY THIS IS A SCRIPT AND NOT AN INLINED MARKDOWN BLOCK (issue #3177):
# this logic used to live verbatim inside templates/skill-blocks/startup-self-update.md,
# which install.sh expands into every installed skill body (Claude SKILL.md,
# Codex prompts/<skill>.md, OpenCode agent/<skill>.md). A harness substitutes
# positional parameters inside a *rendered skill body* at load time, so
# `target="$1"` in that block was rewritten to the caller's slash-command
# argument. heal_autonomous_operator_wrappers() then ignored its own argument
# and wrote a wrapper script over that path (plus chmod +x) on every loop
# iteration — before the daily throttle ever ran. Shell that lives in a .sh
# file is never rendered by a harness, so its positional parameters stay
# positional. Do not move this body back into markdown.
#
# Usage: autospec-startup-self-update.sh [<skill-name>]
#   <skill-name> is an inert provenance label (e.g. autospec-run). It is never
#   used as a path and never written to.
#
# Doctor mode (out-of-band health check, issue #3937): a self-updater cannot
# ship its own repair, so its liveness is asserted from outside.
#   autospec-startup-self-update.sh --doctor [--clear-stale-lock]
# Reports lock age/owner, throttle-stamp age, installed-vs-remote drift and
# the last failure record; exits 1 when any finding is present. The
# --clear-stale-lock option also reclaims a stale lock. Doctor mode performs
# no update and no network call.
#
# Contract (unchanged from the inlined block):
#   - AUTOSPEC_NO_SELF_UPDATE=1 -> exit 0 immediately, no network.
#   - Daily (86400s) throttle on $HOME/.autospec/last-update-check.
#   - mkdir-based lock at $HOME/.autospec/.update.lock.d, with an owner file
#     (PID + creation time); a stale or ownerless lock is reclaimed.
#   - Fail-open: every failure path emits a `WARN:` line on stderr and exits 0.
#   - Failure diagnostics persist to last-update-failure.json + self-update.log.
#   - AUTOSPEC_SCRIPTS_DIR overrides the installed scripts directory.
set +e
DOCTOR=0
CLEAR_STALE_LOCK=0
SKILL_NAME=""
for arg in "$@"; do
    case "$arg" in
        --doctor) DOCTOR=1 ;;
        --clear-stale-lock) CLEAR_STALE_LOCK=1 ;;
        *) [ -z "$SKILL_NAME" ] && SKILL_NAME="$arg" ;;
    esac
done
export AUTOSPEC_SELF_UPDATE_SKILL="$SKILL_NAME"
umask 077
mkdir -p "$HOME/.autospec"
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
REMOTE_VERSION="$HOME/.autospec/remote-version"
FAILURE_RECORD="$HOME/.autospec/last-update-failure.json"
UPDATE_LOG="$HOME/.autospec/self-update.log"
BOOTSTRAP_TMP="$HOME/.autospec/.self-update-bootstrap.$$"
NOW=$(date -u +%s)
THROTTLE_SECS=86400
# A successful check is daily; past 3x the interval, "healthy" and "degraded"
# are indistinguishable on disk, so the age check must fail loudly (#3937).
THROTTLE_ALARM_SECS=259200
# A self-update is minutes, not hours: past this age a lock is stale.
LOCK_STALE_SECS=1800
# mkdir-to-owner-write window of a live concurrent run: a brand-new lock with
# no owner file yet may still have a live writer inside this window.
LOCK_GRACE_SECS=30

path_mtime() { stat -c '%Y' "$1" 2>/dev/null || stat -f '%m' "$1" 2>/dev/null || echo 0; }

iso_from_epoch() {
    date -u -j -f '%s' "$1" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -d "@$1" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || echo unknown
}

# Set LOCK_CREATED, LOCK_AGE, LOCK_CREATED_ISO, LOCK_HAS_OWNER, LOCK_OWNER_PID,
# LOCK_OWNER_LIVE for the current lock directory (must exist).
inspect_lock() {
    LOCK_HAS_OWNER=0
    LOCK_OWNER_PID=""
    LOCK_OWNER_LIVE=0
    LOCK_CREATED="$(path_mtime "$LOCKDIR")"
    if [ -f "$LOCKDIR/owner" ]; then
        LOCK_HAS_OWNER=1
        LOCK_OWNER_PID=$(awk 'NR==1{print $1}' "$LOCKDIR/owner" 2>/dev/null || true)
        case "$LOCK_OWNER_PID" in ''|*[!0-9]*) LOCK_OWNER_PID="" ;; esac
        if [ -n "$LOCK_OWNER_PID" ] && kill -0 "$LOCK_OWNER_PID" 2>/dev/null; then
            LOCK_OWNER_LIVE=1
        fi
    fi
    LOCK_AGE=$((NOW - LOCK_CREATED))
    [ "$LOCK_AGE" -lt 0 ] && LOCK_AGE=0
    LOCK_CREATED_ISO="$(iso_from_epoch "$LOCK_CREATED")"
}

# Fail-open hides drift: the installed-vs-remote gap must be surfaced on
# stderr so a throttled-but-stale install is not silent (#3937).
report_version_drift() {
    local inst rem
    inst=$(cat "$INSTALLED" 2>/dev/null || true)
    rem=$(cat "$REMOTE_VERSION" 2>/dev/null || true)
    if [ -n "$inst" ] && [ -n "$rem" ] && [ "$inst" != "$rem" ]; then
        echo "WARN: installed-version ${inst} differs from last-seen remote-version ${rem}; installed suite is stale" >&2
    fi
}

run_doctor() {
    local clear_stale="$1"
    local findings=0
    echo "autospec self-update doctor ($HOME/.autospec)"
    if [ -e "$LOCKDIR" ]; then
        inspect_lock
        if [ "$LOCK_OWNER_LIVE" -eq 1 ] && [ "$LOCK_AGE" -lt "$LOCK_STALE_SECS" ]; then
            echo "lock:             held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s, owner PID ${LOCK_OWNER_PID} (live)"
        else
            findings=$((findings + 1))
            echo "lock:             STALE — held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s"
            if [ "$LOCK_HAS_OWNER" -eq 1 ]; then
                echo "lock owner:       PID ${LOCK_OWNER_PID:-unrecorded}"
            else
                echo "lock owner:       no live owner found"
            fi
            if [ "$clear_stale" = 1 ]; then
                rm -f "$LOCKDIR/owner" 2>/dev/null
                rmdir "$LOCKDIR" 2>/dev/null || rm -rf "$LOCKDIR" 2>/dev/null
                if [ ! -e "$LOCKDIR" ]; then
                    echo "lock:             cleared"
                    findings=$((findings - 1))
                fi
            else
                echo "lock:             reclaimable; rerun with --clear-stale-lock to clear"
            fi
        fi
    else
        echo "lock:             absent"
    fi
    if [ -f "$LAST" ]; then
        local prev age
        prev=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
            || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
        age=$((NOW - prev))
        [ "$age" -lt 0 ] && age=0
        echo "throttle stamp:   $(cat "$LAST" 2>/dev/null), age ${age}s (interval ${THROTTLE_SECS}s)"
        if [ "$age" -gt "$THROTTLE_ALARM_SECS" ]; then
            findings=$((findings + 1))
            echo "throttle stamp:   STALE — self-update has not completed for more than 3x the interval"
        fi
    else
        echo "throttle stamp:   absent (no successful check recorded)"
    fi
    local inst rem
    inst=$(cat "$INSTALLED" 2>/dev/null || true)
    rem=$(cat "$REMOTE_VERSION" 2>/dev/null || true)
    if [ -n "$inst" ] && [ -n "$rem" ] && [ "$inst" != "$rem" ]; then
        findings=$((findings + 1))
        echo "version drift:    installed ${inst} vs remote ${rem} — installed suite is stale"
    else
        echo "version drift:    in sync (installed ${inst:-unknown} / remote ${rem:-unknown})"
    fi
    if [ -f "$FAILURE_RECORD" ]; then
        findings=$((findings + 1))
        local ts rc
        ts=$(jq -r '.timestamp // "unknown"' "$FAILURE_RECORD" 2>/dev/null || echo unknown)
        rc=$(jq -r '.installer_exit_code // "unknown"' "$FAILURE_RECORD" 2>/dev/null || echo unknown)
        echo "last failure:     ${ts} (installer exit ${rc}) — record: ${FAILURE_RECORD}"
    else
        echo "last failure:     none"
    fi
    if [ "$findings" -gt 0 ]; then
        echo "doctor: ${findings} finding(s)"
        return 1
    fi
    echo "doctor: healthy"
    return 0
}

if [ "$DOCTOR" -eq 1 ]; then run_doctor "$CLEAR_STALE_LOCK"; exit "$?"; fi
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
write_autonomous_operator_wrapper() {
    target="$1"
    subcommand="$2"
    rust_subcommand="$subcommand"
    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf '%s\n' 'set -eu'
        case "$subcommand" in
            ""|start|status|list|timeline|monitor|supervise|logs|watch|cleanup|stop|restart)
                printf '%s\n' 'if command -v autospec >/dev/null 2>&1; then'
                if [ -n "$rust_subcommand" ]; then
                    printf '%s\n' '    exec autospec autonomous '"$rust_subcommand"' "$@"'
                else
                    printf '%s\n' '    exec autospec autonomous "$@"'
                fi
                printf '%s\n' 'fi'
                ;;
        esac
        if [ -n "$subcommand" ]; then
            printf '%s\n' 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh" '"$subcommand"' "$@"'
        else
            printf '%s\n' 'exec "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-autonomous.sh" "$@"'
        fi
    } > "$target"
    chmod +x "$target"
}
autonomous_operator_wrapper_exec_target() {
    wrapper="$1"
    [ -f "$wrapper" ] || return 1
    sed -n 's/^exec "\([^"]*\)".*/\1/p; s/^exec \([^ "$][^ ]*\).*/\1/p' "$wrapper" | head -n 1
}
autonomous_operator_wrapper_needs_heal() {
    wrapper="$1"
    exec_target="$(autonomous_operator_wrapper_exec_target "$wrapper" 2>/dev/null || true)"
    [ -n "$exec_target" ] || return 1
    case "$exec_target" in
        /*)
            case "$exec_target" in
                "$HOME/.autospec/"*) [ -e "$exec_target" ] || return 0 ;;
                *) return 0 ;;
            esac
            ;;
    esac
    return 1
}
heal_autonomous_operator_wrappers() {
    autospec_bin_dir="$HOME/.autospec/bin"
    [ -d "$autospec_bin_dir" ] || return 0
    healed=0
    for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
        target="$autospec_bin_dir/$command"
        [ -f "$target" ] || continue
        if autonomous_operator_wrapper_needs_heal "$target"; then
            old_target="$(autonomous_operator_wrapper_exec_target "$target" 2>/dev/null || true)"
            subcommand="${command#autospec-autonomous-}"
            if [ "$subcommand" = "$command" ]; then subcommand=""; fi
            write_autonomous_operator_wrapper "$target" "$subcommand"
            echo "heal_autonomous_operator_wrappers: healed $target (old exec target: ${old_target:-unknown})"
            healed=$((healed + 1))
        fi
    done
    if [ "$healed" -gt 0 ]; then
        echo "heal_autonomous_operator_wrappers: healed $healed autonomous wrapper(s)"
    fi
}
heal_autonomous_operator_wrappers
# The lock must carry liveness (owner PID + creation time); a stale or
# ownerless lock is reclaimed instead of being mistaken for a concurrent run
# (#3937). Reclaiming is bounded: three attempts, then fail open.
acquire_lock() {
    local attempt=0
    while :; do
        if [ ! -e "$LOCKDIR" ]; then
            if mkdir "$LOCKDIR" 2>/dev/null; then
                printf '%s %s %s\n' "$$" "$(date -u +%s)" "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" > "$LOCKDIR/owner" 2>/dev/null
                return 0
            fi
            # Lost the race: re-loop and report the verified holder, if any.
            sleep 1
            continue
        fi
        attempt=$((attempt + 1))
        inspect_lock
        if [ "$attempt" -gt 3 ]; then
            local owner_desc="no live owner found"
            [ "$LOCK_OWNER_LIVE" -eq 1 ] && owner_desc="PID ${LOCK_OWNER_PID} live"
            echo "WARN: self-update skipped; lock held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s (owner: ${owner_desc}); could not acquire after 3 attempts" >&2
            return 1
        fi
        if [ "$LOCK_OWNER_LIVE" -eq 1 ] && [ "$LOCK_AGE" -lt "$LOCK_STALE_SECS" ]; then
            # Only this case is a verified concurrent run; every other path
            # reports what was observed, not an inferred cause (#3937).
            echo "WARN: self-update skipped; lock held since ${LOCK_CREATED_ISO} by live PID ${LOCK_OWNER_PID} (age ${LOCK_AGE}s; concurrent update in progress)" >&2
            return 1
        fi
        if [ "$LOCK_HAS_OWNER" -eq 0 ] && [ "$LOCK_AGE" -lt "$LOCK_GRACE_SECS" ]; then
            echo "WARN: self-update skipped; lock held for ${LOCK_AGE}s; no live owner found (within ${LOCK_GRACE_SECS}s write-grace window; presumed concurrent)" >&2
            return 1
        fi
        if [ "$LOCK_OWNER_LIVE" -eq 1 ]; then
            echo "WARN: reclaiming stale update lock: held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s > ${LOCK_STALE_SECS}s (owner PID ${LOCK_OWNER_PID} still live)" >&2
        elif [ "$LOCK_HAS_OWNER" -eq 1 ]; then
            echo "WARN: reclaiming stale update lock: held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s (owner PID ${LOCK_OWNER_PID:-unrecorded} not running)" >&2
        else
            echo "WARN: reclaiming stale update lock: held since ${LOCK_CREATED_ISO}, age ${LOCK_AGE}s; no live owner found" >&2
        fi
        rm -f "$LOCKDIR/owner" 2>/dev/null
        rmdir "$LOCKDIR" 2>/dev/null || rm -rf "$LOCKDIR" 2>/dev/null
    done
}

if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    # Out-of-band liveness alarm: a self-updater cannot ship its own repair,
    # so a throttle stamp past 3x the interval fails loudly here and in
    # --doctor (#3937).
    if [ "$((NOW - PREV))" -gt "$THROTTLE_ALARM_SECS" ]; then
        echo "WARN: self-update has not completed in $((NOW - PREV))s (> 3x the ${THROTTLE_SECS}s interval); run autospec-startup-self-update.sh --doctor" >&2
    fi
    if [ "$((NOW - PREV))" -lt 86400 ]; then
        report_version_drift
        exit 0
    fi
fi
if ! acquire_lock; then exit 0; fi
trap 'rm -f "$BOOTSTRAP_TMP" "${INSTALLED_BACKUP:-}"; rm -f "$LOCKDIR/owner" 2>/dev/null; rmdir "$LOCKDIR" 2>/dev/null' EXIT
REMOTE=$(curl -fsSL --max-time 5 \
    "https://api.github.com/repos/berlinguyinca/autospec/commits/main" \
    2>/dev/null | jq -r '.sha // empty' 2>/dev/null | cut -c1-7)
if [ -z "$REMOTE" ]; then
    echo "WARN: self-update skipped (network); continuing on installed version" >&2; exit 0
fi
if ! printf '%s\n' "$REMOTE" > "$REMOTE_VERSION.tmp" \
    || ! mv "$REMOTE_VERSION.tmp" "$REMOTE_VERSION"; then
    rm -f "$REMOTE_VERSION.tmp"
    echo "WARN: self-update state publication failed ($REMOTE_VERSION); continuing on installed version" >&2
    exit 0
fi
LOCAL=$(cat "$INSTALLED" 2>/dev/null || true)
if [ "$REMOTE" = "$LOCAL" ]; then
    if ! date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" || ! mv "$LAST.tmp" "$LAST"; then
        rm -f "$LAST.tmp"
        echo "WARN: self-update state publication failed ($LAST); continuing on installed version" >&2
        exit 0
    fi
    rm -f "$FAILURE_RECORD"
    exit 0
fi
if ! curl -fsSL --max-time 30 \
    "https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh" \
    > "$BOOTSTRAP_TMP"; then
    echo "WARN: self-update skipped (bootstrap download); continuing on installed version" >&2
    exit 0
fi
if [ -f "$UPDATE_LOG" ]; then mv "$UPDATE_LOG" "$UPDATE_LOG.1"; fi
: > "$UPDATE_LOG"
chmod 600 "$UPDATE_LOG" 2>/dev/null || true
bash "$BOOTSTRAP_TMP" --skill all --harness all --update 2>&1 \
    | tail -c 65536 > "$UPDATE_LOG"
RC=${PIPESTATUS[0]}
chmod 600 "$UPDATE_LOG" "$UPDATE_LOG.1" 2>/dev/null || true
if [ "$RC" -ne 0 ]; then
    FAILURE_AT=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
    OUTPUT_TAIL=$(tail -c 16384 "$UPDATE_LOG" 2>/dev/null || true)
    jq -n \
        --arg timestamp "$FAILURE_AT" \
        --arg remote_sha "$REMOTE" \
        --argjson installer_exit_code "$RC" \
        --arg output_tail "$OUTPUT_TAIL" \
        --arg log_path "$UPDATE_LOG" \
        '{timestamp:$timestamp,remote_sha:$remote_sha,installer_exit_code:$installer_exit_code,output_tail:$output_tail,log_path:$log_path}' \
        > "$FAILURE_RECORD.tmp" \
        && chmod 600 "$FAILURE_RECORD.tmp" \
        && mv "$FAILURE_RECORD.tmp" "$FAILURE_RECORD"
    echo "WARN: self-update failed (install rc=$RC); continuing on installed version; diagnostics: $UPDATE_LOG; record: $FAILURE_RECORD" >&2
    exit 0
fi
INSTALLED_BACKUP="$HOME/.autospec/.installed-version.backup.$$"
HAD_INSTALLED=0
if [ -f "$INSTALLED" ]; then
    HAD_INSTALLED=1
    if ! cp "$INSTALLED" "$INSTALLED_BACKUP"; then
        echo "WARN: self-update state publication failed ($INSTALLED backup); continuing on installed version" >&2
        exit 0
    fi
fi
if ! printf '%s\n' "$REMOTE" > "$INSTALLED.tmp" || ! mv "$INSTALLED.tmp" "$INSTALLED"; then
    rm -f "$INSTALLED.tmp"
    rm -f "$INSTALLED_BACKUP"
    echo "WARN: self-update state publication failed ($INSTALLED); continuing on installed version" >&2
    exit 0
fi
if ! date -u +'%Y-%m-%dT%H:%M:%SZ' > "$LAST.tmp" || ! mv "$LAST.tmp" "$LAST"; then
    rm -f "$LAST.tmp"
    if [ "$HAD_INSTALLED" -eq 1 ]; then
        if ! mv "$INSTALLED_BACKUP" "$INSTALLED"; then
            echo "WARN: self-update state rollback failed ($INSTALLED); manual recovery required" >&2
            exit 0
        fi
    else
        rm -f "$INSTALLED"
    fi
    echo "WARN: self-update state publication failed ($LAST); continuing on installed version" >&2
    exit 0
fi
rm -f "$INSTALLED_BACKUP"
rm -f "$FAILURE_RECORD"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
