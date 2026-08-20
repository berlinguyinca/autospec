# Autonomous Runbook

## Dirty integration checkout containment

Before a fresh selection, the conductor synchronizes its configured integration
branch with the remote. A one-shot `run-foreground` invocation still fails closed
when that branch is checked out with tracked or untracked changes.

A continuous conductor records `integration_base_dirty` as a no-progress cycle
instead of terminalizing the run. Status reports that current reason rather than
an older executor outcome, and the next cycle retries after its polling interval.
The conductor does not overwrite, stash, or delete operator work. To restore Tier
1 immediately, restart it with `--repo-dir` pointing at a clean dedicated clone.

## Continuation-aware merge recovery

When an executor restarts from `BridgePhase::Merged`, recovery completes in this
order:

1. Resolve the remote default branch from its authoritative `HEAD`
   advertisement, fetch a stable integration tip, and prove the persisted
   merge OID is its ancestor.
2. For a merge into a non-default integration branch, select exactly
   `current_child`, falling back to legacy `identity.issue` state, and observe
   that same issue number with `gh issue view`.
3. Close the selected issue only when it is open, then observe that exact number
   as closed. An already-closed issue is accepted as an idempotent replay.
4. Reconcile continuation state and transition the claim before advancing to
   `BridgePhase::CleanupPending` and removing local resources.

Default-branch merges rely on the pull request's closing reference and do not
run an explicit `gh issue close`. Any ambiguous branch advertisement,
non-descendant integration rewrite, issue-number mismatch, observation failure,
or close failure leaves the durable phase at `BridgePhase::Merged` so the next
run can replay safely.

## Independent review

After deterministic premerge checks and required CI pass, the native autonomous
executor launches one independent reviewer. No reviewer command is required for
normal operation:

1. Autospec loads the installed four-column harness alias table from
   `AUTOSPEC_HARNESS_RUNTIME_ALIASES`, `AUTOSPEC_CONFIG_DIR`, the user Autospec
   config directory, or the repository fallback.
2. Autospec classifies the review from changed paths, issue labels, logical
   component count, producer/consumer boundaries, and critical authority
   boundaries. Non-normal work uses high reviewer reasoning and requires an
   integration smoke that invokes a repository test under `tests/integration`,
   `tests/smoke`, or `tests/e2e`. High and integration work prefer a provider
   other than the implementer's, while critical work fails closed unless an
   alternate provider is known and available. OpenCode is a harness, not proof
   of a distinct provider, so it never establishes diversity by itself.
3. Normal work retains the implementer's provider when it is available. A
   permitted same-provider fallback is recorded explicitly rather than being
   reported as provider diversity.
4. The reviewer receives the issue contract and exact commit to review.

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
work, but automatic review does not use it. `install.sh` ships a default
adapter (`~/.autospec/scripts/lib/opencode-containment-adapter.sh`) and the
executor bridge auto-discovers it (explicit env override, then
`AUTOSPEC_SCRIPTS_DIR`, then `~/.autospec/scripts`) when no adapter is set;
it applies the implementer permission profile
(deny-by-default, workspace-scoped edit/bash only). Operators wanting OS-level
isolation may replace it with a bwrap/firejail wrapper honoring the same argv
contract; a shipped `~/.autospec/scripts/lib/opencode-containment-bwrap.sh`
variant mounts the host read-only with only the worktree and a private config
dir writable (PID/IPC/UTS unshared, network retained for the model API).

OpenCode model selection maps onto the two-tier AGENTS.md routing through
`AUTOSPEC_OPENCODE_MODEL` (`provider/model`) and `AUTOSPEC_OPENCODE_VARIANT`
(reasoning effort), passed as `--model`/`--variant` on both the implementer and
reviewer invocations. Live usage observability comes from the shipped
`opencode-usage-probe.sh`, which reads the OpenCode SQLite DB for a
trailing-window token tally and feeds `usage-observe.sh` a live percent for the
quota governor.

Inherited `PATH`, XDG roots, `CODEX_HOME`, and `CLAUDE_CONFIG_DIR` are
canonicalized before launch and fail closed if they resolve inside either
reviewed repository. The external normalizer invokes `env`, `wc`, `truncate`,
and `python3` through canonical absolute system paths, so worktree or host
`PATH` shadowing cannot change its verdict checks.

Review command output, error output, and any
harness-specific result are stored under the private executor state tree,
outside reviewed source. An external private normalizer captures normal harness
diagnostics without treating transport traces as findings. Codex uses its final
message artifact as the verdict; Claude and OpenCode use their captured text
output. Every harness must return exactly one closed-schema JSON object with
schema `1`, the exact reviewed commit, a verdict, nonempty examined surfaces and
tests, exact policy-bound integration citations, and blocking findings. The
integration citation array must exactly match the sealed requirements digest,
evidence digest, and command-record paths supplied to the reviewer. Only an
exact-commit `lgtm` with zero blocking findings authorizes the review.

The trusted normalizer validates that structured JSON before emitting the
legacy exact `LGTM` stdout consumed by the state machine. Invalid JSON, unknown
fields, commit drift, missing required evidence, blocking findings, stderr,
nonzero exit, artifact replacement, or truncation all fail closed. The harness
inherits a 1 MiB file-size limit, and reaching that limit in stdout, stderr, or
the Codex result fails review rather than accepting truncated evidence. The
verdict file is cleared before every launch so an interrupted attempt cannot
authorize its retry.

Schema-5 receipts bind the exact commit, the full resolved review requirements,
provider selection, policy digest, changed-path component and producer/consumer
inventory, immutable integration-record citations, structured semantic verdict
and digest, normalizer, transport diagnostics, and raw result artifacts.
Recovery rereads and revalidates each bound artifact before restoring
`ReviewPassed`. Legacy schema-2 through schema-4 receipts are archived, the invocation returns to
`CiPassed`, and review runs again under the current policy. Local, git, GitHub,
or other remote mutation during review fails the gate.

If no configured alias is usable, the executor reports
`executor_harness_unknown` before review can mutate the pull request.

### Explicit override

`AUTOSPEC_EXECUTOR_REVIEW_COMMAND` cannot authorize production autonomous
review. When it is present, the executor fails closed and requires a configured
structured harness alias so a free-form command cannot bypass semantic evidence
or schema-5 receipt binding.
