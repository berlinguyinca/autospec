# Spec: demo-cli subcommand feature

A CLI/backend feature: a new `demo-cli` binary with config parsing,
subcommand dispatch, stdout rendering, exit-code error mapping and usage
text.

## Capabilities

- config parsing from `--config` arguments
- subcommand dispatch table
- stdout rendering in two `--format` styles
- exit-code and stderr error mapping
- usage text for the binary and every registered subcommand

## Hard dependency notes

Wiring and validation consume artifacts that only the config and dispatch
capabilities add; the snapshot and coverage tests verify artifacts of the
output and help capabilities. Everything else is independent.
