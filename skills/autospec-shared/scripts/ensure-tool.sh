#!/usr/bin/env bash
# ensure-tool.sh — Best-effort, idempotent installer for autospec's optional CLI deps.
#
# Generalizes the _ensure_mempalace_installed helper (PR #509) into a reusable
# tool-installer with a baked-in tool table and a platform-appropriate fallback
# chain (brew / apt-get / npm / pipx / uv / pip). Every install is a silent
# best-effort: a missing or failed installer never blocks the caller.
#
# Usage:
#   ensure-tool.sh <tool>          # ensure <tool> is on PATH; install if absent
#
# Supported tools (baked-in table): bats, jq, gh, mempalace
# Unknown tools are a silent no-op (exit 0).
#
# Exit codes:
#   0 always (best-effort by design — see "## ROI-check" / project rules)
#
# Environment:
#   AUTOSPEC_SKIP_ENSURE_TOOL=1          — disable ALL auto-installs
#   AUTOSPEC_SKIP_ENSURE_TOOL_<TOOL>=1   — disable auto-install for one tool
#                                          (<TOOL> uppercased, non-alnum → _)
#
# Requires: bash 3.2+, command. Installers are probed, never assumed present.

set +e

TOOL="${1:-}"

# No tool requested → nothing to do.
[ -n "$TOOL" ] || exit 0

# ── Opt-out gates ─────────────────────────────────────────────────────────────
# Global opt-out.
[ "${AUTOSPEC_SKIP_ENSURE_TOOL:-0}" = "1" ] && exit 0

# Per-tool opt-out: AUTOSPEC_SKIP_ENSURE_TOOL_<TOOL> with <TOOL> uppercased and
# any non-alphanumeric char replaced by underscore (e.g. some-tool → SOME_TOOL).
_tool_upper=$(printf '%s' "$TOOL" | tr '[:lower:]' '[:upper:]' | tr -c 'A-Z0-9' '_')
_skip_var="AUTOSPEC_SKIP_ENSURE_TOOL_${_tool_upper}"
# Strip a possible trailing underscore left by tr on the final char.
_skip_var="${_skip_var%_}"
eval "_skip_val=\${${_skip_var}:-0}"
[ "$_skip_val" = "1" ] && exit 0

# ── Already present → no-op ───────────────────────────────────────────────────
if command -v "$TOOL" > /dev/null 2>&1; then
  exit 0
fi

# ── Installer helpers (each is silent best-effort) ────────────────────────────
# A helper logs a one-line success notice to stderr and returns 0 on success so
# the caller's chain can short-circuit; returns 1 if its installer is absent or
# the install failed.

_try_brew() {  # _try_brew <pkg>
  command -v brew > /dev/null 2>&1 || return 1
  brew install "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via brew" >&2; return 0; }
  return 1
}

_try_apt() {  # _try_apt <pkg>
  command -v apt-get > /dev/null 2>&1 || return 1
  # -y non-interactive; sudo only if not already root and available.
  local sudo=""
  if [ "$(id -u 2>/dev/null || echo 1)" != "0" ] && command -v sudo > /dev/null 2>&1; then
    sudo="sudo"
  fi
  $sudo apt-get install -y "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via apt-get" >&2; return 0; }
  return 1
}

_try_npm() {  # _try_npm <pkg>
  command -v npm > /dev/null 2>&1 || return 1
  npm install -g "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via npm" >&2; return 0; }
  return 1
}

_try_pipx() {  # _try_pipx <pkg>
  command -v pipx > /dev/null 2>&1 || return 1
  pipx install "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via pipx" >&2; return 0; }
  return 1
}

_try_uv() {  # _try_uv <pkg>
  command -v uv > /dev/null 2>&1 || return 1
  uv tool install "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via uv" >&2; return 0; }
  return 1
}

_try_pip() {  # _try_pip <pkg>
  if command -v pip3 > /dev/null 2>&1; then
    pip3 install --user --quiet "$1" > /dev/null 2>&1 \
      && { echo "ensure-tool: installed $TOOL via pip3 --user" >&2; return 0; }
  elif command -v pip > /dev/null 2>&1; then
    pip install --user --quiet "$1" > /dev/null 2>&1 \
      && { echo "ensure-tool: installed $TOOL via pip --user" >&2; return 0; }
  fi
  return 1
}

# ── Baked-in tool table → ordered installer chain ────────────────────────────
# Each case tries installers in preference order; first success wins. Package
# name may differ from the command name (kept explicit per installer).
case "$TOOL" in
  bats)
    _try_brew bats-core || _try_apt bats || _try_npm bats || true
    ;;
  jq)
    _try_brew jq || _try_apt jq || true
    ;;
  gh)
    _try_brew gh || _try_apt gh || true
    ;;
  mempalace)
    # Python tool: pipx (isolated venv) → uv → pip --user. Mirrors PR #509.
    _try_pipx mempalace || _try_uv mempalace || _try_pip mempalace || true
    ;;
  *)
    # Unknown tool: no table entry. Best-effort silent no-op.
    : ;;
esac

# Always succeed: absence of an optional tool must never block the caller.
exit 0
