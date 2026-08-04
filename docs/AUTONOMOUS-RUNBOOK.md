# Autonomous Runbook

## Continuation-aware merge recovery

When an executor restarts from `BridgePhase::Merged`, recovery completes in this
order:

1. Validate the persisted merge OID and resolve the remote default branch from
   the remote's authoritative `HEAD` advertisement.
2. For a merge into a non-default integration branch, select exactly
   `current_child`, falling back to legacy `identity.issue` state, and observe
   that same issue number with `gh issue view`.
3. Close the selected issue only when it is open, then observe that exact number
   as closed. An already-closed issue is accepted as an idempotent replay.
4. Reconcile continuation state and transition the claim before advancing to
   `BridgePhase::CleanupPending` and removing local resources.

Default-branch merges rely on the pull request's closing reference and do not
run an explicit `gh issue close`. Any ambiguous branch advertisement, merge-OID
mismatch, issue-number mismatch, observation failure, or close failure leaves
the durable phase at `BridgePhase::Merged` so the next run can replay safely.

## Independent review

After deterministic premerge checks and required CI pass, the native autonomous
executor launches one independent reviewer. No reviewer command is required for
normal operation:

1. Autospec loads the installed four-column harness alias table from
   `AUTOSPEC_HARNESS_RUNTIME_ALIASES`, `AUTOSPEC_CONFIG_DIR`, the user Autospec
   config directory, or the repository fallback.
2. `AUTOSPEC_HANDOFF_DISPATCHER_KIND` and the active Codex, Claude, or OpenCode
   session marker select the reviewer. With no marker, Autospec chooses the
   first installed alias whose executable is available on `PATH`.
3. The reviewer receives the issue contract and exact commit to review.

The resolved reviewer executable must be external to both the source repository
and its issue worktree. Automatic reviewers run with a sanitized allowlist
environment rather than inheriting the conductor environment. Codex uses its
read-only sandbox and ignores user config plus execpolicy rules while retaining
`CODEX_HOME` authentication; Claude receives only `Read`, `Glob`, and `Grep` in
plan mode;
OpenCode selects a dedicated `autospec-reviewer` agent whose inline agent-level
policy denies every tool except read, glob, grep, list, and LSP. Its config root
is a private executor artifact directory, host and Claude-compatible config
loading is disabled, and the external home/data roots used for provider
authentication remain available. OpenCode's separate
`AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER` remains required for implementation
work, but automatic review does not use it.

Inherited `PATH`, XDG roots, `CODEX_HOME`, and `CLAUDE_CONFIG_DIR` are
canonicalized before launch and fail closed if they resolve inside either
reviewed repository. The external normalizer invokes `env`, `wc`, and `cat`
through canonical absolute system paths, so worktree or host `PATH` shadowing
cannot change its verdict checks.

Review command output, error output, and any
harness-specific result are stored under the private executor state tree,
outside reviewed source. An external private normalizer captures normal harness
diagnostics without treating transport traces as findings. Codex uses its final
message artifact as the verdict; Claude and OpenCode use their captured text
output. The normalizer succeeds only when that harness-specific verdict is
exactly `LGTM`, then emits only `LGTM` on stdout and nothing on stderr to the
strict review gate. The receipt binds the normalizer, captured diagnostics, and
verdict so crash recovery cannot substitute any of them. The harness inherits a
1 MiB file-size limit, and reaching that limit in stdout, stderr, or the Codex
result fails review rather than accepting truncated evidence. The verdict file
is cleared before every launch so an interrupted attempt cannot authorize its
retry. A durable receipt is validated before a restarted executor resolves or
launches another harness. Local, git, GitHub, or other remote mutation during
review fails the gate.

If no configured alias is usable, the executor reports
`executor_harness_unknown` before review can mutate the pull request.

### Explicit override

`AUTOSPEC_EXECUTOR_REVIEW_COMMAND` remains the highest-priority operator
override. When set, Autospec validates and runs that single bounded direct
command instead of reading or resolving the harness alias table. The same exit,
mutation, and result-receipt checks still apply. Because an explicit command is
already the operator-defined trust boundary, its stdout must be exactly `LGTM`
and its stderr must remain empty; it does not receive automatic normalization.
