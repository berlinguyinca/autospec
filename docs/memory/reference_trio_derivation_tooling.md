---
name: reference_trio_derivation_tooling
description: Edit a trio skill via SKILL.md + derive-trio.sh --in-place + gen-skill-goldens.sh — do NOT hand-maintain codex/opencode or hand-regen goldens anymore
metadata: 
  node_type: memory
  type: reference
  originSessionId: 59916493-7c02-4311-889a-2cbeccc1c071
---

As of the 2026-06-16 efficiency wave, the trio skill maintenance workflow is
automated — stop hand-copying the mirrors and hand-hashing goldens:

1. Edit `skills/<name>/SKILL.md` only (the authoritative source).
2. `bash scripts/derive-trio.sh --in-place skills/<name>` — regenerates
   `codex/prompt.md` (frontmatter-stripped SKILL body) and `opencode/agent.md`
   (its own frontmatter + SKILL body). `--check` is the deterministic
   `check_lockstep` equivalent (exit 0 / non-zero + diff); `--stdout <member>`
   prints one mirror.
3. `bash scripts/gen-skill-goldens.sh <name>` — regenerates the
   `tests/fixtures/skill-goldens/<name>.*.sha256` (the `expand-skill-blocks.sh |
   shasum` one-liner, automated).
4. `autospec validate` now runs `check_derive_trio_consistency()` — on drift
   it fails NAMING the skill + the exact fix command. The transform is exact:
   `--check` passed for all 24 trios with zero divergence at ship time.

Also: the decomposer (`autospec-define` Phase 3 + `sizing-check.sh`/`lint-issue.sh`)
now counts a trio + its derived goldens as ONE logical unit, so the ≤3-files cap
no longer forces the prose/golden split-then-recombine tax.

Supersedes the manual steps in [[feedback_skill_golden_derivation_workflow]] and
resolves [[feedback_decompose_trio_prose_goldens_atomic]] (the atomic-unit
exemption shipped). Use the tooling; do not regress to hand edits.
