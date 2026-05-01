# Contributing

This repository stores portable Codex skills.

## Skill Requirements

- Use lowercase hyphenated skill names.
- Keep each skill in `skills/<skill-name>/`.
- Include exactly one required `SKILL.md` per skill.
- Keep the frontmatter `name` aligned with the folder name.
- Keep descriptions focused on when Codex should use the skill.
- Do not commit private credentials, local data, generated caches, or virtual environments.

## Preferred Structure

```text
skills/<skill-name>/
  SKILL.md
  agents/openai.yaml
  scripts/
  references/
  assets/
```

Only include optional directories when they are useful.

## Review Checklist

- Skill has a clear trigger.
- Instructions are actionable and not project-specific unless intentionally scoped.
- Helper scripts have been syntax-checked or run in dry-run mode.
- Large examples or templates are moved out of `SKILL.md`.
- README and `SKILLS.md` are updated.

## Testing

Every PR is expected to leave the test suite green. The recommended local
loop is:

1. **Bootstrap** — install the dev dependencies (`bats`, `gh`, `jq`,
   `python3`, etc.) once per machine:

   ```bash
   bash scripts/dev-bootstrap.sh
   ```

2. **Run the suites** — fast unit + smoke tests plus the repo-wide
   validator:

   ```bash
   bats tests/unit tests/smoke && scripts/validate.sh
   ```

   The `bats` run is well under a minute on commodity hardware. E2E
   tests (`tests/e2e/*.bats`) are opt-in: they create throwaway gh
   repos and should only run when the GitHub credentials in the
   environment are willing to absorb the side-effect.

3. **Validate before pushing** — `scripts/validate.sh` enforces the
   lock-step body rule across every multi-harness skill plus the
   self-update / model-tier invariants. Treat a non-zero exit as a
   blocker.

### Updating goldens

Several tests compare subagent output against committed golden files
under `tests/golden/`. When a deliberate body change makes a golden go
stale, regenerate it via:

```bash
tests/refresh-goldens.sh
```

The script pairs each fixture in `tests/fixtures/` with its target
golden in `tests/golden/` and prints the manual subagent-capture step a
maintainer is expected to run (per spec §6.4 the loop is intentionally
manual). Replay the fixture through the corresponding skill, capture
the output, and replace the golden file in-place. Commit the diff
alongside the body change that motivated the refresh — never commit a
stale golden.
