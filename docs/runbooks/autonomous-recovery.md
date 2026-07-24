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

`autospec-autonomous restart --force` now signals the detached conductor's
entire process group, cleaning up active drains and harness children before the
replacement lifecycle starts.

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
