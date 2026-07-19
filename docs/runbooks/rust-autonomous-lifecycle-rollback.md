# Rust autonomous lifecycle rollback

## Scope

This runbook rolls back the typed Rust lifecycle cutover. It does not restore the legacy shell waterfall or its command-string launch overrides.

## Stop and inspect

1. Stop the scoped run: `autospec autonomous stop --repo OWNER/REPO --immediate --json`.
2. Inspect the final pure decision: `autospec autonomous lifecycle decide --repo OWNER/REPO --stop immediate`.
3. Read `.autospec/autonomous-operator/<scope>/lifecycle.json`, `launch.json`, and the three PID files before changing a release.
4. Confirm no conductor, monitor, or supervisor PID remains live before a replacement starts.

## Restore a compatible Rust release

1. Deploy the last Rust release that understands schema-1 operator metadata.
2. Preserve the operator directory and logs; do not delete `lifecycle.json` during rollback.
3. Start the replacement with `autospec autonomous restart --repo OWNER/REPO --repo-dir DIR --json`.
4. Verify the replacement writes a new `lifecycle.json` with the expected repository and a `run` decision.

## Escalate instead of bypassing

If the lifecycle decision is `reject`, `park`, or `stop` unexpectedly, retain the state files and open a bounded follow-up. Do not set a command-string override, invoke `sh -c`, or revive `autospec-autonomous.sh` as a fallback; those paths are outside this cutover's authority.
