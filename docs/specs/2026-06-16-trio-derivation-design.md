# Trio single-source derivation (`derive-trio.sh`)

## Summary

A multi-harness skill is three files — `SKILL.md` (authoritative), `codex/prompt.md`,
`opencode/agent.md` — kept **byte-identical in the body** by hand. `codex/prompt.md`
is just the SKILL.md body with frontmatter stripped; `opencode/agent.md` is the body
with a frontmatter swap. They are pure mechanical transforms, yet they are
hand-copied and re-hashed, which is the single biggest source of new-feature
friction: the `autospec-run` trio alone is ~245 KB / ~61 k tokens of ~98% identical
prose, ~30 of `validate.sh`'s checks exist only to police the copies, and the
2026-06-16 run tripped `check_lockstep` and had to recombine prose+golden splits 3×.

This spec introduces **`scripts/derive-trio.sh`** — generate `codex/prompt.md` and
`opencode/agent.md` deterministically from `SKILL.md` — plus a skill-golden
auto-regenerator, so the trio collapses to one source of truth and a derive step.

## Concurrency phasing (REQUIRED — high blast radius)

This change touches the maintenance model of **every** trio skill (~20). To avoid
stomping concurrent sessions editing skill trios, it ships in two phases:

- **Phase 1 (ship now — non-contended):** the standalone tools only — new files,
  no existing-trio edits. `scripts/derive-trio.sh`, `scripts/gen-skill-goldens.sh`,
  and their bats. Nothing migrates; the tools are opt-in.
- **Phase 2 (DEFERRED until the queue is clear of concurrent trio edits):** wire
  `derive-trio` into the decompose/validate flow, add the decomposer atomic-unit
  exemption (edits `autospec-define`), and migrate the ~20 trios to be
  generated. Tracked here; filed with a `deferred:concurrency` label (NOT
  `auto-implement`) so no drainer picks them up until the operator promotes them.

## Team personality
- **Selected team:** Tooling/build — build engineer, maintainer, test engineer.
- **Carry into child issues:** the derive transform must be EXACT (codex = body
  frontmatter-stripped; opencode = body + opencode frontmatter); idempotent
  (`derive-trio --check` exits non-zero iff a trio is out of sync, zero edits);
  Phase 1 adds NO edits to any existing trio.

## Review counter-team
- **Selected counter-team:** Correctness + migration safety.
- **Challenge:** does the generated codex/opencode byte-match the current
  hand-maintained files for EVERY skill (else migration is a mass diff)? Does
  `--check` exactly reproduce what `check_lockstep` enforces? Could a
  frontmatter edge case (multi-doc, CRLF, trailing newline) corrupt a mirror?

## Architecture

```
scripts/derive-trio.sh <skill-dir> [--in-place | --check | --stdout <member>]
  SKILL.md ──strip frontmatter────────────────► codex/prompt.md
           └─strip body + swap to opencode FM──► opencode/agent.md
  --check : derive in-memory, diff vs the committed mirrors; exit 0 if identical,
            non-zero + diff if drift (the deterministic equivalent of check_lockstep)

scripts/gen-skill-goldens.sh [<skill>...]
  for each markered trio member: expand-skill-blocks.sh <m> | shasum -a 256
    > tests/fixtures/skill-goldens/<skill>.<suffix>.sha256
  (the one-liner currently living only in feedback_skill_golden_derivation_workflow.md)
```

`derive-trio.sh` reuses the literal-substitution discipline already proven by
`scripts/expand-skill-blocks.sh`. The opencode frontmatter delta is the only
non-strip transform — capture it as a small declared template, not ad-hoc seds.

## Phase 2 (deferred) design — captured, not shipped yet

- **Validate integration:** `check_lockstep` becomes (or is backed by)
  `derive-trio --check` so drift is reported as "run `derive-trio --in-place`",
  and `check_block_expansion` is fed by `gen-skill-goldens.sh`.
- **Decomposer atomic-unit exemption:** `autospec-define` Phase 3 + `sizing-check.sh`
  treat a trio's members + their derived goldens as ONE logical change (the
  ≤3-files cap stops forcing prose/golden splits — the recurring recombine tax,
  per `feedback_decompose_trio_prose_goldens_atomic.md`). Once derive-trio is the
  source of truth, an edit is "edit SKILL.md + run derive + regen goldens",
  which the implementer can do in one commit.
- **Migration:** for each of the ~20 trios, assert `derive-trio --check` already
  passes (proving the generator matches the hand-maintained file), then mark the
  skill as derive-managed. No prose changes — just adopting the generator.

## Testing (Phase 1)
- `tests/derive-trio.bats` — for a fixture skill, `--stdout codex` == the body
  with frontmatter stripped; `--stdout opencode` == body + opencode frontmatter;
  `--check` exits 0 when in sync, non-zero + diff when the mirror is edited;
  `--in-place` writes both mirrors and is idempotent; **`--check` passes for
  EVERY current `skills/*/` trio** (proves the generator already matches today's
  hand-maintained mirrors — the migration-safety guard).
- `tests/gen-skill-goldens.bats` — regenerates the sha256 for a markered skill
  matching `check_block_expansion`; no-op when already current.

## Acceptance (Phase 1 only)
- [ ] `scripts/derive-trio.sh` ships with `--in-place`/`--check`/`--stdout <member>`;
      exact codex (frontmatter-stripped) + opencode (frontmatter-swapped) transforms.
- [ ] `scripts/gen-skill-goldens.sh` regenerates skill-golden sha256s matching
      `check_block_expansion`'s expander.
- [ ] `tests/derive-trio.bats` asserts `--check` passes for EVERY existing
      `skills/*/` trio (generator already byte-matches today's mirrors).
- [ ] `tests/gen-skill-goldens.bats` passes; `autospec validate` green.
- [ ] NO existing trio file, `validate.sh`, or `autospec-define` is edited in
      Phase 1 (deferred to Phase 2).

## Decomposition into child issues

**Phase 1 (file as `auto-implement`, drain now):** 2 children + umbrella.
1. **A — `derive-trio.sh`** + `tests/derive-trio.bats` (incl. the all-trios
   `--check` guard). Files: 2.
2. **B — `gen-skill-goldens.sh`** + `tests/gen-skill-goldens.bats`. Files: 2.

**Phase 2 (file as `deferred:concurrency`, do NOT drain until queue clear):**
3. C — validate integration (`check_lockstep` via `derive-trio --check`).
4. D — decomposer atomic-unit exemption (`autospec-define` + `sizing-check.sh`).
5. E — migrate the ~20 trios to derive-managed.

Plus a Phase 5.5 audit child for Phase 1.

## Out of scope
- Generating `SKILL.md` itself (it stays the authoritative human-authored source).
- Non-trio skills.
