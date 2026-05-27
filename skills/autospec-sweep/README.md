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

The config can declare `documentation.audiences` and `documentation.scopes`.
Each target names the audience or product scope, the Markdown file that should
serve it, the focus for generated work, and whether an `autospec-doc-scope`
marker is required. The sweep reviewer emits one bounded docs gap per missing
target or missing scope marker so `/autospec-run` can build deep documentation
for different readers without treating all docs as one README.

Every sweep run executes the configured full test command. When E2E or
integration tests are configured and the project requires running software first,
the runner executes `project.findings.commands.deploy` before those tests. Any
deploy or test failure fails the sweep.

## Install

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-sweep/install.sh) --harness all
```
