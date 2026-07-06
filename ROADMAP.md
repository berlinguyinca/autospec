# Roadmap

This file is the public roadmap summary. Detailed design history lives under `docs/specs/`.

## V74 Final Release Candidate

Status: release-candidate validation.

Goals:

- Preserve the external-facing README, docs launch set, community files, demo, and launch copy.
- Keep V25/V60/V61/public launch gates current.
- Ship the additive Rust core workspace and CLI reference without replacing the validated shell workflows.
- Capture release-candidate evidence under `.autospec/releases/` and `.autospec/reports/`.

## After V74

- Record the demo video and link it from the README.
- Collect external installation feedback.
- Improve examples for common stacks.
- Continue hardening validation and release gates.
- Replace screenshot and social-preview placeholders with real launch assets.
- Mature `autospec init`, `status`, `plan`, `validate`, `run`, `resume`, `report`, `showcase`, `benchmark`, and `growth-report` from explicit stubs into operational Rust-backed commands.

## Later

- Better hosted-docs publishing.
- More target-repo templates.
- Clearer partial-adoption guides for teams that want planning only, QA only, or release audits only.
