# Autonomous recovery

The autonomous drain defaults `AUTOSPEC_CLAIM_TTL_SECONDS` to `600` seconds so
an abandoned edit lease does not pause the perpetual worker for the longer
interactive default. Set the variable explicitly when a deployment needs a
different recovery window; active workers refresh their claims as they run.

The conductor also defaults `AUTOSPEC_RESCAN_INTERVAL` to `300` seconds after
an empty backlog, while an explicit environment value remains authoritative.

## Integration base synchronization

A fresh foreground selection synchronizes the configured integration base before
scan, review, selection, and admission. The conductor fast-forwards the local
base to the fetched remote head with a compare-and-swap or a `--ff-only` merge;
it never force-pushes and never performs a destructive reset. A recovered lease
already owns a dispatched base and skips this fresh-selection synchronization.
When the local base has diverged from origin, synchronization fails closed
before any issue is selected or admitted, leaving the base and the queue
untouched. Operators recover a divergence by resetting the local base to the
remote head in a clean checkout and relaunching the conductor; they must not
force-push or hand-edit the base ref.

## Startup heartbeat recovery

Authoritative claim recovery treats startup heartbeat evidence as an audit
record, not a disposable lock file. Fresh, live, malformed, mismatched, remote,
or otherwise ambiguous evidence remains blocking. An exact expired heartbeat
whose local process generation is absent is moved into the repository-scoped
`quarantine/startup-heartbeat-handoffs/` directory before the claim becomes
available.

Heartbeat roots, repository directories, quarantine directories, and handoff
directories are owned by the effective user with mode `0700`. Live and retained
heartbeat files, plus handoff receipts, use mode `0600`. Operators may inspect
the retained JSON and completed receipt under the configured
`AUTOSPEC_HEARTBEAT_DIR`; they must not edit, relink, or delete either artifact.
Unsafe ownership, modes, file types, links, or directory bindings fail closed.
Missing receipt ancestry is evidenceless only when descriptor inspection proves
it absent; platforms without that secure inspection backend fail closed.

When selection records `phase: paused`, `selected_issue: null`, and
`pause_reason: no_ready_issue_after_review`, a continuous foreground conductor
stays alive and polls at its configured interval. A later ready-queue snapshot
with an eligible issue durably resets that exact pause to `Scan` before the
normal scan, review, and selection path runs. An empty snapshot retains the
pause without exiting for the supervisor to relaunch every five seconds.
Paused states with another reason, a selected issue, or a resume phase other
than `Select` are not reset; incompatible no-ready state fails closed.

Tier-2 explore drains default `AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS` to `600`
seconds without observable output; set the variable explicitly for slower
repositories or remote research providers.

`autospec autonomous stop --immediate` and `autospec autonomous restart --force`
signal each verified conductor, monitor, and supervisor process group. Restart
releases the terminated conductor's matching lease before acquiring replacement
ownership, so legacy wrapper children cannot revive a stale conductor during
the handoff. New unit metadata binds the PID to its process-group ID and start
time; mismatched live identities fail closed instead of signaling a reused PID.

Detached conductors intentionally exit after finite claim and executor
boundaries. The supervisor treats a verified stopped conductor as recoverable:
unless a stop flag is present, it releases only that terminated owner's lease,
acquires a fresh lease, and relaunches the persisted `run-foreground` options.
Live or ambiguous metadata never starts a second conductor. Supervisor output
reports `restarted-conductor` only after replacement process identity is
verified. On Linux, an exited child in process state `Z` is stopped rather than
live: the owning supervisor reaps that exact PID before releasing its lease,
while non-child zombies are treated as terminated without attempting to reap a
foreign process.

Explore drains export an inherited recursion marker; a nested drain request
returns a dry suppression result instead of launching another harness tree.
They also enforce a 900-second absolute runtime cap via
`AUTOSPEC_AUTONOMOUS_EXPLORE_MAX_SECS`, independent of incidental harness output.

The drain exports `AUTOSPEC_EXPLORE_PARENT_PID`; detached explore scripts watch
that owner and terminate when it disappears.

The explore drain dispatches directly through the active Codex, Claude, or
OpenCode harness. `AUTOSPEC_HANDOFF_DISPATCHER_KIND` is authoritative; when it
is unset, active runtime markers are preferred over installed skill homes. The
verifier command is exported to that child so a mixed-harness installation
cannot silently select an unrelated provider.

If the skeptic itself times out, the verifier uses a deterministic fallback: it
survives only candidates whose evidence names an existing repository path and
line, and refutes all other candidates. Set
`AUTOSPEC_AUTONOMOUS_DETERMINISTIC_VERIFY=0` to require the skeptic exclusively.

Discovery safety review resolves the Rust CLI in this order: `AUTOSPEC_BIN`,
the repository's `target/debug/autospec`, `~/.autospec/bin/autospec`, then PATH.
This keeps filing functional in detached sessions whose PATH does not include
the installed command shim.

Each researcher is bounded by `AUTOSPEC_RESEARCHER_TIMEOUT_SECS` (default 120
seconds). The cycle records selected, successful, and failed sources in
`researcher_health`; timeout, missing-script, non-zero, and malformed-output
failures make the cycle non-zero even when another source produced candidates.
Only a completed zero-candidate scan is clean dry. Timed-out groups are
terminated as a process group to clean up detached researcher children as well.

The outer explore harness is isolated with `setsid` and receives the same
process-group cleanup, preventing a detached model process from holding a cycle.
The default no-output stall bound is 120 seconds and the absolute runtime bound
is 300 seconds; both remain configurable through `AUTOSPEC_AUTONOMOUS_EXPLORE_*`.
Missing dispatchers, authentication failures, non-zero exits, and watchdog
timeouts return non-zero with `dry:false`; they are never counted as repository
exhaustion.

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

## Native Tier 2 discovery

The Rust foreground conductor runs Tier 2 after a completed empty Tier 1.5
scan. It collects bounded repository-local evidence before launching separate
generator and verifier model children. Each child receives only serialized
evidence in one argument, runs in an isolated scratch directory, has no GitHub
mutation authority, and is killed with its process group after 120 seconds.

Codex runs with a read-only ephemeral sandbox and Claude runs in plan mode with
no tools. OpenCode fails closed because its pure mode still permits built-in
mutation tools; native Tier 2 must not run there until a no-tools mode is
proven. Child output is written to a private file, capped while the child is
live, and removed on every exit path. Invalid JSON, non-zero exits, missing
binaries, oversized output, and timeouts produce sealed failure receipts; they
are not clean dry outcomes. Successful empty generation still requires a
successful empty verifier result before Tier 2 advances.

Ranked Tier 2 survivors are local receipt evidence only. A separate publisher
owns idempotent `auto-implement` issue creation, so generator or verifier
children can never mutate GitHub or the worktree. The conductor publishes each
survivor with `auto-implement` and `origin:self`, plus a repository-scoped
publication marker derived from the proposal stable key. Recovery scans both
open and closed issues: one matching marker with both publication labels is
complete, no marker is retried, and a missing label or multiple matches fails
closed. The Tier 2 cursor advances only after every marker from the sealed
receipt is confirmed remotely.

Before a draft can reach issue creation, the publisher synthesizes the expected
implementation diff from its target and regression-test paths and runs the
shared implementation contract. Blocking `OUT_OF_SCOPE` or `MISSING_TEST`
findings reject the draft before it receives `auto-implement` or consumes an
implementation token. Project-native regression artifacts such as
`scripts/test-autonomous-status-panel.mjs` satisfy the test requirement; their
paths must still appear in the draft's `## Implementation outline`.
