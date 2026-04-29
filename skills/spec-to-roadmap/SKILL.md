---
name: spec-to-roadmap
description: Use when a user wants Codex to turn a product/spec issue or document into ordered GitHub issues, a GitHub Project, and an end-to-end branch/PR/merge execution loop.
---

# Spec To Roadmap

## Overview

Convert a large spec into small, ordered, LLM-executable GitHub issues, create or update a GitHub Project, then execute each issue through branch, implementation, self-review, PR, merge, project update, and issue closure.

This is a high-side-effect workflow. Prefer GitHub plugin/app capabilities when available; use `gh` as the reliable fallback.

## Inputs

Determine these before acting:

- Repository: from `git remote -v`, current GitHub context, or user input.
- Spec source: GitHub issue, local file, PR body, markdown doc, or pasted text.
- Project owner/title: default to repo owner and a title derived from the spec.
- Execution mode: default full end-to-end when the user asks to execute; otherwise generate roadmap only.
- Merge policy: default to normal PR merge after self-review and verification, respecting branch protection.

If the repository, spec source, or merge authority is unclear, ask one concise question. Do not ask for confirmation for ordinary issue/project/branch/PR steps after the user has requested the full workflow.

## Workflow

### 1. Fetch And Preserve The Spec

1. Fetch the spec body and comments.
2. Save or reference the canonical spec in an issue/doc if the spec currently exists only in chat.
3. Identify hard constraints, non-goals, destructive risks, external dependencies, and acceptance criteria.
4. Inspect existing issues/projects to avoid duplicates.

### 2. Decompose Into LLM-Sized Issues

Create a structured plan before creating issues. Each issue must fit one implementation context and include:

- Title with product/phase prefix.
- Goal.
- Source spec references.
- `Depends on` links.
- `Unblocks` links.
- Implementation scope.
- Out of scope.
- Acceptance criteria.
- Labels.
- Risk or safety guardrails when relevant.

Prefer fewer, sharper issues over broad phase buckets. Add a tracker issue unless an authoritative tracker already exists.

Use `references/issue-plan-template.json` as the output shape for the issue plan. Use `scripts/create_github_roadmap.py` to create the tracker, child issues, project, project items, and source-spec comment from that plan.

### 3. Create The GitHub Roadmap

1. Ensure required labels exist.
2. Create/update the tracker issue.
3. Create child issues in dependency order.
4. Create/update the GitHub Project.
5. Add the source spec, tracker, and all child issues to the project.
6. Comment on the source spec with project/tracker/child issue links.
7. Verify counts and links before reporting success.

### 4. Execute Each Issue End-To-End

For each issue in dependency order:

1. Sync default branch: fetch and rebase/merge safely according to repo norms.
2. Create a branch named from the issue, e.g. `nexus/205-data-manifest`.
3. Read the issue body and any linked spec sections.
4. Implement only that issue's scope.
5. Run targeted tests, then broader lint/typecheck/test commands appropriate to the blast radius.
6. Self-review before PR:
   - Re-read issue acceptance criteria.
   - Inspect `git diff`.
   - Check for unrelated changes.
   - Fix defects and rerun relevant verification.
7. Commit with the repository's commit-message protocol.
8. Push branch and open a PR linked to the issue.
9. Review the PR as if doing code review: behavior, tests, safety, docs, migration risk.
10. Fix review findings on the same branch.
11. Wait for required checks when available; if checks fail, diagnose and fix.
12. Merge according to branch protection and repo policy.
13. Update the project item status.
14. Close the issue with verification evidence, unless auto-closed by PR keywords.
15. Return to default branch and continue with the next unblocked issue.

Never move, delete, or rewrite protected data paths unless the issue explicitly authorizes it and the user has approved the destructive action.

## Branch And PR Rules

- One issue, one branch, one PR by default.
- Do not mix unrelated issue work.
- Use stacked PRs only when branch protection or dependency order makes it unavoidable, and document the stack.
- Do not merge with known failing required checks.
- Do not close an issue unless the acceptance criteria are met or the issue is explicitly superseded.

## Project Updates

If GitHub Project fields exist, update them opportunistically:

- Status: Todo, In Progress, In Review, Done.
- Phase/order/workstream fields when names are obvious.

If field updates are unavailable or brittle, adding items and using issue comments is sufficient. Do not block the workflow on optional Project field automation.

## Safety Gates

Stop and ask only for:

- Destructive file/data operations.
- Production deployment or credential rotation.
- Repository rename.
- Force-push to shared branches.
- Merge when branch protection fails or checks are unavailable but risk is high.
- Material scope changes not represented by existing issues.

Otherwise proceed autonomously and keep a concise evidence trail in issue/PR comments.

## Verification Before Final Report

Before reporting completion of a roadmap generation run:

- Confirm tracker exists and lists all child issues.
- Confirm project exists and contains source spec, tracker, and children.
- Confirm source spec has a comment linking project/tracker/children.

Before reporting completion of an execution run:

- Confirm every targeted issue is closed or explicitly blocked.
- Confirm merged PR links exist.
- Confirm default branch contains the merged work.
- Confirm final tests/checks run or document why they could not.

