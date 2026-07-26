# Rust Autonomous Executor Bridge Design

## Goal

Make a claimed Rust foreground-conductor issue progress from implementation
through verified draft PR evidence without requiring an operator or another
session to write `.autospec/executor-result.json`.

## Context

The foreground conductor already selects, safety-reviews, and claims one issue
with an exact repository, worker, branch, claim generation, and invocation
identity. Its current `executor-child` does not implement that issue. It reads a
fixed result artifact and otherwise emits `implementation_executor_pending`.
The parent then persists that pending response as terminal, so every later
supervisor relaunch replays the same dead end.

This is observable in the live `berlinguyinca/autospec-gui` run: issues #34,
#35, and #36 were discovered, but #36 remains claimed with no pull request.

## Chosen approach

The Rust conductor will own a native executor bridge. It will launch the
configured Codex, Claude, or OpenCode binary directly with an explicit argument
vector in an isolated issue worktree. It will not call `autospec-run`, `omx`, a
legacy autonomous script, or a shell-owned conductor.

The bridge is a recoverable state machine:

1. Resolve the harness from `AUTOSPEC_HANDOFF_DISPATCHER_KIND`, active-session
   markers, then the installed runtime-alias table and PATH.
2. Resolve the remote default branch, fetch it, and create or adopt the exact
   clean `/tmp/wt-autonomous-issue-<N>` worktree and
   `autonomous/issue-<N>` branch.
3. Persist the invocation identity and `implementing` phase before launch.
4. Launch one implementation harness with a dedicated prompt. The prompt states
   that the claim and worktree already exist, forbids branch switching and
   merging, and requires implementation, tests, commits, a push, one draft PR,
   and exactly one Closeout report.
5. Stream child stdout and stderr to the repository-scoped autonomous log so
   the existing initiating-session `--follow` surface shows progress.
6. After child exit, independently resolve the worktree HEAD and the one open
   pull request for the exact head branch. Reject a dirty worktree, unchanged
   base, wrong head OID, non-draft PR, missing issue-closing reference, or
   malformed Closeout report.
7. Launch bounded QA and security verifier phases through the same harness
   adapter. Their final messages use strict JSON schemas. Rust parses those
   exact artifacts and produces the existing typed QA/security evidence.
8. Evaluate the existing immutable premerge decision, mark the draft ready only
   after a Pass decision, and submit the strict `executor-result` bound to the
   exact claim, PR head, and receipt digest.

The bridge never trusts a harness statement as Git or GitHub proof. Git, GitHub,
claim, PR, and premerge identities are re-read by Rust at every transition.

## Harness contract

The installed `harness-runtime-aliases.tsv` remains the shared source for
canonical binary names and approval aliases:

- Codex: `codex exec -C <worktree> --sandbox danger-full-access --ephemeral
  --output-last-message <artifact> <prompt>`.
- Claude: `claude -p --dangerously-skip-permissions
  --no-session-persistence --output-format text <prompt>`.
- OpenCode: `opencode run --dir <worktree>
  --dangerously-skip-permissions <prompt>`.

An explicit `AUTOSPEC_HANDOFF_DISPATCHER_KIND` wins. Runtime markers select the
initiating harness next. PATH probing uses the alias-table order after that.
Unsupported, missing, relative, or temporary-directory dispatchers fail closed.

## Persistence and recovery

The invocation document lives below the existing repository-scoped autonomous
state root. It records schema, repository, issue, worker, branch, claim ID,
invocation ID, base commit, worktree, harness, phase, child PID/process group,
progress timestamp, PR/head identity when known, and terminal result when one
exists.

`pending` and active phases are non-terminal. A restarted conductor adopts a
clean matching worktree and resumes from the last independently proven
boundary. A live matching child is observed rather than duplicated. A dead
child returns to the last safe phase. Only accepted success, explicit blocked
failure, or exhausted retry state is terminal.

The current fixed `.autospec/executor-result.json` adapter remains available as
a compatibility ingestion path, but it is no longer the default producer.

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

- One issue, claim generation, branch, worktree, and PR per invocation.
- No branch deletion, merge, force-push, protected-branch push, or primary
  checkout mutation by the implementation harness.
- No success from stdout substring matching or exit status alone.
- No success without a changed commit, clean worktree, exact PR head,
  Closeout report, passing QA/security evidence, and Pass receipt.
- No terminal persistence for `implementation_executor_pending`.
- No new dependency and no shell or legacy conductor fallback.

## Alternatives rejected

Calling the legacy shell `$autospec-run` is rejected because it owns queue
selection and claim acquisition and can select a different issue or create a
second ownership authority.

Keeping the external result-file adapter as the only producer is rejected
because perpetual autonomy then depends on an unrelated live operator session.

## Verification

Unit tests cover harness resolution, alias parsing, explicit argument vectors,
strict state decoding, non-terminal recovery, structured result parsing, and
identity rejection.

Integration tests use a real local Git repository and bare remote plus
hermetic executable fixtures. They prove one isolated worktree, one direct
harness launch, streamed progress, restart adoption, process-group stop,
draft-PR proof, QA/security evidence, Pass receipt submission, and fail-closed
foreign/stale/malformed cases. The installed binary is then dogfooded against
autospec-gui issues #36, #34, and #35.
