# Autospec Cross-Tool Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Decomposition:** 5 sequential phases. Each phase = one GitHub issue for `/autospec-split`. M1+M2+M3 ship in parallel (no inter-deps); M4 depends on all three (lockstep-replication wires the SKILL.md startup block to call the new helper); M5 layers smoke + rollback.

**Goal:** Ship the cross-tool memory extension so the autospec trio (Claude Code + Codex CLI + OpenCode) share the same `docs/memory/` directory via per-tool native mechanisms, with mempalace MCP server providing optional rich queries on top. Auto-init handles migration + symlink + AGENTS.md update on first skill invocation — zero manual intervention.

**Architecture:** Filesystem floor at `<repo>/docs/memory/`; CC's auto-memory path becomes a symlink to it; Codex/OpenCode see the memory via a new `## Memory inventory` block in `AGENTS.md`; mempalace MCP server indexes the floor for semantic search / KG / tunnels / diary. Every autospec skill's startup-self-update block gains one line that calls `auto-init-memory.sh`, making the integration self-bootstrapping per repo.

**Tech Stack:** Bash 4+, `gh` CLI, `git`, `ln -s` (Unix-only), mempalace CLI (optional MCP layer), bats for tests. No new language runtimes.

**Spec reference:** `docs/specs/2026-05-23-autospec-cross-tool-memory-design.md` (merged via PR #496, commit `d15cbd9`).

---

## File Structure (locked across phases)

```
skills/autospec-shared/scripts/
  auto-init-memory.sh                    # Phase M1 (new — core state-matrix + symlink + commit)
  mempalace-mine.sh                      # Phase M3 (new — thin wrapper around mempalace + inference)

skills/autospec-define/SKILL.md          # Phase M4 (modify: add bash auto-init-memory.sh line in startup block)
skills/autospec-define/codex/prompt.md   # Phase M4 (lockstep mirror)
skills/autospec-define/opencode/agent.md # Phase M4 (lockstep mirror)
skills/autospec-run/...                  # Phase M4 (same trio update)
skills/autospec-test/...                 # Phase M4
skills/autospec-split/...                # Phase M4
skills/autospec-classify/...             # Phase M4
skills/autospec-review/...               # Phase M4
skills/autospec-listen/...               # Phase M4
skills/autospec-story/...                # Phase M4
skills/autospec-stop/...                 # Phase M4
skills/autospec-e2e-clone/...            # Phase M4

skills/autospec-shared/tests/
  unit/
    auto-init-memory.bats                # Phase M1 (per state-matrix case + fast-path)
    agents-md-inventory.bats             # Phase M2 (idempotent append)
    mempalace-mine.bats                  # Phase M3 (inference rules per existing memory file type)
  integration/
    cross-tool-roundtrip.bats            # Phase M5 (CC writes → Codex reads → OpenCode reads)
    rollback.bats                        # Phase M5 (--rollback restores cleanly)

AGENTS.md                                # Phase M2 (auto-appended Memory inventory block on first run)
```

---

## Phase M1 — `auto-init-memory.sh` core

**GH issue title:** `feat(memory): auto-init-memory.sh state matrix + symlink (M1)`
**Depends on:** none

### Files
- Create: `skills/autospec-shared/scripts/auto-init-memory.sh`
- Create: `skills/autospec-shared/tests/unit/auto-init-memory.bats`

### Tasks

- [ ] **M1.1** Write the failing bats test asserting the 5 state-matrix cases (already-symlinked / 10 CC-has-files-only / 01 in-repo-only / 11 both-exist / 00 neither). Use a tmpdir-rooted fixture repo for each case.

- [ ] **M1.2** Run: `bats skills/autospec-shared/tests/unit/auto-init-memory.bats` — expect FAIL (script missing).

- [ ] **M1.3** Implement `auto-init-memory.sh` per spec §4a verbatim:
  - Detect repo via `git rev-parse --show-toplevel`; exit 0 if not a git repo
  - Compute `CC_PATH` from `$HOME/.claude/projects/<slug>` (slug = repo path with `/` → `-`)
  - Fast-path: if symlink already correct, exit 0 in <50ms
  - Branch on 4 states; each branch: mkdir / cp / mv-backup / ln -s as specified
  - Best-effort `_auto_commit` helper that warns-but-doesn't-block on git failure
  - Set executable bit (`chmod +x`)

- [ ] **M1.4** Run bats; expect PASS for all 5 cases.

- [ ] **M1.5** Add bats case for repo-without-gh-remote — script exits 0 silently, no symlink touched.

- [ ] **M1.6** Add bats case for failed `_auto_commit` — script still creates the symlink, stderr warning is logged, exit 0.

- [ ] **M1.7** Commit:
```bash
git add skills/autospec-shared/scripts/auto-init-memory.sh skills/autospec-shared/tests/unit/auto-init-memory.bats
git commit -m "feat(memory): auto-init-memory.sh state matrix + symlink (closes #<issue>)"
```

### Acceptance criteria
- [ ] All bats cases pass
- [ ] Fast-path runs <50ms (measure via `time bash auto-init-memory.sh` on a fixture with the symlink already in place)
- [ ] Script never blocks on git failures
- [ ] `validate.sh` passes

---

## Phase M2 — AGENTS.md `## Memory inventory` auto-append

**GH issue title:** `feat(memory): AGENTS.md memory inventory auto-append (M2)`
**Depends on:** none (M2 is independent of M1; both ship in parallel; M4 wires them together)

### Files
- Modify: `skills/autospec-shared/scripts/auto-init-memory.sh` (add `_ensure_agents_md_inventory_section` helper)
- Create: `skills/autospec-shared/tests/unit/agents-md-inventory.bats`

### Tasks

- [ ] **M2.1** Write bats test asserting:
  - When `AGENTS.md` lacks `## Memory inventory`, helper appends it
  - When it already has `## Memory inventory`, helper is a no-op (idempotency)
  - When `AGENTS.md` is missing entirely, helper creates it with just the inventory section

- [ ] **M2.2** Run bats; expect FAIL (helper missing).

- [ ] **M2.3** Implement `_ensure_agents_md_inventory_section` in `auto-init-memory.sh`:
```bash
_ensure_agents_md_inventory_section() {
  local agents_md="$REPO_ROOT/AGENTS.md"
  [ -f "$agents_md" ] && grep -q "^## Memory inventory" "$agents_md" && return 0
  cat >> "$agents_md" <<'EOF'

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.
EOF
}
```

- [ ] **M2.4** Wire call site: invoke `_ensure_agents_md_inventory_section` near the end of `auto-init-memory.sh` (after symlink dance).

- [ ] **M2.5** Run bats; expect PASS.

- [ ] **M2.6** Commit.

### Acceptance criteria
- [ ] All bats cases pass
- [ ] Helper is idempotent (running 2x produces a single inventory block)
- [ ] No trailing whitespace introduced into AGENTS.md
- [ ] `validate.sh` passes

---

## Phase M3 — `mempalace-mine.sh` wrapper + inference

**GH issue title:** `feat(memory): mempalace-mine.sh wrapper + wing/room inference (M3)`
**Depends on:** none

### Files
- Create: `skills/autospec-shared/scripts/mempalace-mine.sh`
- Create: `skills/autospec-shared/tests/unit/mempalace-mine.bats`
- Create: `skills/autospec-shared/tests/fixtures/memory-mine/{feedback,project,reference,user}/<sample.md>` (4 small fixture files, one per existing CC metadata.type)

### Tasks

- [ ] **M3.1** Write bats test asserting inference per spec §3b:
  - `metadata.type: feedback` → mined frontmatter gains `wing: synthesis, drawer_class: lesson`
  - `metadata.type: project` → `wing: episodic, drawer_class: session-log`
  - `metadata.type: reference` → `wing: semantic, drawer_class: reference`
  - `metadata.type: user` → `wing: semantic, drawer_class: reference`
  - File without any `metadata.type:` → defaults `wing: synthesis, drawer_class: lesson`
  - Mining is idempotent (running 2x doesn't double-add fields)

- [ ] **M3.2** Run bats; expect FAIL.

- [ ] **M3.3** Implement `mempalace-mine.sh`:
```bash
#!/usr/bin/env bash
# mempalace-mine.sh — add mempalace metadata fields (wing, drawer_class) to memory files.
set -euo pipefail
ROOT=""; QUIET=0
while [ $# -gt 0 ]; do case "$1" in --root) ROOT="$2"; shift 2;; --quiet) QUIET=1; shift;; *) shift;; esac; done
[ -z "$ROOT" ] || [ ! -d "$ROOT" ] && { echo "usage: mempalace-mine.sh --root <dir>" >&2; exit 1; }

for f in "$ROOT"/*.md; do
  [ -e "$f" ] || continue
  [ "$(basename "$f")" = "MEMORY.md" ] && continue
  grep -q "^  wing:" "$f" 2>/dev/null && continue   # already mined
  type=$(grep -oE '^  type: [a-z]+' "$f" 2>/dev/null | awk '{print $2}')
  case "$type" in
    feedback)  wing="synthesis"; drawer="lesson" ;;
    project)   wing="episodic"; drawer="session-log" ;;
    reference) wing="semantic"; drawer="reference" ;;
    user)      wing="semantic"; drawer="reference" ;;
    *)         wing="synthesis"; drawer="lesson" ;;
  esac
  # Insert wing + drawer_class lines right after the type: line in frontmatter
  awk -v wing="$wing" -v drawer="$drawer" '
    /^  type: / && !inserted { print; print "  wing: " wing; print "  drawer_class: " drawer; inserted=1; next }
    { print }
    END { if (!inserted) print "WARN: no type: line found, defaults applied" > "/dev/stderr" }
  ' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
  [ "$QUIET" = "0" ] && echo "mined: $(basename "$f") → wing:$wing drawer:$drawer"
done

# Build initial KG if mempalace CLI present
if command -v mempalace >/dev/null 2>&1; then
  mempalace kg-rebuild --root "$ROOT" >/dev/null 2>&1 || true
fi
```

- [ ] **M3.4** Run bats; expect PASS.

- [ ] **M3.5** Wire call site: `auto-init-memory.sh` invokes `mempalace-mine.sh --quiet` after the symlink dance in the migration paths (states 10 and 11).

- [ ] **M3.6** Commit.

### Acceptance criteria
- [ ] All inference rules correct per spec §3b
- [ ] Mining is idempotent
- [ ] Mempalace KG rebuild best-effort (absent CLI doesn't fail the miner)
- [ ] `validate.sh` passes

---

## Phase M4 — SKILL trio updates (lockstep wiring)

**GH issue title:** `feat(memory): wire auto-init-memory.sh into all autospec SKILL trios (M4)`
**Depends on:** M1, M2, M3

### Files (30 total — 10 skills × 3 trio files each)
- Modify: `skills/autospec-define/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-run/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-test/{SKILL.md,codex/prompt.md,opencode/agent.md}` (autospec-test has no opencode/agent.md; M4 hits 2 files here via duo-lockstep #415)
- Modify: `skills/autospec-split/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-classify/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-review/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-listen/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-story/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-stop/{SKILL.md,codex/prompt.md,opencode/agent.md}`
- Modify: `skills/autospec-e2e-clone/{SKILL.md,codex/prompt.md,opencode/agent.md}`

### Tasks

- [ ] **M4.1** Identify the exact insertion point in each skill's startup block. The pattern is the closing of the `## Startup self-update` bash block. Search for `printf '%s\n' "$REMOTE" > "$INSTALLED.tmp"` (the last line of self-update). Insert the new line immediately AFTER that block.

- [ ] **M4.2** The line to insert (consistent across all 10 skills × 3 files):
```bash
# Auto-init cross-tool memory (idempotent, <50ms fast-path)
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
```

- [ ] **M4.3** For each skill, apply the edit to SKILL.md FIRST, then byte-copy to codex/prompt.md and opencode/agent.md (lockstep). Use the existing `validate.sh check_lockstep` (with duo support from #415) to confirm.

- [ ] **M4.4** Write bats test asserting:
  - Every autospec-*/SKILL.md contains exactly one occurrence of the auto-init line
  - Lockstep validation (`autospec validate`) exits 0

- [ ] **M4.5** Run bats + validate.sh; both must pass.

- [ ] **M4.6** Commit (single commit covering all 30 file edits):
```bash
git add skills/autospec-*/SKILL.md skills/autospec-*/codex/prompt.md skills/autospec-*/opencode/agent.md skills/autospec-shared/tests/unit/skill-trio-memory-wired.bats
git commit -m "feat(memory): wire auto-init-memory.sh into all autospec SKILL trios (closes #<issue>)"
```

### Acceptance criteria
- [ ] All 10 skills' trios contain the auto-init line in their startup block
- [ ] Byte-identical lockstep across SKILL.md ↔ codex/prompt.md ↔ opencode/agent.md per skill
- [ ] `validate.sh` passes
- [ ] Manually invoking any skill in a fresh repo triggers the migration (smoke-tested via M5)

---

## Phase M5 — Rollback flag + integration smoke

**GH issue title:** `feat(memory): rollback flag + cross-tool roundtrip integration (M5)`
**Depends on:** M1, M4

### Files
- Modify: `skills/autospec-shared/scripts/auto-init-memory.sh` (add `--rollback` handling)
- Create: `skills/autospec-shared/tests/integration/rollback.bats`
- Create: `skills/autospec-shared/tests/integration/cross-tool-roundtrip.bats`

### Tasks

- [ ] **M5.1** Write bats test for `--rollback`:
  - Setup: tmpdir repo with a fresh migration (state 10 case from M1)
  - Run: `auto-init-memory.sh --rollback`
  - Assert: symlink removed; most-recent `.pre-migration-<timestamp>` dir restored to CC path; in-repo `docs/memory/` left in place
  - Edge case: `--rollback` against an already-rolled-back state is a no-op (exit 0, idempotent)

- [ ] **M5.2** Run bats; expect FAIL.

- [ ] **M5.3** Implement `--rollback`:
```bash
if [ "${1:-}" = "--rollback" ]; then
  CC_PATH=$(_compute_cc_path)
  [ ! -L "$CC_PATH" ] && exit 0   # nothing to roll back
  backup=$(ls -1d "${CC_PATH}.pre-migration-"* 2>/dev/null | sort | tail -1)
  [ -z "$backup" ] && { echo "ABORT: no backup found" >&2; exit 1; }
  rm "$CC_PATH"
  mv "$backup" "$CC_PATH"
  echo "[autospec] rolled back: $CC_PATH restored from $backup" >&2
  exit 0
fi
```

- [ ] **M5.4** Run rollback bats; expect PASS.

- [ ] **M5.5** Write integration bats for cross-tool roundtrip:
  - Setup: fresh tmpdir repo + minimal mock CC memory dir with 2 files
  - Run: `auto-init-memory.sh` (triggers migration)
  - Assert: in-repo `docs/memory/` has both files
  - Simulate Codex read: `grep "## Memory inventory" AGENTS.md` passes
  - Simulate OpenCode read: same
  - Simulate CC write: `echo "new memory" > $CC_PATH/test.md`
  - Assert: file appears in `docs/memory/test.md` (via symlink)
  - Simulate Codex write: `echo "from codex" > docs/memory/from-codex.md`
  - Assert: `cat $CC_PATH/from-codex.md` returns "from codex" (CC sees Codex's write via symlink)

- [ ] **M5.6** Run integration bats; expect PASS.

- [ ] **M5.7** Commit.

### Acceptance criteria
- [ ] `--rollback` cleanly restores pre-migration state
- [ ] Cross-tool roundtrip integration passes (CC ↔ Codex ↔ OpenCode bidirectional via filesystem)
- [ ] `validate.sh` passes
- [ ] Manually invoking `/autospec-define` in a fresh test repo triggers migration end-to-end (operator-level verification noted in commit message)

---

## Cross-cutting acceptance (Phase 2 closeout)

- [ ] All 5 phases merged
- [ ] Migration auto-fires on first autospec-* skill invocation in any repo
- [ ] CC memory at `~/.claude/projects/-Users-wohlgemuth-IdeaProjects-autospec/memory/` is a symlink to `docs/memory/` (verify on this very repo after the auto-init lands)
- [ ] AGENTS.md contains the `## Memory inventory` block
- [ ] Existing 28+ memory files mined with mempalace metadata
- [ ] mempalace MCP server (if installed) can `search` / `traverse` / `kg_query` against the in-repo files
- [ ] Codex peer-review subagent in a Phase 4 monitor cycle reads at least one `docs/memory/feedback_*.md` file as context (operator confirms via session log)

---

## Self-review

**Spec coverage:**
- §1 goal / non-goals — covered by phase boundaries
- §2 architecture — M1 + M2 + M3 + M4 wire it together
- §3a frontmatter schema — M3 inference + miner enforces
- §3b 4 wings + inference — M3 (table + bats)
- §3c tunnels — no new code; documented in spec §3c; format is read-only-by-mempalace (no separate impl needed)
- §3d diary subdir — convention; no new code (mempalace handles it natively via its diary tools)
- §3e KG overlay — M3 wires `mempalace kg-rebuild` best-effort
- §4a auto-init-memory.sh — M1 + M5 rollback
- §4b SKILL trio wiring — M4
- §4c AGENTS.md inventory — M2
- §4d rollback — M5
- §4e auto-commit failure semantics — M1 (best-effort warn)
- §5 per-tool wiring — exercised end-to-end in M5 integration bats
- §6 testing — M1/M2/M3/M5 bats; cross-cutting acceptance covers manual verification

**Placeholder scan:** clean — all 5 phases have exact file paths + concrete code blocks. No "TBD" / "TODO".

**Type consistency:**
- `auto-init-memory.sh` interface consistent across M1 → M2 → M3 → M5 (single script, growing capability)
- `wing` / `drawer_class` field names consistent across spec + M3 inference + M5 integration test
- Exit-code contract (0 = success including no-op fast-path; non-zero only on hard failure) consistent

**Open follow-ups (not in this plan, per spec §10):** cross-repo memory traversal, web UI for mempalace, memory GC, Cursor/Aider/Copilot/Continue.dev integration.
