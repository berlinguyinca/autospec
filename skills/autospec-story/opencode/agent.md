---
name: autospec-story
description: Use when the user wants a repo-level product story, implementation-state overview, or narrative synthesis from local specs plus GitHub issues and PRs for the current repository.
mode: primary
---

# autospec-story (harness-neutral)

Read-only repo storyteller. Build a complete overview of the current GitHub
repository by reconciling local specs, docs, GitHub issues, GitHub PRs, and
recent git history into one cited Markdown narrative: what the application is,
why it exists, what has been implemented, what remains open, and which parts
are evidence versus inference.

Manage your own context — never exceed 60%. Delegate the final synthesis to a
Tier A subagent whenever your harness supports it.

<!-- autospec-block:startup-self-update SKILL_NAME=autospec-story -->

## Self-update mode

If the feature-request argument matches the regex `^\s*update\s*$`
(case-insensitive, whitespace-padded), this skill enters self-update mode and
does not run the normal story workflow:

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-story/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-story.md`
   - Codex CLI:   `~/.codex/prompts/autospec-story.md`
2. **Re-install the full autospec suite from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched
   copy.
4. **Stop.** Do not gather repo state or write a story report. Print the upgrade
   summary and return to the user.

If no install path is detected, print `Self-update: no installed copy of
autospec-story found; run install.sh first.` and exit.

## Invocation

```
/autospec-story [--since <date>] [--output <path>] [--limit <N>]
```

- `--since <date>` — limit GitHub issue, PR, and git-history collection to
  items created or updated since the date. Accepts GitHub search date syntax
  such as `2026-05-01`.
- `--output <path>` — write the report to a local Markdown file in addition to
  printing the summary. Default: print only.
- `--limit <N>` — cap GitHub issue and PR collection per state bucket. Default:
  `500`.

This skill is read-only with respect to the target repo and GitHub: do not edit
issues, labels, projects, branches, commits, or pull requests.

## Harness detection (run once at skill start)

Detect your harness by checking available tools before dispatching work:

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (model ID: claude-opus-4-7)
   - `TIER_B` = `sonnet`               (model ID: claude-sonnet-4-6)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch` is the primary edit tool.
   - `TIER_A` = current top GPT model + `reasoning_effort=high`
   - `TIER_B` = `gpt-5.1-codex-spark` + `reasoning_effort=medium`

**Fallback rule:** If `TIER_B` is not available in your harness (model unknown, quota/capacity failure, authorization failure, or tool call returns an error for that model), silently retry the same dispatch with `TIER_A`. Preserve the parent context on retry; for Codex native subagents, fork/inherit the current conversation context and use the latest top GPT model instead of moving the work into the main session. Never ask the user.

Hold `TIER_A` and `TIER_B` for the entire skill run.

## Required capabilities & harness adapter

This workflow assumes a small set of capabilities. Map each one to your
harness's actual tool. If a capability is missing, use the listed fallback.

| Capability                  | Claude Code                          | OpenCode                              | Codex CLI                              | Fallback if missing                                |
|-----------------------------|--------------------------------------|---------------------------------------|----------------------------------------|----------------------------------------------------|
| Read-only codebase research | `Agent` (subagent_type=Explore)      | `task` agent in read-only mode        | shell `rg`, `git`, `gh`                | Do the search in-thread with `rg`/`grep`           |
| Foreground delegation       | `Agent` (subagent_type=general-purpose) | nested `task` agent, await output  | spawn nested CLI session               | Do the synthesis in-thread                         |
| Ask the user a question     | `AskUserQuestion`                    | inline prompt                         | inline prompt                          | Ask in the response and wait for the next turn     |
| Subagent model tier         | Tier A: `opus` + `ultrathink`; Tier B: `sonnet` + medium thinking | Tier A: top `task` model + high reasoning; Tier B: smaller-tier `task` + medium reasoning | Tier A: top GPT + `reasoning_effort=high`; Tier B: `gpt-5.1-codex-spark` + `reasoning_effort=medium` | Honor the per-phase tier mapping in AGENTS.md; retry the same subagent UP on unavailability |
<!-- autospec-block:harness-adapter-core -->

**Persistent project notes**: read durable preferences from **`AGENTS.md`** in
the repo root. If multiple scoped AGENTS files exist, honor the deepest one for
files under that scope. Per AGENTS.md, the synthesis subagent uses **Tier A
(spec work)** because the output is a spec-adjacent reasoning artifact. The
orchestrator keeps the user's invoked model. Fall back UP the tier on
unavailability.

## Pre-flight

1. Verify the current directory is inside a git worktree:
   `git rev-parse --show-toplevel`.
2. Verify the repo is connected to GitHub:
   `gh repo view --json nameWithOwner,url -q '.nameWithOwner + " " + .url'`.
3. Verify `gh auth status` is authenticated. If unavailable, continue with local
   evidence only and mark GitHub state as missing evidence.
4. Parse flags. Default `LIMIT=500`; if `--since` is present, add
   `--search "created:>=<date> OR updated:>=<date>"` only where the GitHub CLI
   command supports it.

## Evidence collection

Collect evidence into a temporary working directory under `${TMPDIR:-/tmp}`.
Use structured JSON where possible and keep raw command output so the final
report can cite sources.

### Local repository evidence

Run from the repo root:

```bash
git rev-parse --show-toplevel
git branch --show-current
git status --short --branch
git log --date=short --pretty=format:'%h%x09%ad%x09%s' --max-count 200
rg --files docs .github examples skills scripts tests | sort
```

Read only the files that help establish product story or implementation state:

- `README.md`, `AGENTS.md`, `SKILLS.md`, `CONTRIBUTING.md`.
- `docs/specs/*.md`, `docs/superpowers/specs/*.md`, `.omx/plans/*.md`.
- `docs/architecture.md`, `docs/user-manual.md`, `docs/runbooks/*.md`.
- Repo-specific package manifests, service entry points, and top-level
  architecture docs discovered by `rg --files`.

Do not bulk-paste entire large files into the final answer. Extract titles,
section headings, decisions, acceptance criteria, and completion markers.

### GitHub issue evidence

Collect both open and closed issues; closed issues are the primary source for
completed work.

```bash
gh issue list --state open --limit "$LIMIT" --json number,title,state,labels,assignees,createdAt,updatedAt,url,body
gh issue list --state closed --limit "$LIMIT" --json number,title,state,labels,assignees,createdAt,updatedAt,closedAt,url,body
```

If `--since` is present, prefer GitHub search filters; if a command cannot
express the filter, collect first and filter by timestamp during synthesis.

### GitHub PR evidence

Collect open, merged, and closed PRs:

```bash
gh pr list --state open --limit "$LIMIT" --json number,title,state,labels,createdAt,updatedAt,url,body,headRefName,baseRefName
gh pr list --state merged --limit "$LIMIT" --json number,title,state,labels,createdAt,updatedAt,mergedAt,url,body,headRefName,baseRefName
gh pr list --state closed --limit "$LIMIT" --json number,title,state,labels,createdAt,updatedAt,closedAt,url,body,headRefName,baseRefName
```

Map PRs back to issues by parsing `Closes #N`, `Fixes #N`, `Refs #N`, branch
names, labels, and issue URLs in PR bodies. Treat this mapping as inference
unless the PR body has an explicit close/reference line.

## Story synthesis

> **Model tier:** Tier A (spec work) — top model with extended/maximum thinking
> per AGENTS.md. Claude Code: `opus` + `ultrathink`; Codex: current top GPT +
> `reasoning_effort=high`; OpenCode: top task tier. Fall back UP on
> unavailability.

Synthesize a cited report. Preserve the distinction between evidence and
inference:

- Evidence: direct local file paths, issue numbers, PR numbers, labels, commit
  subjects, spec section titles, and explicit close/reference lines.
- Inference: likely component state, intent that is implied by multiple related
  issues, and PR-to-issue mappings without explicit closing syntax.
- Unknown: anything not supported by current local files, GitHub artifacts, or
  git history.

Prioritize a complete overview over chronological exhaustiveness. Cluster work
by product capability or application subsystem, not by issue number alone.

## Report format

When `--output` is provided, write this exact Markdown shape to that path. When
printing in chat, include the executive summary plus the most important state
tables and cite the report path if one was written.

```markdown
# Autospec Story: <owner/repo>

Generated: <ISO8601 timestamp>
Repo root: `<absolute path>`
GitHub: <repo url or "unavailable">
Evidence window: <all available evidence or --since date>

## Executive Summary

<5-8 bullets describing what the application is, current implementation state,
and the highest-signal open gaps. Each bullet cites at least one source.>

## Application Story

<Narrative of the problem, product direction, and major design decisions.>

## Evidence Map

| Evidence type | Count | Primary sources |
| --- | ---: | --- |
| Specs | N | `docs/specs/...` |
| Open issues | N | #1, #2 |
| Closed issues | N | #3, #4 |
| Open PRs | N | #5 |
| Merged PRs | N | #6 |
| Recent commits | N | `abc1234` |

## Implementation State

| Area | State | Evidence | Notes |
| --- | --- | --- | --- |
| <capability> | Implemented / In progress / Planned / Unknown | <paths, #issues, #PRs> | <short note> |

## Completed Work

<Grouped closed issues, merged PRs, and specs that appear implemented.>

## Open Work

<Open issues and unmerged PRs grouped by capability, with blockers called out.>

## Specs and Decisions

<Specs written, key decisions, superseded plans, and active design contracts.>

## Risks, Gaps, and Unknowns

<Explicitly separate missing evidence from known defects.>

## Next Recommended Questions

<Questions the team should ask next, grounded in current state.>

## Source References

<Local paths, issue URLs, PR URLs, and commit ids used as evidence.>
```

## Quality bar

- Every major claim cites at least one source.
- Closed issue heartbeats, stale labels, and old specs are treated as historical
  evidence, not proof of current implementation.
- Do not claim a feature is implemented solely because an issue exists. Prefer
  `Planned` unless there is a merged PR, closed issue with implementation
  detail, or local code/docs evidence.
- Call out dirty worktree state in the report header so readers know whether
  the local repo includes uncommitted changes.
- If GitHub is unavailable, say so clearly and produce a local-only story.
- If the report is too long for chat, write it to `docs/autospec-story.md` only
  when the user passed `--output docs/autospec-story.md`; otherwise print a
  concise story and recommend an output path.
