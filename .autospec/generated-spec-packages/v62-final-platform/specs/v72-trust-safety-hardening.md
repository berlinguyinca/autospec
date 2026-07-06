# Trust And Safety Hardening

## Version

V72

## Objective

Make dangerous behavior visible, gated, and auditable.

## Scope

- Sandboxing guidance.
- Destructive-operation protection.
- Secret redaction.
- Generated code review expectations.
- Human approval checkpoints.
- Audit trail.
- Security disclosure docs.
- Safe public demo mode.

## Non-Goals

- No production policy engine.
- No enterprise access-control system.

## Dependencies

- `v69-cli-productization`

## Files To Create/Modify

- Create: `crates/autospec-core/src/safety/mod.rs`
- Create: `crates/autospec-core/tests/safety.rs`
- Modify: `SAFETY.md`
- Modify: `SECURITY.md`
- Modify: `docs/quickstart.md`
- Modify: `docs/faq.md`
- Modify: `scripts/demo-recording.sh`

## Implementation Steps

1. Define unsafe operation categories: destructive git, credential access, production mutation, filesystem deletion, network publication.
2. Add safe-mode policy that blocks unsafe actions unless explicitly approved.
3. Add secret redaction for common token/key patterns in logs and evidence bundles.
4. Add human checkpoint metadata to run state.
5. Ensure demo mode sets safe mode and disables GitHub mutation.
6. Document review expectations for generated code.

## Acceptance Criteria

- [ ] Unsafe operation fixtures are blocked in safe mode.
- [ ] Secret-looking fixtures are redacted in evidence output.
- [ ] Human checkpoint state is visible in reports.
- [ ] Safety docs match behavior.

## Validation Commands

```bash
cargo test --all safety
bash scripts/validate.sh --fast
```

## Expected Outputs

- Safe-mode report section in run reports.

## Rollback/Handoff Notes

If redaction is too broad, keep conservative redaction and document false-positive handling.
