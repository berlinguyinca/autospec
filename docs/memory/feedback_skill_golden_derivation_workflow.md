---
name: feedback_skill_golden_derivation_workflow
description: "Editing any autospec trio skill (esp. one with autospec-block markers) requires re-deriving codex/opencode AND regenerating tests/fixtures/skill-goldens sha256, or validate.sh fails closed"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: ba194365-30f2-4add-918a-ed7263b0d5dd
---

When adding or editing an autospec multi-harness skill, two coupled gates in
`autospec validate` must both pass:

1. **Lock-step (raw):** `strip_body(SKILL.md)` (= `awk '/^---$/{c++;next} c>=2'`)
   must byte-equal `codex/prompt.md`, and equal `strip_body(opencode/agent.md)`.
   opencode has its own 2-line frontmatter (`description:` + `mode: primary`).
2. **Block-expansion golden:** for every skill, `tests/fixtures/skill-goldens/
   <skill>.{SKILL.md,codex.prompt.md,opencode.agent.md}.sha256` must equal
   `expand-skill-blocks.sh <src> | shasum -a 256 | cut -d' ' -f1`. SKILL.md
   golden is required (fail-closed on missing); codex/opencode goldens required
   when that member contains `<!-- autospec-block:... -->` markers.

**Why:** a NEW skill fails validate.sh until goldens exist; editing a skill with
block markers (e.g. autospec-run's `harness-adapter-core` / `startup-self-update`)
changes the expanded bytes, so stale goldens fail.

**How to apply (safe procedure):** edit SKILL.md only → re-derive
`codex/prompt.md = strip_body(SKILL.md)` and `opencode/agent.md = <its frontmatter> +
strip_body(SKILL.md)` → regenerate all 3 goldens with the expander+shasum →
verify both lock-step diffs empty → run full `validate.sh` (slow, runs whole bats
suite) expecting exit 0. New skills also need a `## Stop mode` heading in all
three trio files, a `## Self-update mode` section, a Model-tier directive, and the
adapter rows. Extends [[feedback_autospec_decomposer_gotchas]] and
[[feedback_validate_sh_lockstep_checks]].
