#!/usr/bin/env bats
# ensure-tool.bats — Unit tests for ensure-tool.sh.
#
# Run: bats skills/autospec-shared/tests/unit/ensure-tool.bats
#
# Strategy: each test builds an isolated fake PATH containing only the stub
# binaries it wants the script to "see" (command -v resolves against PATH).
# Installer stubs (brew/apt-get/winget/choco/scoop/pipx/uv/pip3/npm) log their invocation to a
# file so we can assert which installer branch fired. No real installs happen.

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/ensure-tool.sh"

setup() {
  TMP_DIR="$(mktemp -d /tmp/autospec-ensuretool-XXXXXX)"
  BIN="$TMP_DIR/bin"
  mkdir -p "$BIN"
  LOG="$TMP_DIR/installer.log"
  export TMP_DIR BIN LOG
  # Keep a minimal real PATH for coreutils the script itself needs.
  REAL_PATH="$PATH"
  export REAL_PATH

  # Coreutils-only dir: symlink ONLY the basic commands ensure-tool.sh needs
  # (bash/env/tr/printf/id) so the script runs, while the target tool and all
  # installers stay genuinely absent unless we stub them into $BIN.
  CORE="$TMP_DIR/core"
  mkdir -p "$CORE"
  local c
  for c in bash env tr printf id cat dirname; do
    local p
    p="$(command -v "$c" 2>/dev/null)" && ln -sf "$p" "$CORE/$c"
  done
  export CORE
}

teardown() {
  rm -rf "$TMP_DIR"
}

# Create a stub installer that records "<name> <args>" to $LOG and exits 0.
mk_installer() {
  local name="$1" rc="${2:-0}"
  cat > "$BIN/$name" <<SHIM
#!/usr/bin/env bash
echo "$name \$*" >> "$LOG"
exit $rc
SHIM
  chmod +x "$BIN/$name"
}

# Control the privilege branch without invoking the host's real identity or
# sudo command. The package-manager implementation remains real shell code;
# only its process boundary is replaced.
mk_id() {
  local uid="$1"
  cat > "$BIN/id" <<SHIM
#!/usr/bin/env bash
[ "\${1:-}" = "-u" ] && printf '%s\n' '$uid'
SHIM
  chmod +x "$BIN/id"
}

mk_sudo() {
  cat > "$BIN/sudo" <<SHIM
#!/usr/bin/env bash
echo "sudo \$*" >> "$LOG"
exec "\$@"
SHIM
  chmod +x "$BIN/sudo"
}

# Create a stub of the target tool so command -v finds it.
mk_tool() {
  local name="$1"
  cat > "$BIN/$name" <<'SHIM'
#!/usr/bin/env bash
exit 0
SHIM
  chmod +x "$BIN/$name"
}

# Run ensure-tool.sh with PATH = our fake bin first, then the real PATH
# (so bash/cat/etc. still resolve). The script's own command -v probes see
# only what we put in $BIN ahead of everything.
run_ensure() {
  run env PATH="$BIN:$REAL_PATH" HOME="$TMP_DIR/home" bash "$SCRIPT" "$@"
}

# Run with the fake bin + a coreutils-only dir on PATH, so the target tool and
# every installer are genuinely absent unless we stubbed them into $BIN.
run_ensure_isolated() {
  run env PATH="$BIN:$CORE" HOME="$TMP_DIR/home" bash "$SCRIPT" "$@"
}

# ── No-op when tool already present ──────────────────────────────────────────

@test "bats present → no-op, no installer invoked" {
  mk_tool bats
  mk_installer brew
  run_ensure bats
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]
}

@test "jq present → no-op" {
  mk_tool jq
  mk_installer apt-get
  run_ensure jq
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]
}

@test "gh present → no-op" {
  mk_tool gh
  mk_installer brew
  run_ensure gh
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]
}

@test "mempalace present → no-op" {
  mk_tool mempalace
  mk_installer pipx
  run_ensure mempalace
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]
}

# ── Install branch fires when tool absent ────────────────────────────────────

@test "bats absent + brew available → installs via brew" {
  mk_installer brew
  run_ensure_isolated bats
  [ "$status" -eq 0 ]
  grep -q "brew .*bats" "$LOG"
}

@test "jq absent + brew available → installs via brew" {
  mk_installer brew
  run_ensure_isolated jq
  [ "$status" -eq 0 ]
  grep -q "brew .*jq" "$LOG"
}

@test "gh absent + brew available → installs via brew" {
  mk_installer brew
  run_ensure_isolated gh
  [ "$status" -eq 0 ]
  grep -q "brew .*gh" "$LOG"
}

@test "mempalace absent + pipx available → installs via pipx" {
  mk_installer pipx
  run_ensure_isolated mempalace
  [ "$status" -eq 0 ]
  grep -q "pipx .*mempalace" "$LOG"
}

# ── Fallback chain (no brew, apt-get available) ──────────────────────────────

@test "bats absent + no brew + apt-get available → installs via apt-get" {
  mk_installer apt-get
  run_ensure_isolated bats
  [ "$status" -eq 0 ]
  grep -q "apt-get .*bats" "$LOG"
}

@test "cargo absent + apt + non-root uses sudo to install cargo and rustc" {
  mk_id 1000
  mk_sudo
  mk_installer apt-get
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "sudo apt-get install -y cargo rustc" "$LOG"
  grep -q "apt-get install -y cargo rustc" "$LOG"
}

@test "cargo absent + apt + root installs without sudo" {
  mk_id 0
  mk_installer apt-get
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "apt-get install -y cargo rustc" "$LOG"
  ! grep -q '^sudo ' "$LOG"
}

@test "cargo absent + winget installs Rustup" {
  mk_installer winget
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "winget install --id Rustlang.Rustup" "$LOG"
}

@test "python3 absent + winget installs Python" {
  mk_installer winget
  run_ensure_isolated python3
  [ "$status" -eq 0 ]
  grep -q "winget install --id Python.Python.3.12" "$LOG"
}

@test "cargo absent + apt + non-root without sudo leaves failure to strict verifier" {
  mk_id 1000
  mk_installer apt-get 1
  run_ensure_isolated cargo
  [ "$status" -eq 0 ]
  grep -q "apt-get install -y cargo rustc" "$LOG"
  ! grep -q '^sudo ' "$LOG"
}

@test "yq absent + brew available → installs via brew" {
  mk_installer brew
  run_ensure_isolated yq
  [ "$status" -eq 0 ]
  grep -q "brew .*yq" "$LOG"
}

@test "node absent + winget available → installs Node.js LTS via winget" {
  mk_installer winget
  run_ensure_isolated node
  [ "$status" -eq 0 ]
  grep -q "winget .*OpenJS.NodeJS.LTS" "$LOG"
}

@test "codex absent + npm available → installs OpenAI Codex CLI via npm" {
  mk_installer npm
  run_ensure_isolated codex
  [ "$status" -eq 0 ]
  grep -q "npm .*@openai/codex" "$LOG"
}

@test "claude absent + npm available → installs Claude Code via npm" {
  mk_installer npm
  run_ensure_isolated claude
  [ "$status" -eq 0 ]
  grep -q "npm .*@anthropic-ai/claude-code" "$LOG"
}

@test "opencode absent + npm available → installs OpenCode via npm" {
  mk_installer npm
  run_ensure_isolated opencode
  [ "$status" -eq 0 ]
  grep -q "npm .*opencode-ai" "$LOG"
}

@test "omx absent + npm available → installs oh-my-codex via npm" {
  mk_installer npm
  run_ensure_isolated omx
  [ "$status" -eq 0 ]
  grep -q "npm .*oh-my-codex" "$LOG"
}

@test "omc absent + npm available → installs oh-my-claude via npm" {
  mk_installer npm
  run_ensure_isolated omc
  [ "$status" -eq 0 ]
  grep -q "npm .*oh-my-claude-sisyphus" "$LOG"
}

@test "oh-my-opencode absent + npm available → installs oh-my-opencode via npm" {
  mk_installer npm
  run_ensure_isolated oh-my-opencode
  [ "$status" -eq 0 ]
  grep -q "npm .*oh-my-opencode" "$LOG"
}

# ── mempalace pip fallback chain ─────────────────────────────────────────────

@test "mempalace absent + no pipx + uv available → installs via uv" {
  mk_installer uv
  run_ensure_isolated mempalace
  [ "$status" -eq 0 ]
  grep -q "uv .*mempalace" "$LOG"
}

@test "mempalace absent + only pip3 available → installs via pip3" {
  mk_installer pip3
  run_ensure_isolated mempalace
  [ "$status" -eq 0 ]
  grep -q "pip3 .*mempalace" "$LOG"
}

# ── Best-effort silent failure: installer fails, exit still 0 ────────────────

@test "installer fails (rc=1) → ensure-tool still exits 0" {
  mk_installer brew 1
  run_ensure_isolated bats
  [ "$status" -eq 0 ]
}

@test "no installer available at all → exit 0 silently" {
  # $BIN has nothing; tool absent and no installer present
  run_ensure_isolated bats
  [ "$status" -eq 0 ]
}

# ── Opt-out env vars ──────────────────────────────────────────────────────────

@test "AUTOSPEC_SKIP_ENSURE_TOOL=1 → no install attempted" {
  mk_installer brew
  run env PATH="$BIN:$CORE" HOME="$TMP_DIR/home" AUTOSPEC_SKIP_ENSURE_TOOL=1 bash "$SCRIPT" bats
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]
}

@test "AUTOSPEC_SKIP_ENSURE_TOOL_BATS=1 → bats skipped, jq still installs" {
  mk_installer brew
  run env PATH="$BIN:$CORE" HOME="$TMP_DIR/home" AUTOSPEC_SKIP_ENSURE_TOOL_BATS=1 bash "$SCRIPT" bats
  [ "$status" -eq 0 ]
  [ ! -f "$LOG" ]

  # Same per-tool opt-out must NOT block a different tool
  run env PATH="$BIN:$CORE" HOME="$TMP_DIR/home" AUTOSPEC_SKIP_ENSURE_TOOL_BATS=1 bash "$SCRIPT" jq
  [ "$status" -eq 0 ]
  grep -q "brew .*jq" "$LOG"
}

# ── Unknown tool ──────────────────────────────────────────────────────────────

@test "unknown tool → exit 0 silently (no table entry, best-effort)" {
  run_ensure_isolated some-unknown-tool-xyz
  [ "$status" -eq 0 ]
}

# ── Missing argument ──────────────────────────────────────────────────────────

@test "no argument → exit 0 (no-op)" {
  run_ensure_isolated
  [ "$status" -eq 0 ]
}
