# FAQ

## Is AutoSpec a hosted service?

No. AutoSpec is a repository of skills, scripts, schemas, and docs that run in supported AI coding harnesses.

## Which harnesses are supported?

The repository carries skill surfaces for Claude Code, Codex CLI, and OpenCode. Each skill keeps its harness-specific prompt mirrors in lock-step.

## Does AutoSpec require GitHub?

The full issue-to-PR workflow assumes GitHub issues, labels, branches, PRs, and checks. Some docs and local validation workflows can be read or run without GitHub access.

## Can I use it for one small bug fix?

Yes, but it may be more ceremony than you want. AutoSpec is most valuable when the work benefits from specs, decomposition, validation, and an audit trail.

## Does it run autonomously forever?

AutoSpec has advanced autonomous workflows, but V61 is not about expanding them. The external launch focuses on making the repo understandable, trustworthy, and demoable.

## What should humans still review?

Anything that affects production data, credentials, security posture, destructive Git operations, infrastructure, legal obligations, or public communication.

## How do I know a run succeeded?

Look for fresh validation output, a closeout report with cited artifacts, passing CI where relevant, and a release verdict when shipping.

