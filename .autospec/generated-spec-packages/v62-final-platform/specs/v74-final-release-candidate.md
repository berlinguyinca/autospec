# Final Release Candidate

## Version

V74

## Objective

Produce the final release candidate and public launch proof.

## Scope

- Full validation.
- Evidence bundle.
- Release candidate report.
- Public launch readiness gate.
- Human handoff.
- Changelog and roadmap update.

## Non-Goals

- No new features.
- No speculative roadmap expansion.

## Dependencies

- `v61-recovery-public-launch-validation`
- `v62-rust-core-workspace`
- `v63-spec-metadata-parser`
- `v64-dependency-graph-ordering`
- `v65-spec-state-validation`
- `v66-autonomous-execution-queue`
- `v67-agent-integration-contracts`
- `v68-evidence-release-reporting`
- `v69-cli-productization`
- `v70-documentation-hardening`
- `v71-demo-launch-assets`
- `v72-trust-safety-hardening`
- `v73-metrics-growth-tooling`

## Files To Create/Modify

- Create: `.autospec/releases/final-release-candidate.md`
- Create: `.autospec/reports/final-platform-evidence.md`
- Modify: `.autospec/releases/launch-candidate.md`
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`
- Modify: `.autospec/handoff/codex-final-handoff.md`

## Implementation Steps

1. Run all validation commands.
2. Generate evidence bundle.
3. Generate final release candidate markdown and JSON.
4. Update changelog with V62-V74 summary.
5. Update roadmap with post-launch focus.
6. Mark public launch ready only when all gates pass.

## Acceptance Criteria

- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features` passes.
- [ ] `cargo test --all` passes.
- [ ] `bash scripts/validate.sh` passes.
- [ ] `bash scripts/validate-public-launch-readiness.sh` prints `AUTOSPEC_PUBLIC_LAUNCH_READY=true`.
- [ ] Final handoff includes exact launch command sequence.

## Validation Commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all
bash scripts/validate.sh
bash scripts/validate-public-launch-readiness.sh
```

## Expected Outputs

- `.autospec/releases/final-release-candidate.md`
- `AUTOSPEC_PUBLIC_LAUNCH_READY=true`

## Rollback/Handoff Notes

If any command fails, keep public launch false and write `.autospec/handoff/v74-final-release-candidate-blocker.md` with the failing command and log path.
