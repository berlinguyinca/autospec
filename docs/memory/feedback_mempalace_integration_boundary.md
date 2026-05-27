---
name: feedback-mempalace-integration-boundary
description: "Autospec ships the shared-memory FLOOR natively; mempalace stays an optional external CLI/MCP plugin — clarify scope when discussing \"integration\""
metadata:
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 2e9dc40b-7a14-4cff-8d65-1043d1718ed0
---

User asked 2026-05-23: "so memplace is no external plugn anymore and completely integrated?" — revealing the assumption that "integration" meant "autospec vendors mempalace." It does not.

**The boundary:**
- **Native in autospec:** `docs/memory/` filesystem floor, `auto-init-memory.sh` (symlink + AGENTS.md inventory), `mempalace-mine.sh` (frontmatter mining for `wing:` + `drawer_class:`), the AGENTS.md `## Memory inventory` block.
- **External (optional):** mempalace CLI (`/Users/wohlgemuth/.local/bin/mempalace`), mempalace MCP server (search/KG/traverse/diary). Best-effort: miner calls `mempalace kg-rebuild` if CLI present, silent skip otherwise.

**Why this matters:** the shared-memory contract works without mempalace. Files have mempalace-compatible frontmatter so *any* future indexer can use them, but the rich-query layer is opt-in. Don't conflate "speaks mempalace schema" with "bundles mempalace."

**How to apply:** when describing autospec memory features, distinguish (1) the native floor (always works) from (2) the mempalace overlay (requires external install). When user says "integrated" — clarify they're asking about scope, not assume completion.

**Update 2026-05-23 (PR #509):** user followed up with *"no i want you to completely iplement mempalaca here so its not an external dependency"* — when offered 4 scopes (vendor submodule / bundle source / slim bash reimpl / auto-install), picked **auto-install on first run**. Decision: keep mempalace external (9.1K LOC Python, MIT, chromadb dep) but make `auto-init-memory.sh` `pipx install mempalace` on first migration (with pipx → uv → pip3 → pip fallback chain, opt-out via `AUTOSPEC_SKIP_MEMPALACE_INSTALL=1`). Avoids vendoring a large Python project into a bash-first repo while making mempalace effectively zero-config for end users.
