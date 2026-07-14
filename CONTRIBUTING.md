# Contributing

AutoSpec welcomes focused contributions that make the workflow easier to understand, safer to run, or easier to validate.

## Good First Contributions

- Fix unclear docs.
- Improve `examples/hello-autospec/`.
- Add validation coverage for an existing script.
- Tighten an error message.
- Improve install or quickstart diagnostics.

Avoid opening a first PR that adds broad autonomy, new dependencies, or large prompt rewrites.

## Local Setup

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
bash scripts/dev-bootstrap.sh
autospec validate --fast
```

Optional tools such as `bats`, `ajv`, `yq`, `gh`, and browser automation unlock deeper validation.

## Development Rules

- Use conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`.
- Keep diffs small, reviewable, and reversible.
- Do not commit credentials, private data, generated caches, or virtual environments.
- Do not bypass hooks with `--no-verify`.
- Update docs for public behavior changes.
- Add or update validation when changing scripts or workflow behavior.

## Skill Lock-Step Rule

Many skills are mirrored across harnesses:

```text
skills/<name>/SKILL.md
skills/<name>/codex/prompt.md
skills/<name>/opencode/agent.md
```

When editing a multi-harness skill, keep the bodies identical except for frontmatter. `autospec validate` enforces this.

## Validation

Run the relevant focused test first, then the repo validator:

```bash
autospec validate --fast
bash scripts/validate-launch-readiness.sh
```

Before a release or broad workflow change, run:

```bash
autospec validate
```

E2E tests may create throwaway GitHub resources and should only run with credentials that are safe for that purpose.

## Pull Request Checklist

- Explain the user-facing result.
- List validation commands and outcomes.
- Call out likely hidden failure.
- Keep unrelated cleanup out of the PR.
- For docs-only changes, still run the launch-readiness or docs-relevant validator.
