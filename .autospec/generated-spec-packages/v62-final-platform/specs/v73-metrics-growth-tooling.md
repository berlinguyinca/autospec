# Metrics And Growth Tooling

## Version

V73

## Objective

Add non-invasive launch and growth reporting without hidden telemetry.

## Scope

- GitHub star/readiness checklist.
- Shareable showcase report.
- `growth-report` command spec.
- Non-invasive metrics only.
- No hidden telemetry.
- Awesome-list submission tracker.
- Outreach tracker.

## Non-Goals

- No automatic external posting.
- No user tracking.
- No analytics beacon.

## Dependencies

- `v71-demo-launch-assets`

## Files To Create/Modify

- Create: `crates/autospec-core/src/growth/mod.rs`
- Create: `crates/autospec-core/tests/growth_report.rs`
- Modify: `crates/autospec-cli/src/commands/growth_report.rs`
- Create: `.autospec/growth/awesome-list-tracker.md`
- Create: `.autospec/growth/outreach-tracker.md`
- Modify: `docs/roadmap.md`

## Implementation Steps

1. Define local-only growth report inputs: README readiness, demo readiness, docs completeness, issue templates, launch posts.
2. Implement `autospec growth-report --json` and markdown output.
3. Add star/readiness checklist with no API calls required.
4. Add awesome-list and outreach trackers as editable markdown files.
5. Document no hidden telemetry policy.

## Acceptance Criteria

- [ ] `autospec growth-report --json` emits local-only metrics.
- [ ] No network calls occur during growth report.
- [ ] Trackers are markdown and user-editable.
- [ ] Docs state that telemetry is not hidden or automatic.

## Validation Commands

```bash
cargo test --all growth_report
autospec growth-report --json
```

## Expected Outputs

- `.autospec/reports/growth-report.md`
- `.autospec/reports/growth-report.json`

## Rollback/Handoff Notes

If any metric requires network access, remove it or make it an explicit opt-in flag.
