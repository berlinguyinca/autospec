# Workflows

## Plan A Feature

Use `/autospec-define` when you want a spec and issue queue before implementation starts.

Output:

- `docs/specs/*.md` design spec
- Parent issue
- Linked child issues
- Model-fit labels

## Ship Ready Issues

Use `/autospec-run` when issues already carry `auto-implement`.

Output:

- Branch per issue
- Pull request per issue
- Validation output
- Reviewer result
- Closeout report

## Split An Existing Spec

Use `/autospec-split` when a design already exists under `docs/specs/`.

Output:

- Parent issue
- Child issues
- Classification
- Handoff to `/autospec-run`

## Audit Release Readiness

Use `/autospec-release` when you need a release verdict.

Output:

- Validation summary
- QA status
- Docs drift status
- Blocker list
- Release verdict

## Explain The Repository

Use `/autospec-story` when you need a cited narrative of what the repo does and what has shipped.

Output:

- Product story
- Implementation-state overview
- References to specs, docs, issues, PRs, and git history

## Stop Or Resume

Use `/autospec-stop` and `/autospec-resume` for long-running monitor control.

Output:

- Graceful stop, immediate pause, or resume action
- Preserved issue context
- Clean monitor state

