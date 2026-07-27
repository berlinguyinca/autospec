# Rust Autonomous Executor Bridge Design

## Goal

Make a claimed Rust foreground-conductor issue progress from implementation
through verified merge and cleanup without requiring an operator or another
session to write `.autospec/executor-result.json`.

## Context

Before this change, the foreground conductor selected, safety-reviewed, and
claimed one issue with an exact repository, worker, branch, claim generation,
and invocation identity, but its `executor-child` did not implement that issue.
It read a fixed result artifact and otherwise emitted
`implementation_executor_pending`. The parent then persisted that pending
response as terminal, so every later supervisor relaunch replayed the same dead
end.

This is observable in the live `berlinguyinca/autospec-gui` run: issues #34,
#35, and #36 were discovered, but #36 remains claimed with no pull request.

## Chosen approach

The Rust conductor owns a native executor bridge. It launches the
configured Codex, Claude, or OpenCode binary directly with an explicit argument
vector in an isolated issue worktree. It will not call `autospec-run`, `omx`, a
legacy autonomous script, or a shell-owned conductor.

The bridge is a recoverable state machine:

1. Resolve the harness from `AUTOSPEC_HANDOFF_DISPATCHER_KIND`, active-session
   markers, then the installed runtime-alias table and PATH.
2. When `.autospec/explore-mode.json` exists, resolve and validate its sandbox
   branch as the mandatory base and reject any main-targeting mutation.
   Otherwise resolve `AUTOSPEC_BASE_BRANCH`, then `.autospec/autospec.yml`
   `git.base_branch`, then the remote default branch. Persist the ref and OID,
   fetch it, and create or adopt the exact clean private
   `/tmp/autospec-executor/<repository-scope>/issue-<N>` worktree and
   `feat/autonomous-issue-<N>` branch.
3. Persist the invocation identity and `implementing` phase before launch.
4. Start or adopt the target repository's isolated runtime through
   `autospec runtime env session --repo <worktree> --mode auto -- ...` when a
   manifest exists, then launch one local-only implementation harness with a
   dedicated prompt. The prompt states that the claim and worktree already
   exist, forbids branch switching and remote mutations, and requires
   implementation, tests, and local commits with exactly one Closeout report
   artifact. Rust, not the model, owns push and pull-request creation.
5. Stream child stdout and stderr to the repository-scoped autonomous log so
   the existing initiating-session `--follow` surface shows progress.
6. After child exit, compare protected base/primary state and remote refs
   against the pre-launch snapshot, run the deterministic implementation
   linter, re-read the base OID, push only the exact issue branch, and create one
   draft PR with the validated Closeout report.
7. Independently run the issue's Primary smoke line as a bounded direct-command
   plan. It supports sequential `&&` commands without a shell and rejects pipes,
   redirects, substitutions, backgrounding, and other shell operators. Run
   deterministic implementation/security lint as well. These runtime and static
   results—not a model verdict—produce the existing typed QA/security evidence.
   A bounded model reviewer may only add findings or block; it cannot originate
   a Pass.
8. Evaluate the existing immutable premerge decision and require Pass. Mark the
   exact draft PR ready, wait for all non-advisory required CI checks, dispatch
   the independent LGTM reviewer, and require its strict LGTM result.
9. Re-read the configured base and head before admission. If either changed,
   update without force and repeat smoke, full suite, scanners, typed evidence,
   Pass receipt, push, CI, and LGTM until one stable head/base pair spans every
   gate.
10. While that exact PR remains open, ingest the strict successful
    `executor-result` bound to its head and receipt. Then admin-squash-merge
    under the existing authority, observe merged state, write the terminal
    merged claim transition, tear down only the invocation's runtime session,
    and remove the owned worktree. A merge failure preserves the accepted
    result and resumes at merge without re-implementing.

The bridge never trusts a harness statement as Git or GitHub proof. Git, GitHub,
claim, PR, and premerge identities are re-read by Rust at every transition.

## Harness contract

The installed `harness-runtime-aliases.tsv` remains the shared source for
canonical binary names and approval aliases:

- Codex: `codex exec -C <worktree> --sandbox workspace-write --ephemeral
  --output-last-message <artifact> <prompt>`.
- Claude: `claude -p --permission-mode acceptEdits --allowedTools <local-only>
  --no-session-persistence --output-format text <prompt>`.
- OpenCode: fail closed unless an installed, explicitly configured containment
  adapter proves that built-in mutation tools cannot escape the worktree;
  `--pure` alone is not containment.

An explicit `AUTOSPEC_HANDOFF_DISPATCHER_KIND` wins. Runtime markers select the
initiating harness next. PATH probing uses the alias-table order after that.
Unsupported, missing, relative, or temporary-directory dispatchers fail closed.
Where a harness cannot provide OS-enforced workspace containment, Rust snapshots
the primary checkout, protected refs, worktree registry, and open PR set before
launch and quarantines the invocation on any extra mutation. GitHub credentials
are removed from the child environment where the harness authentication model
allows it. Only Rust receives authority to push or create, ready, or merge the
pull request.

The alias table and argument builder still recognize OpenCode so installation,
selection, and diagnostics stay synchronized across all three harnesses.
Execution returns `executor_harness_uncontained` before launch until an exact
containment adapter is configured and its path passes the same safety checks.

The installer treats gitleaks, semgrep, trivy, and license-checker as required
autonomous executor dependencies. It invokes the existing cross-platform
dependency mechanism, including the approved sudo path for system packages,
then verifies every scanner. An explicit install opt-out remains available but
causes executor security admission to fail closed rather than degrade silently.

## Persistence and recovery

The invocation document lives below the existing repository-scoped autonomous
state root. It records schema, repository, issue, worker, branch, claim ID,
invocation ID, base ref/OID, worktree, runtime-session identity, harness, phase,
canonical child executable, argv digest, boot/start identity, child PID/process
group, progress timestamp, PR/head identity when known, and terminal result
when one exists.

`pending` and active phases are non-terminal. State and terminal receipts are
scoped to the exact claim generation. A restarted conductor adopts a
clean matching worktree and resumes from the last independently proven
boundary. It may adopt a live claim only when a private local acquisition
receipt matches the authoritative repository, issue, worker, branch, and claim
ID exactly; missing, foreign, malformed, symlinked, or non-private receipts fail
closed. A live child is observed rather than duplicated only when PID,
process-group, canonical executable, argv digest, and start/boot identity all
match. Ambiguous or reused process identity is never signaled. A dead child
returns to the last safe phase. Only an observed merged PR, explicit blocked
failure, or exhausted retry state is terminal.

Runtime recovery persists the exact environment directory, session identity,
and original manifest snapshot before provisioning. A transient conductor
failure relinquishes only the process lease; it does not tear down resources
that the next matching invocation must adopt. Final cleanup reconstructs the
original context from that durable snapshot, so changing or removing the
repository's current runtime manifest cannot redirect or strand owned
resources.

Active sessions created with the earlier schema-1 snapshot remain recoverable.
Their private authoritative plan supplies the missing digest only after its own
identity and integrity validation, and their missing bypass evidence is
conservatively recovered as bypassed rather than verified isolation.

During implementation, verification, CI wait, and review, the bridge
periodically performs a compare-and-set refresh of the exact GitHub claim
generation. It preserves worker, branch, claim ID, and claimed-at identity
while advancing the remote updated-at heartbeat. A takeover or stale
generation makes subsequent work inert and aborts every remote mutation.
Transient GitHub reads retain a typed retryable classification at every phase.
Once implementation has started, those retries resume the same durable
invocation and exact claim generation rather than releasing ownership and
launching another harness.

Receipt retirement is ordered through the conductor state machine. Terminal
completion and ownership loss first persist a dedicated retirement boundary,
then clear and sync the acquisition receipt, and only then return to scanning.
Restart at either side of the clear therefore resumes retirement instead of
replaying the completed or superseded generation.

Before failure teardown, the bridge publishes a claim-generation-bound cleanup
intent. Restart consumes that intent before runtime reattachment. Claim takeover
closes only the old exact runtime and records the worktree HEAD plus status
digest while holding the per-issue worktree lock; the successor generation must
atomically adopt that receipt and preserve the worktree, including uncommitted
WIP. Initial provisioning records the active generation under the same lock,
closing the predecessor-finalization race. A failed claim mutation is ownership
loss only when its authoritative reread shows a different generation; unchanged
authority remains a typed transient. Terminal observation uses the same rule.
Pre-receipt dispatch state migrates only from one exact private invocation
or terminal receipt matching the authoritative lease. With no such proof, the
conductor enters ownership retirement instead of guessing.

The current fixed `.autospec/executor-result.json` adapter remains available as
a compatibility ingestion path, but it is no longer the default producer.

On retryable failure the bridge stops only its runtime session, preserves the
private worktree, records exact failure evidence, and releases the exact claim
back to `auto-implement`. On exhausted or non-retryable failure it adds
`autospec:needs-human`, removes both claim and queue ownership, preserves the
worktree evidence, and continues scanning other work. No terminal invocation
may retain `in-progress-by-bot`.

## Progress and stop behavior

Every phase transition and bounded child-output line is written as a structured
executor event to the scoped conductor log. `autospec-autonomous` follow mode
already streams that log to the Codex, Claude, or OpenCode session that started
the run. No desktop notification is added.

Immediate stop terminates the owned harness process group, preserves committed
and uncommitted work in the isolated worktree, and leaves non-terminal state
for a later resume. Graceful stop allows the current executor phase to finish
and prevents another issue dispatch.

## Safety boundaries

- One issue, claim generation, repository-scoped worktree, runtime session, and
  PR per invocation.
- No branch deletion, merge, force-push, protected-branch push, or primary
  checkout mutation by the implementation harness.
- No success from stdout substring matching or exit status alone.
- No success without a changed commit, clean worktree, exact PR head, Closeout
  report, passing runtime smoke, the complete resolved target-repository suite,
  deterministic implementation lint, passing gitleaks/semgrep/trivy/license
  scans, Pass receipt, required CI, independent LGTM, and observed merged PR.
- No terminal persistence for `implementation_executor_pending`.
- No new dependency and no shell or legacy conductor fallback.
- The strict pull-request representation includes `isDraft`; draft is required
  before verification and non-draft is required before success and merge.

## Alternatives rejected

Calling the legacy shell `$autospec-run` is rejected because it owns queue
selection and claim acquisition and can select a different issue or create a
second ownership authority.

Keeping the external result-file adapter as the only producer is rejected
because perpetual autonomy then depends on an unrelated live operator session.

## Verification

Unit tests cover harness resolution, alias parsing, explicit argument vectors,
OpenCode containment refusal, base resolution, strict state decoding, process
identity, non-terminal recovery, claim refresh, draft state, direct command-plan
parsing, full-suite resolution, scanner evidence, structured result parsing,
terminal release, and identity rejection.

Integration tests use a real local Git repository and bare remote plus
hermetic executable fixtures. They prove repository-scoped worktrees for equal
issue numbers, runtime-manifest isolation, one direct harness launch, streamed
progress, restart adoption, PID-reuse-safe stop, claim refresh beyond TTL,
takeover abort, draft-PR proof, direct QA/security evidence, ready transition,
required-CI wait, LGTM, admin merge, terminal claim release, and fail-closed
foreign/stale/malformed/extra-mutation cases. Retry exhaustion is also proven to
release the claim and let another issue run. The installed binary is then
dogfooded against autospec-gui issues #36, #34, and #35.
