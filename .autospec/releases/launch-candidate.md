# AutoSpec Launch Candidate

Date: 2026-07-06

## Repository Status

- V25 baseline: ready.
- V60 release candidate: ready with stale historical QA verdict noted.
- V61 launch readiness: ready.
- V62-V73 final-platform slice: ready for V74 release-candidate validation.
- Public launch readiness: ready — final proof is captured by `.autospec/releases/final-release-candidate.md` and `.autospec/reports/final-platform-evidence.md`.

## Launch Evidence

- `bash scripts/validate-v25-baseline.sh`
- `bash scripts/validate-v60-release.sh`
- `bash scripts/validate-launch-readiness.sh`
- `bash scripts/validate-public-launch-readiness.sh`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features`
- `cargo test --all`
- `bash scripts/validate.sh`
- `bats tests/launch/test_launch_readiness.bats`
- `bash scripts/demo-recording.sh`

## Launch Assets

- README external developer pitch and quickstart.
- Docs launch set under `docs/`.
- Community files and GitHub templates.
- `examples/hello-autospec/` deterministic demo.
- `marketing/` launch kit.
- Release and public launch checklists.
- V74 final release candidate and platform evidence reports.

## Gate

AUTOSPEC_PUBLIC_LAUNCH_READY=true
