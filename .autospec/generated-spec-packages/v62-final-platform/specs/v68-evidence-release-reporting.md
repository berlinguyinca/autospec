# Evidence And Release Reporting

## Version

V68

## Objective

Create a durable evidence bundle and release reporting layer.

## Scope

- Evidence bundle format.
- Test result capture.
- Logs/report capture.
- Docs validation result capture.
- Release validation.
- Spec coverage validation.
- Dependency validation.
- Final public launch validation integration.

## Non-Goals

- No analytics dashboard.
- No remote artifact storage.

## Dependencies

- `v65-spec-state-validation`
- `v67-agent-integration-contracts`

## Files To Create/Modify

- Create: `crates/autospec-core/src/evidence/mod.rs`
- Create: `crates/autospec-core/src/report/mod.rs`
- Create: `crates/autospec-core/tests/evidence_bundle.rs`
- Create: `schemas/autospec-evidence-bundle.schema.json`
- Create: `schemas/autospec-release-report.schema.json`
- Modify: `scripts/validate-public-launch-readiness.sh`

## Implementation Steps

1. Define evidence bundle JSON with commands, exit codes, stdout/stderr paths, artifacts, and timestamps.
2. Capture validation command output to files under `.autospec/evidence/<run-id>/`.
3. Render markdown release report from evidence bundle.
4. Implement spec coverage report: every spec is passed, blocked, deferred, or superseded.
5. Add dependency validation report from graph engine.
6. Wire final public launch validation to require current evidence bundle.

## Acceptance Criteria

- [ ] Evidence bundle validates against schema.
- [ ] Release report renders markdown and JSON.
- [ ] Spec coverage fails if any spec has unknown state.
- [ ] Dependency validation fails on missing or cyclic dependencies.

## Validation Commands

```bash
cargo test --all evidence release_report
bash scripts/validate-public-launch-readiness.sh
```

## Expected Outputs

- `.autospec/evidence/<run-id>/bundle.json`
- `.autospec/releases/<version>.md`

## Rollback/Handoff Notes

If output capture creates noisy diffs, keep evidence under ignored `.autospec/evidence/` and commit only schemas/docs.
