#!/usr/bin/env bash
# ensure-tool.sh — Best-effort, idempotent installer for autospec's optional CLI deps.
#
# Generalizes the _ensure_mempalace_installed helper (PR #509) into a reusable
# tool-installer with a baked-in tool table and a platform-appropriate fallback
# chain (brew / apt/dnf/yum/pacman/apk / winget/choco/scoop / npm / pipx / uv /
# pip). Every install is a silent best-effort: a missing or failed installer
# never blocks the caller.
#
# Usage:
#   ensure-tool.sh <tool>          # ensure <tool> is on PATH; install if absent
#
# Supported tools (baked-in table): ajv, bash, bats, bun, claude, codex, curl,
# gh, git, gitleaks, jq, license-checker, mempalace, node, npm, omc, omx,
# oh-my-opencode, opencode, pipx, python3, semgrep, trivy, uv, yq
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

_sudo_cmd() {
  if [ "$(id -u 2>/dev/null || echo 1)" != "0" ] && command -v sudo > /dev/null 2>&1; then
    printf 'sudo'
  fi
}

_try_brew() {  # _try_brew <pkg...>
  command -v brew > /dev/null 2>&1 || return 1
  brew install "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via brew" >&2; return 0; }
  return 1
}

_try_apt() {  # _try_apt <pkg...>
  command -v apt-get > /dev/null 2>&1 || return 1
  local sudo
  sudo="$(_sudo_cmd)"
  $sudo apt-get install -y "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via apt-get" >&2; return 0; }
  return 1
}

_try_dnf() {  # _try_dnf <pkg...>
  command -v dnf > /dev/null 2>&1 || return 1
  local sudo
  sudo="$(_sudo_cmd)"
  $sudo dnf install -y "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via dnf" >&2; return 0; }
  return 1
}

_try_yum() {  # _try_yum <pkg...>
  command -v yum > /dev/null 2>&1 || return 1
  local sudo
  sudo="$(_sudo_cmd)"
  $sudo yum install -y "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via yum" >&2; return 0; }
  return 1
}

_try_pacman() {  # _try_pacman <pkg...>
  command -v pacman > /dev/null 2>&1 || return 1
  local sudo
  sudo="$(_sudo_cmd)"
  $sudo pacman -Sy --noconfirm "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via pacman" >&2; return 0; }
  return 1
}

_try_apk() {  # _try_apk <pkg...>
  command -v apk > /dev/null 2>&1 || return 1
  local sudo
  sudo="$(_sudo_cmd)"
  $sudo apk add --no-cache "$@" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via apk" >&2; return 0; }
  return 1
}

_try_winget() {  # _try_winget <id>
  command -v winget > /dev/null 2>&1 || return 1
  winget install --id "$1" -e --accept-package-agreements --accept-source-agreements > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via winget" >&2; return 0; }
  return 1
}

_try_choco() {  # _try_choco <pkg>
  command -v choco > /dev/null 2>&1 || return 1
  choco install -y "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via choco" >&2; return 0; }
  return 1
}

_try_scoop() {  # _try_scoop <pkg>
  command -v scoop > /dev/null 2>&1 || return 1
  scoop install "$1" > /dev/null 2>&1 \
    && { echo "ensure-tool: installed $TOOL via scoop" >&2; return 0; }
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
  ajv)
    _try_npm ajv-cli || true
    ;;
  bash)
    _try_brew bash || _try_apt bash || _try_dnf bash || _try_yum bash || _try_pacman bash || _try_apk bash \
      || _try_winget Git.Git || _try_choco git || _try_scoop git || true
    ;;
  bats)
    _try_brew bats-core || _try_apt bats || _try_dnf bats || _try_yum bats || _try_pacman bats || _try_apk bats \
      || _try_choco bats || _try_scoop bats || _try_npm bats || true
    ;;
  bun)
    _try_brew bun || _try_winget Oven-sh.Bun || _try_choco bun || _try_scoop bun || true
    ;;
  claude)
    _try_npm @anthropic-ai/claude-code || true
    ;;
  codex)
    _try_npm @openai/codex || true
    ;;
  curl)
    _try_brew curl || _try_apt curl || _try_dnf curl || _try_yum curl || _try_pacman curl || _try_apk curl \
      || _try_winget cURL.cURL || _try_choco curl || _try_scoop curl || true
    ;;
  jq)
    _try_brew jq || _try_apt jq || _try_dnf jq || _try_yum jq || _try_pacman jq || _try_apk jq \
      || _try_winget jqlang.jq || _try_choco jq || _try_scoop jq || true
    ;;
  gh)
    _try_brew gh || _try_apt gh || _try_dnf gh || _try_yum gh || _try_pacman github-cli || _try_apk github-cli \
      || _try_winget GitHub.cli || _try_choco gh || _try_scoop gh || true
    ;;
  git)
    _try_brew git || _try_apt git || _try_dnf git || _try_yum git || _try_pacman git || _try_apk git \
      || _try_winget Git.Git || _try_choco git || _try_scoop git || true
    ;;
  gitleaks)
    _try_brew gitleaks || _try_winget gitleaks.gitleaks || _try_choco gitleaks || _try_scoop gitleaks || true
    ;;
  license-checker)
    _try_npm license-checker || true
    ;;
  mempalace)
    # Python tool: pipx (isolated venv) → uv → pip --user. Mirrors PR #509.
    _try_pipx mempalace || _try_uv mempalace || _try_pip mempalace || true
    ;;
  node|npm)
    _try_brew node || _try_apt nodejs npm || _try_dnf nodejs npm || _try_yum nodejs npm || _try_pacman nodejs npm \
      || _try_apk nodejs npm || _try_winget OpenJS.NodeJS.LTS || _try_choco nodejs-lts || _try_scoop nodejs-lts || true
    ;;
  oh-my-opencode)
    _try_npm oh-my-opencode || true
    ;;
  omc)
    _try_npm oh-my-claude-sisyphus || true
    ;;
  omx)
    _try_npm oh-my-codex || true
    ;;
  opencode)
    _try_npm opencode-ai || true
    ;;
  pipx)
    _try_brew pipx || _try_apt pipx || _try_dnf pipx || _try_yum pipx || _try_pacman python-pipx || _try_apk pipx \
      || _try_choco pipx || _try_scoop pipx || true
    ;;
  python3)
    _try_brew python || _try_apt python3 python3-pip || _try_dnf python3 python3-pip || _try_yum python3 python3-pip \
      || _try_pacman python python-pip || _try_apk python3 py3-pip || _try_winget Python.Python.3.12 \
      || _try_choco python || _try_scoop python || true
    ;;
  semgrep)
    # Python tool: pipx (isolated venv) → uv → pip --user → brew.
    _try_pipx semgrep || _try_uv semgrep || _try_pip semgrep || _try_brew semgrep || true
    ;;
  trivy)
    _try_brew trivy || _try_winget AquaSecurity.Trivy || _try_choco trivy || _try_scoop trivy || true
    ;;
  uv)
    _try_brew uv || _try_pipx uv || _try_pip uv || _try_choco uv || _try_scoop uv || true
    ;;
  yq)
    _try_brew yq || _try_apt yq || _try_dnf yq || _try_yum yq || _try_pacman yq || _try_apk yq \
      || _try_winget MikeFarah.yq || _try_choco yq || _try_scoop yq || true
    ;;
  *)
    # Unknown tool: no table entry. Best-effort silent no-op.
    : ;;
esac

# Always succeed: absence of an optional tool must never block the caller.
exit 0
