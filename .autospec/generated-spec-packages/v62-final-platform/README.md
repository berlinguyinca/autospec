# AutoSpec V62 Final Platform Spec Package

This package is the canonical V62+ plan candidate for the remaining ordered work needed to turn AutoSpec from the current launch-readiness repository into a public, demoable, trustworthy autonomous software engineering platform.

It is generated from the current repository state, not from the assumed history alone. Important findings:

- V25 baseline artifacts exist.
- V61 launch readiness artifacts exist.
- Final public launch gates pass in the current checkout via `bash scripts/validate-public-launch-readiness.sh`.
- `.autospec/qa-verdict.json` is retained only as historical stale QA evidence and is superseded by current validation gates.
- This checkout does not contain `Cargo.toml` or Rust source files, so Rust platform work must start with an explicit workspace recovery/scaffold spec.
- Existing implementation is mostly shell, Python, JavaScript, schemas, docs, and multi-harness skill prompts.
- `.gitignore` allows this package path; commit the whole `.autospec/generated-spec-packages/v62-final-platform/` tree when adopting this plan.

## How To Execute

1. Read `missing-work-report.md`.
2. Execute `super-spec.md` in order.
3. For each spec in `specs/`, create one implementation PR or one tightly-scoped change group.
4. Run the spec validation command before moving to the next spec.
5. Update `spec-index.json` status after each completed spec.

## Prerequisites

- Normal Git checkout with permission to create git worktrees.
- Bash, Git, `jq`, `bats`, Python 3.
- Rust toolchain before executing the Rust-core specs.
- GitHub CLI for workflow specs that touch issues or PRs.

## Expected Final State

At the end of this package, AutoSpec can accept a large software goal, turn it into ordered specs, validate dependency order, hand work to supported AI agents, resume after interruption, collect evidence, generate release reports, and support a safe public launch/demo path.

Final gate:

```text
AUTOSPEC_PUBLIC_LAUNCH_READY=true
```
