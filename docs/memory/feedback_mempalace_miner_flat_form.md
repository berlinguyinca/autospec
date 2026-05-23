---
name: feedback-mempalace-miner-flat-form
description: "mempalace-mine.sh must handle both nested AND flat frontmatter forms — real CC memory files use flat `type:`, not spec's nested `metadata.type:`"
metadata: 
  node_type: memory
  type: feedback
  wing: synthesis
  drawer_class: lesson
  originSessionId: 2e9dc40b-7a14-4cff-8d65-1043d1718ed0
---

`mempalace-mine.sh` was written against the spec's nested frontmatter form (`metadata: { type: feedback }` → grep `^  type:`), but real Claude Code auto-memory files use flat top-level frontmatter (`type: feedback` → no indent). Result: the miner silently mined 0 of 28 files after the M3 PR landed, even though all 11 bats unit tests passed (the fixtures matched spec form, not reality).

**Why:** Two valid frontmatter conventions coexist — the user-written CC memory style is flat (single-level), while the spec proposed a nested wrapper for clarity. Bats fixtures encoded the spec form only, so the gap wasn't caught until manual verification on the live repo.

**How to apply:**
- When writing a "format normalizer" tool, list ALL real-world variants of the input format BEFORE writing fixtures. Skim a handful of actual files in the wild.
- Add a fixture per real-world variant, not just per spec variant.
- The fix: try nested → flat → metadata-block-only → bare-frontmatter (insert after opening `---`). Idempotency guard must match both `^wing:` and `^  wing:`.
- Related anti-pattern: spec-driven test fixtures that diverge from production data shape.
