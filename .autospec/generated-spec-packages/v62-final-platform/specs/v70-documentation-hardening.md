# Documentation Hardening

## Version

V70

## Objective

Bring public documentation in line with the Rust CLI and final platform behavior.

## Scope

- README.
- Quickstart.
- Concepts.
- Architecture.
- Workflows.
- CLI reference.
- Examples.
- FAQ.
- Roadmap.
- Contributing.
- Safety.
- Security.
- Known limitations.
- Release checklist.

## Non-Goals

- No new core behavior.

## Dependencies

- `v69-cli-productization`

## Files To Create/Modify

- Modify: `README.md`
- Modify: `docs/quickstart.md`
- Modify: `docs/concepts.md`
- Modify: `docs/architecture.md`
- Modify: `docs/workflows.md`
- Modify: `docs/faq.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/release-checklist.md`
- Create/modify: `docs/cli-reference.md`

## Implementation Steps

1. Update README quickstart to use actual CLI commands.
2. Update architecture to show Rust core plus existing skill/script layer.
3. Add command examples to workflows.
4. Add known limitations section naming shell legacy and optional tool requirements.
5. Update safety/security docs with CLI safe-mode behavior.
6. Validate internal links.

## Acceptance Criteria

- [ ] README command examples work.
- [ ] CLI reference matches `autospec --help`.
- [ ] Docs link to safety, security, and known limitations.
- [ ] Launch-readiness validation passes.

## Validation Commands

```bash
bash scripts/validate-launch-readiness.sh
bash scripts/validate.sh --fast
```

## Expected Outputs

- Public docs describe the actual product rather than aspirational behavior.

## Rollback/Handoff Notes

If CLI behavior is incomplete, docs must say so clearly in known limitations.
