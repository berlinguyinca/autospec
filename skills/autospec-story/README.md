# autospec-story

Read-only story mode for an existing GitHub repository. It reconciles local specs,
docs, GitHub issues, GitHub PRs, and recent git history into a complete product
story and implementation-state report.

Use it when you need to answer:

- What is this application trying to become?
- Which capabilities are implemented, in progress, planned, or unknown?
- Which specs, issues, PRs, and commits support that conclusion?
- What should the team inspect next?

## Usage

```bash
/autospec-story
/autospec-story --output docs/autospec-story.md
/autospec-story --since 2026-05-01 --limit 200 --output docs/autospec-story.md
```

The workflow is read-only against GitHub and the target repo unless `--output` is
provided, in which case it writes the requested Markdown report path.

## Evidence Sources

- Local docs and specs, including `docs/specs/*.md`, `docs/superpowers/specs/*.md`, `.omx/plans/*.md`, `README.md`, `AGENTS.md`, and architecture/runbook docs.
- GitHub issues in both open and closed states.
- GitHub PRs in open, merged, and closed states.
- Recent git history and dirty worktree status.

## Output

The report is grouped by product capability, not just by issue number. It
separates direct evidence from inference and unknowns, and every major claim is
expected to cite a local path, issue, PR, or commit id.

## Self-update

Once installed, run the skill with the literal argument `update` to refresh in
place:

```bash
/autospec-story update
```
