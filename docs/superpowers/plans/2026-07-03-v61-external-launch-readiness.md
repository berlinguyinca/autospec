# V61 External Launch Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make AutoSpec understandable, trustworthy, demoable, and ready for an external GitHub launch without adding major autonomy features.

**Architecture:** V61 is a documentation and release-readiness layer over the existing shell-validated repository. It adds public-facing docs, community files, demo assets, launch copy, a deterministic launch-readiness validator, and a final V60 report while preserving existing behavior.

**Tech Stack:** Markdown, Bash, Bats, Mermaid, existing `autospec validate` validation harness.

## Global Constraints

- Do not add major autonomy features in this release.
- Keep diffs focused on docs, examples, launch materials, and launch-readiness validation.
- Follow existing shell validation style and avoid new dependencies.
- Run existing repository validation plus the new launch-readiness gate before reporting completion.

---

### Task 1: Baseline Validation And Report

**Files:**
- Create: `docs/reports/v60-final-report.md`

**Interfaces:**
- Consumes: Existing validation scripts and test output.
- Produces: A human-readable V60 state report referenced by launch docs.

- [ ] Run `autospec validate`.
- [ ] Run focused spec/release gates available in the repo, including QA artifact and release verdict scripts where prerequisites exist.
- [ ] Record pass/fail status and constraints in `docs/reports/v60-final-report.md`.

### Task 2: Public README And Docs Set

**Files:**
- Modify: `README.md`
- Create: `docs/index.md`
- Create: `docs/quickstart.md`
- Create: `docs/concepts.md`
- Create: `docs/architecture.md`
- Create: `docs/workflows.md`
- Create: `docs/faq.md`
- Create: `docs/roadmap.md`

**Interfaces:**
- Consumes: Existing skill READMEs, `SKILLS.md`, `AGENTS.md`, and current docs.
- Produces: External developer entry points.

- [ ] Rewrite `README.md` around pitch, quickstart, demo, architecture, comparison, maturity, and contribution.
- [ ] Add docs pages with consistent terminology and links back to skill-level references.

### Task 3: Community And Safety Files

**Files:**
- Modify: `CONTRIBUTING.md`
- Create: `SECURITY.md`
- Create: `SAFETY.md`
- Update: `CHANGELOG.md`
- Create: `ROADMAP.md`
- Create: `.github/ISSUE_TEMPLATE/bug_report.md`
- Create: `.github/ISSUE_TEMPLATE/feature_request.md`
- Create: `.github/ISSUE_TEMPLATE/docs_feedback.md`
- Create: `.github/pull_request_template.md`

**Interfaces:**
- Consumes: Existing engineering standards and safety posture.
- Produces: Trust signals for external contributors.

- [ ] Replace the narrow skill-only contribution guide with repo-level contribution instructions.
- [ ] Add security, safety, changelog, roadmap, issue templates, and PR template.

### Task 4: Demo And Launch Kit

**Files:**
- Create: `examples/hello-autospec/README.md`
- Create: `examples/hello-autospec/spec.md`
- Create: `examples/hello-autospec/sample-issue.md`
- Create: `examples/hello-autospec/expected-closeout.md`
- Create: `scripts/demo-recording.sh`
- Create: `docs/assets/architecture.mmd`
- Create: `docs/assets/demo-placeholder.md`
- Create: `marketing/launch-post-github.md`
- Create: `marketing/launch-post-reddit.md`
- Create: `marketing/launch-post-hackernews.md`
- Create: `marketing/launch-post-linkedin.md`
- Create: `marketing/launch-post-x.md`
- Create: `marketing/demo-video-script.md`
- Create: `marketing/faq-for-comments.md`

**Interfaces:**
- Consumes: Public docs and the launch positioning.
- Produces: Demoable artifacts and launch copy.

- [ ] Add a no-side-effect hello example that demonstrates spec to issue to closeout.
- [ ] Add a demo script that prints a reproducible recording outline without mutating GitHub.
- [ ] Add launch posts and comment FAQ.

### Task 5: Launch Readiness Validator

**Files:**
- Create: `scripts/validate-launch-readiness.sh`
- Create: `tests/launch/test_launch_readiness.bats`

**Interfaces:**
- Consumes: Required V61 artifact paths.
- Produces: `AUTOSPEC_V61_LAUNCH_READY=true` on success.

- [ ] Write Bats coverage for the success marker and at least one missing-file failure.
- [ ] Confirm the test fails before the script exists.
- [ ] Implement the script with deterministic file/content checks.
- [ ] Run the focused Bats test, then run the full validation suite and the launch validator.
