# Mainline health admission

`autospec autonomous main-health --repo OWNER/REPO [--branch BRANCH]` is the Rust-owned foreground health probe for autonomous Tier-1 admission.

- `--branch BRANCH` is an explicit override and is resolved before repository default-branch metadata.
- Without `--branch`, the CLI resolves the default branch from GitHub metadata and does not silently fall back to `main`.
- Observations are persisted under the repo-scoped autonomous state as `mainline-health.json` with branch, outcome, diagnostic reason, and check evidence.
- `autospec autonomous run-foreground` calls this Rust admission gate before entering the legacy shell conductor so missing branches or failed/pending checks cannot dispatch ready work.
