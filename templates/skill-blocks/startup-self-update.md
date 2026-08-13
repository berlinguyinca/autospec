## Startup self-update

```bash
#!/usr/bin/env bash
# autospec-startup-self-update — see docs/specs/2026-05-01-autospec-startup-self-update-design.md
set +e
SKILL_NAME={{SKILL_NAME}}   # per-skill: autospec-define / autospec-run / autospec-listen / autospec-classify
if [ "${AUTOSPEC_NO_SELF_UPDATE:-0}" = "1" ]; then exit 0; fi
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
heal_autonomous_operator_wrappers
LOCKDIR="$HOME/.autospec/.update.lock.d"
LAST="$HOME/.autospec/last-update-check"
INSTALLED="$HOME/.autospec/installed-version"
REMOTE_VERSION="$HOME/.autospec/remote-version"
FAILURE_RECORD="$HOME/.autospec/last-update-failure.json"
UPDATE_LOG="$HOME/.autospec/self-update.log"
BOOTSTRAP_TMP="$HOME/.autospec/.self-update-bootstrap.$$"
NOW=$(date -u +%s)
if [ -f "$LAST" ]; then
    PREV=$(date -u -j -f '%Y-%m-%dT%H:%M:%SZ' "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null \
        || date -u -d "$(cat "$LAST" 2>/dev/null)" +%s 2>/dev/null || echo 0)
    if [ "$((NOW - PREV))" -lt 86400 ]; then exit 0; fi
fi
if ! mkdir "$LOCKDIR" 2>/dev/null; then
    echo "WARN: self-update skipped (concurrent update in progress)" >&2; exit 0
fi
trap 'rm -f "$BOOTSTRAP_TMP" "${INSTALLED_BACKUP:-}"; rmdir "$LOCKDIR" 2>/dev/null' EXIT
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
```
