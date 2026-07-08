# Constitution and Baseline Integration Follow-up Issues

Source spec: `docs/specs/2026-06-27-constitution-baseline-integration.md`

These issue drafts decompose implementation after the spec-first PR. Each draft is
small, validation-backed, and preserves the current autonomous platform contract:
blocking source failures produce deterministic validation evidence, and
autonomous runs quarantine the affected work item instead of prompting or blocking
unrelated queue items.

## Issue 1: feat: add constitution and baseline config schema

## Goal

Add `.autospec/autospec.yml` schema validation for `constitution` and `baselines`.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `.autospec/autospec.yml`
- `scripts/validate.sh`

## Files touched

- `schemas/autospec-state/constitution-baseline.schema.json`
- `scripts/validate.sh`
- `tests/unit/test_constitution_config_schema.bats`

## Implementation outline

- Add schema fields for `constitution.source` and `baselines.sources`.
- Reject duplicate `baselines.sources[].id` values.
- Wire the schema check into `scripts/validate.sh`.

## Tests required

- `tests/unit/test_constitution_config_schema.bats`

## Acceptance criteria

- [ ] `constitution.source.type` accepts `local` and `github` in `*.yml` fixtures.
- [ ] Duplicate `baselines.sources[].id` values fail `test_constitution_config_schema.bats`.
- [ ] GitHub sources validate a 40-character `commit` when locked.

## Dependencies

none

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_constitution_config_schema.bats
```

## Issue 2: feat: add local constitution loader

## Goal

Add a local-path Constitution loader that emits normalized `constitution.json`.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-constitution-validate.sh`
- `tests/unit/test_constitution_validation.bats`

## Files touched

- `scripts/autospec-constitution-validate.sh`
- `scripts/autospec-constitution-rules.py`
- `tests/unit/test_constitution_validation.bats`

## Implementation outline

- Load `constitution.source.type: local` from repo-contained relative paths.
- Reject relative paths that traverse outside the repository root.
- Allow absolute local paths only as explicit workstation-local input.

## Tests required

- `tests/unit/test_constitution_validation.bats`

## Acceptance criteria

- [ ] `tests/fixtures/constitution` loads a `kind: constitution` manifest.
- [ ] `../autospec-constitution` exits non-zero in `test_constitution_validation.bats`.
- [ ] Profile inheritance cycles in fixture `cycle` exit non-zero.

## Dependencies

Depends on issue #1

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_constitution_validation.bats
```

## Issue 3: feat: add GitHub constitution loader

## Goal

Add a GitHub Constitution loader that resolves versions to pinned `commit` SHAs.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-constitution-validate.sh`
- `tests/unit/test_policy_sources_v2.bats`

## Files touched

- `scripts/autospec-load-policy-sources.sh`
- `scripts/autospec-lock-policy-sources.sh`
- `tests/unit/test_policy_sources_v2.bats`

## Implementation outline

- Resolve `version` through GitHub only in explicit refresh flows.
- Refuse normal execution for unlocked GitHub sources.
- Emit current-repo lockfile diffs without companion-repo writes.

## Tests required

- `tests/unit/test_policy_sources_v2.bats`

## Acceptance criteria

- [ ] Tag `v1.0.0` fixture refs resolve to a 40-character `commit`.
- [ ] A `version` and `commit` mismatch exits non-zero.
- [ ] Refresh output writes current-repo lockfile evidence only.

## Dependencies

Depends on issue #1

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_policy_sources_v2.bats
```

## Issue 4: feat: add baseline pack loader

## Goal

Add a baseline pack loader that emits normalized `baseline-profiles.json`.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-baseline-compose.sh`
- `tests/unit/test_baseline_composition.bats`

## Files touched

- `scripts/autospec-baseline-compose.sh`
- `scripts/autospec-baseline-gap.sh`
- `tests/unit/test_baseline_composition.bats`

## Implementation outline

- Load local and locked GitHub baseline pack manifests.
- Resolve profile references as `<source-id>/<profile-id>`.
- Keep baseline recommendations subordinate to Constitution law.

## Tests required

- `tests/unit/test_baseline_composition.bats`

## Acceptance criteria

- [ ] `core/spec-first` resolves from configured `baselines.sources[].id`.
- [ ] Unknown profile reference `core/missing` exits non-zero.
- [ ] Advisory baseline rules remain separate from Constitution law.

## Dependencies

Depends on issue #2

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_baseline_composition.bats
```

## Issue 5: feat: add constitution lockfile

## Goal

Add `.autospec/constitution.lock.json` generation and refresh semantics.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-lock-policy-sources.sh`
- `tests/unit/test_policy_sources_v2.bats`

## Files touched

- `scripts/autospec-lock-policy-sources.sh`
- `schemas/autospec-state/constitution-lock.schema.json`
- `tests/unit/test_policy_sources_v2.bats`

## Implementation outline

- Generate deterministic lockfile JSON with Constitution and baseline pins.
- Report changed pins as dependency-update evidence.
- Refuse normal GitHub-source execution without locked commits.

## Tests required

- `tests/unit/test_policy_sources_v2.bats`

## Acceptance criteria

- [ ] `.autospec/constitution.lock.json` records Constitution `repo`, `version`, and `commit`.
- [ ] Each baseline source records `id` and 40-character `commit`.
- [ ] `--refresh-lock` rewrites one deterministic JSON file.

## Dependencies

Depends on issue #3

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_policy_sources_v2.bats
```

## Issue 6: feat: add profile composition resolver

## Goal

Add an `effective-profile.json` resolver for law, playbooks, overrides, and flags.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-baseline-compose.sh`
- `tests/unit/test_baseline_composition.bats`

## Files touched

- `scripts/autospec-baseline-compose.sh`
- `schemas/autospec-state/effective-profile.schema.json`
- `tests/unit/test_baseline_composition.bats`

## Implementation outline

- Apply the four-layer precedence order from the source spec.
- Reject attempts to weaken non-overrideable Constitution law.
- Emit JSON-pointer provenance for winning composed values.

## Tests required

- `tests/unit/test_baseline_composition.bats`

## Acceptance criteria

- [ ] Resolver output is deterministic JSON for fixture `composition/basic`.
- [ ] Constitution-required validation cannot be removed by local override.
- [ ] Duplicate profile references load exactly 1 time.

## Dependencies

Depends on issue #4

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_baseline_composition.bats
```

## Issue 7: feat: add constitution validation command

## Goal

Add `autospec-constitution-validate.sh` gate for config, sources, lockfile, and composition.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-constitution-validate.sh`
- `scripts/validate.sh`

## Files touched

- `scripts/autospec-constitution-validate.sh`
- `scripts/validate.sh`
- `tests/unit/test_constitution_validation.bats`

## Implementation outline

- Run config, source, lockfile, and composition validation in order.
- Return non-zero for required source failures.
- Preserve JSON evidence for autonomous quarantine records.

## Tests required

- `tests/unit/test_constitution_validation.bats`

## Acceptance criteria

- [ ] Valid local fixtures exit 0 in `test_constitution_validation.bats`.
- [ ] An unlocked GitHub source exits non-zero.
- [ ] Validation JSON includes `quarantine_evidence` for required source failures.

## Dependencies

Depends on issue #6

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_constitution_validation.bats
```

## Issue 8: feat: add gap analysis input contract

## Goal

Add a generator for `gap-analysis-input.json` from repo inventory and profile data.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-baseline-gap.sh`
- `tests/unit/test_repository_intelligence.bats`

## Files touched

- `scripts/autospec-baseline-gap.sh`
- `scripts/autospec-discover-metadata.sh`
- `tests/unit/test_repository_intelligence.bats`

## Implementation outline

- Read effective profile, lockfile, repo inventory, and prior findings.
- Emit repo branch/SHA metadata and profile identifiers.
- Keep analyzer execution out of scope for this issue.

## Tests required

- `tests/unit/test_repository_intelligence.bats`

## Acceptance criteria

- [ ] Generated input includes `.git` branch and SHA metadata.
- [ ] Generated input includes Constitution profile and 1 rule entry.
- [ ] Generated input includes baseline profile IDs from `.autospec/autospec.yml`.

## Dependencies

Depends on issue #7

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_repository_intelligence.bats
```

## Issue 9: feat: add constitution metadata generation

## Goal

Add metadata generation from lockfile, effective profile, git metadata, and command context.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `scripts/autospec-discover-metadata.sh`
- `tests/unit/test_repository_intelligence.bats`

## Files touched

- `scripts/autospec-discover-metadata.sh`
- `schemas/autospec-state/constitution-metadata.schema.json`
- `tests/unit/test_repository_intelligence.bats`

## Implementation outline

- Read `.autospec/constitution.lock.json` and effective profile digest.
- Accept command context such as `autospec-define` or `autospec-run`.
- Mark enforcement as absent unless validation actually ran.

## Tests required

- `tests/unit/test_repository_intelligence.bats`

## Acceptance criteria

- [ ] Metadata includes Constitution `repo`, `version`, `commit`, and `profile`.
- [ ] Metadata includes baseline source `id`, `version`, `commit`, and profiles.
- [ ] Metadata includes command context and current git SHA.

## Dependencies

Depends on issue #7

## Verification

### Primary smoke test (inner loop)

```bash
bats tests/unit/test_repository_intelligence.bats
```

## Issue 10: dogfood: consume locked constitution and baselines

## Goal

Configure this repo to consume locked `autospec-constitution` and `autospec-baselines` inputs.

## Files to read first

- `docs/specs/2026-06-27-constitution-baseline-integration.md`
- `.autospec/autospec.yml`
- `docs/companion-repositories.md`

## Files touched

- `.autospec/autospec.yml`
- `.autospec/constitution.lock.json`
- `docs/companion-repositories.md`

## Implementation outline

- Add real Constitution and baseline sources after loader and lockfile phases ship.
- Commit pinned SHAs in `.autospec/constitution.lock.json`.
- Keep companion repository updates proposal-only.

## Tests required

- `bash scripts/validate.sh`

## Acceptance criteria

- [ ] `.autospec/autospec.yml` references `autospec-constitution` as a locked source.
- [ ] `.autospec/constitution.lock.json` contains 2 companion repository SHAs.
- [ ] `docs/companion-repositories.md` states companion writes remain proposal-only.

## Dependencies

Depends on issue #9

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh
```
