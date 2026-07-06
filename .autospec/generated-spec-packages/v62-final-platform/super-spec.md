# Super Spec: AutoSpec Final Platform

## Objective

Complete the remaining platform work after V61 so AutoSpec is usable, demoable, trustworthy, and launch-ready.

## Current State Summary

- V25 baseline files exist under `.autospec/baselines/` and `.autospec/releases/`.
- V60 and V61 reports exist, and the public launch gate currently passes.
- `bash scripts/validate.sh --fast` and `bash scripts/validate-public-launch-readiness.sh` passed during V61 evidence preparation.
- `.autospec/qa-verdict.json` is historical stale evidence, not current launch proof.
- V62-V73 Rust core, CLI, docs, demo, safety, evidence, and growth surfaces exist in this checkout.
- V61 docs, demo, community files, and marketing materials remain part of the launch surface.

## Execution Order

1. V61 Recovery: unblock final launch validation.
2. V62 Rust Core Workspace Recovery.
3. V63 Spec Metadata And Parser.
4. V64 Dependency Graph And Execution Ordering.
5. V65 Spec State And Validation Framework.
6. V66 Autonomous Execution Queue.
7. V67 Agent Integration Contracts.
8. V68 Evidence And Release Reporting.
9. V69 CLI Productization.
10. V70 Documentation Hardening.
11. V72 Trust And Safety Hardening.
12. V71 Demo And Launch Assets.
13. V73 Metrics And Growth Tooling.
14. V74 Final Release Candidate.

## Dependency Rules

- V61 must pass before public launch claims can be true.
- V62 must land before any Rust CLI or core-engine spec.
- V63-V65 build the spec engine and must land before execution automation.
- V66 depends on V63-V65.
- V67 depends on V66.
- V68 depends on V65-V67.
- V69 depends on V62-V68.
- V72 can run after V69 exposes stable commands.
- V71 can run after V70 hardens docs and should consume V72 safety claims before public demo/growth messaging.
- V73 can run after V71 produces launch assets.
- V74 depends on all previous specs.

## Validation Gates

Every spec must run its own validation command plus:

```bash
bash scripts/validate.sh --fast
```

Before V74 completion, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all
bash scripts/validate.sh
bash scripts/validate-public-launch-readiness.sh
```

## Rollback

Each spec must keep changes scoped. If validation fails after a spec:

1. Write `.autospec/handoff/<spec-id>-blocker.md`.
2. Revert only that spec's changes.
3. Keep earlier completed specs intact.
4. Mark the spec as `blocked` in `spec-index.json`.

## Handoff

Use `handoff.md` as the implementation prompt. Implement one spec at a time, in the order in `execution-order.json`.
