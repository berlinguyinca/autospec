# autospec-playwright

Thin dispatcher for the `autospec-test` Stage 2A disciplined Playwright
authoring pipeline. Reads `.autospec/test.yml` authoring and reset blocks,
invokes Stage 2A, and prints the e2e coverage report.

## Install

```sh
# All harnesses (Claude Code, OpenCode, Codex CLI):
./install.sh --harness all

# Or pipe from curl:
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/skills/autospec-playwright/install.sh \
  | sh -s -- --harness all
```

## Usage

```
/autospec-playwright [--dry-run]
```

Set `e2e.authoring.enabled: true` in `.autospec/test.yml` to enable Stage 2A.

## Uninstall

```sh
./uninstall.sh --harness all
```

## Validate

```sh
bash validate.sh
```
