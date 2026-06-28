# Constitution and Baseline Integration Foundation

**Date:** 2026-06-27
**Status:** spec-first foundation; no engine implementation in this phase.

## Goal

Teach autospec how it will discover, pin, validate, and compose external
Autospec Constitution and Autospec Baseline repositories before any runtime engine
behavior is built.

Autospec Constitution is the law: normative rules, severity definitions, and
non-negotiable gates. Autospec Baselines are the playbooks: reusable profile
packs, workflow defaults, validation recipes, metadata templates, and gap-analysis
schemas. Autospec is the engine: it reads the law and playbooks, resolves a local
profile, then drives specs, issues, validation, metadata, and gap reports from the
resolved contract.

## Non-goals

- No constitution enforcement engine in this phase.
- No baseline pack execution in this phase.
- No automatic repository cloning, network fetching, or lockfile writer yet.
- No behavior changes to existing autospec skills, scripts, or validation gates.
- No migration of existing `.autospec/autospec.yml` files beyond documenting the
  future shape.

## Configuration contract

The project-level source of truth remains `.autospec/autospec.yml`. This phase
reserves a top-level `constitution` section and a top-level `baselines` section.

```yaml
version: 1

constitution:
  source:
    type: github
    repo: berlinguyinca/autospec-constitution
    path: .
    version: v1.0.0
    commit: 0123456789abcdef0123456789abcdef01234567
  profile: default
  required: true

baselines:
  sources:
    - id: core
      type: github
      repo: berlinguyinca/autospec-baselines
      path: packs/core
      version: v1.0.0
      commit: 89abcdef0123456789abcdef0123456789abcdef
    - id: local-overrides
      type: local
      path: .autospec/baselines/local
  profiles:
    - core/spec-first
    - core/autonomous-safe
    - local-overrides/project
  overrides:
    validation:
      commands:
        test: bash scripts/validate.sh
```

### Source fields

| Field | Applies to | Required | Meaning |
|---|---|---:|---|
| `type` | all | yes | `local` or `github`. |
| `id` | baseline sources | yes | Stable identifier used by profile references. |
| `path` | all | yes | Path to the constitution root or baseline pack root. |
| `repo` | github | yes | GitHub `owner/name` repository. |
| `version` | github | no | Human-facing tag or release name, for example `v1.0.0`. |
| `commit` | github | yes after lock | Full 40-character SHA used for reproducibility. |
| `profile` | constitution | no | Constitution profile to load; defaults to `default`. |
| `required` | constitution | no | When true, validation fails if the source cannot resolve. |

`version` is convenient intent; `commit` is the authority for execution once a
lockfile exists. A config may start with `version` only during authoring, but the
loader must refuse execution without a pinned commit unless explicitly running in
`--refresh-lock` mode.

## Supported source types

### Local path

Local sources read from a path relative to the repository root unless the path is
absolute. The loader must not follow paths outside the repository unless the user
explicitly uses an absolute path. Local sources are trusted only for local
development; metadata must record that they are unpinned and not reproducible
unless a content digest is captured in the lockfile.

Example:

```yaml
constitution:
  source:
    type: local
    path: ../autospec-constitution
```

### GitHub repository

GitHub sources identify `owner/name`, an optional subpath, and a desired version.
The resolver maps `version` to a commit SHA, records the SHA in the lockfile, and
future reads use the SHA. Network access is allowed only in lock refresh or initial
resolution paths; normal engine runs consume the lockfile.

Example:

```yaml
baselines:
  sources:
    - id: regulated
      type: github
      repo: berlinguyinca/autospec-baselines
      path: packs/regulated
      version: v1.2.0
      commit: fedcba9876543210fedcba9876543210fedcba98
```

### Version or tag

`version` is resolved through the GitHub refs API or `git ls-remote` and may point
to a tag or branch-like release channel. Tags are preferred. Branches are allowed
only for authoring and must lock to a commit before execution.

### Pinned commit SHA

`commit` pins the exact source revision. A source with both `version` and
`commit` is valid only when the version resolves to the same commit or an
annotated tag object whose peeled commit is the same SHA.

## Constitution loader responsibilities

The constitution loader is responsible for producing a normalized Constitution
document that the engine can treat as authoritative.

- Read the configured constitution source from local disk or a locked GitHub SHA.
- Validate the source manifest, expected files, profile name, and schema version.
- Resolve inherited constitution profiles inside the same source.
- Normalize rule IDs, severity names, gate semantics, and required metadata fields.
- Reject ambiguous or conflicting law, such as duplicate rule IDs with different
  meanings.
- Expose a stable JSON document for downstream consumers.
- Emit provenance for every rule: source type, repo/path, version, commit, file,
  and section anchor.
- Fail closed when `constitution.required: true` and resolution or validation
  fails.

The loader must not interpret baseline playbooks as law. Baselines may recommend
checks and workflows, but only the Constitution defines mandatory rule semantics.

## Baseline pack loader responsibilities

The baseline pack loader is responsible for producing normalized playbook data
from one or more baseline sources.

- Read each configured baseline source in declared order.
- Validate each pack manifest, profile list, schema version, and declared
  compatibility with the loaded Constitution.
- Resolve profile references of the form `<source-id>/<profile-id>`.
- Load workflow defaults, validation command templates, issue templates, metadata
  templates, and gap-analysis input presets.
- Preserve source provenance for each imported value.
- Report but do not silently ignore unknown profile references.
- Keep baseline recommendations subordinate to Constitution requirements.
- Expose normalized pack data to profile composition.

## Profile composition rules

Profile composition creates one effective Autospec profile for the current repo.
The resolver applies inputs in this order:

1. Constitution defaults.
2. Baseline profiles in `.autospec/autospec.yml` order.
3. Local `.autospec/autospec.yml` overrides.
4. Runtime flags for the current command.

Conflict rules:

- Constitution rules cannot be disabled by baseline or local override.
- A stricter local value may raise severity, add validation, or narrow scope.
- A weaker local value may only be accepted when the Constitution marks the field
  as advisory or explicitly overrideable.
- Lists are merged by stable ID when items have IDs; otherwise they append in
  source order.
- Maps merge shallowly by default unless a schema marks a field as replace-only.
- Repeated profile IDs are idempotent and load once.
- Cyclic profile inheritance is invalid.
- The resolver must emit a trace explaining the winning source for each composed
  field.

The output contract is `effective-profile.json`: a deterministic JSON document
with sorted keys, resolved values, and a `provenance` object keyed by JSON pointer.

## Validation rules

Validation has three layers.

### Configuration validation

- `.autospec/autospec.yml` must parse as YAML and use `version: 1`.
- `constitution.source.type` must be `local` or `github`.
- `baselines.sources[].type` must be `local` or `github`.
- GitHub sources must include `repo`; locked GitHub sources must include a
  40-character `commit`.
- Baseline source IDs must be unique and match `^[a-z0-9][a-z0-9._-]*$`.
- Profile references must resolve to configured baseline source IDs.

### Source validation

- Constitution and baseline manifests must declare schema version and kind.
- Source kind must match usage: Constitution source cannot be loaded as baseline.
- Referenced files must stay inside the source root.
- Unknown required schema fields are fatal; unknown optional fields are warnings.
- GitHub `version` and `commit` must agree when both are present.

### Composition validation

- Effective profile must include one Constitution identity.
- No Constitution-required validation command may be removed.
- Rule IDs must be globally unique after composition.
- Metadata fields required by the Constitution must have either a generated value
  provider or an explicit local value.
- Gap-analysis contract must include every required Constitution and baseline
  input.

## Lockfile design

The first implementation should use `.autospec/constitution.lock.json` as the
single lockfile for both the Constitution and baseline packs. The name is chosen
to make the law-first relationship explicit while still locking the playbooks
that were composed against it.

```json
{
  "version": 1,
  "generated_at": "2026-06-27T00:00:00Z",
  "config_path": ".autospec/autospec.yml",
  "constitution": {
    "source": {
      "type": "github",
      "repo": "berlinguyinca/autospec-constitution",
      "path": ".",
      "version": "v1.0.0",
      "commit": "0123456789abcdef0123456789abcdef01234567"
    },
    "manifest_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    "profile": "default",
    "rules_digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222"
  },
  "baselines": [
    {
      "id": "core",
      "source": {
        "type": "github",
        "repo": "berlinguyinca/autospec-baselines",
        "path": "packs/core",
        "version": "v1.0.0",
        "commit": "89abcdef0123456789abcdef0123456789abcdef"
      },
      "manifest_digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
      "profiles": ["core/spec-first", "core/autonomous-safe"]
    }
  ],
  "composition": {
    "profile_digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
    "profiles": ["core/spec-first", "core/autonomous-safe", "local-overrides/project"]
  }
}
```

Lockfile rules:

- Normal runs read the lockfile and fail if locked GitHub sources are missing.
- `--refresh-lock` may resolve versions and rewrite commits.
- Local sources record content digests and `locked: false` unless a future local
  snapshot mechanism is added.
- The lockfile is committed when the project intends reproducible execution.
- A changed lockfile is a reviewable dependency update, not an incidental
  generated artifact.

## Gap analysis input/output contract

Gap analysis compares the repo's current state against the effective Constitution
and baseline playbooks. This phase defines the contract; it does not build the
analyzer.

Input:

```json
{
  "version": 1,
  "repo": {
    "root": "/workspace/project",
    "git_sha": "abc123",
    "branch": "feat/example"
  },
  "constitution": {
    "profile": "default",
    "rules": []
  },
  "effective_profile": {},
  "baseline_profiles": [],
  "inventory": {
    "files": [],
    "commands": [],
    "skills": [],
    "docs": []
  },
  "prior_findings": []
}
```

Output:

```json
{
  "version": 1,
  "summary": {
    "status": "pass",
    "blocking": 0,
    "advisory": 0
  },
  "findings": [
    {
      "id": "constitution.rule-id.repo-path",
      "severity": "blocking",
      "rule_id": "constitution.rule-id",
      "source": "constitution",
      "message": "Repository is missing required metadata.",
      "evidence": [{"path": ".autospec/autospec.yml"}],
      "recommended_issue": {
        "title": "feat: add required metadata generation",
        "labels": ["auto-implement"]
      }
    }
  ],
  "metadata": {
    "constitution_commit": "0123456789abcdef0123456789abcdef01234567",
    "profile_digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444"
  }
}
```

Blocking findings come only from Constitution law or from baseline rules that the
Constitution explicitly promotes to required status. Advisory findings may come
from baseline playbooks.

## Metadata generation contract

Metadata generation turns the resolved Constitution and baseline profile into
repo-local metadata that later autospec phases can attach to specs, issues, PRs,
and closeout reports.

Input:

- `effective-profile.json`
- `.autospec/constitution.lock.json`
- current git metadata
- command context, such as `/autospec-define`, `/autospec-run`, or validation

Output:

```json
{
  "version": 1,
  "generated_at": "2026-06-27T00:00:00Z",
  "constitution": {
    "repo": "berlinguyinca/autospec-constitution",
    "version": "v1.0.0",
    "commit": "0123456789abcdef0123456789abcdef01234567",
    "profile": "default"
  },
  "baselines": [
    {
      "id": "core",
      "repo": "berlinguyinca/autospec-baselines",
      "version": "v1.0.0",
      "commit": "89abcdef0123456789abcdef0123456789abcdef",
      "profiles": ["core/spec-first", "core/autonomous-safe"]
    }
  ],
  "effective_profile_digest": "sha256:4444444444444444444444444444444444444444444444444444444444444444",
  "command": "autospec-define",
  "repo": {
    "branch": "feat/example",
    "git_sha": "abc123"
  }
}
```

Generation rules:

- Metadata is deterministic for the same inputs except `generated_at`.
- Generated metadata must never claim enforcement that did not run.
- Specs and issues may cite metadata as provenance, but validation commands must
  still read the effective profile directly.
- The metadata contract must be safe to embed in PR bodies and closeout reports.

## Future implementation phases

1. **Config schema.** Add schema and validation for `constitution` and `baselines`
   blocks in `.autospec/autospec.yml`.
2. **Local constitution loader.** Load and validate a local Constitution source.
3. **GitHub constitution loader.** Resolve GitHub refs, require commit pins, and
   cache locked content.
4. **Baseline pack loader.** Load baseline manifests and profile packs from local
   and GitHub sources.
5. **Lockfile writer.** Generate and refresh `.autospec/constitution.lock.json`.
6. **Profile composition resolver.** Compose law, playbooks, local overrides, and
   runtime flags into `effective-profile.json`.
7. **Validation command.** Add an explicit command that validates config, sources,
   lockfile, and composition.
8. **Gap analysis input contract.** Produce the analyzer input envelope from repo
   inventory and effective profile.
9. **Metadata generation input contract.** Generate provenance metadata for specs,
   issues, PRs, and reports.
10. **Dogfood.** Configure this repository to consume `autospec-constitution` and
    `autospec-baselines` through the documented contract.

## Acceptance criteria

- [ ] `.autospec/autospec.yml` has a documented Constitution and baseline shape.
- [ ] Local, GitHub, version/tag, and pinned commit sources are defined.
- [ ] Constitution loader responsibilities are separated from baseline loader
      responsibilities.
- [ ] Profile composition order, conflict handling, and provenance are specified.
- [ ] `.autospec/constitution.lock.json` format and refresh semantics are specified.
- [ ] Gap analysis input and output contracts are defined.
- [ ] Metadata generation input and output contracts are defined.
- [ ] Follow-up implementation issues are decomposed in
      `docs/specs/constitution-baseline-integration-issues.md`.
