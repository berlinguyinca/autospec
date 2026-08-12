#!/usr/bin/env bash
# Reusable install helpers for the autonomous operator command wrappers.
# Source this file; do not execute.

write_autonomous_operator_wrapper() {
    target="$1"
    subcommand="$2"
    rust_subcommand="$subcommand"

    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf '%s\n' 'set -eu'
        printf '%s\n' 'AUTOSPEC_WRAPPER_BIN_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"'
        printf '%s\n' 'case ":$PATH:" in'
        printf '%s\n' '    *":$AUTOSPEC_WRAPPER_BIN_DIR:"*) ;;'
        printf '%s\n' '    *) PATH="$AUTOSPEC_WRAPPER_BIN_DIR:$PATH"; export PATH ;;'
        printf '%s\n' 'esac'
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
                "$HOME/.autospec/"*)
                    [ -e "$exec_target" ] || return 0
                    ;;
                *)
                    return 0
                    ;;
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
            if [ "$subcommand" = "$command" ]; then
                subcommand=""
            fi
            write_autonomous_operator_wrapper "$target" "$subcommand"
            info "heal_autonomous_operator_wrappers: healed $target (old exec target: ${old_target:-unknown})"
            healed=$((healed + 1))
        fi
    done

    if [ "$healed" -gt 0 ]; then
        info "heal_autonomous_operator_wrappers: healed $healed autonomous wrapper(s)"
    fi
}

install_autonomous_operator_commands() {
    autospec_bin_dir="$HOME/.autospec/bin"
    autospec_scripts_dir="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
    launcher="$autospec_scripts_dir/autospec-autonomous.sh"
    canonical_launcher="$HOME/.autospec/scripts/autospec-autonomous.sh"

    if [ "$DRY_RUN" -eq 1 ]; then
        info "[dry-run] install_autonomous_operator_commands: would install autospec-autonomous command wrappers in $autospec_bin_dir"
        return 0
    fi

    case "$autospec_scripts_dir/" in
        "$HOME/.autospec/"*) ;;
        *)
            warn "install_autonomous_operator_commands: ignoring non-persistent AUTOSPEC_SCRIPTS_DIR=$autospec_scripts_dir for wrapper exec target; wrappers resolve at runtime via \${AUTOSPEC_SCRIPTS_DIR:-\$HOME/.autospec/scripts}"
            launcher="$canonical_launcher"
            ;;
    esac

    if [ ! -f "$launcher" ]; then
        warn "install_autonomous_operator_commands: launcher not present yet at $launcher; writing runtime-resolving wrappers anyway"
    fi

    mkdir -p "$autospec_bin_dir"
    heal_autonomous_operator_wrappers
    [ -f "$launcher" ] && chmod +x "$launcher"
    for command in autospec-autonomous autospec-autonomous-start autospec-autonomous-status autospec-autonomous-list autospec-autonomous-timeline autospec-autonomous-monitor autospec-autonomous-supervise autospec-autonomous-logs autospec-autonomous-watch autospec-autonomous-cleanup autospec-autonomous-stop autospec-autonomous-restart; do
        target="$autospec_bin_dir/$command"
        subcommand="${command#autospec-autonomous-}"
        if [ "$subcommand" = "$command" ]; then
            subcommand=""
        fi
        write_autonomous_operator_wrapper "$target" "$subcommand"
    done
    info "install_autonomous_operator_commands: installed autonomous command wrappers in $autospec_bin_dir"
}
