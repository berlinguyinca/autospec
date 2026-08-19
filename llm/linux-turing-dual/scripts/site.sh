#!/usr/bin/env bash
# Load this site's own coordinates.
#
# This repository is public, so the node's address, its interface and the path
# its weights live on are NOT committed. They live in site.conf, and the
# committed example carries <angle-bracket> placeholders.
#
# site.conf uses the `: "${VAR:=value}"` form, which means an environment
# variable that is already set WINS over the file -- so a one-off
#
#     QT_PORT=9999 serve-router.sh router
#
# needs no file edit.
#
# This file is SOURCED. It therefore never calls `exit`, and sets no RETURN
# trap: both leak into the caller's shell. require_site() returns 78
# (EX_CONFIG) and callers use `require_site || exit $?`.

# Search order, first readable wins:
#   1. $QT_SITE_CONF          explicit override, and what the tests use
#   2. /etc/qwen-turing/      SYSTEM config -- this is the one the service reads
#   3. ~/.config/qwen-turing/ per-operator, for running the tools by hand
#
# The system path is not a nicety. The unit sets ProtectHome=true, so the service
# user cannot see /home at all; a config that lives only in an operator's home
# works from the shell and then fails inside systemd with an empty bind address.
if [ -z "${QT_SITE_CONF:-}" ]; then
  for _qt_cand in \
      "/etc/qwen-turing/site.conf" \
      "${XDG_CONFIG_HOME:-$HOME/.config}/qwen-turing/site.conf"; do
    if [ -r "$_qt_cand" ]; then
      QT_SITE_CONF="$_qt_cand"
      break
    fi
  done
  # Nothing readable: still name the system path, so the error is actionable.
  QT_SITE_CONF="${QT_SITE_CONF:-/etc/qwen-turing/site.conf}"
  unset _qt_cand
fi

if [ -r "$QT_SITE_CONF" ]; then
  # shellcheck disable=SC1090
  . "$QT_SITE_CONF"
fi

# Every value the node cannot start without.
# QT_DASH_ADDR is separate from QT_NODE_ADDR on purpose: the inference endpoint
# and the stats dashboard are exposed to different audiences, so they must be
# able to bind different interfaces rather than sharing one address.
QT_REQUIRED_VARS="QT_NODE_ADDR QT_UPLINK_IF QT_MODELS_DIR QT_PORT QT_DASH_ADDR QT_DASH_PORT"

require_site() {
  local missing="" v val
  for v in $QT_REQUIRED_VARS; do
    # ${!v} is the Bash 3.2-safe indirect lookup, and avoids eval -- these
    # values come from a file, and eval on file content is a command-injection
    # waiting to happen.
    val="${!v:-}"
    if [ -z "$val" ]; then
      missing="${missing} ${v}"
      continue
    fi
    case "$val" in
      *'<'*'>'*) missing="${missing} ${v}(placeholder)" ;;
    esac
  done

  if [ -n "$missing" ]; then
    echo "site.conf is incomplete:${missing}" >&2
    echo "  edit ${QT_SITE_CONF}" >&2
    echo "  (copy the template from config/site.conf.example)" >&2
    return 78
  fi

  export QT_NODE_ADDR QT_UPLINK_IF QT_MODELS_DIR QT_PORT QT_DASH_ADDR QT_DASH_PORT
  return 0
}
