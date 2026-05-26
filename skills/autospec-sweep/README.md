# autospec-sweep

First-run configuration and continuous-improvement sweep mode for autospec.

`autospec-sweep` creates the tracked project config at `.autospec/autospec.yml`
and uses it to keep specs, docs, tests, and code health improving over time.

## Usage

```bash
/autospec-sweep init
/autospec-sweep configure
/autospec-sweep run
```

The default config enables all autospec steps and strict isolation. It records
repo findings and follow-up questions so project-specific onboarding stays small.

The installed runner is available at:

```bash
bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-sweep-run.sh" run --dry-run
```

It validates `.autospec/autospec.yml`, runs the bundled
`autospec-sweep-review.sh`, writes `.autospec/sweep/latest.json`, and can hand
emitted gap JSON to the existing gap-remediation loop.

Every sweep run executes the configured full test command. When E2E or
integration tests are configured and the project requires running software first,
the runner executes `project.findings.commands.deploy` before those tests. Any
deploy or test failure fails the sweep.

## Install

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-sweep/install.sh) --harness all
```
