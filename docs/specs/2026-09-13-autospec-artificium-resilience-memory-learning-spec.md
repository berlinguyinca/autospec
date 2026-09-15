# AutoSpec Resilient Agent Runtime, Durable Work, Shared Memory, and Verified Learning

**Status:** Implementation specification  
**Date:** 2026-09-13  
**Primary owner:** `berlinguyinca/autospec`  
**Related repositories:** every `berlinguyinca/autospec-*` repository discovered at execution time  
**Inspiration:** selected architectural ideas from `officialgr/agent-artificium`, independently implemented to fit AutoSpec's existing multi-repository architecture  
**Execution assumption:** the implementation agent is launched from the parent/main workspace directory in which AutoSpec repositories should live as sibling directories.

---

## 1. Purpose

Implement five related capabilities that make long-running AutoSpec engineering work durable, resumable, context-efficient, observable, and progressively better over time:

1. **Context Guardian + Structured Continuation Checkpoints**
2. **Dynamic Memory Map over the existing shared-memory/Black Hole integration**
3. **Repository Attention Streams for resumable large-corpus analysis**
4. **Durable Agent Work Protocol with receipts, attempts, leases, recovery, and idempotence**
5. **Verified Engineering Learning that promotes evidence-backed lessons into shared memory**

These capabilities must extend the existing AutoSpec control, dispatch, execution, telemetry, and UI planes rather than create a competing agent runtime.

The implementation must preserve the architectural separation already established in the AutoSpec repositories:

- `autospec`: control plane; intent, specs, issues, project policy, review policy, workflow contracts.
- `autospec-dispatcher`: dispatch/observe/judge/act plane; selects work and interprets outcomes without allowing model judgement to override deterministic gates.
- `autospec-orchestrator`: execution plane; workers, worktrees, runtime isolation, Pi harness sessions, resource ownership, recovery, artifacts, cleanup.
- `autospec-db`: optional, non-blocking, lossy telemetry and Grafana data; **never the sole source of truth for correctness-critical state**.
- `autospec-gui`: read-oriented operational UI over telemetry/data projections.
- Other `autospec-*` repositories: inspect and integrate only according to their actual ownership and contracts.

The implementation must use the existing Black Hole/shared-memory integration if present in the checked-out source. Do not create a parallel memory subsystem merely because its implementation is located in a different repository or is newer than the public default branch.

---

# 2. Mandatory Workspace Bootstrap

## 2.1 Run from the parent/main workspace directory

The implementation prompt is intended to be executed from the directory that should contain the repositories as siblings:

```text
$PWD/
  autospec/
  autospec-orchestrator/
  autospec-dispatcher/
  autospec-db/
  autospec-gui/
  ...
```

Treat the initial `$PWD` as `AUTOSPEC_WORKSPACE_ROOT`.

Do **not** assume that a manually maintained repository list is complete.

## 2.2 Discover every AutoSpec repository dynamically

The implementation agent MUST use authenticated GitHub CLI discovery before planning changes.

The discovered set MUST contain:

- `berlinguyinca/autospec`
- every repository owned by `berlinguyinca` whose name starts with `autospec-`

Example discovery:

```bash
set -euo pipefail

AUTOSPEC_WORKSPACE_ROOT="${AUTOSPEC_WORKSPACE_ROOT:-$PWD}"
cd "$AUTOSPEC_WORKSPACE_ROOT"

gh auth status

gh repo list berlinguyinca \
  --limit 1000 \
  --json name,nameWithOwner,url,isArchived,isFork \
  --jq '.[] |
        select(.name == "autospec" or (.name | startswith("autospec-"))) |
        [.nameWithOwner, .url, (.isArchived|tostring), (.isFork|tostring)] |
        @tsv' \
  | sort
```

The runtime GitHub result is authoritative. A static repository list in this document is not.

## 2.3 Clone all matching repositories; preserve local work

Every discovered repository MUST be available locally before the cross-repository architecture survey begins.

For each repository:

1. If the directory does not exist, clone it with `gh repo clone`.
2. If it exists and is a Git repository, verify its `origin`, inspect status, and fetch all refs/tags with pruning.
3. Never `reset --hard`, discard changes, clean untracked files, or overwrite local work.
4. If the repository is clean and currently on its remote default branch, `pull --ff-only`.
5. If it is dirty or on another branch, leave the checkout intact after `fetch`; record that fact in the implementation report.
6. If the path exists but is not the expected Git repository, fail closed and report the collision.

Reference bootstrap:

```bash
set -euo pipefail

AUTOSPEC_WORKSPACE_ROOT="${AUTOSPEC_WORKSPACE_ROOT:-$PWD}"
cd "$AUTOSPEC_WORKSPACE_ROOT"

mapfile -t AUTOSPEC_REPOS < <(
  gh repo list berlinguyinca \
    --limit 1000 \
    --json name,nameWithOwner \
    --jq '.[] |
          select(.name == "autospec" or (.name | startswith("autospec-"))) |
          .nameWithOwner' \
    | sort
)

printf 'Discovered %s AutoSpec repositories\n' "${#AUTOSPEC_REPOS[@]}"

for repo in "${AUTOSPEC_REPOS[@]}"; do
  dir="${repo#*/}"

  if [[ -e "$dir" && ! -d "$dir/.git" ]]; then
    echo "ERROR: $dir exists but is not a Git repository" >&2
    exit 1
  fi

  if [[ ! -d "$dir/.git" ]]; then
    gh repo clone "$repo" "$dir"
  else
    actual_origin="$(git -C "$dir" remote get-url origin || true)"
    echo "$repo -> $dir origin=$actual_origin"
    git -C "$dir" fetch --all --prune --tags
  fi

  default_branch="$(gh repo view "$repo" --json defaultBranchRef --jq '.defaultBranchRef.name')"
  current_branch="$(git -C "$dir" branch --show-current)"
  dirty="$(git -C "$dir" status --porcelain)"

  if [[ -z "$dirty" && "$current_branch" == "$default_branch" ]]; then
    git -C "$dir" pull --ff-only
  else
    echo "Preserving checkout for $repo: branch=$current_branch dirty=$([[ -n "$dirty" ]] && echo yes || echo no)"
  fi
done
```

If `mapfile` is unavailable on the host shell, implement an equivalent portable loop; do not omit dynamic discovery.

## 2.4 Classify repositories before modifying them

Produce a machine-readable and human-readable workspace inventory containing at least:

```yaml
repository:
default_branch:
current_branch:
dirty:
archived:
role:
implementation_target:
reason:
build_system:
primary_languages:
relevant_specs:
relevant_tests:
relevant_memory_integration:
```

At minimum inspect:

- README
- `docs/`
- `docs/specs/`
- ADRs
- package/workspace manifests
- database migrations
- CLI entry points
- harness/runtime abstractions
- existing event/state schemas
- tests
- any `blackhole`, `black-hole`, `memory`, `openviking`, `context`, `checkpoint`, `resume`, `lease`, `receipt`, `attempt`, or `learning` implementation

Repositories named like `autospec-e2e-*` MUST still be cloned because the user explicitly requested all `autospec-*` data. However, treat generated/empty E2E repositories as fixtures/evidence unless inspection shows they are durable source repositories. Do not create production implementations in ephemeral test repositories.

---

# 3. Current Architectural Placement

Do not collapse the current multi-plane architecture.

## 3.1 Control plane — `autospec`

Owns the user-visible product/workflow contracts:

- context-checkpoint policy and schema
- memory-map policy and retrieval contract
- attention-stream user-facing commands/contracts
- engineering-learning policy
- spec/issue/PR linkage
- deterministic validation/review requirements
- global role policy
- backwards-compatible CLI/workflow surfaces

`autospec` should define what these capabilities mean and when they are required.

## 3.2 Dispatch plane — `autospec-dispatcher`

Owns:

- attempt selection/reselection
- capacity-aware dispatch decisions
- deterministic observation of outcomes
- model judgement of ambiguous outcomes
- retry/escalation decisions
- collision/idempotence decisions
- transition requests that follow validated judgement
- determining whether a lesson candidate has enough evidence to proceed to validation

Critical invariant:

> Deterministic checks determine truth about gates and execution facts. Model judgement interprets evidence and recommends action. A model cannot convert a deterministic failure into a passing gate.

## 3.3 Execution plane — `autospec-orchestrator`

Owns:

- harness session lifecycle
- Pi integration
- context-utilization observation where available
- required checkpoint handshake before context exhaustion
- execution attempts
- claims/leases/heartbeats
- crash recovery
- durable execution receipts
- worktree/runtime ownership
- artifacts
- restart/resume from checkpoint

A correctness-critical checkpoint or work receipt cannot exist only in telemetry.

## 3.4 Telemetry plane — `autospec-db`

Owns projections/telemetry for:

- checkpoint requested/started/completed
- context utilization
- resume/recovery count
- attempt/lease lifecycle
- receipt latency
- attention-stream progress
- memory-map retrieval counts/latency
- lesson candidate/validation/promotion metrics
- failure/retry/unknown counts

`autospec-db` is optional and lossy by design. Every feature MUST continue to function correctly with telemetry disabled or unavailable.

## 3.5 UI — `autospec-gui`

Extend the existing read-only dashboard model to expose useful projections where data exists:

- active work and attempts
- last heartbeat and lease state
- current context utilization
- checkpoint history
- resumes/recoveries
- attention-stream progress
- memory retrieval summary/provenance
- lesson candidate state
- blocked/failed/unknown work

Do not turn the GUI into a second source of truth.

---

# 4. Feature A — Context Guardian

## 4.1 Goal

Long-running Pi/agent sessions must proactively preserve durable continuation state before useful context is lost to truncation or opaque compaction.

The system must support a clean transition:

```text
active agent session
    -> context threshold reached
    -> checkpoint requested
    -> structured checkpoint validated
    -> durable checkpoint persisted
    -> execution/session may compact or restart
    -> new session hydrates minimal continuation context
    -> work resumes from explicit next actions
```

## 4.2 Configurable thresholds

Provide configuration with sensible defaults:

```yaml
context_guardian:
  enabled: true
  soft_checkpoint_percent: 60
  checkpoint_warning_percent: 75
  required_checkpoint_percent: 85
  max_checkpoint_tokens: 6000
  resume_memory_map_tokens: 3000
```

Exact names may adapt to existing configuration conventions.

Semantics:

- **soft threshold:** checkpoint may be generated opportunistically when a coherent milestone is reached.
- **warning threshold:** checkpoint should be generated at the next safe boundary.
- **required threshold:** no new substantial implementation phase should begin until a valid durable checkpoint exists.

Do not deadlock a session solely because the provider cannot report an exact context count. Implement a capability hierarchy:

1. provider/harness exact context usage if available
2. model metadata + tokenizer/estimator
3. conservative local estimate
4. explicit `unknown` state with milestone-based checkpointing

Expose provenance for the estimate.

## 4.3 Structured checkpoint schema

Create a versioned schema, e.g. `autospec.context-checkpoint.v1`.

Required conceptual fields:

```yaml
schema:
checkpoint_id:
created_at:
execution_id:
attempt_id:
session_id:
repository:
worktree:
branch:
issue:
pull_request:
spec_refs:

objective:
acceptance_criteria:

completed:
in_progress:
next_actions:

changed_files:
important_code_refs:
evidence_refs:
decisions:
assumptions:

validation:
  passed:
  failed:
  not_run:
  commands:

review:
  findings:
  unresolved:

known_failures:
blockers:
unresolved_questions:

memory_refs:
attention_stream_refs:
artifact_refs:

context:
  model:
  provider:
  window_tokens:
  estimated_used_tokens:
  utilization_percent:
  estimate_source:

resume:
  recommended_entrypoint:
  required_files:
  required_memory_queries:
  required_commands:
```

Checkpoint content must be concise and continuation-oriented, not a full transcript dump.

## 4.4 Checkpoint validation

Before acknowledging a checkpoint as durable:

- schema validates
- identifiers match the active execution/attempt
- worktree/branch identity is consistent
- referenced artifacts that are required for resume exist
- secret-like values are rejected/redacted
- checkpoint is atomically persisted
- duplicate submission is idempotent
- persistence acknowledgement is returned to the harness

## 4.5 Resume behavior

A resumed session should receive:

1. role/policy definition
2. objective + acceptance criteria
3. latest valid checkpoint
4. compact dynamic memory map
5. exact artifacts/evidence explicitly referenced by the checkpoint
6. current Git/worktree state verification
7. changed external state since the checkpoint, if any

Do not blindly replay the old conversation.

If repository state diverged from the checkpoint, mark the resume as needing reconciliation rather than pretending the checkpoint is current.

## 4.6 Crash behavior

A process crash before checkpoint acknowledgement must not falsely mark the checkpoint complete.

Recovery must distinguish:

- checkpoint requested
- checkpoint generated
- checkpoint durably persisted
- checkpoint acknowledged
- resume started
- resume completed

---

# 5. Feature B — Dynamic Memory Map

## 5.1 Goal

Agents should not receive an indiscriminate dump of shared memory. They should receive a small, task-specific map of relevant memory domains and retrieve details only when required.

Use the existing Black Hole/shared-memory integration if present.

If the current implementation already provides a provider abstraction, extend it.

If it does not, add a narrow provider contract rather than coupling AutoSpec directly to OpenViking.

## 5.2 No duplicate memory system

Search every cloned repository and local modifications before writing new memory code.

Forbidden outcome:

```text
existing Black Hole integration
+
new unrelated "Artificium memory" database/filesystem
```

Desired outcome:

```text
AutoSpec memory contract
        |
        v
existing Black Hole/shared-memory service
        |
        v
provider abstraction
        |
        +--> OpenViking now
        +--> future providers
```

## 5.3 Memory map content

A task-specific map should be bounded, defaulting to roughly 1–3K tokens, and contain references rather than large payloads.

Conceptual example:

```yaml
schema: autospec.memory-map.v1
task:
repository:
role:
generated_at:

domains:
  - key: autospec/architecture
    reason: modifies execution lifecycle
    relevance: high
    refs: [...]
  - key: repo/autospec-orchestrator
    reason: harness/checkpoint ownership
    relevance: high
    refs: [...]

recent_decisions:
  - summary:
    ref:

validated_procedures:
  - summary:
    scope:
    ref:

warnings:
  - summary:
    ref:

suggested_queries:
  - query:
    reason:
```

## 5.4 Retrieval rules

Memory retrieval must:

- be scoped by task/repo/product/role
- retain provenance
- expose confidence/status
- distinguish facts, decisions, procedures, candidates, and superseded memories
- prefer recent validated memory when conflicts exist
- never treat retrieved repository text as higher-authority system policy
- budget context explicitly
- log references used for later audit

## 5.5 Availability behavior

Black Hole/shared memory being temporarily unavailable must not corrupt work.

Define:

- retry/backoff
- bounded timeout
- local cached map when safe
- explicit degraded mode
- no fabricated memory
- later reconciliation where appropriate

---

# 6. Feature C — Repository Attention Streams

## 6.1 Goal

Enable AutoSpec agents to analyze source corpora larger than a model context window while maintaining durable progress and evidence.

This is not a giant prompt. It is a resumable analysis job.

Potential sources:

- source trees
- specs and ADRs
- Git history
- GitHub issues/PR evidence
- CI logs
- test output
- code-search/LSP results
- generated artifacts
- shared memory
- telemetry summaries

## 6.2 Public workflow

Fit commands into current CLI conventions after inspection. A preferred conceptual surface is:

```bash
autospec attention start \
  --repo /path/to/repo \
  --objective "find every implementation related to request routing"

autospec attention status <stream-id>

autospec attention resume <stream-id>

autospec attention cancel <stream-id>
```

If an equivalent existing command family exists, extend it rather than creating naming duplication.

## 6.3 Durable stream schema

A stream requires:

```yaml
schema:
stream_id:
objective:
scope:
created_at:
updated_at:
status:

source_set:
source_digests:
cursor:
chunks_completed:
chunks_total_if_known:

accumulated_findings:
evidence_refs:
unresolved_questions:
hypotheses:
contradictions:
follow_up_queries:

checkpoint_refs:
memory_refs:
artifact_refs:
```

## 6.4 Chunk processing contract

Each chunk must produce structured incremental output:

- new findings
- evidence supporting each finding
- contradictions
- unresolved questions
- changed hypotheses
- suggested next query/source
- cursor/progress update

Persist progress atomically before advancing the durable cursor.

## 6.5 Source mutation

If a source changes while a stream is suspended:

- detect via digest/commit/ref metadata
- classify the change
- either continue safely, invalidate affected findings, or require reconciliation
- never silently resume against materially different source content

## 6.6 Integration

Attention streams should be usable by:

- planner
- architecture reviewer
- implementation agent
- test architect
- documentation agent
- UI/UX reviewer where source/screenshots are in scope
- security reviewer

A checkpoint may reference active/completed attention streams instead of reproducing their corpus.

---

# 7. Feature D — Durable Agent Work Protocol

## 7.1 Goal

AutoSpec must know the difference between:

- work existing
- work being assigned
- a worker receiving it
- a worker claiming it
- an attempt actually running
- a checkpoint being durable
- implementation completion
- deterministic validation completion
- independent review completion
- merge completion

Do not infer these facts from one process exit code.

## 7.2 Canonical state model

Inspect current state enums first and evolve/migrate them without unnecessary duplication.

The conceptual lifecycle must cover:

```text
CREATED
  -> ASSIGNED
  -> DELIVERED
  -> CLAIMED
  -> RUNNING
  -> CHECKPOINTED (repeatable milestone)
  -> COMPLETED
  -> VALIDATED
  -> REVIEWED
  -> MERGED
```

Non-happy states:

```text
BLOCKED
FAILED
ABANDONED
RETRY_PENDING
NEEDS_HUMAN
SUPERSEDED
CANCELLED
UNKNOWN
```

A `CHECKPOINTED` event does not necessarily replace `RUNNING`; represent it as an event/milestone if that better fits existing state models.

## 7.3 Separate work, attempt, claim, and session identities

Do not overload one ID.

Minimum concepts:

- `work_id`: durable unit of requested engineering work
- `attempt_id`: one execution attempt
- `claim_id`: ownership lease for an attempt/work item
- `session_id`: harness/model session
- `checkpoint_id`
- `receipt_id`
- `idempotency_key`

Retries create new attempts while preserving work identity.

## 7.4 Receipts

Support durable acknowledgement stages where applicable:

```text
sent
delivered
claimed/read
handled
```

A receipt must be attributable to the correct attempt and consumer.

Duplicate delivery must be safe.

A process returning `0` without required output/session evidence cannot be interpreted as successful implementation merely because the process exited cleanly.

## 7.5 Leases and heartbeats

Claims must be leases, not permanent flags.

Requirements:

- lease acquisition is atomic
- lease has expiry
- worker heartbeat renews lease
- crashed worker lease can be reclaimed
- stale worker cannot later finalize an attempt after ownership has moved
- lease fencing token/generation prevents split-brain completion
- retry policy is explicit
- cancellation is durable and observable

## 7.6 Recovery

On dispatcher/orchestrator restart:

- reconcile nonterminal attempts
- inspect lease expiry/fencing generation
- inspect required artifacts/checkpoints
- resume, retry, block, or escalate deterministically
- do not double-dispatch completed work
- do not lose a valid durable checkpoint
- record recovery action as an event

## 7.7 Ownership

Prefer:

- control/work definition in `autospec`
- dispatch transitions and judgement in `autospec-dispatcher`
- attempt/session/lease mechanics in `autospec-orchestrator`
- telemetry mirror in `autospec-db`
- read projection in `autospec-gui`

Use actual existing contracts discovered in code to refine this split.

---

# 8. Feature E — Verified Engineering Learning

## 8.1 Goal

Convert engineering outcomes into reusable knowledge without allowing unverified agent guesses to poison shared memory.

Learning loop:

```text
implementation attempt
    -> deterministic validation
    -> independent review
    -> outcome evidence
    -> lesson candidate
    -> critic/validator
    -> scope + confidence assignment
    -> durable promotion to shared memory
    -> later retrieval through memory map
```

## 8.2 Lesson candidate schema

Conceptual `autospec.lesson-candidate.v1`:

```yaml
candidate_id:
created_at:
source_work_id:
source_attempt_id:
repository:
commit:
issue:
pull_request:

kind:
  # procedure | warning | architecture | debugging | test | tooling | failure-pattern

statement:
scope:
preconditions:
anti_conditions:

evidence:
validation_results:
review_results:
counterevidence:

confidence:
status:
  # candidate | validated | rejected | promoted | superseded

supersedes:
expires_at:
memory_target:
```

## 8.3 Promotion requirements

Do not automatically convert every successful run into authoritative knowledge.

A promoted lesson must have:

- traceable source work
- deterministic validation evidence appropriate to the claim
- independent review/critic evidence
- scoped applicability
- no unresolved contradiction that invalidates the claim
- explicit confidence
- provenance
- version/schema identity

Low-confidence but potentially useful observations may remain candidates and be retrievable only when explicitly requested.

## 8.4 Failure learning

Failures are useful.

Examples of valid lessons:

- a specific build/test procedure required by a repository
- a recurring integration mistake
- a pre-existing CI baseline failure
- a toolchain mismatch pattern
- a migration ordering rule
- a known dangerous refactor pattern
- a repository-specific review checklist

Do not promote transient secrets, generated IDs, incidental paths, or one-off noise.

## 8.5 Role policy is not learned memory

Agent role definitions, safety constraints, merge policy, review independence, and gate authority are version-controlled policy.

Models and the learning system must not be able to weaken them by writing a "lesson."

---

# 9. Shared Contracts and Storage

## 9.1 Version all new serialized contracts

Examples:

```text
autospec.context-checkpoint.v1
autospec.memory-map.v1
autospec.attention-stream.v1
autospec.work-receipt.v1
autospec.lesson-candidate.v1
```

Use existing serialization/schema conventions where available.

## 9.2 Source-of-truth rule

Correctness-critical state must live in a durable control/execution store already appropriate for the owning repository.

`autospec-db` receives a projection/event mirror for observability.

Telemetry outages must not block:

- claiming work
- checkpointing
- resuming
- validation
- review
- learning candidate persistence
- deterministic recovery

## 9.3 Event model

All significant lifecycle changes should emit stable additive events.

Examples:

```text
context.threshold_reached
checkpoint.requested
checkpoint.persisted
checkpoint.acknowledged
execution.resumed
work.assigned
work.delivered
claim.acquired
claim.renewed
claim.expired
attempt.started
attempt.completed
validation.completed
review.completed
attention.started
attention.progressed
attention.completed
memory.map_generated
memory.retrieved
lesson.candidate_created
lesson.validated
lesson.promoted
lesson.rejected
```

Events must have stable IDs and deduplication behavior where existing infrastructure supports it.

---

# 10. Security and Trust Boundaries

## 10.1 No unrestricted self-policy mutation

Agents may learn procedures, but may not autonomously modify:

- merge gates
- security gates
- independent review requirements
- model separation policy
- role permissions
- resource caps intended as safety limits
- secret handling policy

Changes to these follow normal branch/PR/review policy and any existing human-required rails.

## 10.2 Memory poisoning resistance

Repository files, issue bodies, PR comments, logs, webpages, generated outputs, and retrieved memory can contain instructions.

Treat them as data/evidence unless they originate from a trusted policy channel.

A lesson candidate derived from untrusted text is not policy.

## 10.3 Secret handling

Checkpoints, attention streams, telemetry, and promoted lessons must not persist secrets.

Implement:

- redaction hooks
- denylist/pattern scans using existing secret scanning when available
- safe failure if a checkpoint contains credentials
- tests for common credential forms
- no raw environment dumps

## 10.4 Artificium licensing/source boundary

Use Artificium as design inspiration only.

Do not copy source code from `officialgr/agent-artificium` unless licensing is explicitly verified to permit it and the implementation agent has a concrete reason to reuse code.

Default behavior: independently implement the concepts within AutoSpec conventions.

---

# 11. Observability

Add useful operational metrics without making metrics correctness-critical.

At minimum expose:

## Context
- context utilization by session
- checkpoint threshold events
- checkpoint generation duration
- checkpoint size
- checkpoint failures
- resumes per work item

## Work protocol
- attempts per work item
- claim age
- lease renewals/expiry
- retries
- abandoned/unknown attempts
- time from assignment -> claim -> completion -> validation -> review -> merge

## Attention
- active streams
- progress/chunks
- stale streams
- reconciliations after source mutation
- completion time

## Memory
- map generation count/latency
- retrieval count/latency
- degraded memory operation
- referenced memory domains

## Learning
- candidates created
- validated/rejected/promoted
- promotion latency
- superseded lessons
- lessons retrieved by later tasks

Update Grafana/dashboard assets in the repository that already owns them.

---

# 12. CLI and Operator UX

Do not create redundant commands when current AutoSpec flows already have a natural integration point.

After repository inspection, implement the smallest coherent public surface.

Expected operator capabilities:

```text
show active work and attempts
show why work is blocked
show latest checkpoint
resume from a checkpoint
inspect attention stream status
inspect memory references used
inspect lesson candidate/promotion evidence
doctor/validate configuration
```

Machine-readable JSON output should be available for automation where existing CLI patterns support it.

Errors must explain:

- what failed
- what state was preserved
- whether retry is safe
- which ID should be used to resume/reconcile

---

# 13. Migration and Backward Compatibility

1. Existing AutoSpec workflows must continue when new features are disabled.
2. Existing agents without exact context reporting must still run with degraded checkpoint estimation.
3. Telemetry remains optional.
4. Memory service outages degrade explicitly rather than fabricating data.
5. Existing work state must migrate safely.
6. Existing event schemas remain additive/backward compatible where promised.
7. New database migrations must be reversible where current project conventions require it or explicitly documented if not.
8. Do not break existing Pi harness contracts unnecessarily.
9. Keep old checkpoint/state readers working where practical; otherwise provide migration tooling and a clear version error.
10. No destructive migration without explicit evidence and review.

---

# 14. Required Tests

The implementation is not complete until the relevant repositories have automated proof for the following.

## 14.1 Context Guardian
- soft/warning/required thresholds
- exact usage source
- estimated usage source
- unknown usage source
- required checkpoint blocks unsafe continuation
- checkpoint validation failure
- idempotent duplicate checkpoint
- crash before persistence acknowledgement
- successful resume
- repository divergence on resume
- secret rejection/redaction
- bounded checkpoint size

## 14.2 Work protocol
- duplicate delivery
- duplicate receipt
- atomic lease acquisition
- heartbeat renewal
- lease expiry
- fencing old worker after reassignment
- crash + recovery
- retry creates new attempt
- terminal attempt not double-run
- cancellation
- zero-byte/no-session output is not successful work
- unknown judgement remains `UNKNOWN`

## 14.3 Memory map
- token budget
- scope filtering
- provenance
- conflicting memory preference
- unavailable provider/degraded mode
- no fabricated memory
- role/policy is not overridden by memory

## 14.4 Attention streams
- multi-chunk source
- restart/resume
- atomic cursor advancement
- source changes between runs
- evidence references remain valid
- cancellation
- large repository fixture

## 14.5 Learning
- candidate from successful work
- candidate from failure
- rejected candidate
- validated promotion
- supersession
- contradictory evidence
- unsafe/policy mutation rejected
- secret-containing lesson rejected
- later task retrieves promoted lesson

## 14.6 Cross-plane
- telemetry completely disabled
- telemetry database unavailable
- Black Hole unavailable
- dispatcher restart
- orchestrator restart
- concurrent workers
- end-to-end Pi execution crossing a checkpoint boundary and resuming

---

# 15. Implementation Phases

The coding agent should execute these phases continuously through completion; do not stop after producing a plan.

## Phase 0 — Inventory and architecture reconciliation

- clone/fetch all AutoSpec repositories
- produce workspace inventory
- detect local uncommitted work
- inspect current docs/specs/ADRs
- discover current Black Hole/shared-memory code
- discover existing context/compaction/checkpoint support
- discover work/attempt/lease/event schemas
- identify exact repository ownership
- write/update implementation issue tree if current AutoSpec conventions require it

Deliverable: concise architecture reconciliation showing where each feature will land and what existing code it extends.

## Phase 1 — Shared contracts

Implement versioned domain contracts and compatibility tests first.

Avoid premature service duplication.

## Phase 2 — Durable work protocol foundation

Implement attempt/claim/lease/receipt/recovery semantics necessary for reliable checkpoint ownership.

This comes before depending on checkpoints for autonomous recovery.

## Phase 3 — Context Guardian

Implement harness observation, threshold policy, checkpoint lifecycle, persistence, validation, and resume.

Prove with Pi.

## Phase 4 — Dynamic Memory Map

Integrate with the existing Black Hole/shared-memory provider path.

Implement bounded map generation and explicit retrieval.

## Phase 5 — Repository Attention Streams

Implement durable resumable corpus analysis using the new checkpoint/work primitives where appropriate.

## Phase 6 — Verified Engineering Learning

Implement candidate generation, validation, promotion, rejection, supersession, and memory integration.

## Phase 7 — Telemetry and UI

Add additive telemetry/events/views, Grafana assets, and read projections.

Do not allow this phase to alter correctness behavior.

## Phase 8 — End-to-end hardening

Run cross-repo integration tests, crash tests, concurrency tests, docs validation, lint, typecheck, and builds.

Exercise a real representative AutoSpec task through:

```text
dispatch
-> execution
-> context threshold
-> checkpoint
-> resume
-> implementation completion
-> validation
-> independent review
-> lesson candidate
-> promotion or rejection
```

---

# 16. Acceptance Criteria

The feature is complete only when all applicable criteria are satisfied.

- [ ] Every current `berlinguyinca/autospec-*` repo plus `berlinguyinca/autospec` was dynamically discovered and cloned/fetched before architecture changes.
- [ ] Existing local work was preserved.
- [ ] Generated `autospec-e2e-*` repos were classified and not accidentally used as production implementation repos.
- [ ] Existing shared-memory/Black Hole implementation was reused/extended rather than duplicated.
- [ ] A long-running Pi execution can checkpoint before context exhaustion.
- [ ] A new Pi session can resume from the latest validated checkpoint without replaying the full prior conversation.
- [ ] Checkpoint correctness does not depend on `autospec-db`.
- [ ] Work/attempt/claim/session IDs are not conflated.
- [ ] Lease fencing prevents stale workers from completing reassigned work.
- [ ] Duplicate delivery/receipts are idempotent.
- [ ] Crash/restart recovery is deterministic and tested.
- [ ] AutoSpec can build a bounded task-specific memory map with provenance.
- [ ] A large repository/source analysis can stop and resume from a durable attention cursor.
- [ ] Source mutation during an attention stream is detected.
- [ ] Successful and failed engineering outcomes can create lesson candidates.
- [ ] Unvalidated lesson candidates cannot silently become authoritative shared memory.
- [ ] Promoted lessons include evidence, scope, confidence, and provenance.
- [ ] Role/safety/merge policy cannot be weakened through learned memory.
- [ ] All new event/state contracts are versioned.
- [ ] Telemetry-off and memory-degraded modes are tested.
- [ ] Relevant Grafana/UI projections expose the new lifecycle.
- [ ] Documentation and operator runbooks are updated.
- [ ] Tests, builds, lint/clippy/typecheck, schema validation, and repository-specific validation commands pass.
- [ ] Any pre-existing failures are identified separately from regressions introduced by this work.
- [ ] Implementation uses branches/PRs according to existing AutoSpec policy; no direct unsafe writes to protected/default branches.
- [ ] Final report contains changed repositories, branches/PRs, migrations, test commands/results, unresolved risks, and exact follow-up work if anything remains.

---

# 17. Required Engineering Behavior During Implementation

The coding agent executing this spec MUST:

1. **Show its work.** Stream meaningful command output and progress; do not run silently.
2. **Continue through implementation.** Do not stop after analysis, planning, or issue creation.
3. **Use existing AutoSpec workflows where they are already the canonical mechanism.**
4. **Inspect before inventing.** Reuse current schemas, events, traits, services, Black Hole integration, and CLI conventions.
5. **Keep changes reviewable.** Prefer coherent cross-repo slices and small commits/PRs over one giant unreviewable patch.
6. **Run tests continuously.** Do not defer all validation to the end.
7. **Separate pre-existing failures from regressions.**
8. **Never discard local changes.**
9. **Never weaken deterministic gates because a model thinks the failure is acceptable.**
10. **Keep independent planning/implementation/review separation according to current AutoSpec policy.**
11. **Use Pi as the primary harness wherever the orchestrator currently defines Pi as primary.**
12. **Do not replace InferWeave model-routing responsibilities with AutoSpec execution logic.**
13. **Do not make optional telemetry a required runtime dependency.**
14. **Do not create a second memory backend beside the shared-memory provider architecture.**
15. **Document architectural decisions that materially change cross-repository ownership.**

---

# 18. Initial Repository Snapshot

At spec-authoring time, GitHub discovery showed durable source repositories including:

```text
berlinguyinca/autospec
berlinguyinca/autospec-baselines
berlinguyinca/autospec-constitution
berlinguyinca/autospec-design
berlinguyinca/autospec-gui
berlinguyinca/autospec-orchestrator
berlinguyinca/autospec-db
berlinguyinca/autospec-dispatcher
berlinguyinca/autospec-ui-pilot
```

It also showed generated `autospec-e2e-*` repositories.

This list is intentionally **not authoritative**. The execution-time dynamic GitHub discovery in Section 2 is mandatory so newly created repositories are not missed.

---

# 19. Suggested Final Implementation Report

At completion, print and save a report containing:

```markdown
# AutoSpec Resilience/Memory/Learning Implementation Report

## Workspace
- root:
- repositories discovered:
- repositories modified:
- repositories reference-only:
- dirty checkouts preserved:

## Architecture
- control-plane changes:
- dispatch-plane changes:
- execution-plane changes:
- telemetry changes:
- UI changes:
- memory-provider changes:

## Features
### Context Guardian
status:
proof:

### Dynamic Memory Map
status:
proof:

### Repository Attention Streams
status:
proof:

### Durable Agent Work Protocol
status:
proof:

### Verified Engineering Learning
status:
proof:

## Database/schema migrations
...

## Tests and validation
command:
result:

## End-to-end scenario
...

## Pull requests / branches
...

## Pre-existing failures
...

## Remaining risks
...

## Follow-up issues
...
```

The final implementation report must be specific enough that another agent can resume any unfinished item without rediscovering the architecture from scratch.

---

# 20. Definition of Done

"Implemented" means running code, migrations where needed, tests, documentation, and end-to-end proof across the actual AutoSpec repository set.

A design document, issue tree, or partial scaffold alone is **not** completion.
