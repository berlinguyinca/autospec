# Demo And Launch Assets

## Version

V71

## Objective

Make AutoSpec demoable without private infrastructure.

## Scope

- Deterministic demo project.
- Demo recording script.
- Architecture diagram.
- Screenshots/GIF placeholders or final assets.
- Website/docs scaffold.
- Launch posts.
- Video scripts.
- FAQ for launch comments.
- Good first issues.
- Public roadmap.

## Non-Goals

- No live production integration.
- No hidden telemetry.

## Dependencies

- `v70-documentation-hardening`

## Files To Create/Modify

- Modify: `examples/hello-autospec/`
- Modify: `scripts/demo-recording.sh`
- Modify: `docs/assets/architecture.mmd`
- Modify: `docs/assets/screenshots-placeholder.md`
- Modify: `docs/assets/social-preview-placeholder.md`
- Modify: `docs/site/`
- Modify: `marketing/`
- Modify: `docs/good-first-issues.md`

## Implementation Steps

1. Make `scripts/demo-recording.sh` exercise only deterministic local files.
2. Add `autospec showcase --demo examples/hello-autospec --json` output fixture.
3. Update architecture diagram to match final CLI/core architecture.
4. Replace placeholders with assets or mark them explicitly deferred in release checklist.
5. Ensure launch posts match README claims.
6. Add a public roadmap section for post-launch feedback.

## Acceptance Criteria

- [ ] Demo script exits 0 with no network or GitHub mutation.
- [ ] Showcase command emits JSON and markdown.
- [ ] Launch posts do not claim unsupported features.
- [ ] Website/docs scaffold links to quickstart and demo.

## Validation Commands

```bash
bash scripts/demo-recording.sh
autospec showcase --demo examples/hello-autospec --json
bash scripts/validate-launch-readiness.sh
```

## Expected Outputs

- Demo transcript or JSON fixture under `examples/hello-autospec/`.

## Rollback/Handoff Notes

If media assets are not ready, keep placeholders but explicitly list them as non-blocking in the release checklist.
