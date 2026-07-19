# Handoff Prompt

You are implementing `.autospec/generated-spec-packages/v62-final-platform/` in `Berlinguyinca/autospec`.

Rules:

1. Execute specs in `execution-order.json`.
2. Implement exactly one spec per PR or clearly separated change group.
3. Preserve existing validated shell/Python/JS behavior.
4. If a spec fails validation, write `.autospec/handoff/<spec-id>-blocker.md` and stop that spec.
5. Do not mark public launch ready until `autospec validate` and `bash scripts/validate-public-launch-readiness.sh` both pass.

Start with:

```bash
cat .autospec/generated-spec-packages/v62-final-platform/specs/v61-recovery-public-launch-validation.md
```

Then implement its steps, run its validation commands, update `spec-index.json`, and continue to the next spec.
