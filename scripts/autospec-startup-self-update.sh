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
# Usage: autospec-startup-self-update.sh [<skill-name>] [--doctor] [--clear-stale-lock]
#   <skill-name>        inert provenance label (e.g. autospec-run). It is never
#                       used as a path and never written to.
#   --doctor            report lock age/owner, throttle-stamp age, version drift and
#                       the last failure record; exits 1 when anything is degraded.
#                       Read-only, no network, and runs even under the opt-out below
#                       because it is an explicit operator command.
#   --clear-stale-lock  implies --doctor and removes a lock with no live owner.
#
# Contract:
#   - AUTOSPEC_NO_SELF_UPDATE=1 -> exit 0 immediately, no network (unless --doctor).
#   - Daily (86400s) throttle on $HOME/.autospec/last-update-check.
#   - mkdir-based lock at $HOME/.autospec/.update.lock.d, stamped with the owner PID
#     and creation time (issue #3937). A lock whose owner is not a live process, or
#     that is older than AUTOSPEC_SELF_UPDATE_LOCK_STALE_SECS, is reclaimed.
#   - Degradation is observable without manual `stat`: version drift and the age of
#     last-update-check are printed on stderr and recorded to self-update-health.json.
#     The age alarm fires at AUTOSPEC_SELF_UPDATE_STALE_ALARM_SECS (default 3 days).
#   - Fail-open: every failure path emits a `WARN:` line on stderr and exits 0.
#   - Failure diagnostics persist to last-update-failure.json + self-update.log.
#   - AUTOSPEC_SCRIPTS_DIR overrides the installed scripts directory.
set +e
SKILL_NAME=""
DOCTOR=0
CLEAR_STALE_LOCK=0
for _autospec_arg in "$@"; do
    case "$_autospec_arg" in
        --doctor) DOCTOR=1 ;;
        --clear-stale-lock) DOCTOR=1; CLEAR_STALE_LOCK=1 ;;
        *) SKILL_NAME="$_autospec_arg" ;;
    esac
done
export AUTOSPEC_SELF_UPDATE_SKILL="$SKILL_NAME"
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ] && [ "$DOCTOR" -eq 0 ]; then exit 0; fi
umask 077
mkdir -p "$HOME/.autospec"
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
if [ "$DOCTOR" -eq 0 ]; then heal_autonomous_operator_wrappers; fi
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
REMOTE_VERSION="$HOME/.autospec/remote-version"
FAILURE_RECORD="$HOME/.autospec/last-update-failure.json"
HEALTH_RECORD="$HOME/.autospec/self-update-health.json"
UPDATE_LOG="$HOME/.autospec/self-update.log"
BOOTSTRAP_TMP="$HOME/.autospec/.self-update-bootstrap.$$"
NOW=$(date -u +%s)
UPDATE_INTERVAL_SECS=86400
# A self-update takes minutes; a lock older than this has no live owner in any
# healthy run, so holding it would be the #3937 failure mode again.
LOCK_STALE_SECS="${AUTOSPEC_SELF_UPDATE_LOCK_STALE_SECS:-1800}"
# "A small multiple of the update interval": past this, self-update is broken
# and the operator is told so instead of finding out by `stat`.
STALE_ALARM_SECS="${AUTOSPEC_SELF_UPDATE_STALE_ALARM_SECS:-259200}"

# --- portable time helpers (bash 3.2, GNU and BSD date/stat) -----------------
iso_to_epoch() {
    _s="$1"
    [ -n "$_s" ] || { printf '0\n'; return; }
    date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$_s" +%s 2>/dev/null \
        || date -u -d "$_s" +%s 2>/dev/null \
        || date -u -r "$_s" +%s 2>/dev/null \
        || printf '0\n'
}

epoch_to_iso() {
    _e="$1"
    case "$_e" in ''|*[!0-9]*|0) printf 'unknown\n'; return ;; esac
    date -u -d "@$_e" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || date -u -r "$_e" +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null \
        || printf 'unknown\n'
}

path_mtime_epoch() {
    stat -c %Y "$1" 2>/dev/null || stat -f %m "$1" 2>/dev/null || printf '0\n'
}

pid_alive() {
    case "$1" in ''|*[!0-9]*) return 1 ;; esac
    [ "$1" -gt 0 ] && kill -0 "$1" 2>/dev/null
}

# --- state readers -----------------------------------------------------------
read_installed_version() { cat "$INSTALLED" 2>/dev/null || true; }
read_remote_version() { cat "$REMOTE_VERSION" 2>/dev/null || true; }
read_last_check_iso() { cat "$LAST" 2>/dev/null || true; }

# Seconds since the last completed check; -1 when it was never recorded or the
# stamp cannot be parsed (an unparseable stamp must not read as "just checked").
last_check_age_secs() {
    _iso="$(read_last_check_iso)"
    [ -n "$_iso" ] || { printf -- '-1\n'; return; }
    _e="$(iso_to_epoch "$_iso")"
    case "$_e" in ''|0) printf -- '-1\n'; return ;; esac
    printf '%s\n' "$((NOW - _e))"
}

# --- durable degradation record ----------------------------------------------
# Fail-open on stderr alone makes "degraded" and "healthy" indistinguishable to
# anyone who is not watching stderr (issue #3937), so every degraded outcome is
# also written to disk for `--doctor` and post-hoc inspection.
record_self_update_degradation() {
    _reason="$1"
    _tmp="$HEALTH_RECORD.tmp"
    jq -n \
        --arg timestamp "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" \
        --arg reason "$_reason" \
        --arg installed_version "$(read_installed_version)" \
        --arg remote_version "$(read_remote_version)" \
        --arg last_update_check "$(read_last_check_iso)" \
        --arg log_path "$UPDATE_LOG" \
        '{timestamp:$timestamp,reason:$reason,installed_version:$installed_version,remote_version:$remote_version,last_update_check:$last_update_check,log_path:$log_path}' \
        > "$_tmp" 2>/dev/null || { rm -f "$_tmp"; return 0; }
    chmod 600 "$_tmp" 2>/dev/null || true
    mv "$_tmp" "$HEALTH_RECORD" 2>/dev/null || rm -f "$_tmp"
    return 0
}

# --- invariants 3 and 4: drift and staleness must be surfaced ----------------
surface_self_update_degradation() {
    _reason=""
    _installed="$(read_installed_version)"
    _remote="$(read_remote_version)"
    if [ -n "$_installed" ] && [ -n "$_remote" ] && [ "$_installed" != "$_remote" ]; then
        echo "WARN: self-update drift: installed ${_installed} is behind remote ${_remote} (last successful check $(read_last_check_iso)); diagnose with --doctor" >&2
        _reason="version-drift"
    fi
    _age="$(last_check_age_secs)"
    if [ "$_age" -gt "$STALE_ALARM_SECS" ]; then
        echo "WARN: self-update has not completed for $(( _age / 3600 ))h (last check $(read_last_check_iso), alarm at $((STALE_ALARM_SECS / 3600))h); diagnose with --doctor" >&2
        _reason="${_reason:+$_reason,}throttle-stamp-stale"
    fi
    [ -n "$_reason" ] && record_self_update_degradation "$_reason"
    return 0
}

# --- invariant 1: the lock carries liveness ----------------------------------
LOCK_OBSERVED_PID=""
LOCK_OBSERVED_EPOCH=""
LOCK_OBSERVED_SINCE=""

observe_update_lock() {
    LOCK_OBSERVED_PID="$(cat "$LOCKDIR/owner.pid" 2>/dev/null || true)"
    LOCK_OBSERVED_EPOCH="$(cat "$LOCKDIR/owner.epoch" 2>/dev/null || true)"
    case "$LOCK_OBSERVED_EPOCH" in ''|*[!0-9]*) LOCK_OBSERVED_EPOCH="$(path_mtime_epoch "$LOCKDIR")" ;; esac
    LOCK_OBSERVED_SINCE="$(epoch_to_iso "$LOCK_OBSERVED_EPOCH")"
}

lock_age_secs() {
    case "$LOCK_OBSERVED_EPOCH" in ''|*[!0-9]*) printf '%s\n' "$NOW" ;; *) printf '%s\n' "$((NOW - LOCK_OBSERVED_EPOCH))" ;; esac
}

lock_owner_description() {
    if [ -n "$LOCK_OBSERVED_PID" ]; then
        printf 'owner pid %s is not a live process' "$LOCK_OBSERVED_PID"
    else
        printf 'no owner recorded'
    fi
}

# A failed mkdir is only evidence of a concurrent run when the owner is alive.
# Everything else is stale and reclaimable (issue #3937).
update_lock_is_stale() {
    if pid_alive "$LOCK_OBSERVED_PID"; then
        [ "$(lock_age_secs)" -lt "$LOCK_STALE_SECS" ] && return 1
    fi
    return 0
}

claim_update_lock() {
    printf '%s\n' "$$" > "$LOCKDIR/owner.pid" 2>/dev/null || true
    printf '%s\n' "$NOW" > "$LOCKDIR/owner.epoch" 2>/dev/null || true
}

acquire_update_lock() {
    if mkdir "$LOCKDIR" 2>/dev/null; then
        claim_update_lock
        return 0
    fi
    observe_update_lock
    if update_lock_is_stale; then
        echo "WARN: self-update reclaiming stale lock $LOCKDIR (held since $LOCK_OBSERVED_SINCE, age $(lock_age_secs)s; $(lock_owner_description))" >&2
        record_self_update_degradation "stale-lock-reclaimed"
        rm -rf -- "$LOCKDIR" 2>/dev/null || true
        if mkdir "$LOCKDIR" 2>/dev/null; then
            claim_update_lock
            return 0
        fi
        echo "WARN: self-update skipped (lock $LOCKDIR could not be reclaimed); continuing on installed version" >&2
        return 1
    fi
    echo "WARN: self-update skipped (concurrent update in progress: owner pid $LOCK_OBSERVED_PID is live, lock age $(lock_age_secs)s)" >&2
    return 1
}

# --- invariant 5: out-of-band health report ----------------------------------
self_update_doctor() {
    problems=0
    echo "autospec self-update doctor (state dir: $HOME/.autospec)"
    if [ -d "$LOCKDIR" ]; then
        observe_update_lock
        if update_lock_is_stale; then
            echo "  lock: STALE $LOCKDIR held since $LOCK_OBSERVED_SINCE (age $(lock_age_secs)s, threshold ${LOCK_STALE_SECS}s; $(lock_owner_description))"
            problems=$((problems + 1))
            if [ "$CLEAR_STALE_LOCK" -eq 1 ]; then
                if rm -rf -- "$LOCKDIR"; then
                    echo "  lock: cleared $LOCKDIR"
                else
                    echo "  lock: CLEAR FAILED $LOCKDIR"
                fi
            fi
        else
            echo "  lock: held by live pid $LOCK_OBSERVED_PID (age $(lock_age_secs)s)"
        fi
    else
        echo "  lock: absent"
    fi

    _age="$(last_check_age_secs)"
    if [ "$_age" -lt 0 ]; then
        if [ -f "$LAST" ]; then
            echo "  throttle: UNREADABLE stamp $(read_last_check_iso) in $LAST"
            problems=$((problems + 1))
        else
            echo "  throttle: never recorded ($LAST absent)"
            [ -n "$(read_installed_version)" ] && problems=$((problems + 1))
        fi
    elif [ "$_age" -gt "$STALE_ALARM_SECS" ]; then
        echo "  throttle: STALE last successful check $(read_last_check_iso) ($(( _age / 86400 ))d ago, alarm at $((STALE_ALARM_SECS / 86400 ))d)"
        problems=$((problems + 1))
    else
        echo "  throttle: ok last successful check $(read_last_check_iso) ($(( _age / 3600 ))h ago)"
    fi

    _installed="$(read_installed_version)"
    _remote="$(read_remote_version)"
    if [ -z "$_installed" ] && [ -z "$_remote" ]; then
        echo "  version: unknown (no installed-version or remote-version receipt)"
    elif [ "$_installed" = "$_remote" ]; then
        echo "  version: ok installed $_installed == remote $_remote"
    else
        echo "  version: DRIFT installed ${_installed:-none} != remote ${_remote:-none} (self-update pending or failing)"
        problems=$((problems + 1))
    fi

    if [ -s "$FAILURE_RECORD" ]; then
        echo "  last failure: $(tr -d '\n' < "$FAILURE_RECORD" | head -c 400)"
        problems=$((problems + 1))
    else
        echo "  last failure: none"
    fi

    if [ -s "$HEALTH_RECORD" ]; then
        echo "  degradation record: $(tr -d '\n' < "$HEALTH_RECORD" | head -c 400)"
    else
        echo "  degradation record: none"
    fi

    [ "$problems" -eq 0 ]
}

if [ "$DOCTOR" -eq 1 ]; then
    self_update_doctor
    exit $?
fi

surface_self_update_degradation

if [ -f "$LAST" ]; then
    PREV_AGE="$(last_check_age_secs)"
    # An unknown stamp is treated as "due", never as "just checked".
    [ "$PREV_AGE" -lt 0 ] && PREV_AGE="$UPDATE_INTERVAL_SECS"
    if [ "$PREV_AGE" -lt "$UPDATE_INTERVAL_SECS" ]; then exit 0; fi
fi
if ! acquire_update_lock; then exit 0; fi
trap 'rm -f "$BOOTSTRAP_TMP" "${INSTALLED_BACKUP:-}"; rm -f "$LOCKDIR/owner.pid" "$LOCKDIR/owner.epoch" 2>/dev/null; rmdir "$LOCKDIR" 2>/dev/null' EXIT
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
    rm -f "$HEALTH_RECORD"
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
# A completed check retires the degradation record: `--doctor` must not keep
# reporting a problem the next successful update already fixed.
rm -f "$HEALTH_RECORD"
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
echo "[autospec] updated ${LOCAL:-fresh} → $REMOTE"
