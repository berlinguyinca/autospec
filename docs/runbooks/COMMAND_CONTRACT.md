# Autospec Command Contract

## Standard Behavior

- `--help` must print usage and exit 0.
- `--dry-run` must not perform GitHub writes.
- `--confirm` is required for GitHub writes.
- Commands writing local generated files must report paths.
- Commands should write Markdown plus JSON reports.
- Commands should return nonzero for blocking failures.
- Commands should return zero for pass and pass_with_warnings.
- Commands should never print raw JSON as the only human output.

## Safety

No GitHub Actions, cron, scheduler, auto-merge, or self-approval is added by the command contract.
