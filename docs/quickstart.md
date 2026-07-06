# Quickstart

This path gets you from a fresh checkout to a local validation run and a no-side-effect demo.

## Prerequisites

- Git
- Bash
- GitHub CLI (`gh`) for workflows that create issues or PRs
- `jq`
- Python 3
- One supported AI coding harness: Claude Code, Codex CLI, or OpenCode

Optional but useful: `bats`, `ajv`, `yq`, and browser automation tools for deeper validation.

## Install From A Checkout

```bash
git clone https://github.com/berlinguyinca/autospec.git
cd autospec
bash scripts/validate.sh --fast
bash install.sh --skill all --harness all
```

## One-Command Install

```bash
curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash
```

Use the checkout path above when developing AutoSpec itself. Use the one-command installer when trying AutoSpec as a user.

## Try The Demo

```bash
bash scripts/demo-recording.sh
```

The demo script prints a recording outline and points at `examples/hello-autospec/`. It does not create GitHub issues, push branches, or mutate another repository.

## Try The Rust CLI

The Rust CLI is additive while the existing skills and shell scripts remain the main workflow surface.

```bash
cargo run --bin autospec -- doctor --json
cargo run --bin autospec -- --help
```

Commands such as `run`, `resume`, `init`, and `benchmark` are present as explicit non-zero stubs until later V62+ work wires them to the execution queue.

## Use AutoSpec In A Target Repo

In your AI coding harness, start with planning:

```text
/autospec-define Add an export button to the reports page with tests and documentation
```

Review the generated spec and issues. When ready:

```text
/autospec-run
```

For release readiness:

```text
/autospec-release
```

## Validate This Repository

```bash
bash scripts/validate.sh
bash scripts/validate-launch-readiness.sh
bash scripts/validate-public-launch-readiness.sh
```

The launch validator succeeds only when the public docs, community files, demo materials, and launch kit are present.
