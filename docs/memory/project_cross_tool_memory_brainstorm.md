---
name: project-cross-tool-memory-brainstorm
description: In-design 2026-05-23 — autospec extension for cross-tool persistent memory across CC + Codex + OpenCode trio
metadata:
  type: project
  wing: episodic
  drawer_class: session-log
---

User asked 2026-05-23: "currently codex/claude/etc have no persistent memory between sessions. What is the best approach to solve this and actaully remember the content of a repo. Provide me with different highly rated examples to provide decent cross system memory"

**Decision: B+C** — survey-informed autospec extension. Survey landscape covered the highly-rated examples (llmstxt.org, mempalace, MCP memory servers, AGENTS.md convention, Karpathy LLM wiki, Mem0, MemGPT/Letta, Cody, Continue.dev, etc.). Then narrowed to the autospec-extension design.

**Locked design decisions:**
- Q1 memory types: ALL FOUR (semantic + episodic + procedural + synthesis). User: "all of the above"
- Q2 target tools: i + ii + iii = Claude Code + Codex CLI + OpenCode (the autospec trio; no IDE-level tools in scope)
- Q3 storage location: A = in-repo, gitted (`docs/memory/`)
- Q4 directory layout: B = flat dir + `MEMORY.md` index (mirrors existing CC auto-memory pattern; 28 existing files migrate)
- Q5 per-tool read mechanism: A = per-tool config files point at shared dir (CC via symlink; Codex/OpenCode via AGENTS.md `## Memory inventory` section)
- Q6 migration: A = symlink CC's auto-memory dir → in-repo dir (one-shot, reversible)

**Section 1 of design presented:** architecture + integration. Awaiting user confirmation. Sections 2-5 still to present: memory schema/frontmatter, migration playbook, Codex/OpenCode wiring, decision-log appendix with survey embedded.

**Why this matters:** the existing CC auto-memory pattern works well WITHIN Claude Code but Codex/OpenCode can't see the 28 saved feedback/project memories. This extension makes the cross-tool autospec workflow durable — saved lessons (no RETURN traps, lockstep duo gap, prompt caching, etc.) become visible to peer-review subagents and OpenCode workers.

**How to apply when resuming:** confirm Section 1 → present Section 2 (memory schema) → Section 3 (migration playbook) → Section 4 (per-tool wiring) → Section 5 (decision-log + survey appendix) → write spec to `docs/specs/2026-05-23-autospec-cross-tool-memory-design.md` → user review → /writing-plans.
