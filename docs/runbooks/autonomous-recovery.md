# Autonomous recovery

The autonomous drain defaults `AUTOSPEC_CLAIM_TTL_SECONDS` to `600` seconds so
an abandoned edit lease does not pause the perpetual worker for the longer
interactive default. Set the variable explicitly when a deployment needs a
different recovery window; active workers refresh their claims as they run.

The conductor also defaults `AUTOSPEC_RESCAN_INTERVAL` to `300` seconds after
an empty backlog, while an explicit environment value remains authoritative.

Tier-2 explore drains default `AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS` to `600`
seconds without observable output; set the variable explicitly for slower
repositories or remote research providers.

`autospec autonomous stop --immediate` and `autospec autonomous restart --force`
signal each verified conductor, monitor, and supervisor process group. Restart
releases the terminated conductor's matching lease before acquiring replacement
ownership, so legacy wrapper children cannot revive a stale conductor during
the handoff. New unit metadata binds the PID to its process-group ID and start
time; mismatched live identities fail closed instead of signaling a reused PID.

Explore drains export an inherited recursion marker; a nested drain request
returns a dry suppression result instead of launching another harness tree.
They also enforce a 900-second absolute runtime cap via
`AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS`, independent of incidental harness output.

The drain exports `AUTOSPEC_EXPLORE_PARENT_PID`; detached explore scripts watch
that owner and terminate when it disappears.

The verifier command is also embedded as a quoted environment assignment in the
harness command because `omx exec` may not preserve the caller's exported
environment. This prevents an otherwise healthy discovery pass from being
mistaken for a no-verifier fail-closed cycle.

If the skeptic itself times out, the verifier uses a deterministic fallback: it
survives only candidates whose evidence names an existing repository path and
line, and refutes all other candidates. Set
`AUTOSPEC_AUTONOMOUS_DETERMINISTIC_VERIFY=0` to require the skeptic exclusively.

Discovery safety review resolves the Rust CLI in this order: `AUTOSPEC_BIN`,
the repository's `target/debug/autospec`, `~/.autospec/bin/autospec`, then PATH.
This keeps filing functional in detached sessions whose PATH does not include
the installed command shim.

Each researcher is bounded by `AUTOSPEC_RESEARCHER_TIMEOUT_SECS` (default 120
seconds). A timed-out source is recorded as `researcher_failed` while other
sources continue, so one provider cannot stall the autonomous discovery loop.
Timed-out groups are terminated as a process group to clean up detached
researcher children as well.

The outer explore harness is isolated with `setsid` and receives the same
process-group cleanup, preventing a detached `omx exec` from holding a cycle.
The default no-output stall bound is 120 seconds and the absolute runtime bound
is 300 seconds; both remain configurable through `AUTOSPEC_AUTONOMOUS_EXPLORE_*`.

If a harness reports `AUTOSPEC_EXPLORE_VERIFY_CMD_not_executed`, the drain runs
the local explore entrypoint directly with the verifier command and uses its
JSON contract, avoiding a model-reported dry result. That fallback runs in its
own process group and shares `AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS`, so a slow
researcher cannot hold the conductor indefinitely.

When `AUTOSPEC_EXPLORE_VERIFY_CMD` is configured, one-shot discovery preserves
raw proposals through deduplication, verification, and finalization before
candidate filing.

The one-shot dedup handoff materializes proposals only as input to the external
verifier; no candidate is filed without an explicit survivor verdict.

If a skeptic returns successfully but emits no recoverable verdict JSON, the
bridge applies the same deterministic evidence fallback used for timed-out
skeptics instead of discarding the entire discovery cycle.
