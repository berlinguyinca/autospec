# Autospec Cross-Tool Persistent Memory

**Status:** Draft design (2026-05-23)
**Author:** berlinguyinca + brainstorm
**Scope:** Autospec extension. Makes the 28+ memory files (currently CC-only at `~/.claude/projects/-Users-wohlgemuth-IdeaProjects-autospec/memory/`) cross-tool-accessible across the autospec trio (Claude Code + Codex CLI + OpenCode). Mempalace MCP layer added on top for semantic search + KG queries. Zero manual intervention.

## 1. Goal & non-goals

### Goal
Persistent memory that survives across sessions AND across tools. Today, CC's auto-memory works only within Claude Code; Codex/OpenCode subagents that autospec dispatches have no access to the 28 hard-won lessons (no RETURN traps, lockstep duo gap, prompt caching, drift gate scope, etc.). This design moves the memory to an in-repo location read natively by all three tools, with a mempalace-backed query layer on top for tools with MCP support. Auto-init handles migration + wiring on first skill invocation — operator never types a command.

### Non-goals
- Cross-tool memory beyond the autospec trio (Cursor / Aider / Copilot / Continue.dev out of scope; can be added later via the same in-repo files)
- Memory beyond the repo (operator-personal notes, cross-repo learnings)
- Replacing AGENTS.md / SKILL.md as the source of truth for conventions (memory is *learned*; AGENTS.md is *declared*)
- Hosted/SaaS memory layer (everything stays local + in-repo)

## 2. Architecture

```
<repo>/docs/memory/                              ← canonical storage (gitted)
  MEMORY.md                                      ← flat index (existing convention)
  feedback_*.md / project_*.md / etc.            ← memory files w/ YAML frontmatter
  diary/<YYYY-MM-DD>-<slug>.md                   ← episodic timeline
  .kg/                                           ← mempalace KG shards (small JSON)

~/.claude/projects/<hash>/memory  →  symlink → docs/memory/    ← CC reads here natively

AGENTS.md
  ## Memory inventory                            ← Codex + OpenCode read AGENTS.md;
    (block points at docs/memory/ + MEMORY.md)     this section advertises the dir

mempalace MCP server (optional)                  ← indexes docs/memory/ files;
                                                   exposes search / traverse / kg / diary
```

Three integration layers, each independent:

1. **Filesystem floor** (always-on) — markdown files at a stable in-repo path. Every tool with file-read capability sees them. Zero infrastructure.
2. **Per-tool pointers** — CC via symlink, Codex/OpenCode via AGENTS.md inventory section. Each tool reads via its native mechanism.
3. **Mempalace MCP layer** (optional) — indexes the floor, exposes structured queries. Tools with MCP support get rich search; tools without still get the floor.

## 3. Memory schema

### 3a. Extended YAML frontmatter

```markdown
---
name: feedback-bash-return-trap-leak
description: RETURN traps in bash functions leak into caller frames under set -u — use inline cleanup instead.
metadata:
  type: feedback                    # existing CC convention: user|feedback|project|reference
  # ── mempalace metadata layer (additive — old files without these still work) ──
  wing: engineering                 # top-level palace partition
  room: bash-patterns               # within-wing topic
  drawer_class: anti-pattern        # diary|anti-pattern|invariant|playbook|reference
  tags: [bash, set-u, cleanup, autospec-run]
  tunnels:                          # cross-references (mempalace walks these)
    - feedback-bash-set-e-short-circuit
    - feedback-validate-sh-lockstep-checks
  created_at: 2026-05-18T14:22:11Z
  last_referenced_at: 2026-05-22T19:30:00Z
---

Body — the rule, the why, the how-to-apply. Free-form markdown.
Tunnels also expressed inline as [[feedback-bash-set-e-short-circuit]] wiki-links.
```

### 3b. The 4 wings (map to the 4 memory types)

| Wing | Existing `metadata.type:` | Default drawer_class |
|---|---|---|
| **semantic** | reference + project facts | invariant / reference / architecture |
| **episodic** | project (in-flight) + diary | diary / session-log / ship-log |
| **procedural** | (overlaps with AGENTS.md rules) | playbook / runbook / recipe |
| **synthesis** | feedback (lessons learned) | anti-pattern / lesson / gotcha |

Inference rules for the migration miner (no operator labeling required for the 28 existing files):
- `metadata.type: feedback` → `wing: synthesis, drawer_class: lesson`
- `metadata.type: project` → `wing: episodic, drawer_class: session-log`
- `metadata.type: reference` → `wing: semantic, drawer_class: reference`
- `metadata.type: user` → `wing: semantic, drawer_class: reference`
- Defaults if absent: `wing: synthesis, drawer_class: lesson`

### 3c. Tunnels

Cross-references via two equivalent mechanisms (mempalace `find_tunnels` walks both):

- Frontmatter `tunnels:` array — structured, queryable
- Inline `[[name]]` wiki-style links in body — readable in plain markdown

Existing CC memories already use `[[name]]` in many places. This formalizes the pattern.

### 3d. Diary subdir

Time-ordered episodic entries land in `docs/memory/diary/<YYYY-MM-DD>-<slug>.md` with frontmatter `wing: episodic, drawer_class: diary`. Mempalace's `mempalace_diary_read/write` MCP tools manage these natively. Storage is the same flat files; diary is a convention, not a separate substrate.

### 3e. KG (knowledge graph) overlay

Mempalace's `mempalace_kg_add/query/timeline` auto-extracts subject-verb-object triples from memory bodies. Built incrementally as files are added. Storage: `docs/memory/.kg/` (gitted JSON shards, small enough to commit). Re-indexed by a post-commit hook installed by auto-init.

## 4. Auto-init (zero manual intervention)

### 4a. `auto-init-memory.sh`

Lives at `$AUTOSPEC_SCRIPTS_DIR/auto-init-memory.sh`. Called from the existing `autospec-startup-self-update` block at the top of every autospec SKILL.md. Fast-path (already-migrated) is <50ms.

```bash
#!/usr/bin/env bash
# Auto-migrate CC memory to in-repo location on first invocation per repo.
# Idempotent: subsequent runs are no-op probes.
set +e

REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
REPO_SLUG=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null | tr '/' '-')
[ -z "$REPO_SLUG" ] && exit 0

CC_PATH="$HOME/.claude/projects/$(echo "$REPO_ROOT" | tr '/' '-')/memory"
REPO_PATH="$REPO_ROOT/docs/memory"

# Fast path: symlink already in place → no-op
if [ -L "$CC_PATH" ] && [ "$(readlink "$CC_PATH")" = "$REPO_PATH" ]; then exit 0; fi

CC_HAS_FILES=$([ -d "$CC_PATH" ] && [ -n "$(ls -A "$CC_PATH"/*.md 2>/dev/null)" ] && echo 1 || echo 0)
REPO_HAS_DIR=$([ -d "$REPO_PATH" ] && echo 1 || echo 0)

case "${CC_HAS_FILES}${REPO_HAS_DIR}" in
  10) # CC has memory, in-repo dir missing → migrate + symlink
    mkdir -p "$REPO_PATH"
    cp -p "$CC_PATH"/*.md "$REPO_PATH"/
    mv "$CC_PATH" "${CC_PATH}.pre-migration-$(date +%Y%m%d-%H%M%S)"
    ln -s "$REPO_PATH" "$CC_PATH"
    bash "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" --root "$REPO_PATH" --quiet
    _auto_commit "feat: auto-migrate cross-tool memory to docs/memory/"
    ;;
  01) # in-repo dir exists (from teammate's commit), CC memory missing → symlink
    mkdir -p "$(dirname "$CC_PATH")"
    ln -s "$REPO_PATH" "$CC_PATH"
    ;;
  11) # both exist → sync (CC wins on dup) + symlink
    rsync -a --ignore-existing "$CC_PATH"/*.md "$REPO_PATH"/ 2>/dev/null
    mv "$CC_PATH" "${CC_PATH}.pre-migration-$(date +%Y%m%d-%H%M%S)"
    ln -s "$REPO_PATH" "$CC_PATH"
    bash "${AUTOSPEC_SCRIPTS_DIR}/mempalace-mine.sh" --root "$REPO_PATH" --quiet
    ;;
  00) # neither exists → create empty in-repo dir + symlink
    mkdir -p "$REPO_PATH" "$(dirname "$CC_PATH")"
    touch "$REPO_PATH/MEMORY.md"
    ln -s "$REPO_PATH" "$CC_PATH"
    ;;
esac

# Ensure AGENTS.md has the memory inventory section
_ensure_agents_md_inventory_section

exit 0
```

Auto-commit + push are best-effort; failures log to stderr but never block the skill invocation.

### 4b. Wired into every SKILL.md

Existing `autospec-startup-self-update` block at the top of each SKILL.md (and lockstep `codex/prompt.md` + `opencode/agent.md`) gains one line:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/auto-init-memory.sh"
```

Applies across every skill: autospec-define, autospec-run, autospec-test, autospec-split, autospec-classify, autospec-review, autospec-listen, autospec-story, autospec-stop, autospec-e2e-clone.

### 4c. AGENTS.md `## Memory inventory` block

Auto-appended by `auto-init-memory.sh` if absent:

```markdown
## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.
```

### 4d. Rollback

Operator escape hatch: `bash $AUTOSPEC_SCRIPTS_DIR/auto-init-memory.sh --rollback`. Removes the symlink, restores the most-recent `.pre-migration-<timestamp>` backup. Idempotent.

### 4e. Auto-commit failure semantics

If git fails (no push permission, uncommitted user work in the way, hooks block), the script:
- Logs one-line warning to stderr (visible in skill invocation but non-blocking)
- Leaves files staged for operator to commit
- Skill invocation continues

The migration itself (move + symlink) always completes; git side is best-effort.

## 5. Per-tool wiring

### 5a. Claude Code (read+write via symlink)

- Reads `~/.claude/projects/<hash>/memory/MEMORY.md` (now symlink) at session start. CC mechanism unchanged.
- Writes new memories via `oh-my-claudecode:remember` or direct `Write` to the memory dir. Land in `docs/memory/` automatically via the symlink.
- Zero CC config change.

### 5b. Codex CLI (read via AGENTS.md, write via direct file edit)

- Reads AGENTS.md `## Memory inventory` section at session start; pulls relevant `docs/memory/*.md` files as context when relevant.
- Writes via `apply_patch` to `docs/memory/<name>.md` + updates `MEMORY.md` in the same patch.
- One-time AGENTS.md update lands via auto-init.

### 5c. OpenCode (same mechanism as Codex)

- Reads AGENTS.md `## Memory inventory`.
- Writes via OpenCode's file-edit tools.
- Identical wiring to Codex; same AGENTS.md section.

### 5d. Mempalace MCP server (optional rich layer)

Indexes `docs/memory/` files. Exposes:

| MCP tool | Purpose |
|---|---|
| `mempalace_search "topic"` | Semantic search across all wings |
| `mempalace_traverse <name>` | Walk tunnels from a starting memory |
| `mempalace_kg_query "X → ?"` | KG triple query |
| `mempalace_diary_read --since <date>` | Episodic timeline slice |
| `mempalace_find_tunnels <name>` | All cross-refs to/from a memory |
| `mempalace_list_wings / list_rooms` | Hierarchy navigation |

CC speaks MCP natively today; Codex/OpenCode get this when their MCP support matures. Until then, they use the flat file floor — no loss.

### 5e. Lockstep replication for the new line in SKILL.md

The `bash auto-init-memory.sh` line gets added to:
- Every `skills/autospec-*/SKILL.md` startup block
- Every matching `skills/autospec-*/codex/prompt.md`
- Every matching `skills/autospec-*/opencode/agent.md`

`validate.sh` (with the duo-lockstep extension shipped in #415) keeps them byte-identical.

### 5f. Conflict handling

If two tools edit the same memory file in the same session, second write wins (standard file semantics). Conflicts surface at git-commit time if both made it to disk. Post-commit hook (installed by auto-init) re-indexes the KG so stale tunnels are caught.

## 6. Testing

### 6a. Unit tests (bats)

- `auto-init-memory.sh` per state-matrix case (00 / 01 / 10 / 11 + already-symlinked fast-path) → expected fs state
- `_ensure_agents_md_inventory_section` — idempotent (running 2x = single block)
- `_auto_commit` — failure paths (no remote, dirty WT, pre-commit hook reject) → stderr warning + non-zero, skill continues
- `--rollback` — restores backup correctly
- Mempalace-mine inference per existing file → expected wing/room/drawer_class

### 6b. Integration tests

- Fresh repo + CC memory present → first skill invocation triggers migration; verify symlink + commit + AGENTS.md
- Fresh repo + in-repo memory committed by collaborator + CC empty → first skill invocation triggers symlink-only (no migration); CC sees the memories
- Repo where both exist independently → sync + symlink; no data loss
- Empty repo + empty CC → both created; symlinked; AGENTS.md gets inventory

### 6c. Cross-tool round-trip

- CC writes memory → Codex reads it via AGENTS.md → OpenCode reads it → all three see identical content
- Codex writes memory → CC reads it → mempalace search returns it
- Mempalace `search` against fixture corpus returns expected ranking

## 7. Dependencies & scope boundaries

| Dependency | Status | Failure mode |
|---|---|---|
| Existing autospec-startup-self-update block in SKILL.md files | live | one-line addition per skill |
| `mempalace:init` skill / `npx mempalace` | installed via skill catalog | optional rich layer; floor works without it |
| `gh` CLI | live | required for repo-slug detection |
| `git` | required | install pre-flight |
| `validate.sh check_lockstep` duo-mode | live (#415) | enforces SKILL trio identity |

### Out of scope (deferred to follow-on specs)

- Cross-repo memory traversal (lessons from repo A visible in repo B)
- Web UI for mempalace
- Memory garbage collection (timestamp + reference-count heuristic)
- Cursor / Aider / Copilot / Continue.dev integration (same in-repo files; readers per-tool)

## 8. Appendix A — Landscape survey (highly-rated approaches)

| Approach | Cross-tool? | Adopted here? | Why / why not |
|---|---|---|---|
| `AGENTS.md` convention | ✅ | **Yes** | Universal floor for Codex/OpenCode |
| `CLAUDE.md` | ❌ Claude-only | No | Tool-siloed |
| `.cursorrules` / `.cursor/rules/` | ❌ Cursor-only | No | Out of scope (Cursor not in target trio) |
| `llms.txt` / `llms-full.txt` (llmstxt.org) | ✅ Emerging standard | Adjacent — shipped via docs-amendment | Complementary; could carry the inventory pointer |
| MCP memory servers (`mcp-memory-service`, `mcp-knowledge-graph`) | ⚠️ tool-MCP-support-dependent | Considered, deferred | Mempalace is the chosen MCP server (already installed) |
| **mempalace** | ⚠️ MCP-only | **Yes** | Adopted as the rich query layer; floor works without it |
| Mem0 (mem0.ai) | ✅ via API | No | SaaS; data leaves machine |
| MemGPT / Letta | App-embedded | No | Heavyweight; tool integration nontrivial |
| Karpathy "LLM Wiki" pattern | ✅ tool-agnostic | Conceptually adopted | The flat-markdown floor IS this pattern |
| Sourcegraph Cody contexts | Cody-locked | No | Vendor-locked |
| Continue.dev `.continue/index/` | Continue-only | No | Out of scope |
| Aider repo-map | Aider-only | No | Spatial index, not memory |
| Claude Code auto-memory | ❌ CC-only | **Yes (extended)** | Existing system; this design adds cross-tool surface |
| ChatGPT Saved Memories | ❌ ChatGPT only | No | Not for code |
| GitHub Copilot Repository Instructions | ❌ Copilot only | No | Out of scope; equivalent of AGENTS.md inside Copilot |

**3 adopted directly:** AGENTS.md convention, mempalace, existing CC auto-memory pattern. 12 documented as considered-not-adopted with reasons.

## 9. Decision log

| Q | Decision | Rationale |
|---|---|---|
| Memory types in scope | All 4 (semantic / episodic / procedural / synthesis) | User explicit |
| Target tools | CC + Codex + OpenCode (autospec trio) | User explicit |
| Storage location | In-repo `docs/memory/`, gitted | Repo-scoped facts belong with the repo |
| Directory layout | Flat + `MEMORY.md` index | Existing CC convention; trivial migration |
| Per-tool read | CC via symlink; Codex/OpenCode via AGENTS.md inventory | Each tool's native mechanism; no new per-tool infra |
| Migration | Auto-init on first skill invocation; idempotent fast-path | User explicit ("no manual intervention") |
| File format | Existing YAML frontmatter + additive mempalace fields | Backward-compatible |
| Mempalace integration | Hybrid — markdown is canonical, MCP layer indexes | Per user request to fold mempalace concepts |
| Tunnels syntax | Both frontmatter array AND inline `[[name]]` | Both already in use |
| Diary subdir | `docs/memory/diary/<YYYY-MM-DD>-<slug>.md` | Convention not new substrate; mempalace native |
| KG storage | `docs/memory/.kg/` (gitted JSON shards) | In-repo so collaborators see same KG |
| Auto-commit on migration | Best-effort; warn-but-don't-block | Skill invocation never fails on git issues |
| Rollback | Single `--rollback` flag | Optional; default operator never touches |

## 10. Open follow-ups (separate specs)

1. **Cross-repo memory traversal** — when a lesson from repo A is relevant in repo B (e.g., personal feedback rules). Mempalace's `find_tunnels` could span repos if operator opts in.
2. **Web UI for mempalace** — currently CLI/MCP only; browser navigator helps for large memory sets.
3. **Memory garbage collection** — surface stale memories (referenced project shipped, feedback no longer relevant). Manual today; could automate via timestamp + reference-count heuristic.
4. **Cursor / Aider / Copilot / Continue.dev** — add their native pointers to the same in-repo files. Same architecture, different reader configs.
