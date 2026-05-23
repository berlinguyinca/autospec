---
name: project-cross-tool-memory-brainstorm
description: SHIPPED 2026-05-23 — autospec cross-tool memory extension fully integrated; M1-M5 merged + migration applied to this repo
metadata: 
  node_type: memory
  type: project
  wing: episodic
  drawer_class: session-log
  originSessionId: 2e9dc40b-7a14-4cff-8d65-1043d1718ed0
---

**Status:** SHIPPED + INTEGRATED on autospec repo (2026-05-23).

**Merged PRs:** #496 (spec/d15cbd9), #503 (M1 auto-init), #504 (M2 AGENTS.md), #505 (M3 mempalace-mine), #506 (M4 SKILL trio wiring), #507 (M5 rollback + integration).

**Live state on this repo:**
- `~/.claude/projects/-Users-wohlgemuth-IdeaProjects-autospec/memory/` is symlink → `docs/memory/` (verified)
- 30 memory files migrated, 29 mined with `wing` + `drawer_class` frontmatter
- `AGENTS.md` has `## Memory inventory` block (commit da395cf, then auto-init re-appended)
- mempalace CLI installed at `/Users/wohlgemuth/.local/bin/mempalace`

**Post-merge bug found + fixed (2026-05-23):** `mempalace-mine.sh` grep pattern `^  type:` only matched the nested-frontmatter form from the spec; actual CC auto-memory files use flat `^type:`. Fix extends miner to try nested → flat → metadata-block → raw frontmatter insertion. Now mines all 4 forms idempotently. PR for the fix going up.

**Why this matters:** the cross-tool autospec workflow (CC↔Codex↔OpenCode) now has durable shared memory. Saved lessons (no RETURN traps, lockstep duo gap, prompt caching) are visible to peer-review subagents and OpenCode workers via the in-repo `docs/memory/` floor + mempalace MCP overlay.

**How to apply:** When invoking any /autospec-* skill in a new repo, the startup self-update block auto-fires `auto-init-memory.sh` which: (1) computes CC slug path, (2) migrates+symlinks on first run, (3) appends AGENTS.md inventory, (4) mines mempalace metadata. Idempotent <50ms fast-path on subsequent runs. Rollback via `auto-init-memory.sh --rollback`.

Related: [[feedback-mempalace-miner-flat-form]] (new lesson from the post-merge bug).
