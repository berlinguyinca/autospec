# Autospec Sweep Configuration Design

Date: 2026-05-26

## Goal

Add `/autospec-sweep` as the project-level configuration and continuous-improvement surface for autospec.

## Problem

Autospec has strong individual skills for defining, running, reviewing, testing,
and clone-based validation, but first-time users still have to discover how those
pieces fit together. Project-specific retest or sweep workflows become bespoke
prompts instead of reusable configuration. That makes onboarding slow and makes
it easy for specs, docs, tests, and code reality to drift apart.

## Design

`/autospec-sweep init` creates `.autospec/autospec.yml` as tracked project source.
The file is the single user-facing configuration surface for autospec defaults:
enabled steps, safety posture, team personality, sweep profile, continuous
improvement checks, and project-specific questions discovered from repo files.

The first-run wizard asks only a small default set:

- sweep profile, default `full`
- environment safety, default `strict_isolation`
- team personality, default `auto`
- competitor research, default `false`

The wizard supplements those answers with repo findings such as package
manifests, test scripts, Playwright config, and missing base URLs. It refuses to
write over an existing config without `--force` and refuses if git ignore rules
hide `.autospec/autospec.yml`.

`/autospec` runs a configuration preflight before normal phases. If the config is
missing in an existing repo, it routes through the same first-run wizard. If the
repo is being bootstrapped, it writes the config after the initial scaffold and
before investigation.

`/autospec-sweep run` starts with `autospec-sweep-run.sh run`. The runner loads
and validates the config, executes the bundled deterministic
`autospec-sweep-review.sh` by default, writes `.autospec/sweep/latest.json`,
accepts emitted gap JSON through `--gaps`, and can execute a configured richer
review command through `AUTOSPEC_SWEEP_REVIEW_CMD`. When gaps exist, it hands
them to the existing gap-remediation loop unless `--no-file` is set.

Every run executes the configured full test command. If E2E or integration tests
are configured and the project requires running software first, the runner
executes `project.findings.commands.deploy` before those tests. Deploy, unit,
integration, or E2E failures fail the sweep and are recorded in
`.autospec/sweep/latest.json`.

## Continuous Improvement Defaults

The default sweep enables improvement areas for:

- documentation: docs drift, user manual, API reference, runbooks
- tests: coverage gaps, E2E surface gaps, regression gaps
- code: complexity, duplication, dead code, security-sensitive shortcuts

The sweep does not silently make broad refactors. It updates or creates specs
first when reality and specs disagree, then files bounded issues and routes code
fixes through `/autospec-run`.

## Safety

The config stores secret references, never secret values. Production-like sweeps
default to strict isolation. Scoped production access remains opt-in and must use
existing backup/restore and autospec-test rails.

## Verification

- `bats tests/unit/test_autospec_sweep_config.bats tests/unit/test_autospec_sweep_skill.bats`
- `bats tests/unit/test_autospec_sweep_run.bats`
- `bats tests/smoke/test_install_all_skills.bats`
- `bash tests/install/test_gitignore_offer.sh`
- `autospec validate`
