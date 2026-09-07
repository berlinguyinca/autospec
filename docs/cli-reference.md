# CLI Reference

The Rust CLI is additive. Runtime environment management is implemented by the
Rust command family below; existing `/autospec-*` skills and unrelated shell
scripts remain operational surfaces while V62+ commands mature.

| Command | JSON | Status |
| --- | --- | --- |
| `autospec init --spec <id> [--spec <id>]... [--json]` | yes | initialize persisted planned state without executing work; refuses existing state |
| `autospec doctor --json` | yes | implemented |
| `autospec aar classify --title <text> [--body <text>\|--body-file <path>] [--label <l>]... [--path <p>]... [--files <n>] [--language <name>] [--json]` | yes | deterministic task classification with evidence and confidence; no LLM call |
| `autospec aar plan --title <text> [...] [--policy-version <v>] [--json]` | yes | full execution policy: topology, model, reasoning budget, retrieval ladder, guards, escalation |
| `autospec aar explain --title <text> [...]` | no | prose explanation of the selected profile and why |
| `autospec aar memory init [--worktree <dir>] [--json]` | yes | scaffolds `.autospec/` durable task memory; never overwrites existing state |
| `autospec aar rules` | no | prints the harness working rules injected into every agent session |
| `autospec doctor --readiness --json` | yes | implemented target-repo readiness report |
| `autospec doctor code-intel [--json]` | yes | code intelligence backend, language-server and fallback health ([docs](code-intelligence.md)) |
| `autospec status --json` | yes | persisted local spec-lifecycle counts |
| `autospec plan [--input <package-dir>] [--json]` | yes | read-only inspection of generated spec metadata |
| `autospec initiative init --id INIT-YYYY-NNNN --slug <slug> [--spec <path>] [--root <dir>]` | yes | creates the Initiative artifact registry and its first audit event; refuses an existing Initiative |
| `autospec initiative validate --id INIT-YYYY-NNNN [--json]` | yes | checks the Definition, workspace, plan, and task DAG against each other; exits `1` when the Initiative is not executable |
| `autospec initiative ready --id INIT-YYYY-NNNN [--now <unix>] [--json]` | yes | read-only scheduler pass; lists releasable tasks and the first reason each blocked task is held |
| `autospec initiative coverage --id INIT-YYYY-NNNN [--json]` | yes | requirement coverage matrix, including evidence discarded for lacking independence |
| `autospec initiative verify --id INIT-YYYY-NNNN [--json]` | yes | final completion gate; exits `1` while an unwaived requirement is unverified |
| `autospec initiative project --id INIT-YYYY-NNNN [--json]` | yes | renders the GitHub projection from canonical state and stores it; performs no GitHub mutation |
| `autospec initiative status --id INIT-YYYY-NNNN [--json]` | yes | Initiative snapshot: stage, repository and owner span, task states, requirement coverage, completion |
| `autospec validate [--path <changed-path>]... [--json]` | yes | read-only affected-check planner; shell wrapper remains the executor |
| `autospec validate --shadow-results <captured-results.json> [--json]` | yes | aggregates captured shell outcomes without executing commands; returns non-zero when a required captured result failed |
| `autospec runtime classify <path> --json` | yes | implemented R0-R4 ownership classification for one repository path |
| `autospec runtime audit --json` | yes | implemented read-only R0-R4 inventory; it neither migrates nor executes candidates |
| `autospec runtime env init [--repo <path>] [--manifest agent\|autospec] [--force]` | no | creates a conservative v1 runtime manifest; refuses an existing manifest without `--force` |
| `autospec runtime env up [--repo <path>] [--mode <mode>]` | no | provisions or reuses the selected environment, runs its manifest command on first provision, and prints the sourceable environment protocol |
| `autospec runtime env status [--repo <path>] [--mode <mode>]` | no | prints a provisioned environment or returns status `3` when it is inactive |
| `autospec runtime env down [--repo <path>] [--mode <mode>] [--purge-maven]` | no | removes owned Compose resources; `down --purge-maven` also removes the guarded Maven 4 environment prefix |
| `autospec runtime env exec [--repo <path>] [--mode <mode>] -- <command> [args...]` | no | provisions or reuses state, then runs one direct child with the runtime environment |
| `autospec runtime env session [--repo <path>] [--mode <mode>] [--keep-alive] -- <command> [args...]` | no | runs one direct child with lifecycle cleanup, manifest auto-init/bypass controls, and Unix interruption cleanup |
| `autospec runtime env gc [--repo <path>] [--mode <mode>]` | no | removes only stale resources whose generation and ownership labels are proven; ambiguity fails closed with a recovery command |
| `autospec runtime env normalize-compose --repo <path> --check\|--apply [--fingerprint SHA256]` | yes | plans or transactionally applies a manifest-v2 Compose migration without a second YAML transformer |
| `autospec claim state read\|upsert\|clear\|reconcile-linked-pr ...` | yes | manages the schema-1 GitHub run-state comment using lowest-comment-ID selection |
| `autospec claim acquire\|release ...` | yes | applies the typed safety gate, heartbeat/label ordering, lease CAS, and terminal release transitions |
| `autospec issue promote --repo OWNER/REPO --number N [--remove-label needs-autospec-template]` | yes | validates the canonical GitHub issue, records review with owned labels without editing its body, and verifies authoritative re-reads |
| `autospec queue ready [--repo OWNER/REPO] [--batch-size N]` | yes | scans every Rust-owned GitHub issue page and returns typed eligibility, gate totals, and scan scope |
| `autospec queue review-safety --repo OWNER/REPO --limit N [--issue N]` | yes | writes bounded Rust issue-intent safety decisions and reports outcome totals |
| `autospec autonomous resilience decide --repo OWNER/REPO [--issue N] [--budget-tokens N] [--budget-issues N]` | yes | reads resilient admission state without migration; atomic lifecycle ownership writes only canonical `owner__repo` state and starts no shell process |
| `autospec autonomous drain --repo OWNER/REPO --repo-dir DIR [--stall-secs N] [--poll-secs N]` | yes | directly supervises the fixed `omx exec ... $autospec-run` child, preserving local/external progress and terminating only a genuinely stalled live child |
| `autospec autonomous blast-radius --changed-files FILE [--fenced-surfaces YML] [--json]` | yes | classifies changed paths against configured fenced surfaces; fenced matches exit non-zero and report quarantine evidence |
| `autospec autonomous main-health --repo OWNER/REPO --repo-dir DIR [--branch BRANCH] [--json]` | yes | runs the Rust repository-local mainline-health probe without dispatching work |
| `autospec autonomous start --repo OWNER/REPO --repo-dir DIR [--epic N] [--max-cycles N] [--poll-interval-sec N]` | no | refreshes to the checkout's exact immutable runtime, creates or adopts one verified run epic, then launches a lease-owned conductor |
| `autospec autonomous resume --epic N --repo OWNER/REPO --repo-dir DIR` | no | verifies and reconstructs a managed accountability epic, reopening a closed or parked epic when safe, before conductor spawn |
| `autospec autonomous run-foreground --repo OWNER/REPO --repo-dir DIR [--branch BRANCH]` | no | runs one native foreground cycle when invoked directly; a child launched by `start` inherits lifecycle ownership and repeats cycles |
| `autospec autonomous lifecycle decide --repo OWNER/REPO [--claim-repo OWNER/REPO --claim-issue N --claim-worker ID --claim-branch NAME --claim-state active\|terminal] [--lease-age-sec N] [--stop graceful\|immediate] [--health continue\|wait\|halt] [--budget within\|soft\|hard] [--ready-tier 1\|1.5\|2\|3\|4\|5\|6\|7\|idle]` | yes | evaluates one pure typed lifecycle decision without filesystem, process, GitHub, shell, or `omx` effects |
| `autospec autonomous executor-result --repo OWNER/REPO --issue N [--worker-id ID --branch NAME --outcome succeeded\|blocked\|retryable ...]` | yes | records either the exact legacy deferred receipt or one strictly validated executor outcome; it never launches work, releases a claim, or merges a PR |
| `autospec run --run <id> --spec <id>... [--json]` | yes | creates a local persisted queue only; it does not launch an agent or validation command |
| `autospec run --ingest <agent-result.json> --run <id> --spec <id> --result-id <id> --outcome <passed\|failed\|blocked> [--failure-kind <kind>] [--retry-limit <n>] [--json]` | yes | validates and records an explicit local agent result; it does not launch an agent or validation command |
| `autospec resume [--json]` | yes | reports the newest incomplete local queue and its next entry; it does not execute it |
| `autospec report --json` | yes | local release summary from persisted spec state |
| `autospec rag config [--set KEY=VALUE] [--json]` | yes | renders the effective `agentic_rag:` configuration and rejects an invalid one (a revision-blind cache, an unknown key) |
| `autospec rag policy [--role ROLE] [--json]` | yes | prints per-role source ordering, context ceiling, sufficiency threshold, and whether the role must verify independently |
| `autospec rag sources [--role ROLE] [--external] [--json]` | yes | reports, per source, the administrator's availability setting and whether that role and task may actually reach it |
| `autospec rag route --task TASK [--context N] [--node id:reasoning:free_context:speed:seats]... [--json]` | yes | explains one InferWeave routing decision: required context including margin, the selected node, and why each other node was rejected |
| `autospec showcase --json` | yes | demo stub |
| `autospec benchmark` | no | documented stub, exits non-zero |
| `autospec growth-report --json` | yes | local-only metrics stub |

`autospec rag` is read-only and performs no retrieval. It reports what the Agentic RAG
subsystem's configuration and policy *would* do, so an operator can check a role budget or a
routing rejection without running a retrieval or reaching a knowledge source. Retrieval itself
is a library API (`autospec_core::rag::RetrievalCoordinator`), not yet a command.

`autospec plan` only reads and parses Markdown from one generated package. It does not
execute validation, calculate an execution order, or report persisted lifecycle state.

`autospec validate --shadow-results` accepts the strict schema-1 captured-result shape used
by `crates/autospec-cli/tests/fixtures/validation-results/`: each row supplies a unique name,
Boolean `required`, and signed `exit_code`. Rust computes the pass/fail aggregate only; it never
spawns the captured command. The compatibility wrapper still delegates all real validation to
`autospec validate` until a full fixture-backed cutover is approved.

`autospec run` is deliberately a state-management command, not an execution engine. Queue
creation requires an explicit run ID and one or more spec IDs. Result ingestion requires a
strict `schemas/autospec-agent-result.schema.json` document plus an explicit outcome and
result ID; `failed` also requires `--failure-kind` (`validation`, `environment`, `agent`,
`dependency`, or `safety`). Results are retained append-only below
`.autospec/runs/<run-id>/agent-results/<spec-id>/<result-id>.json`, so a retry can safely
replay the same result ID without consuming another queue attempt. `resume` only reports the
current queue position. Use `/autospec-run` for the existing agent-execution workflow.

For the v1 runtime-manifest grammar, state behavior, child-command semantics, and cleanup
procedure, see [Agent runtime manifests](runbooks/agent-runtime-manifest.md).

Manifest `version: 2` adds typed Maven and Compose ownership. The opt-outs
`AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
`AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1`; evidence produced under an
opt-out is not verified isolation. Unix state is private (`0700` directories, `0600` files),
and `RUNTIME_STATE_SYMLINK_REJECTED` prevents destructive cleanup through a linked root.

`autospec claim state` is the Rust-owned transport and codec for the existing GitHub
run-state comment protocol. `read` fails closed when the lowest marked comment is malformed
or bound to a different issue; `upsert` patches that lowest comment and removes higher-ID
duplicates; `clear` removes only marked run-state comments; and `reconcile-linked-pr` records
one eligible linked PR before posting the idempotent post-PR handoff reminder. `acquire` validates
the current issue safety review before it writes a startup heartbeat and moves labels, then uses
the lowest GitHub comment ID plus a server-side timestamp to decide the lease. `release` writes
terminal merge evidence before state and label transitions, and then retires the local evidence for
that claim: the issue heartbeat and the session binding whose recorded identity matches the released
claim. Retiring the session binding is what lets one worker session claim a second issue — the
binding is create-once, so a binding left naming a finished issue makes every later `acquire` from
that session fail with `heartbeat_write_failed`. Retirement is best effort and only runs when local
evidence exists; the remote record is already authoritative, and the next acquirer's predecessor
path retires anything left behind. When that predecessor retirement fails, the refusal reports the
predecessor's `claim_id`, which is the value `claim release --claim-id` needs. Legacy script
entrypoints remain only as compatibility surfaces until every caller is redirected to this command
family.

`autospec issue promote` owns the remote admission transaction. It fetches the canonical GitHub
issue, applies the repository's trusted-actor and regex policy, validates the existing canonical
safety section, records review with `safety:reviewed` without editing the body, re-reads the exact
state, and only then adds `auto-implement`. A final re-read detects concurrent title, body, author,
state, or label changes and rolls back transaction-owned labels. Completed admissions are
idempotent, and `--remove-label needs-autospec-template` lets the same transaction finish the
groomer's owned label transition with verified rollback on cleanup failure or drift.

The JSON response emits `"auto-implement": true` only for a passing verdict and reports
`eligible` for final-payload queue-policy eligibility plus `changed` for remote mutation. It
returns structured ambiguous/blocked/indeterminate decisions without admission and groups
blocked or indeterminate verdicts by inner safety reason in `blocked_by_reason`. `eligible` is
not a live claim, dependency, pull-request, worker-capacity, or path-conflict decision.

`autospec queue ready` follows every GitHub REST page for open `auto-implement` work and active
claims, counts raw issue-page records before filtering pull requests, and cursor-paginates linked
pull-request evidence while preserving check snapshots. A malformed or incomplete later evidence
page blocks selection rather than shortening the scan. Its JSON includes a stable `gate_counts`
object for discovered, candidate, reviewed,
blocked, dependency-blocked, linked-PR-blocked, path-conflicted, ready, claimed, and selected
issues. `scan_scope` is `repository` for a full scan and `slice` when
`AUTOSPEC_RUN_ONLY_ISSUES` constrains the result, so callers cannot mistake a completed slice for
whole-queue completion. It also carries a `collision` object — present only when the dispatch
batch is not fully parallel — that predicts file collisions before anything is dispatched
(issue #3564). Each issue's estimated touch set comes from path-shaped tokens in its title and
body (`path`, `path:line`, trailing-slash `dir/` references), hotspot files declared in
`AGENTS.md`/`CONTRIBUTING` conflict lines, and files appearing in at least 40% of the last 200
commits behind the VCS-agnostic `CommitHistory` trait (no GitHub or git-host assumption). A
referenced directory is upgraded to the hotspot files beneath it. `collision.waves` lists
dispatch order breadth-first: issues inside one wave are predicted disjoint and run in parallel;
predicted colliders are serialised into later waves. Each `collision.warnings` entry names the
count and the file ("6 of 8 issues are likely to touch cmd/gateway/main.go; consider serialising
or splitting that file first") and is echoed to stderr, as are `collision.refactor_suggestions`
for files that stay hotspots across batches, tracked in `~/.autospec/collision-ledger.json`.
A batch with no predicted overlap emits no `collision` field and dispatches fully in parallel,
unchanged.

`autospec queue review-safety --repo OWNER/REPO --limit N [--issue N]` requires a positive bound
and scans only unreviewed open `auto-implement` issues. `--issue N` re-reads and reviews that
exact admitted issue without scanning the queue, so groomers can safely bridge queue admission to
Rust-owned safety writeback. It emits
`pass`, `ambiguous`, `block`, `stale`, `conflicted`, and `skipped` totals. A pass writes one
canonical review block, adds `safety:reviewed`, and re-reads the issue through the typed claim
gate. Ambiguous issues receive `autospec:needs-human`; blocking issues receive
`security:quarantined`; neither becomes reviewed-eligible. Conflicting or malformed remote
evidence is fail-closed and counted as `conflicted`.

`autospec autonomous run-foreground` is the typed Rust control-plane and executor entrypoint.
One cycle performs bounded mainline-health admission and queue safety review, selects and claims
at most one ready issue, and persists strict conductor state as
`.autospec/autonomous-operator/<scope>/foreground-conductor-<scope-key>.json`, where the scope
key distinguishes repository runs from each explicit issue slice. For a selected issue it calls
the native executor bridge directly. The bridge resolves the configured Codex, Claude, or
OpenCode harness from the installed runtime-alias table, creates or adopts the exact private
issue worktree and runtime session, and launches the harness with an explicit local-only argument
vector. It never delegates implementation to a shell, `omx`, `/autospec-run`, or a second queue
owner.

Harness output and phase changes are appended to the repository-scoped autonomous log consumed
by `--follow`. Rust independently proves the resulting commit, Closeout report, runtime smoke,
full resolved suite, implementation and security scans, immutable premerge decision, required
CI, and LGTM review before it marks the draft ready or admin-merges. The harness cannot push,
create or edit a pull request, mutate claim state, or merge; those remote actions remain inside
the bridge and are rebound to the exact claim generation and head commit immediately before
mutation.

Bridge state is claim-generation scoped. A pending or active invocation remains non-terminal.
After a conductor restart, exact process identity permits observation of the existing supervisor
without launching a second harness; after process exit, the next run resumes from the last
durable phase. A private local acquisition receipt must match the authoritative repository,
issue, worker, branch, and claim ID before a restarted conductor adopts a live claim. Runtime
cleanup uses the invocation's persisted environment, session, and original manifest snapshot
even if the repository manifest later changes. Schema-1 snapshots from a pre-upgrade active
session reattach against the validated authoritative plan and are conservatively reported as
isolation-bypassed. Before clearing the acquisition receipt, the conductor persists an explicit
terminal- or ownership-retirement boundary so a crash cannot replay completed or lost work.
Transient GitHub reads after implementation retry the same claim generation and durable
invocation; a retryable terminal result preserves
recoverable committed or uncommitted work, advances it onto a changed base without force after
it becomes clean, and starts a fresh claim generation. Only an observed merged pull request,
explicit blocked result, or exhausted retry can become terminal, and no terminal result may
retain `in-progress-by-bot`.

Failure cleanup intent is persisted before runtime teardown. Ownership takeover closes only the
old exact runtime, records the worktree HEAD and status digest under the per-issue lock, and lets
the successor generation adopt the unchanged worktree. A pre-upgrade dispatch without a local
acquisition receipt is migrated only when an exact private invocation or terminal receipt proves
the authoritative claim; otherwise the conductor durably retires the stale ownership.
Terminal claim/label observation outages retain their transient classification and replay the
same completed invocation; an unchanged authoritative claim ref after a failed push does not
retire ownership.

The fixed `.autospec/executor-result.json` ingestion and bare
`executor-result --repo OWNER/REPO --issue N` deferred receipt remain compatibility inputs, but
they are no longer the default producer or a terminal conductor result. Explicit executor-result
ingestion is described below. Direct `run-foreground` remains one-cycle for bounded use.
Detached `autonomous start` and `restart` launch it as a direct Rust child with inherited
lifecycle ownership; that child repeats cycles, emits each completed cycle to its scoped log,
re-checks stop and budget admission before the next cycle, and exits only for a named terminal
condition or `--max-cycles`. Monitor and supervisor remain separate compatibility observers.

Every start-family launch refreshes the requested checkout before lifecycle acquisition. A stale
or missing runtime is rebuilt into an immutable source-digest generation, the build is rejected
if its source identity moves, and the launcher executes the exact verified generation path.
Read-only and stop commands do not require a rebuild.

Autonomous accountability is mandatory: every live launch creates or adopts exactly one verified managed run epic before conductor spawn.
A normal `start` creates its own epic. `start --epic N`
adopts only an active issue in the requested repository with the managed marker, recovery manifest,
and `epic`, `type:tracker`, `no-auto`, and `autospec:run-accountability` labels. `autospec autonomous
resume --epic N` may reopen a verified closed or parked epic and reconstruct a chained local journal
segment from its acknowledged recovery manifest. It records `resumed_from_epic` before work and
never attaches to an arbitrary issue. There is no bypass flag or environment variable for the epic
or private journal.

Autospec's marker-bounded epic projection preserves human text and contains short What, Why, and
Evidence paragraphs, linked issues and pull requests, a Mermaid dependency/deliverable flowchart,
a Mermaid run-state diagram, current work, blockers, verification, and next steps. Events are
durable locally before projection; later GitHub edit failures remain visible and retryable without
creating a replacement epic. Optional GitHub Project assignment may organize the issue but never
replaces it or blocks launch after the epic itself is verified.

`autospec autonomous status --json` and `autospec autonomous list --json` read accountability
health locally. Their accountability object includes `run_id`, `epic_number`, `epic_url`, `event_count`,
`pending_projection_count`, desired and acknowledged high watermarks, projection state, and any
local error. A missing or ambiguous marker, invalid recovery manifest, lost lifecycle lease, or
local journal failure blocks spawn or the next work mutation rather than silently replacing state.

`autospec autonomous main-health` and `run-foreground` read the strict
repository-local Rust health policy at `<repo-dir>/.autospec/autonomous.yml`.
The optional `--branch` override takes precedence over that file and then the
GitHub default branch; exact ignored names become advisory health evidence only.
See [mainline health admission](runbooks/mainline-health-admission.md) and the
[configuration reference](CONFIG_REFERENCE.md#repository-local-rust-mainline-health)
for the supported schema and fail-closed behavior.

`autospec autonomous blast-radius` reads newline-delimited changed paths from
`--changed-files` and matches them against `.autospec/autospec.yml`
`fenced_surfaces` by default, or a caller-supplied `--fenced-surfaces` registry.
Fenced matches emit `blast:fenced`, `decision:"quarantine"`, and non-zero exit
status so policy config changes such as `.autospec/autospec.yml` cannot be
treated as low-blast-radius.

`autospec autonomous drain --repo OWNER/REPO --repo-dir DIR [--stall-secs N] [--poll-secs N]
[--json]` is the Rust Tier-1 watchdog for the fixed direct `omx exec ... $autospec-run` child.
It forwards child output, resets the stall window when output, progress artifacts, scoped
heartbeats, or the timeout-boundary GitHub snapshot advances, and returns the child’s actual exit
status if it completes. External-only progress emits one
`quiet_stdout_external_progress` warning; a genuinely silent, live child emits a
`terminate_stalled` decision and exits `124`. Each decision is atomically recorded as
`drain-observation.json` in the repository’s autonomous-operator scope, without raw output or
lease tokens. The command uses direct `omx`, `gh`, and process arguments—never a shell or a
legacy drain fallback. Existing shell launcher wiring is intentionally a separate #2076 deletion
child; it must redirect to this command rather than reimplement this watchdog.

`autospec autonomous resilience --help` describes the one supported action and its canonical
write slug, `owner__repo`. `resilience decide` reads state, per-issue failure, and spend records
for `owner/repo` in the strict order `owner__repo`, `owner_repo`, then `owner-repo`. The first
existing record is authoritative: malformed records fail closed instead of falling through, and a
state record for another repository returns the `foreign_state` rejection. Read-only admission
does not migrate a compatibility layout. Every acquisition, adoption, or release targets only
`owner__repo`; neither legacy layout is written. The separate
`.autospec/autonomous-operator/<scope>/` directory is lifecycle-only and never stores resilience
compatibility state.

The command prints exactly one JSON decision. `available` and `reclaim` exit `0`; `held` and
capacity parks exit `20`; malformed, foreign, or failure-cap rejections exit `3`. Claimed leases
are reclaimable at the inclusive 300-second boundary and all leases at the inclusive 10,800-second
abandoned boundary (with missing heartbeats and dead same-host PIDs also reclaimable). Capacity is
also inclusive: a nonzero usage limit is evaluated first, then a nonzero issue limit; `0` disables
the corresponding limit. This adapter reads and evaluates state only: it does not invoke
`scripts/autonomous-resilience.sh`, `sh`, or `bash`.

`start` and `restart` finish read-only repository, stored-stop, and lifetime-budget validation
before atomically taking the local non-blocking Unix lease at
`autonomous/owner__repo/conductor.lease.lock`. Only then may they create operator directories,
persist lifecycle/launch metadata, terminate a unit, or clear a stop flag. A fresh held lease
parks with `conductor_lease_held` (`20`), so a held `restart` cannot kill a process or remove a
stop flag. Capacity/failure policy results retain their park/reject JSON and do not create a
claimed lease. The opaque claimed token and monotonic generation fence delayed children: native
launch passes the token only in `AUTOSPEC_CONDUCTOR_LEASE_TOKEN` through `Command::env`, never in
arguments, launch JSON, or logs. A launch failure terminates already-started children and releases
only the matching lease.

Foreground first honors a persisted stop for executable work. When a launcher supplied a token, it
first adopts that token solely to gain matching release authority, then releases it before returning
the persisted-stop or a pre-admission diagnostic. Otherwise it atomically adopts the environment
token (or, when absent, acquires its own) before lifecycle, health, queue, claim, or foreground-state
work. `conductor_lease_token_mismatch` is a reject (`3`) before any local or GitHub mutation.
Matching ownership is rechecked for final selection and dispatch, preserving their admission gates;
terminal foreground work persists its decision before releasing only its exact matching token.
`autonomous status --json` reports the same scoped
`AUTOSPEC_AUTONOMOUS_SPEND_DIR/<owner__repo>/spend.json` ledger used by admission, not the retired
global spend file. Both `autonomous status --json` and `autonomous list --json` also include a
top-level `toolchain` object with `installed_version`, `remote_version`,
`installed_age_secs`, `last_update_failed`, and `last_update_failure_path`. The version and failure
records are read from `~/.autospec/`; missing data is represented explicitly as JSON `null` or
`false`. Both background `autonomous start` and direct foreground entry warn without blocking when
the failure record exists. I/O and transaction failures are diagnostics (`2`) with no decision
JSON, while malformed/foreign records and token fencing are
JSON rejects (`3`). The local lease coordinates only the shared filesystem; GitHub claim ownership
remains the remote mutation arbiter.

`autospec autonomous lifecycle decide` evaluates the typed repository scope, issue, worker,
claim branch, lease freshness, stop, ownership, retry, health, budget, waterfall-tier, and
idle-rescan policy without any side effects. A claim requires its complete typed identity
(`--claim-repo`, `--claim-issue`, `--claim-worker`, and `--claim-branch`); `--claim-state
terminal` and lease ages above 10,800 seconds have distinct non-executable decisions. `--repo`
is required. It emits exactly one JSON decision: `run` exits `0`, `stop` and `park` exit `20`,
claim or scope rejection (including malformed observed claim state) exits `3`, and malformed
flags exit `2`. `start`, `restart`,
`run-foreground`, and `stop` write the same collision-safe atomic schema-1
`.autospec/autonomous-operator/<scope>/lifecycle.json` decision record. Start and restart
launch conductor, monitor, and supervisor as direct Rust executable-plus-argument-vector
children; they do not accept command-string companion overrides or use `sh -c`. Foreground
reads a stop flag or stored stop record before health or queue work and between launched cycles,
returns the same JSON decision and exit class for health parks, and preflights the observed
GitHub claim before any claim label, heartbeat, or run-state mutation.

`autospec autonomous executor-result` emits one JSON result and has two deliberately distinct
forms. The bare compatibility form is exactly `--repo OWNER/REPO --issue N`: it is the successful
legacy deferred receipt above and exits `0`. An explicit result must include a repository, a
positive issue number, `--worker-id`, `--branch`, and `--outcome`. Its fields are strict: unknown
or repeated flags, or mixed outcome fields, are malformed. `succeeded` requires a positive `--pr`
and forbids `--reason`; `blocked` and `retryable` require a nonempty `--reason` and forbid `--pr`.

An explicit successful result is accepted only when its worker ID and branch match a fresh,
nonterminal claim, and its PR remains open, closes the issue, contains exactly one
`## Closeout report` heading, and has that same branch as its head ref. Rust appends an immutable
receipt and re-reads the active claim before accepting it; it never patches the shared claim for an
explicit outcome. That evidence is not release or merge authority. JSON exit codes are
`0` for accepted success or the legacy deferred receipt, `10` for retryable, `20` for blocked or
evidence-unavailable, `2` for malformed input, and `3` for ownership lost.
`result_recording_failed` is also a blocked (`20`) reason: evidence creation or confirmation
failed, so any receipt is unconfirmed/inert, the shared claim is unchanged, and callers must not
treat it as a recorded executor-blocked outcome.
