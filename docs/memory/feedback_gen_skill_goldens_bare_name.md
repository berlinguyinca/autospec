---
name: feedback_gen_skill_goldens_bare_name
description: gen-skill-goldens.sh takes a BARE skill name (derive-trio.sh takes a PATH); passing a path silently no-ops with exit 3 and stale goldens survive
metadata: 
  node_type: memory
  type: feedback
  originSessionId: d87fae41-0795-45c0-9afe-909bd9bc37fb
---

The two trio-maintenance scripts take DIFFERENT argument forms:

- `scripts/derive-trio.sh skills/<skill> --in-place|--check` — takes a **PATH**.
- `scripts/gen-skill-goldens.sh <skill>` — takes a **BARE NAME** (no `skills/`).

**Why it bites:** if you pass `gen-skill-goldens.sh skills/autospec-grow-run`
(the path form that works for derive-trio), it exits **3** = "named skill does
not exist" and writes NOTHING — a silent no-op. `git status` then shows the
goldens unchanged, which reads as "already current" but actually means "never
regenerated." The stale block-expansion goldens then fail `validate.sh`'s
`check_block_expansion` with an `expanded sha256 mismatch`. Bit me twice editing
the grow-run SKILL.md (Plan 4): each SKILL.md prose change needs the goldens
regenerated, and the wrong-arg no-op let a stale-goldens commit slip to the
gate.

**How to apply:** after ANY edit to a trio `SKILL.md`, run BOTH, minding the
arg forms:
```
scripts/derive-trio.sh skills/<skill> --in-place    # PATH
scripts/gen-skill-goldens.sh <skill>                # BARE NAME
scripts/derive-trio.sh skills/<skill> --check       # PATH, expect exit 0
```
Confirm `gen-skill-goldens.sh` prints `wrote .../tests/fixtures/skill-goldens/
<skill>.*.sha256` and exits **0** — treat exit 3 (or "no files written") as a
FAILURE, never as "already current". Verify with `git status
tests/fixtures/skill-goldens/<skill>.*` actually changing when the SKILL.md
changed. Goldens are EXPANDED-content sha256, so a prose edit inside a
non-block region still changes them. Related:
[[feedback_skill_golden_derivation_workflow]], [[reference_trio_derivation_tooling]],
[[feedback_decompose_trio_prose_goldens_atomic]].
