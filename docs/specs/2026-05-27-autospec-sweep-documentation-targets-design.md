# Autospec Sweep Documentation Targets Design

## Goal

Make `/autospec-sweep` answer "does this build deep documentation for different
people and scopes?" with a concrete yes: the project config declares the
documentation audiences and product/operational scopes that must exist, and the
sweep emits bounded docs gaps when those targets are missing or cannot be kept
in sync.

## Team Personality

**Selected team:** Documentation platform engineering.

**Roles:** documentation owner, developer-experience engineer, backend/tooling
engineer, test engineer, product maintainer.

**Why this fits:** the feature is not prose generation alone; it is a config and
review contract that lets downstream autospec issues build the right docs for
the right readers without hard-coding one README as the whole documentation
surface.

**Risks to notice:** config drift, vague doc targets, missing doc-scope metadata,
generated issues that are too broad, and docs work that cannot be deduped.

**Review counter-team:** operations and security review.

**Counter-team roles:** operator, security advisor, maintainer, QA engineer.

**Blind spots to challenge:** unsafe production assumptions, missing rollback or
incident docs, security docs treated as optional, and broad docs gaps that would
produce unreviewable PRs.

## Architecture

Add an optional top-level `documentation` section to `.autospec/autospec.yml`.
It contains `enabled`, `audiences[]`, and `scopes[]`. Each target has an `id`,
`label`, `path`, `focus`, and optional `require_scope`. The schema validates the
shape while remaining backward-compatible with older configs that lack the
section.

The first-run wizard emits default targets for users, developers, operators,
security reviewers, repository overview, user workflows, API/reference,
operations runbooks, and troubleshooting.

## Sweep Behavior

`autospec-sweep-review.sh` reads the documentation matrix whenever docs checks
are enabled. For each target it emits:

- a medium docs gap if the configured Markdown file is missing;
- a low docs gap if `require_scope: true` and the file lacks an
  `autospec-doc-scope` marker.

Each gap has a stable `dedupe_key`, concrete file path, and focus text so
gap-remediation can file one small issue per missing audience/scope target.

## Testing

Unit tests cover wizard defaults, schema acceptance, and review output for both
missing documentation files and missing scope metadata. Standard repo validation
continues to run `scripts/validate.sh`, the sweep config tests, the sweep runner
tests, and schema compilation.

## Out Of Scope

This change does not synthesize final prose inside the deterministic sweep
runner. It creates the contract and emits work items; the existing autospec issue
and implementation loop remains responsible for writing and reviewing the docs.
