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
