#!/usr/bin/env bash
# autospec-supervisor-install.sh — install/uninstall/status the boot unit that
# runs autospec-supervisor.sh once per boot (child 2 of
# docs/specs/2026-06-03-crash-resume-design.md §Interactivity / API shape).
#
# Platform selection (spec "AUTONOMOUS ASSUMPTION"):
#   darwin -> launchd user LaunchAgent (~/Library/LaunchAgents)
#   linux  -> systemd user unit         (~/.config/systemd/user)
#   else   -> `@reboot` crontab fallback
#
# All three install paths are IDEMPOTENT: a second `install` overwrites in place
# and leaves exactly one entry; `uninstall` removes it; `status` reports presence.
#
# Usage:
#   autospec-supervisor-install.sh install
#   autospec-supervisor-install.sh uninstall
#   autospec-supervisor-install.sh status
#
# Environment:
#   AUTOSPEC_SUPERVISOR_SH        path to autospec-supervisor.sh (auto-resolved)
#   AUTOSPEC_PLATFORM             override platform detection: darwin|linux|cron
#   AUTOSPEC_LAUNCH_AGENTS_DIR    override launchd dir (tests)
#   AUTOSPEC_SYSTEMD_USER_DIR     override systemd user dir (tests)

set -eu

LABEL="com.berlinguyinca.autospec.supervisor"
UNIT="autospec-supervisor"           # systemd unit base name
CRON_TAG="# autospec-supervisor"     # crontab marker line

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
STATE_DIR="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}"

LAUNCH_AGENTS_DIR="${AUTOSPEC_LAUNCH_AGENTS_DIR:-$HOME/Library/LaunchAgents}"
SYSTEMD_USER_DIR="${AUTOSPEC_SYSTEMD_USER_DIR:-$HOME/.config/systemd/user}"
PLIST_PATH="$LAUNCH_AGENTS_DIR/$LABEL.plist"
SERVICE_PATH="$SYSTEMD_USER_DIR/$UNIT.service"

err()  { printf 'autospec-supervisor-install: %s\n' "$1" >&2; }
die()  { err "$1"; exit 1; }
say()  { printf '%s\n' "$1"; }

usage() {
    cat <<'EOF'
Usage:
  autospec-supervisor-install.sh install
  autospec-supervisor-install.sh uninstall
  autospec-supervisor-install.sh status
EOF
}

# ── Resolve the supervisor entrypoint we install a boot unit for. ──────────────
resolve_supervisor() {
    if [ -n "${AUTOSPEC_SUPERVISOR_SH:-}" ] && [ -f "$AUTOSPEC_SUPERVISOR_SH" ]; then
        printf '%s' "$AUTOSPEC_SUPERVISOR_SH"; return
    fi
    for c in "$SCRIPT_DIR/autospec-supervisor.sh" "$STATE_DIR/scripts/autospec-supervisor.sh"; do
        [ -f "$c" ] && { printf '%s' "$c"; return; }
    done
    printf ''
}

detect_platform() {
    if [ -n "${AUTOSPEC_PLATFORM:-}" ]; then printf '%s' "$AUTOSPEC_PLATFORM"; return; fi
    case "$(uname -s 2>/dev/null || echo unknown)" in
        Darwin) printf 'darwin' ;;
        Linux)  printf 'linux' ;;
        *)      printf 'cron' ;;
    esac
}

# ── launchd (macOS) ────────────────────────────────────────────────────────────
launchd_install() {
    sup="$1"
    mkdir -p "$LAUNCH_AGENTS_DIR"
    tmp="$(mktemp "${PLIST_PATH}.XXXXXX")"
    cat > "$tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/bash</string>
        <string>$sup</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
EOF
    mv "$tmp" "$PLIST_PATH"       # atomic overwrite -> idempotent (single entry)
    if command -v launchctl >/dev/null 2>&1; then
        launchctl unload "$PLIST_PATH" >/dev/null 2>&1 || true
        launchctl load "$PLIST_PATH"  >/dev/null 2>&1 || true
    fi
    say "installed launchd agent: $PLIST_PATH"
}
launchd_uninstall() {
    if command -v launchctl >/dev/null 2>&1 && [ -f "$PLIST_PATH" ]; then
        launchctl unload "$PLIST_PATH" >/dev/null 2>&1 || true
    fi
    rm -f "$PLIST_PATH"
    say "uninstalled launchd agent: $PLIST_PATH"
}
launchd_status() {
    if [ -f "$PLIST_PATH" ]; then say "installed (launchd): $PLIST_PATH"; return 0; fi
    say "not installed (launchd)"; return 1
}

# ── systemd (Linux, user unit) ─────────────────────────────────────────────────
systemd_install() {
    sup="$1"
    mkdir -p "$SYSTEMD_USER_DIR"
    tmp="$(mktemp "${SERVICE_PATH}.XXXXXX")"
    cat > "$tmp" <<EOF
[Unit]
Description=autospec crash-resume boot supervisor
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/bin/bash $sup

[Install]
WantedBy=default.target
EOF
    mv "$tmp" "$SERVICE_PATH"     # atomic overwrite -> idempotent (single entry)
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
        systemctl --user enable "$UNIT.service" >/dev/null 2>&1 || true
    fi
    say "installed systemd user unit: $SERVICE_PATH"
}
systemd_uninstall() {
    if command -v systemctl >/dev/null 2>&1 && [ -f "$SERVICE_PATH" ]; then
        systemctl --user disable "$UNIT.service" >/dev/null 2>&1 || true
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
    rm -f "$SERVICE_PATH"
    say "uninstalled systemd user unit: $SERVICE_PATH"
}
systemd_status() {
    if [ -f "$SERVICE_PATH" ]; then say "installed (systemd): $SERVICE_PATH"; return 0; fi
    say "not installed (systemd)"; return 1
}

# ── @reboot crontab fallback ───────────────────────────────────────────────────
cron_line() {
    sup="$1"
    printf '@reboot /bin/bash %s >/dev/null 2>&1 %s\n' "$sup" "$CRON_TAG"
}
cron_current() { crontab -l 2>/dev/null || true; }
cron_without_ours() {
    # Drop any prior autospec-supervisor line so re-install stays idempotent.
    cron_current | grep -vF "$CRON_TAG" || true
}
cron_install() {
    sup="$1"
    command -v crontab >/dev/null 2>&1 || die "crontab not available for @reboot fallback"
    { cron_without_ours; cron_line "$sup"; } | crontab -
    say "installed @reboot cron entry for $sup"
}
cron_uninstall() {
    command -v crontab >/dev/null 2>&1 || { say "uninstalled (cron): crontab absent"; return 0; }
    remaining="$(cron_without_ours)"
    if [ -n "$remaining" ]; then
        printf '%s\n' "$remaining" | crontab -
    else
        crontab -r >/dev/null 2>&1 || true
    fi
    say "uninstalled @reboot cron entry"
}
cron_status() {
    if command -v crontab >/dev/null 2>&1 && cron_current | grep -qF "$CRON_TAG"; then
        say "installed (cron): @reboot entry present"; return 0
    fi
    say "not installed (cron)"; return 1
}

# ── Dispatch ───────────────────────────────────────────────────────────────────
action="${1:-}"
[ -n "$action" ] || { usage >&2; exit 1; }

platform="$(detect_platform)"

case "$action" in
    install)
        sup="$(resolve_supervisor)"
        [ -n "$sup" ] || die "autospec-supervisor.sh not found (set AUTOSPEC_SUPERVISOR_SH)"
        case "$platform" in
            darwin) launchd_install "$sup" ;;
            linux)  systemd_install "$sup" ;;
            *)      cron_install "$sup" ;;
        esac
        ;;
    uninstall)
        case "$platform" in
            darwin) launchd_uninstall ;;
            linux)  systemd_uninstall ;;
            *)      cron_uninstall ;;
        esac
        ;;
    status)
        case "$platform" in
            darwin) launchd_status ;;
            linux)  systemd_status ;;
            *)      cron_status ;;
        esac
        ;;
    --help|-h) usage ;;
    *) usage >&2; exit 1 ;;
esac
