# Constitution and Baseline Integration Follow-up Issues

Source spec:
`docs/specs/2026-06-27-constitution-baseline-integration.md`

These issues intentionally decompose implementation after the spec-first PR. Each
issue should stay small, ship with validation, and avoid enabling full
Constitution enforcement before the loader, lockfile, and composition contracts
exist.

## 1. feat: add constitution and baseline config schema

## Goal

Add schema validation for `.autospec/autospec.yml` `constitution` and `baselines` blocks.

## Implementation outline

- `schemas/` or the existing config-schema location: add fields from the source
  spec.
- `scripts/validate.sh`: include the new schema check if config schema checks are
  already gated there.
- `tests/unit/`: add valid and invalid config fixtures.

## Acceptance criteria

- [ ] `.autospec/autospec.yml` accepts `constitution.source.type` values `local` and `github`.
- [ ] `.autospec/autospec.yml` rejects duplicate `baselines.sources[].id` values.
- [ ] GitHub sources require `repo` and validate a 40-character `commit` when locked.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 2. feat: add local constitution loader

## Goal

Add a local-path Constitution loader that emits normalized Constitution JSON.

## Implementation outline

- Add a loader script or library entry point for `constitution.source.type: local`.
- Validate source root, manifest kind, schema version, profile name, and required
  files.
- Reject path traversal outside the source root.
- Add fixtures under `tests/fixtures/constitution/`.

## Acceptance criteria

- [ ] A local path source loads a `kind: constitution` manifest from `tests/fixtures/constitution`.
- [ ] Loader rejects a missing manifest with a non-zero exit.
- [ ] Loader rejects profile inheritance cycles in fixture `cycle`.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 3. feat: add GitHub constitution loader

## Goal

Add a GitHub Constitution loader that resolves versions to pinned commit SHAs.

## Implementation outline

- Add resolver support for `constitution.source.type: github`.
- Resolve `version` through `git ls-remote` or GitHub API.
- Require locked execution to use `commit`.
- Cache or stage fetched content without changing engine behavior.

## Acceptance criteria

- [ ] Loader resolves tag `v1.0.0` fixture refs to a 40-character `commit`.
- [ ] Loader rejects a `version` and `commit` mismatch.
- [ ] Normal execution refuses an unlocked GitHub Constitution source.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 4. feat: add baseline pack loader

## Goal

Add a baseline pack loader that emits normalized playbook profile data.

## Implementation outline

- Load baseline pack manifests from configured local and GitHub sources.
- Resolve profile references as `<source-id>/<profile-id>`.
- Validate pack schema version and Constitution compatibility declaration.
- Preserve provenance per loaded field.

## Acceptance criteria

- [ ] Loader resolves `core/spec-first` from a configured `baselines.sources[].id`.
- [ ] Loader rejects an unknown profile reference.
- [ ] Loader distinguishes advisory baseline recommendations from Constitution law.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 5. feat: add constitution lockfile

## Goal

Add `.autospec/constitution.lock.json` generation and refresh semantics.

## Implementation outline

- Add lockfile schema with Constitution, baseline, and composition sections.
- Implement `--refresh-lock` path for resolving versions to commits.
- Refuse normal GitHub-source execution without a lockfile commit.
- Add fixtures for deterministic lockfile output.

## Acceptance criteria

- [ ] `.autospec/constitution.lock.json` records Constitution `repo`, `version`, and `commit`.
- [ ] `.autospec/constitution.lock.json` records each baseline source `id` and `commit`.
- [ ] `--refresh-lock` rewrites a changed version pin in one deterministic JSON file.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 6. feat: add profile composition resolver

## Goal

Add a resolver that composes Constitution defaults, baseline profiles, local overrides, and runtime flags.

## Implementation outline

- Implement the four-layer precedence order from the source spec.
- Merge lists by stable ID and maps by schema-defined strategy.
- Reject attempts to weaken non-overrideable Constitution law.
- Emit `effective-profile.json` plus JSON-pointer provenance.

## Acceptance criteria

- [ ] Resolver output is deterministic JSON for the same fixture input.
- [ ] Constitution-required validation cannot be removed by local override.
- [ ] Duplicate profile references are idempotent and load once.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 7. feat: add constitution/baseline validation command

## Goal

Add a command that validates config, sources, lockfile, and profile composition.

## Implementation outline

- Add a CLI or script entry point for Constitution/Baseline validation.
- Run config validation, source validation, lockfile validation, and composition
  validation in order.
- Emit machine-readable JSON and human-readable summary output.
- Wire the command into repository validation only after fixtures pass.

## Acceptance criteria

- [ ] Validation command exits 0 for valid local fixtures.
- [ ] Validation command exits non-zero for an unlocked GitHub source.
- [ ] Validation command reports missing profile references by source ID and profile ID.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 8. feat: add gap analysis input contract

## Goal

Add a generator for the gap-analysis input envelope defined by the source spec.

## Implementation outline

- Read effective profile, lockfile, repo inventory, and prior findings.
- Emit `version`, `repo`, `constitution`, `effective_profile`,
  `baseline_profiles`, `inventory`, and `prior_findings`.
- Keep the analyzer itself out of scope.

## Acceptance criteria

- [ ] Generated input includes `.git` branch and SHA metadata.
- [ ] Generated input includes Constitution profile and rule data.
- [ ] Generated input includes baseline profile IDs from `.autospec/autospec.yml`.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 9. feat: add metadata generation input contract

## Goal

Add metadata generation from effective profile, lockfile, git metadata, and command context.

## Implementation outline

- Read `.autospec/constitution.lock.json` and effective profile digest.
- Accept command context such as `autospec-define` or `autospec-run`.
- Emit deterministic metadata safe for spec, issue, PR, and closeout embedding.
- Explicitly mark enforcement as absent unless validation actually ran.

## Acceptance criteria

- [ ] Metadata output includes Constitution `repo`, `version`, `commit`, and `profile`.
- [ ] Metadata output includes baseline source `id`, `version`, `commit`, and profiles.
- [ ] Metadata output includes command context and current git SHA.
- [ ] `bash scripts/validate.sh --changed=origin/main` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh --changed=origin/main
```

## 10. dogfood: configure autospec to use autospec-constitution and autospec-baselines

## Goal

Configure this repository's `.autospec/autospec.yml` to consume `autospec-constitution` and `autospec-baselines`.

## Implementation outline

- Add real Constitution and baseline sources after the loader and lockfile phases
  are shipped.
- Commit `.autospec/constitution.lock.json` with pinned SHAs.
- Run the validation command and the existing repo validation suite.
- Document any repo-specific local overrides.

## Acceptance criteria

- [ ] `.autospec/autospec.yml` references `berlinguyinca/autospec-constitution`.
- [ ] `.autospec/autospec.yml` references `berlinguyinca/autospec-baselines`.
- [ ] `.autospec/constitution.lock.json` pins both repositories by 40-character SHA.
- [ ] `bash scripts/validate.sh && bats tests/unit tests/smoke` passes.

### Primary smoke test (inner loop)

```bash
bash scripts/validate.sh && bats tests/unit tests/smoke
```

