# Roadmap

V74 is the completed release-candidate milestone for the launch slice: V61
external readiness plus the additive V62-V73 Rust core, CLI, evidence, safety,
demo, and growth-reporting foundation. The current post-launch horizon is the
Phase-4 never-idle autonomous platform workstream.

## Near Term

- Keep the Rust CLI additive and honest: implemented commands return useful output, while unwired mutating commands fail explicitly.
- Improve first-run documentation from real external user feedback.
- Record and publish the `hello-autospec` demo.
- Tighten installation diagnostics for missing optional tools.
- Add more small examples for common repo types.
- Replace screenshot and social-preview placeholders with real launch assets.
- Keep public launch gates green as release evidence evolves.
- Preserve V74 release-candidate evidence under `.autospec/releases/` and `.autospec/reports/`.
- Keep the Phase-4 autonomous platform source of truth current in
  `docs/specs/2026-07-06-autospec-autonomous-platform-design.md`.
- Sweep open `needs-classify` and `needs-autospec-template` issues so autonomous
  work can enter the normal queue cleanly.

## Medium Term

- Wire the V62+ Rust core to real queue execution, evidence reports, and CLI commands without deleting validated shell behavior.
- Improve docs generated from real runs.
- Expand release-readiness examples.
- Add clearer guidance for teams adopting only part of the workflow.
- Continue reducing prompt and validation drift across harnesses.
- Build production evidence for RAG freshness, citation verification, and
  eval-gated tuning beyond the deterministic local scaffold.
- Keep companion repository write bridges proposal-only until separate proof and
  approval gates exist.

## Explicitly Not V74

- No major new autonomy features inside the completed launch slice; those are
  handled by the post-launch Phase-4 autonomous platform workstream.
- No hosted SaaS control plane.
- No broad dependency migration.
- No new model-provider abstraction layer.

## Growth Reporting

`autospec growth-report --json` is local-only. It summarizes README, demo, docs, issue-template, and launch-post readiness without network calls, tracking pixels, analytics beacons, or automatic external posting. Editable outreach trackers live under `.autospec/growth/`.
