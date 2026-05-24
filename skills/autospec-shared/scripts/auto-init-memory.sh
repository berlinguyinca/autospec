#!/usr/bin/env bash
# auto-init-memory.sh — Auto-migrate CC memory to in-repo location on first invocation per repo.
#
# Idempotent: subsequent runs are no-op probes (fast-path < 50ms).
#
# Usage:
#   auto-init-memory.sh              # normal run
#   auto-init-memory.sh --rollback   # restore most-recent .pre-migration-* backup
#
# Exit codes:
#   0 always (git failures are warned but non-blocking)
#
# Environment:
#   AUTOSPEC_SCRIPTS_DIR  — directory containing sibling scripts (default: script dir)
#   HOME                  — used to compute CC_PATH (~/.claude/projects/<slug>/memory)
#
# Requires: bash 3.2+, git, ln, cp, mv, mkdir
# Optional: gh CLI (for REPO_SLUG; exits 0 silently if absent), rsync (for case 11)

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"

# ── Helpers ───────────────────────────────────────────────────────────────────

_auto_commit() {
  local msg="$1"
  local repo_root="$2"
  # Best-effort: warn on failure, never block
  if ! git -C "$repo_root" add -- docs/memory/ AGENTS.md 2>/dev/null; then
    echo "auto-init-memory: git add failed (continuing)" >&2
    return 0
  fi
  if ! git -C "$repo_root" diff --cached --quiet 2>/dev/null; then
    if ! git -C "$repo_root" commit -m "$msg" 2>/dev/null; then
      echo "auto-init-memory: git commit failed — files staged for manual commit" >&2
    fi
  fi
  return 0
}

_ensure_agents_md_inventory_section() {
  local agents_md="$1"
  # Idempotent: skip if block already present
  if [ -f "$agents_md" ] && grep -q "^## Memory inventory" "$agents_md" 2>/dev/null; then
    return 0
  fi
  # Create file if missing, then append the block
  cat >> "$agents_md" <<'AGENTS_BLOCK'

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is auto-installed on first run via pipx/uv/pip (opt out: `AUTOSPEC_SKIP_MEMPALACE_INSTALL=1`).
AGENTS_BLOCK
  return 0
}

_ensure_mempalace_installed() {
  # Best-effort install of mempalace CLI on first migration. Idempotent — no-op
  # if already on PATH. Opt out with AUTOSPEC_SKIP_MEMPALACE_INSTALL=1.
  [ "${AUTOSPEC_SKIP_MEMPALACE_INSTALL:-0}" = "1" ] && return 0
  command -v mempalace > /dev/null 2>&1 && return 0

  # Prefer pipx (isolated venv per tool), then uv, then pip --user. All silent
  # failures — mempalace is optional; absence only loses the KG/search overlay.
  if command -v pipx > /dev/null 2>&1; then
    pipx install mempalace > /dev/null 2>&1 && \
      echo "auto-init-memory: installed mempalace via pipx" >&2
  elif command -v uv > /dev/null 2>&1; then
    uv tool install mempalace > /dev/null 2>&1 && \
      echo "auto-init-memory: installed mempalace via uv" >&2
  elif command -v pip3 > /dev/null 2>&1; then
    pip3 install --user --quiet mempalace > /dev/null 2>&1 && \
      echo "auto-init-memory: installed mempalace via pip3 --user" >&2
  elif command -v pip > /dev/null 2>&1; then
    pip install --user --quiet mempalace > /dev/null 2>&1 && \
      echo "auto-init-memory: installed mempalace via pip --user" >&2
  fi
  return 0
}

_rollback() {
  local cc_path="$1"
  # Find most recent backup
  local backup
  backup=$(ls -dt "${cc_path}.pre-migration-"* 2>/dev/null | head -1)
  if [ -z "$backup" ]; then
    echo "auto-init-memory: no backup found at ${cc_path}.pre-migration-*" >&2
    return 0
  fi
  # Remove symlink if present
  if [ -L "$cc_path" ]; then
    rm "$cc_path"
  fi
  # Restore backup
  mv "$backup" "$cc_path"
  echo "auto-init-memory: rolled back to $backup" >&2
  return 0
}

# ── Main ──────────────────────────────────────────────────────────────────────

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)
if [ -z "$REPO_ROOT" ]; then
  exit 0
fi

# CC_PATH: ~/.claude/projects/<repo-root-with-slashes-replaced-by-dashes>/memory
# (matches Claude Code's own path convention)
CC_SLUG=$(echo "$REPO_ROOT" | tr '/' '-')
CC_PATH="$HOME/.claude/projects/${CC_SLUG}/memory"
REPO_PATH="$REPO_ROOT/docs/memory"

# ── Rollback mode ─────────────────────────────────────────────────────────────
if [ "${1:-}" = "--rollback" ]; then
  _rollback "$CC_PATH"
  exit 0
fi

# ── Fast-path: symlink already correct → no-op ────────────────────────────────
if [ -L "$CC_PATH" ] && [ "$(readlink "$CC_PATH")" = "$REPO_PATH" ]; then
  exit 0
fi

# ── Compute state bits ────────────────────────────────────────────────────────
if [ -d "$CC_PATH" ] && [ -n "$(ls -A "$CC_PATH"/*.md 2>/dev/null)" ]; then
  CC_HAS_FILES=1
else
  CC_HAS_FILES=0
fi

if [ -d "$REPO_PATH" ]; then
  REPO_HAS_DIR=1
else
  REPO_HAS_DIR=0
fi

# ── Ensure mempalace CLI (best-effort, optional KG overlay) ──────────────────
_ensure_mempalace_installed

# ── State matrix ──────────────────────────────────────────────────────────────
BACKUP_SUFFIX="pre-migration-$(date +%Y%m%d-%H%M%S)"

case "${CC_HAS_FILES}${REPO_HAS_DIR}" in
  10)
    # CC has memory files; in-repo dir missing → migrate + symlink
    mkdir -p "$REPO_PATH"
    cp -p "$CC_PATH"/*.md "$REPO_PATH"/
    mv "$CC_PATH" "${CC_PATH}.${BACKUP_SUFFIX}"
    ln -s "$REPO_PATH" "$CC_PATH"
    # Stub: mempalace-mine (invoked by M3; skipped here if not present)
    if [ -x "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" ]; then
      bash "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" --root "$REPO_PATH" --quiet
    fi
    _auto_commit "feat: auto-migrate cross-tool memory to docs/memory/" "$REPO_ROOT"
    ;;
  01)
    # In-repo dir exists (e.g. from teammate's commit); CC memory missing → symlink only
    mkdir -p "$(dirname "$CC_PATH")"
    ln -s "$REPO_PATH" "$CC_PATH"
    ;;
  11)
    # Both exist → sync (repo wins on dup via --ignore-existing) + symlink
    rsync -a --ignore-existing "$CC_PATH"/*.md "$REPO_PATH"/ 2>/dev/null || \
      cp -n "$CC_PATH"/*.md "$REPO_PATH"/ 2>/dev/null || true
    mv "$CC_PATH" "${CC_PATH}.${BACKUP_SUFFIX}"
    ln -s "$REPO_PATH" "$CC_PATH"
    # Stub: mempalace-mine (invoked by M3; skipped here if not present)
    if [ -x "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" ]; then
      bash "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" --root "$REPO_PATH" --quiet
    fi
    _auto_commit "feat: auto-migrate cross-tool memory to docs/memory/ (merge)" "$REPO_ROOT"
    ;;
  00)
    # Neither exists → create empty in-repo dir + symlink
    mkdir -p "$REPO_PATH" "$(dirname "$CC_PATH")"
    touch "$REPO_PATH/MEMORY.md"
    ln -s "$REPO_PATH" "$CC_PATH"
    ;;
esac

# ── Ensure AGENTS.md inventory section ───────────────────────────────────────
_ensure_agents_md_inventory_section "$REPO_ROOT/AGENTS.md"

# ── Lazy AAAK compression gate ───────────────────────────────────────────────
# Trigger once per N invocations (default: every 10) to keep memory LOC small.
# Falls through silently if mempalace-compress.sh is absent or mempalace is not installed.
if [ -x "${AUTOSPEC_SCRIPTS_DIR}/mempalace-compress.sh" ]; then
  _invoke_counter_file="$HOME/.autospec/auto-init-memory-count"
  _invoke_count=$(cat "$_invoke_counter_file" 2>/dev/null || echo 0)
  _invoke_count=$(( _invoke_count + 1 ))
  printf '%s\n' "$_invoke_count" > "$_invoke_counter_file"
  _compress_every="${AUTOSPEC_COMPRESS_EVERY:-10}"
  if [ "$(( _invoke_count % _compress_every ))" -eq 0 ]; then
    bash "${AUTOSPEC_SCRIPTS_DIR}/mempalace-compress.sh" \
      --dir "$REPO_PATH" \
      --threshold "${AUTOSPEC_COMPRESS_THRESHOLD:-5000}" \
      --quiet
  fi
fi

exit 0
