# AGENTS.md

## Engineering standards

- **Conventional commits** (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- **Branch-per-issue**: `feat/<slug>`. Never push to `main`.
- **Never bypass hooks** (`--no-verify`) or signing flags.
- **Never amend** committed PRs; create a new commit instead.
- **Lock-step rule** (per `CONTRIBUTING.md`): every multi-harness skill keeps `SKILL.md` / `opencode/agent.md` / `codex/prompt.md` bodies identical; only frontmatters differ.
- **Validation and tests**: run the Rust test suite with `cargo test --workspace --no-fail-fast`. Without `--no-fail-fast` cargo stops at the first failing test binary, so one failure hides every later binary -- that masked six failures on `main`. Also run the shell validation scripts that check lock-step diffs, frontmatter parsing, `bash -n` on install scripts, and file presence. Each PR adds or extends a validation script that passes after the change.
- **Build gate compiles all targets** (#3702): a gate whose green exit code is read as "this code compiles" must compile everything the patch touched, i.e. `cargo build --workspace --all-targets` (or an equivalent that compiles test targets, such as `cargo test --no-run`). Plain `cargo build` skips test targets, so its `rc=0` is about a different program than the one under review. A patch's own test file failing to compile is a hard failure, distinct from "tests ran and some failed" -- both surface as `test_rc=101`. Record the exact gate command with the result it produced.

## Runtime resource isolation

- Use `autospec runtime env up|status|down|exec|session|gc|normalize-compose` for every
  manifest-v2 stack; use `down --purge-maven` only for the guarded Maven 4 prefix.
- `AUTOSPEC_MAVEN_ISOLATION=off`, `AUTOSPEC_COMPOSE_ISOLATION=off`, and
  `AUTOSPEC_ENV_DISABLE=1` export `AUTOSPEC_ISOLATION_BYPASSED=1`; never call the resulting
  isolation evidence verified.
- Runtime state is private on Unix (`0700` directories and `0600` files). Treat
  `RUNTIME_STATE_SYMLINK_REJECTED` and ownership ambiguity as fail-closed recovery signals;
  never delete an environment or session root manually.

## Child-process content channel and flake triage (issue #4150)

- **Content goes on stdin, never in argv.** An argv entry is a NUL-terminated C
  string: a value containing a NUL byte (every ELF binary does) is rejected with
  `InvalidInput: nul byte found in provided data` before the child is spawned,
  and a value past `ARG_MAX` fails with `E2BIG` on some inputs and not others,
  which presents as a size-dependent flake. The mechanical rule: if a value's
  length is determined by data rather than by configuration, it is not an
  argument. argv is for parameters — paths, flags, modes. Where an
  out-of-process write exists for a reason (e.g. the #3495 ETXTBSY fix requires
  the child, not the parent, to hold the write descriptor on the published
  inode), stdin preserves it: the child opens the target, the parent only
  writes to a pipe.
- **Prove isolation before blaming load.** A suite with many sub-100 ms
  deadlines on loaded hosts makes "probably flaky, probably load" the cheap and
  usually-correct explanation — which is how a deterministic failure sat
  unexamined. Before attributing a failure to load or environment, run it
  alone, single-threaded, three times, and record the outcome in the issue:
  fails 3/3 = deterministic defect, fix it; fails 1–2/3 = genuinely
  nondeterministic race or deadline; passes 3/3 = environmental, reproduce
  under the original conditions before touching code. A failure at 0.00 s
  elapsed cannot be a timing failure — there was no time in which to race.
- **Check that the harness actually ran.** `cargo test --test <name>` against a
  name that is a support *module* rather than a test target emits no
  `test result:` line at all. No result line means "the harness never ran" —
  not a pass, not a failure (same rule as #4105).

## Host-local changes through ambiguous names (issue #3683)

A name in ssh config may be a round-robin alias over several machines, each
with its own crontab, and `ssh name` lands on a different one every time. A
host-local change (crontab, systemd unit, `/etc` file, installed binary) made
through such a name has a scope question before it has a content question:

- **Resolve the name first** (`dig +short` / `getent hosts`): more than one
  address is a host set, not a host.
- **Address the change to a specific host** and record which host in the
  change's own note — `hostname` at the start of a host-modifying session.
- **Verify on every host in the set.** A read-back through the alias is not
  verification: the read may reach a different machine than the write, with
  nothing in the output distinguishing the two. Never trust a read-back that
  could have come from somewhere else.
- **Diff the hosts before assuming they are copies** — "identical" is
  observed, never presumed.

Checkable in `autospec_core::host_set` (`resolve`, `AddressedChange::record`,
`VerificationLedger`, `diff_hosts`).

## Truncated views and negative claims (issue #4167)

An agent inspected a runner's `out/issue-N/status.txt` with `head -6`, concluded
"records `issue`, `status`, `agent_rc`, `agent_secs`, `changed_files` — and no
SHA", and filed that as an invariant in a queued issue. The file has 18 lines;
line 17 is `base_sha=...`, present in every patch on the cluster. Left alone, an
implementer would have added a duplicate field. **The blast radius of a bad issue
in an autonomous pipeline is a code change, not a note**: a wrong invariant does
not stay wrong on paper, it becomes a duplicate field, a second source of truth,
and the divergence between them becomes a later bug.

- **Read the whole artefact before asserting what it lacks.** A negative claim
  about a file requires the whole file: `cat`, or an explicit `grep -c` for the
  thing claimed missing. `head`/`tail`/`| head -N` support positive claims only.
  Where output is genuinely too large, the negative claim must be made by search
  (`grep -L`, `grep -c`), never by eyeballing a window. Absence within a window
  is not absence.
- **An invariant that asks for a new field must first demonstrate its absence.**
  Include the check in the issue: `grep -c '^base_sha=' status.txt` → 0. Filing
  "the system must record X" is a request for a schema change; the evidence bar
  is showing X is not already there, not noticing it wasn't in what you happened
  to look at.
- **Prefer "use the existing field" over "add a field" whenever both are
  possible.** Two fields carrying the same fact will drift, and nothing will say
  which is authoritative.
- **Correct a filed issue the moment its premise fails, and say which invariants
  are withdrawn by number.** An issue in the queue is live work. A correction
  buried in prose is not enough — name the invariant that is withdrawn so an
  implementer cannot miss it.

And the measurement the invariant was requested for, once possible with the
existing field, contradicted the reasoning that demanded it: the patches that
landed were further behind main (median 189 commits) than the conflict-held ones
(130). Measuring instead of arguing falsified the hypothesis rather than
confirming it — the ordinary outcome when the data is read whole.

## Configuration before storage: negative existence claims (issue #4192)

Asked "is GLM deployed?", one check listed a single storage directory, found
four Qwen files, and filed an issue stating GLM was benchmarked but never
deployed and DeepSeek never started. Both statements were false: 534 GB of
weights sat in a sibling directory that was never listed, and the model
catalogue (`models.tsv`) named both models plainly, with the GPU count,
context limits and slot count each needs. The catalogue and the runtime that
reads it (`pick-config.py`) disagreed — the catalogue had both models, the
runtime had neither — and that single divergence was the whole defect.

**To answer "does this system have X", read the configuration that would
reference X — not the storage where X might live.** Configuration is a smaller
search space than storage, it is authoritative about *intent* rather than
accident, and it is the thing the system itself consults. Storage answers "what
is on this disk", which is a different question and only becomes the right one
after configuration says X should exist and you are checking whether it does.

The scale is the tell: a check that can miss half a terabyte is not a weak
check, it is the wrong check. When the answer to "is X here?" comes back "no"
from an inspection of one location, the next step is to search where X would be
*declared*, before concluding anything. Concretely, before filing "X is not set
up":

1. `grep -rl X` the configuration files, not the data directories.
2. Confirm from at least two independent places — the catalogue and the
   runtime that reads it — because they can disagree, and the disagreement is
   usually the actual bug.
3. State in the issue *where you looked*. "Checked `$L/models`, found only
   Qwen" makes the gap in the evidence visible to a reviewer without knowing
   anything about the system.

This is the second instance of the same error (the first is #4167 at
file scale: `head -6`, then a claim about the whole file; see the
"Truncated views and negative claims" section above). A filed issue in
this pipeline is dispatched to an agent and implemented, so a wrong premise
becomes a wrong change — both times the correction had to chase a live issue
before an agent acted on it.

### Refuse and name what would tell you

The thing that caught this was a guard doing exactly the right thing:

```
FATAL: 'qwen3.8-27b-vision-q8' is not in models.tsv; refusing to guess GPUs
```

It had every input needed to guess a plausible GPU count and refused, naming
the file that would have authorised it. **A component that cannot determine a
safety-relevant parameter must refuse and name what would tell it** — the
opposite of the failure modes in #4135 and #4190, where a missing input
silently became a permissive default. A refusal that names its missing input is
both the fix and the diagnosis: it tells the operator which file to consult
instead of guessing for them.

### Fit is derived, not listed (#4244)

Scheduling eligibility is a property of the catalog plus the card, not a
property of the model name. **Answer "on which classes may this model be
scheduled?" by reading the catalog's `vram_mib` and comparing it to each
candidate card — never by a `case "$MODEL"` allowlist of names.** A list of
names drifts the moment the catalog gains a row; the derived answer cannot.
The fit test is exactly `card.vram_mib >= entry.vram_mib` — no margin is
invented at the guard, and a card whose VRAM is unmeasured (`vram_mib == 0`)
refuses with a distinct variant rather than being treated as fitting
everything (the permissive default is a lie) or silently dropped. A name the
catalog does not know is its own refusal (`FitCheck::Unknown`), never an
empty class list: a narrow probe's exit code is not a membership test.

Checkable in `autospec_core::fleet_models` (`FleetRegistry::eligible_classes`,
`GpuCard`, `FitCheck` — pure in-memory, no subprocess, so the node repo's
`worker.sh` / `pick-config.py` can adopt them as the single source of truth).
Tests: `crates/autospec-core/tests/fleet_models.rs`.

## Semantic code intelligence

- Every code intelligence query names a workspace; a workspace resolves to exactly one
  worktree root. Never reuse a result across worktrees or revisions -- cache keys are
  `workspace:revision:operation:request`.
- Three gates are mandatory (`.autospec/code-intelligence.yaml` -> `workflow`): the planner
  produces a semantic Impact Set, the implementer produces a post-change diagnostic delta
  with no new errors, and the reviewer runs its own analysis instead of reusing the
  implementer's report.
- Baseline diagnostics stay visible but never fail a task on their own; only errors the
  change introduced block completion under `block_new_errors`.
- A backend failure degrades `lsp -> ast-grep -> ripgrep`. Degraded results carry
  `confidence: structural|textual` and `degraded: true` -- never call a degraded result
  semantic evidence. `code.hover`, `code.type_hierarchy` and `code.diagnostics` have no
  fallback and fail closed.
- `autospec doctor code-intel [--json]` reports backend, server, fallback and security
  health. See [`docs/code-intelligence.md`](docs/code-intelligence.md).

## Subagent model selection (two-tier, cost-aware)

When the workflow dispatches a subagent, choose tier based on the **type of work**, not by phase number alone. Two tiers:

### Tier A — Specification work (top model + extended/maximum thinking)

Used by: research subagents (Phase 1), decomposition subagents (Phase 3 — turning a spec into linked GitHub issues).

Reasoning: spec/issue quality is the bottleneck. A cheap model here costs you N cheap-implementer cycles correcting it downstream. The orchestrator/user is also typically running on a top model in Phase 2 (design + spec writing); subagents in spec-adjacent phases match that quality.

**Tier right-sizing for classification (Phase 3.5 review-and-label + `/autospec-classify`):** classification is now **deterministic-first**. The deterministic rubric (file counts under `## Files to read first`, verb keywords — see `scripts/classify-model-fit.sh`, tracker #421) runs FIRST and gates any LLM call. Only issues the rubric scores as ambiguous (confidence below `LLM_ESCALATION_THRESHOLD`) get an LLM call, and that call runs at **Tier B**, not Tier A. Sibling normalization stays deterministic. This keeps the common case zero-LLM-cost while preserving a cheap LLM tie-breaker for genuinely ambiguous issues.

| Harness     | Preferred model | Thinking budget | Fallback (next-tier UP on unavailability) |
|-------------|-----------------|-----------------|--------------------------------------------|
| Claude Code | `opus` — the alias, which resolves to the current Claude Opus generation (`claude-opus-5` as of 2026-08) | `ultrathink` (max thinking budget) | latest available top model |
| Codex CLI   | the configured top non-spark model from `~/.codex/config.toml` (`gpt-5.6-sol` as of 2026-08) | `reasoning_effort=high` | latest top variant |
| OpenCode    | top tier configured for `task` agents | provider-equivalent of "high" reasoning | next available |

### Tier B — Implementation work (cheaper model + medium thinking)

Used by: implementer subagents inside Phase 4's `process(ISSUE)` (the one writing code on `feat/*` branches); the fused guardian + LGTM reviewer subagent (the inner-loop self-review of a PR — Tier B for ALL issues including `regression`/`priority:high`, see the env hatch below); and the Tier-B LLM tie-breaker for ambiguous classification (above).

Reasoning: implementation follows a well-specified contract from Tier A. The work is mechanical relative to the spec. We run this loop many times per spec, so cheaper-tier amortizes well.

| Harness     | Preferred model | Thinking budget | Fallback (UP on unavailability) |
|-------------|-----------------|-----------------|----------------------------------|
| Claude Code | `sonnet` — the alias, which resolves to the current Claude Sonnet generation (`claude-sonnet-5` as of 2026-08) | medium thinking | `opus` → latest |
| Codex CLI   | the spark / cost-optimized variant of the configured model when one exists, else the configured model | `reasoning_effort=medium` | next-larger Codex → latest |
| OpenCode    | smaller-tier task model | medium reasoning | next-larger configured tier |

### Harness detection protocol

Detect your harness by checking available tools before any phase, then resolve `TIER_A` and
`TIER_B` once for the entire skill run. Every "Tier A" and "Tier B" reference in skill bodies
maps to these harness-specific values.

1. **Claude Code** — the `Agent` tool with a `subagent_type` parameter is available.
   - `TIER_A` = `opus` + `ultrathink`  (see Tier A table row above)
   - `TIER_B` = `sonnet`               (see Tier B table row above)

2. **OpenCode** — a `task` tool with model/tier configuration is available (no `subagent_type`).
   - `TIER_A` = top-tier task model + high reasoning
   - `TIER_B` = smaller-tier task model + medium reasoning

3. **Codex CLI** — neither `Agent` nor a configurable `task` tool is available; `apply_patch`
   is the primary edit tool.
   - `TIER_A` = configured top model (see Tier A table row above) + `reasoning_effort=high`
   - `TIER_B` = spark variant if present, else configured model + `reasoning_effort=medium`

**Fallback rule:** If TIER_B is unavailable in your harness (model unknown, quota/capacity
failure, authorization failure, or tool call returns an error for that model), silently
retry the same subagent dispatch with TIER_A instead. Never collapse the delegated work
back into the main session solely because the Spark/cheap model failed.

**Context preservation rule:** Model fallback must preserve the parent context. For Codex
native subagents, fork/inherit the current conversation context when retrying on the
latest top GPT model so the upgraded subagent sees the same task state, repository
findings, and constraints as the failed Spark attempt.

### Flexibility rule (both tiers)

If the preferred model name is rejected (deprecated, quota/capacity, unauthorized), retry
the subagent with the next tier **UP** — never silently downgrade below the tier's intent
and never switch to main-lane execution just because the cheaper model failed. Never
hard-code exact version strings in dispatch code; resolve "current Opus / Sonnet / spark /
top GPT" at call time so the skill survives model-family churn.

### Tier assignment by phase (quick reference)

| Phase | Skill(s) | Tier |
|-------|----------|------|
| 1 — Investigate (research) | autospec, autospec-define | A |
| 2 — Brainstorm + design | autospec, autospec-define | (orchestrator only — no subagent dispatch; user invokes skill on top model) |
| 3 — Decompose into issues | autospec, autospec-define | A |
| 3.5 — Review and label | autospec, autospec-define | deterministic-first; **B** on ambiguity |
| classify (per-issue review) | autospec-classify | deterministic-first; **B** on ambiguity |
| 4 — Implementer (process(ISSUE) in worktree) | autospec, autospec-run | B |
| 4 — Fused guardian + LGTM reviewer | autospec, autospec-run | **B** for ALL issues (incl. `regression`/`priority:high`); `AUTOSPEC_REVIEWER_TIER=opus` → A |
| 4 — Implementation guardian (Tier-A escape hatch) | autospec, autospec-run | **A** when `AUTOSPEC_REVIEWER_TIER=opus` (otherwise folded into the Tier-B fused reviewer row above) |

**`AUTOSPEC_REVIEWER_TIER` (reviewer escape hatch):** the fused guardian + LGTM reviewer runs at **Tier B for every issue**, including `regression` and `priority:high`. The former second Tier-A regression meta-review pass is folded into the single reviewer brief (the reviewer self-asks "would the reviewer have caught the original gap?" and writes any missing checks to `reports/autospec-review/reviewer-lessons.md`). To restore Tier A for the reviewer, set `AUTOSPEC_REVIEWER_TIER=opus`; unset (or any other value) keeps Tier B (sonnet). This is the one-variable revert if a high-stakes run shows the cheaper reviewer missing real bugs.

## Auto-merge authority for auto-implement PRs

Admin-merge `auto-implement` PRs (`gh pr merge <#> --admin --squash --delete-branch`) when:
- The full target-repo validation/test suite has passed locally after the branch is current with `main`.
- All **non-advisory** required CI checks pass — checks matching `AUTOSPEC_PR_ADVISORY_CHECKS` (default `AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS`; e.g. self-hosted TeamCity) are advisory and may be pending **or failing** once the full local suite is green.
- The self-review subagent returned `LGTM`.
- PR closes an `auto-implement` issue from a `feat/*` branch.

Spec PRs (head branch matches `feat/spec-*` OR body contains a `Source spec` line
referencing `docs/specs/`) carry the same admin-merge authority: orchestrators run
`gh pr merge <#> --admin --squash --delete-branch` once required CI checks pass and
the body matches one of those criteria.
Escape hatch: set `AUTOSPEC_NO_AUTOMERGE_SPEC=1` to short-circuit the auto-merge and
fall back to "open PR + ask user".

## Stop mode authority

Operators can halt a running autospec monitor in two ways, both leaving clean state:

- **Graceful** (`/autospec-stop --graceful`, default): the monitor finishes the current `process(ISSUE)` to its natural end (success → admin-merge, or 3-iter failure → label restore + comment). The outer loop exits BEFORE dispatching the next issue.
- **Immediate** (`/autospec-stop --immediate`): the current `process(ISSUE)` commits any uncommitted work (`chore: WIP — autospec stop`), pushes the branch, marks the issue `paused-by-user`, inserts a `## Resume context` block, and exits at the next major-step boundary.

**Sentinel file**: `~/.autospec/stop.flag`. Two-line format: `<mode>\n<ISO8601> <user>@<host>`. Atomic write via `temp+mv`. Stale flags (>24h) are ignored with a WARN to stderr. Corrected rule (issue #3689): `temp+mv` is atomic for *future* openers, not for a process *already reading* the file — after the rename it keeps the old inode, which a network filesystem tears down ("Stale file handle"). Never replace a file a live job may be streaming: check for open holders and either refuse or publish a versioned path (`autospec_core::safe_publish`).

**`paused-by-user` label**: color `#d4c5f9` (lavender), created idempotently by the abort path. Issues carrying this label are removed from the `auto-implement` queue until `/autospec-stop --resume` strips the label.

**Resume procedure**: `/autospec-stop --resume` strips `paused-by-user` from every paused issue, restores `auto-implement`, and deletes `~/.autospec/stop.flag`. The `## Resume context` block is kept as an audit trail. The next `/autospec-run` invocation picks up the restored issues normally.

**Inline sub-modes**: both `/autospec` and `/autospec-run` accept `stop [--flag]` as a feature-request argument (regex `^\s*stop(\s+--\w+)*\s*$`, case-insensitive), routing through the same `scripts/autospec-stop.sh`.

## Autonomy charter (default-on)

The operator's standing preference is **autonomous by default**: when
`~/.autospec/autonomous.flag` is present, `/autospec-define` and `/autospec-run`
skip design-ratification, spec→plan→run handoff gates, and per-issue
confirmations. The governing rule is **recommendation = action** — if the agent
is confident enough to recommend a next step, it takes it and reports rather than
asking permission. Safety is unchanged: `scripts/autospec-autonomy-gate.sh
--check all` still surfaces a confirmation for destructive remote actions,
force-push to a protected branch, out-of-scope files, cost over the aggressive
token cap (`AUTOSPEC_AUTONOMOUS_TOKEN_CAP`), and
genuine no-clear-winner forks. Full policy and rationale (mined from session
transcripts): [`docs/AUTONOMY-CHARTER.md`](docs/AUTONOMY-CHARTER.md).

## Autonomous run accountability

Every live `autospec autonomous` launch creates or adopts exactly one verified managed GitHub epic before conductor spawn. There is no bypass for the epic or its private local journal. A normal start generates its own epic; `start --epic N` adopts an active managed epic, and `resume --epic N` may reconstruct and reopen a verified closed or parked epic for the same repository. Explicit adoption must validate the immutable run marker, recovery manifest, and the `epic`, `type:tracker`, `no-auto`, and `autospec:run-accountability` labels; it never converts an arbitrary issue.

The epic is the concise human accountability projection for one conductor generation. It preserves human text outside managed markers and records short What, Why, and Evidence entries, linked issues and PRs, verification, blockers, next steps, one Mermaid dependency/deliverable flowchart, and one Mermaid run-state diagram. Local typed events are durable before projection. Later GitHub edit failure remains retryable and visible in local `status --json` / `list --json` fields without creating a replacement epic. For products configured with `project_board.mode: managed`, assignment to the product's verified GitHub Project is mandatory and any failed assignment remains one retryable pending projection; local typed events and the private journal remain authoritative while GitHub projection is degraded. External or unconfigured products retain their compatibility behavior.

Start-family commands also refresh the requested checkout before acquiring the lifecycle lease. A stale or missing runtime must rebuild an immutable source-digest generation and execute that exact verified generation path. Read-only and stop commands may bypass runtime rebuilding, but no live conductor may knowingly continue on an outdated generation.

## Startup self-update

Every multi-harness skill runs a preflight at startup that updates the installed copy
from `main` at most once per 24 hours (fail-open: any network or install error logs a
`WARN:` line and continues). Set `AUTOSPEC_NO_SELF_UPDATE=1` to skip. The canonical
bash lives in `scripts/autospec-startup-self-update.sh`. The injected
`## Startup self-update` block (`templates/skill-blocks/startup-self-update.md`) only
resolves and invokes that script, and is mirrored byte-identically (modulo `SKILL_NAME=`)
across all multi-harness skill trios. `autospec validate` (`check_startup_preflight`)
enforces byte-identity.

The lock (`~/.autospec/.update.lock.d/owner`) carries the holder's PID and start time; a
lock whose owner is dead — or that has no owner — is reclaimed automatically after a 30s
write-grace window (and unconditionally after 30 min), so a crashed run can no longer
silently disable self-update (issue #3937). Run `bash scripts/autospec-startup-self-update.sh
--doctor [--clear-stale-lock]` for an operator health check (lock state, throttle stamp age,
installed-vs-remote version drift, last failure record; exits 1 on any finding; bypasses
`AUTOSPEC_NO_SELF_UPDATE`).

**Never inline shell that assigns a positional parameter into an injected skill block.**
A harness substitutes `$1` inside a *rendered* skill body at load time, so `target="$1"`
becomes the caller's slash-command argument (issue #3177). `check_startup_preflight`
rejects `="$1"` / `="$2"` in the block; put the shell in a `.sh` file instead.

## Small-LLM target

Generated child issues are sized for 32B-class local LLMs. Pre-staged context, sectional spec anchors, checkbox AC, one Primary smoke test per inner loop.

The nodes that serve those models now live in their own private repository,
**[berlinguyinca/autospec-node](https://github.com/berlinguyinca/autospec-node)**,
extracted from this one with history. It holds `QWEN-NODE-SPEC.md` (the portable
method: hardware audit, quantisation from memory rather than name, measuring a
context ceiling that survives a prompt actually filling it, serving concurrent
sessions), `linux-qwen38/` (the measured RTX 4090 build, its operator toolkit,
and the Slurm deployment) and `linux-turing-dual/` (the dual-Turing node), along
with the design documents that describe them.

It is a pointer rather than a subtree because two copies drift the moment either
is edited, and the drift is silent.

Two results from that work bind anything that sizes a local context window:
the usable window is **client-specific** — the context present before any work
begins was 14,492 tokens median on OpenCode and 39,655 on Claude Code, so a
tier below the client's p90 floor cannot start a session at all — and a shared
KV pool has **no admission control**, so over-subscribing it fails every live
session rather than the greedy one.

## Anti-loop guardrails

Per spec §5.1, both the Phase 1 research subagent and the Phase 4
implementer subagent run under hard, no-wall-clock-cap limits to keep a
runaway model from burning tokens or getting stuck rewriting the same
file forever:

- **Phase 1 research subagent.** Max **25 tool calls**. If 3 consecutive
  read/grep calls return nothing useful, stop and write a best-effort
  summary even if it is incomplete. Never retry the same query verbatim.
- **Phase 4 implementer subagent.** Max **40 tool calls** per issue. Max
  **3 self-review iterations**. If the implementer rewrites the same
  file twice with no test progress, abort: comment the blocker on the
  issue, release the `locked-by-autospec-processor` label, and exit.
- **No wall-clock cap.** Both limits are tool-call / iteration based,
  not time based, so stalled work is detected by behavior, not clock
  time.
- **Where they live.** These limits live inline in
  `skills/autospec/SKILL.md` (Phases 1 and 4) and
  `skills/autospec-run/SKILL.md` (Phase 4). The lock-step rule
  replicates the same body to `opencode/agent.md` and
  `codex/prompt.md`.

## Listener-filed issues lifecycle

Per spec §4.1 and §5.3, issues filed by `autospec-listen` follow a
distinct two-step lifecycle on the way to the `auto-implement` queue:

- **Step 1: listener creates with `needs-classify`.** When the listener
  fires on an issue trigger and the user confirms, the resulting
  `gh issue create` call carries `--label needs-classify` (color
  `#fbca04`, idempotently created via
  `gh label create needs-classify --color fbca04 --force`). The issue
  is NOT yet on the implementation queue — it is a draft awaiting
  classification.
- **Step 2: classifier transitions to `auto-implement`.**
  `/autospec-classify` walks BOTH `auto-implement` AND `needs-classify`
  issues. After applying `ctx:*` / `reasoning:*` labels and inserting
  the `## Model fit` block, on any issue carrying `needs-classify` it
  ALSO performs:
  `gh issue edit <N> --add-label auto-implement --remove-label needs-classify`.
  Issues that already carried `auto-implement` (and not
  `needs-classify`) are re-classified in place; no label transition.
- **No auto-promotion.** There is no TTL-based promotion. Stuck
  `needs-classify` issues are swept by re-running `/autospec-classify`
  manually or via the sample crontab in
  `docs/runbooks/needs-classify-sweep.md`.

## Issue-quality contract

Every GitHub issue created by autospec (Phase 3 decomposer, Phase 3.5 reviewer, or
`/autospec-classify`) must satisfy the rules below before an implementation
agent picks it up. The enforcer is `scripts/lint-issue.sh` (exits 0 on pass, N on
fail where N = number of findings).

### Goal concreteness

The `## Goal` section must contain exactly one sentence (one terminal `.`, `?`, or
`!`). It must NOT use bare vague verbs (`improve`, `enhance`, `optimize`, `polish`,
`simplify`, `refactor`, `harden`) unless the same sentence also contains a concrete
object (a file path, a backtick-quoted command/identifier, a number, or an
`UPPER_SNAKE` label/env-var). It must NOT use hedging words (`should`, `might`,
`could try`, `try to`).

PASS: `Add \`scripts/lint-issue.sh\` that exits non-zero if the body fails the §3 quality contract.`

FAIL: `Improve the decomposer prompt for better issue quality.`

### AC machine-checkability

Every non-blank line in the `## Acceptance criteria` section must start with
`- [ ] ` followed by content. Each item must contain at least one of: a path-shaped
token, a backtick-quoted span, an integer, or a regex literal. Each item must NOT
use subjective adjectives (`looks`, `feels`, `seems`, `clean`, `elegant`). Each
item ≤120 characters. Section must contain ≥1 item.

### Scope-hedge rule (issue #4275)

The scope sections — `## Goal`, `## Acceptance criteria`, `## Implementation
outline` — must contain nothing optional. An agent implements the smallest
thing a spec can be read as permitting: a hedged or interim phrase names an
alternative acceptable outcome and becomes the delivered scope. Hedged
phrasing — `interim`, `for now`, `at minimum`, `ideally`, `conservative` —
belongs in a rationale section, or the smaller thing becomes its own issue
with its own acceptance criteria. Enforced by `scripts/lint-issue.sh` as
`SCOPE_HEDGE` (one finding per scope section, first matched phrase cited).

FAIL: `A conservative interim: gate ScaleDown now and leave the other 3 signals for later.`

PASS: `Gate ScaleDown, Rebalance, ScaleUp and Recreate behind the four signals in scripts/scale-policy.sh.`

### Primary smoke test shape

The first fenced code block under `### Primary smoke test (inner loop)` must contain
exactly one non-blank, non-comment line. It must NOT contain `...`, `<TODO>`, `TBD`,
or `XXX`.

### Files-touched path grammar

Every non-blank line in `## Files touched` declares exactly one safe repo-relative
path, optionally wrapped in backticks and preceded by `- `. Absolute paths, `.`,
`..` segments, root `/`, prose, and multiple paths on one line are invalid. A file
declaration authorizes only that exact path; a directory authorizes descendants only
when declared with a trailing `/`. `## Implementation outline` may also contribute
safe paths embedded in prose when each path is individually wrapped in backticks;
this does not relax the standalone-entry grammar for `## Files touched`.

## Subagent vs inline decision matrix

Every autospec skill author MUST consult this matrix before choosing between a
nested subagent dispatch and inline (main-session) execution. Skills that fan
out work inline burn orchestrator context tokens; skills that over-dispatch pay
subagent boilerplate cost on trivial work. The defaults below normalize that
tradeoff across the skill family.

| Work shape | Choose | Why |
|---|---|---|
| Read-only exploration across many files (grep, file enumeration, pattern survey) | **Subagent (Explore type)** | Bounded context, returns short summary, doesn't pollute orchestrator |
| 2+ independent tasks with disjoint scopes | **Parallel subagents (foreground, worktree-isolated per PR #691)** | True parallelism; isolated branches prevent collision |
| Single-purpose long-running work (Phase 4 implementer, peer-review) | **Subagent (general-purpose, single-agent absorbed discipline per PR #653)** | Quarantines context; orchestrator stays lean |
| Risky / quarantine-worthy work (codex peer-review, security scans) | **Subagent (foreground)** | Failure doesn't poison orchestrator |
| Orchestration / decision-making | **Inline (main session)** | State must persist across turns |
| One-shot tool calls (single grep, single gh) | **Inline (Bash tool)** | Subagent overhead exceeds work |
| Short edits (1-3 file modifications) | **Inline (Edit tool)** | Subagent boilerplate dominates cost |
| Routing / control flow (stop, listen, classify-trigger) | **Inline** | Decision is the work |
| Multi-area fan-out (specs / docs / tests / impl / QA) | **Parallel subagents, one per area** | Token cost per area is independent; main session aggregates findings |
| Any fan-out whose children run on a **local** model | **Width capped by the capacity gate** | Token cost per area is NOT independent on one GPU: children share a fixed KV pool with no admission control, so over-subscribing it fails every live session rather than the newest |

Each skill's `## Required capabilities & harness adapter` table carries a
**Subagent dispatch policy** row pointing back to this matrix; the
`autospec validate::check_agents_md_subagent_matrix` gate enforces lockstep
across every adapter trio.

### Capacity gate for local-model fan-out

The matrix decides the fan-out a task *wants*. On a local model the host decides
what it can *fund*, and the two are unrelated: a subagent costs no VRAM (that is
committed once, when the server starts) but does claim a share of a fixed KV
pool and one of a fixed number of slots. Over-subscribing that pool fails every
live session, not the greedy one.

Effective width is therefore `min(work-shape width, what the pool funds)`:

```bash
W=$(context-budget-check.py --width "$PLANNED" --json | jq -r .max_width)
```

Exit 0 fits, 1 does not, 2 could not tell. `W <= 1` means run inline. On exit 2
— server unreachable, config unreadable — fall back to the cloud tier rather
than guessing a width; a wrong guess here is not slower, it is a failed run for
every session on that node.

This is a client-side gate because the client's declared context limit is the
only admission control that exists. Nothing on the server refuses a session it
cannot fund.

## Implementation-quality contract

Every PR produced by an `auto-implement` agent must satisfy the rules below before
the LGTM reviewer is dispatched. The enforcer is `scripts/lint-implementation.sh`
(exits 0 on pass, N on fail where N = number of blocking findings, capped at 200).

### RULE_ID table

| RULE_ID | Detector | Tier | Threshold / regex |
|---|---|---|---|
| `PR_SIZE` | det | git diff/numstat | **advisory by default** (INFO) above 400 additions+deletions, 8 raw files, or 3 normalized logical units; blocking only when `AUTOSPEC_PR_SIZE_STRICT=1`; binary rows always block in strict mode |
| `OUT_OF_SCOPE` | det | exact/prefix path compare | files touched ∉ exact files or trailing-slash directories declared in `## Implementation outline` ∪ `## Files touched` |
| `MISSING_TEST` | det | path-prefix scan | required test type from issue body `## Tests required` not present in diff under `tests/{unit,integration,smoke,e2e}/` |
| `COMPLEXITY` | det | line/regex scan | function >50 LOC, file >500 LOC, nesting >4 |
| `SECURITY` | det | regex match | `eval\(`, `exec\(`, `--no-verify`, `git reset --hard`, `rm -rf /`, AWS-key shape `AKIA[0-9A-Z]{16}`, GitHub-token shape `gh[pousr]_[A-Za-z0-9]{36,}`, private-key markers `-----BEGIN [A-Z ]*PRIVATE KEY-----` |
| `TODO_LEFT` | det | regex on non-test diff | `\b(TODO\|XXX\|FIXME)\b` |
| `MOCK_DB` | det | regex on test diff | `\b(mock\|stub)\b` near DB-symbol heuristics (`db\.`, `database`, `DataSource`, `pg`, `mysql`, `sqlite`); `*.diff`/`*.patch` are captured data and exempt, as they are for `TODO_LEFT` |
| `HALLUCINATED_API` | LLM | semantic | symbol referenced in diff not defined in diff, not in pre-PR repo (verifiable via repo search), not in dependency manifests |
| `DUPLICATE_CODE` | LLM | semantic | new code mirrors an existing helper (must cite `<path>:<line>`) |
| `STRING_MATCH_DOMAIN_LOGIC` | LLM | semantic | code uses substring checks against free-form text to encode domain meaning, AND a proper-representation library is imported in the file. Recognized primitives — Python: `rdkit`/`ast`/`urllib.parse`/`datetime`/`ipaddress`/`lxml`/`jsonschema`; JS/TS: `URL`/`Date`/`@babel/parser`/`acorn`/`ts-morph`/`zod`/`ajv`/`joi`; Go: `net/url`/`time`/`go/ast`/`net.ParseIP`/`encoding/json` + struct tags; Java: `java.net.URI`/`java.time.*`/`JavaParser`/`com.github.javaparser`/`javax.validation`; Scala: `java.net.URI`/`java.time.*`/`scalameta`/refined types/circe schemas; Rust: `url::Url`/`chrono`/`time`/`syn`/`std::net::IpAddr`/`serde` with strong types |
| `REPEATED_STRUCTURE_AS_CODE` | LLM | semantic | ≥5 branches in the same function/method sharing identical structural shape (same return-tuple/case-class/struct-literal shape, same predicate signature, same side-effect line). Language-agnostic — Python if/elif, Java/Scala switch/match, Rust match arms, Go switch cases, JS if/else |
| `DOC_OUT_OF_SYNC` | hybrid | det+LLM | det: any change to public surface (CLI flag, env var, exported function, config key) WITHOUT a touched doc file (`README*`, `AGENTS.md`, `docs/**`, `SKILL.md`, `skills/*/prompts/*.md`, `skills/*/references/*.md` — matched at any depth, so a subproject's own `README.md` or `docs/` counts). Markdown and `*.diff` are never scanned for the surface itself: prose describes a flag, it does not introduce one. `CHANGELOG.md` earns no credit, or every commit would satisfy the rule; LLM: judges semantic accuracy when a doc IS touched |
| `INVENTED_CONFIG` | LLM | semantic | flag/env-var/config-key introduced in diff not present in issue body or referenced spec |
| `BATS_SUITE_UNREGISTERED` | det | pre-commit path scan | staged `.bats` file added under `tests/unit/` or `tests/lint/` whose quoted path appears in neither `crates/autospec-core/src/validation/catalog.rs` nor `BATS_REGISTRATION_BASELINE` in `crates/autospec-core/src/validation/external/bats_registration_baseline.rs`; suites at `tests/` root are exempt — the authoritative scan is `run_bats_suite_registration` at conversion (#3919) |
| `COMMAND_NOT_REGISTERED` | det | pre-commit staged-diff scan | a new command name introduced to the `COMMANDS` table or the dispatch match in `crates/autospec-cli/src/commands/mod.rs` (new = staged name set minus base name set) whose remaining registration sites — the `COMMANDS` table entry, the dispatch match arm, or the `\`autospec <name> ...\`` row in `docs/cli-reference.md` — are not also staged in the same commit; the finding names every unvisited site with file:line and the value to add (#3964, repro #3793) |
| `CATALOG_ENTRY_INCOMPLETE` | det | pre-commit staged-diff scan | a new catalog check id (new = staged id set minus base id set) that is only half-registered: present in `STANDARD_CHECK_IDS` (`crates/autospec-core/src/validation/catalog/catalog_ids.rs`) without a match arm in `ValidationCheck::catalog_entry` (`crates/autospec-core/src/validation/catalog.rs`, dead code), or present as a match arm without the id (runtime panic, #3964) |
| `GATE_PROMOTION_UNEVIDENCED` | det | workflow diff scan | a `.github/workflows/*.yml` change promotes a job to a blocking gate (adds it to another job's `needs:` or removes `continue-on-error: true`) without a cited green run (a GitHub Actions run/job URL or a captured exit status 0) in the issue or PR body; the finding names the workflow file and the job |

### Corrective directive map

Each RULE_ID has a single-line corrective directive injected into the implementer's
retry prompt as cumulative context.

| RULE_ID | Directive |
|---|---|
| `PR_SIZE` | "Advisory: prefer splitting the diff into a stack of small PRs (each layer ≤ cap) so the cap can be enforced per layer. If it must ship as one diff, note why in the PR body; set `AUTOSPEC_PR_SIZE_STRICT=1` to make it blocking." |
| `OUT_OF_SCOPE` | "Restrict the diff to exact files or descendants of trailing-slash directories declared in `## Implementation outline` or `## Files touched`. Revert undeclared files; incomplete scope must be corrected by the issue author." |
| `MISSING_TEST` | "Add a test under tests/<TIER>/ for the listed required test type before re-pushing." |
| `COMPLEXITY` | "Split functions >50 LOC, files >500 LOC, nesting >4. No copy-paste branches." |
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets, NEVER use --no-verify or git reset --hard, validate input at boundaries." |
| `TODO_LEFT` | "Remove TODO/XXX/FIXME from non-test code. File a follow-up issue if the work is genuinely deferred." |
| `MOCK_DB` | "Remove DB mock/stub. Use the real DB per AGENTS.md ## Engineering standards." |
| `HALLUCINATED_API` | "The flagged symbol does not exist. Verify identifier names against the pre-PR repo and dependency manifests." |
| `DUPLICATE_CODE` | "Reuse the existing helper at <path>:<line> instead of re-implementing." |
| `STRING_MATCH_DOMAIN_LOGIC` | "Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case." |
| `REPEATED_STRUCTURE_AS_CODE` | "Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a `Map`/sealed-trait registry; in Rust use a `&[(predicate, value)]` slice; in Go use a `[]struct{...}` table. Each new entry should be one row, not a ~10-line block." |
| `DOC_OUT_OF_SYNC` | "Update the doc file(s) covering the changed public surface in this same PR." |
| `INVENTED_CONFIG` | "Remove the invented flag/env/key, or amend the issue body to introduce it as scope." |
| `BATS_SUITE_UNREGISTERED` | "Register the new bats suite as a typed ExternalCheck::BatsSuite owner in crates/autospec-core/src/validation/catalog.rs, or add its path to BATS_REGISTRATION_BASELINE in crates/autospec-core/src/validation/external/bats_registration_baseline.rs; suites at tests/ root need no registration." |
| `COMMAND_NOT_REGISTERED` | "Visit every registration site the finding names for the new command: the COMMANDS table entry and the dispatch match arm in crates/autospec-cli/src/commands/mod.rs, plus the \`autospec <name> ...\` row in docs/cli-reference.md — all in this commit." |
| `CATALOG_ENTRY_INCOMPLETE` | "Keep the two catalog sites in lockstep: the id must appear in STANDARD_CHECK_IDS (crates/autospec-core/src/validation/catalog/catalog_ids.rs) and have a match arm in ValidationCheck::catalog_entry (crates/autospec-core/src/validation/catalog.rs) — add the missing one in this commit." |
| `GATE_PROMOTION_UNEVIDENCED` | "Cite a green run of the promoted job in the issue or PR body (a GitHub Actions run/job URL or a captured exit status 0) before promoting it to a blocking gate, or revert the promotion. If the verification could not be executed, record the command and why it could not run in the Closeout report and the PR body." |

### Enforcement

The following rules are enforced deterministically by `scripts/lint-implementation.sh`.
Each rule has an inline escape hatch for genuine exceptions.

| Rule | Enforcing check | `linter:allow-` escape hatch |
|---|---|---|
| TDD non-negotiable (test must accompany every non-docs change) | `MISSING_TEST` detector | `# linter:allow-MISSING_TEST <reason>` on the issue body or in-file |
| No DB mocks/stubs in tests | `MOCK_DB` detector | `# linter:allow-MOCK_DB <reason>` — allowed only for unit tests with no accessible DB |
| No hardcoded secrets or unsafe git operations | `SECURITY` detector | `# linter:allow-SECURITY <reason>` — allowed only for test fixtures with non-secret values |

**Inline escape hatch syntax** (in source code, not issue body):

```
# linter:allow-MOCK_DB integration test requires mock — no test DB available in CI
# linter:allow-MISSING_TEST docs-only change, no behavior to test
# linter:allow-SECURITY fixture value is not a real secret
```

The `linter:allow-` comment must appear on the same line as or the line immediately before
the offending pattern. A bare `# linter:allow-X` without a reason is rejected and the
rule remains active. Allowed escape hatches are emitted as `INFO:RULE_ID:...` (audit trail)
but do NOT block the merge.

The existing `Guardian: skip-RULE_ID` opt-out grammar in the issue body remains valid for
per-PR-level skips. Inline `# linter:allow-*` is for line-level exceptions inside the code.

### Per-issue opt-out grammar

The issue body MAY declare per-RULE_ID opt-outs with mandatory justification.
Parsed by both the deterministic script and the LLM guardian.

```
Guardian: skip-MISSING_TEST # docs-only refactor, no behavior change
Guardian: skip-OUT_OF_SCOPE, skip-COMPLEXITY # large rename touching many files
```

Grammar (regex):

```
^Guardian:\s+(skip-[A-Z_]+(,\s*skip-[A-Z_]+)*)\s+#\s+\S.+$
```

Rules:

- Justification (text after `#`) is **mandatory**. Bare `Guardian: skip-X` is
  rejected (treated as malformed and ignored — RULE remains active).
- `PR_SIZE` is **advisory by default** (emitted as `INFO`, never blocks the merge).
  Set `AUTOSPEC_PR_SIZE_STRICT=1` to make it blocking; in strict mode a
  `Guardian: skip-PR_SIZE # <category>` waiver accepts only `generated migration:
  <generator>`, `dependency-solver lockfile: <solver>`, or `mandatory lock-step
  artifacts: <identity>` (the measured diff must prove the stated category; binary,
  forged, mixed manual code, and nested test paths stay blocking).
- Skipped RULE_IDs are still emitted by the linter as `INFO:RULE_ID...`
  (audit-trail visibility) but do NOT block the merge.
- Skips apply only to the specific PR derived from this issue; they do NOT cascade
  to other issues.

### Bats suite registration (`tests/unit/` and `tests/lint/` are the exception, not the rule)

Bats suites at the **root of `tests/` need no registration** — ~170 of them run
unregistered, and that is the convention. Suites under `tests/unit/` or `tests/lint/`
are the exception: `run_bats_suite_registration`
(`crates/autospec-core/src/validation/external.rs`) scans those two directories and
**fails conversion** on any suite owned by no typed `ExternalCheck::BatsSuite` check in
`crates/autospec-core/src/validation/catalog.rs` and absent from
`BATS_REGISTRATION_BASELINE` (`crates/autospec-core/src/validation/external/bats_registration_baseline.rs`).
A held patch here is a lost patch, not a warning — this is how two of eighteen
agent patches died in issue #3919.

- **Register in the same commit that adds the suite**, as a typed catalog owner
  (`bats_suite("tests/unit/<name>.bats")` under a named `validate` check). Baseline
  entries are for genuinely orphaned suites and the list is shrink-only — do not
  grow it for new work.
- The pre-commit gate surfaces the same rule early:
  `scripts/lint-implementation.sh --pre-commit` emits `BATS_SUITE_UNREGISTERED` for a
  staged suite whose quoted path is in neither file, so the fix lands before
  conversion instead of at it.
- If a child issue's scope is a `tests/unit/` or `tests/lint/` suite, its spec must
  name the registration (catalog check or baseline) in `## Files touched`.

### Per-layer stack guard (`scripts/stack-guard.sh`)

`PR_SIZE` above is measured on the whole accumulated diff. `scripts/stack-guard.sh`
enforces the same cap on a **single stack layer** — the `base...head` delta — so a
large change can ship as a chain of small PRs, each within the cap. It reuses
`lint-implementation.sh`'s `PR_SIZE` detector via `--diff-file` (single source of
truth) and adds a **linearity** check: a PR's base must be the default branch or the
head branch of another open PR (no orphan target branches).

- Advisory by default (`INFO` findings, exit 0); blocking under
  `AUTOSPEC_PR_SIZE_STRICT=1` (`ERROR` findings, exit 1).
- CI: `.github/workflows/stack-guard.yml` runs it per PR in advisory mode. Set the
  `STACK_GUARD_STRICT` repository variable to `1` to make it blocking.
- `AUTOSPEC_STACK_DEFAULT_BRANCH` overrides the default-branch name (else `gh repo view`).
- Tests: `tests/stack-guard.bats`.

The bats suite that pins the `BATS_SUITE_UNREGISTERED` pre-commit gate lives in the
already-registered `tests/unit/test_lint_implementation.bats` (catalog owner
`bats_suite_lint_implementation`, check `lint_implementation_gates`).

### Env-var contract

- `AUTOSPEC_NO_GUARDIAN=1` — short-circuit guardian, fall back to LGTM-only path.
  Mirrors `AUTOSPEC_NO_AUTOMERGE_SPEC=1` and `AUTOSPEC_NO_SELF_UPDATE=1`. Logged
  as `WARN: guardian disabled by AUTOSPEC_NO_GUARDIAN` on every Phase 4 dispatch.
- `AUTOSPEC_PR_SIZE_STRICT=1` — make `PR_SIZE` (and the stack-guard's per-layer size
  + linearity) blocking instead of advisory.
- `AUTOSPEC_STACK_DEFAULT_BRANCH` — default-branch name for `stack-guard.sh`
  (else `gh repo view`).
- `AUTOSPEC_CAPABILITIES_FILE` — path to the capability probe config for the
  `queue ready` frontier (default `.autospec/capabilities.json` in the current
  directory). See ## Capability prerequisites and zero-output review routing
  below.
- `AUTOSPEC_ZERO_OUTPUT_STATE_FILE` — path to the per-issue zero-output streak
  state for review routing (default `~/.autospec/state/zero-output-streaks.json`).
- `AUTOSPEC_TEST_FAILURE_BASELINE` — path to the known-failing-test baseline
  read by `scripts/test-failures-baseline.sh` (default
  `autospec/baseline-failures.txt` at the repo root). The entry format and the
  three rules that keep the file a baseline rather than a tolerated number are
  in [`docs/conversion-gate.md`](docs/conversion-gate.md) §5.

## Capability prerequisites and zero-output review routing

The ready-queue frontier models world-state prerequisites, not just
issue→issue dependencies (issue #3908):

- An issue body may declare capability prerequisites in a `## Requires`
  section — one capability name per `- ` item (charset `[A-Za-z0-9._:/-]+`,
  e.g. `- gateway:running`). A task with any unmet prerequisite is blocked in
  the queue with a hold message naming every missing capability (the
  `capability_blocked` gate count); it is never dispatched.
- Capability state is probed by the CLI, not the core. Probe commands live in
  `.autospec/capabilities.json` (override with `$AUTOSPEC_CAPABILITIES_FILE`);
  exit 0 = satisfied. Undeclared, unprobed, or failing capabilities are
  fail-closed (unmet), so a task is re-admitted automatically the moment its
  probes start passing — no re-labeling.
- Zero-output runs are tracked per issue in local state (override the file
  with `$AUTOSPEC_ZERO_OUTPUT_STATE_FILE`). After two consecutive zero-output
  completions the next offer routes the task to review instead of re-dispatch:
  the issue is blocked with reason `zero_output_review` (the
  `zero_output_review` gate count) and `queue ready` applies the
  `autospec:needs-human` label to it. Any successful completion clears the
  streak.

## Service-address resolution and pool health

A service address is a fact with an expiry: it is decided by whatever placed
the service and changes there, not in the argument list of a process that
started earlier. Resolve it from the authoritative record **at use time**
(the record a gateway writes for itself, e.g. `state/gateway-url`), never from
a value captured at launch. Checkable in `autospec_core::service_address`:

- `AddressResolver::resolve` reads the record. A launch argument is a fallback
  used only when the record is unreadable, and the resolution reports
  `AddressOrigin::LaunchArgument` so the stale capture is visible in the output.
  Empty and malformed records are errors, never an empty address.
- A cache in a long-lived process is invalidated on **any** failure
  (`AddressResolver::record_failure`, called by `register`), so the next use
  re-reads and follows a relocated service without a restart. `000` /
  connection-refused means the address may be stale
  (`RegistrationOutcome::address_may_be_stale`); a `401` proves reachability
  and indicts auth instead.
- A component that serves but cannot join the pool it was created for is
  **degraded, not healthy** (`component_health` ->
  `ComponentHealth::DegradedNotInPool`). "Serving anyway" is the fold that hid
  a fleet-wide registration failure for a day: agents dispatched directly, GPUs
  stayed busy, tests passed, and only the gateway's inventory was wrong.
- A health check asserts that the **service responded**
  (`service_health(HealthEvidence::SchedulerJobState{..})` ->
  `AssertedWrongThing`, never `Up`). Container or job liveness is not service
  health, and reachability of an address some third party recorded is not
  either.
- A reconciler over a pool reports the pool **size** on the same line as its
  verdict (`PoolMonitor::reconcile_line`) and flags `decline_window` consecutive
  declines as `PoolTrend::Draining` — slow drain must be visible before it
  reaches zero, not after.
- A merged fix is a deployed one only when the running revision matches the
  expected tip (#4228). The service reports the revision it is running
  (`branch @ sha` via `parse_revision`; a report that does not parse is "no
  revision", never "current"), the reconciler compares it against the expected
  tip by sha (`drift` — the same commit under a different branch name is in
  sync) and says "nothing to do" only when the two agree (`decide`). A
  redeploy is `Refused`, naming the unverified preconditions, until restart
  safety is tested: the build works, the preflight refuses to bind without
  auth, the reconciler starts a replacement (`PreconditionLedger`). The report
  line (`reconcile_line`) names the running revision next to the verdict, so
  "nothing to do" is never stated over stale or unreported code.

Tests: `crates/autospec-core/tests/service_address.rs`, including the
regression case "relocate the gateway, start a worker, it registers" with
nothing else restarted, and the #4228 redeploy cases: drift against the
expected tip, a refused redeploy naming its unverified preconditions, and the
moved gateway breaking the consumer that held the old address.

## Stored-output lifecycle and blocker escalation

A guard that prevents an action must name the action that releases it
(issue #4170). "I will not destroy this" is only half a policy; the other
half is who decides it may be destroyed, and when. A hold whose release is
unnamed is a deadlock reporting itself as normal operation, and it stays
armed forever — two issues sat undispatched behind such a hold while the
blocker line repeated itself, unchanged, on every run.

- A held state always names its release
  (`release(state)` -> `Release::ConvertPatch | ArchiveSuperseded |
  ReviewFailure`); a hold rendered with no named release is a
  defect (`"no release path (deadlock)"`), never an ordinary hold.
- Stored agent output has a lifecycle decided from cheap evidence
  (`classify(OutputEvidence)`): `awaiting_conversion`, `converted`,
  `superseded`, `failed`. The cheap test for *superseded* is the one the
  guard already has — the patch no longer applies to the trunk
  (`git apply --check` rejects it, `ApplyCheck::Rejected`). A check that
  cannot run (`Unrunnable`, or never run) is **fail-closed**: it is never
  read as "no longer applies", the output stays live.
- Superseded output is expendable and gets archived; archiving is the
  release action that disarms the guard. The guard itself must not be the
  thing that decides the output is expendable.
- A blocker that persists across runs escalates: `BlockerLedger` records
  the first observation (the caller persists the ledger as JSON between
  runs) and `escalation_phrase` renders its age ("blocked for 2d") once it
  passes `DEFAULT_ESCALATION_AFTER` — so "blocked 2 days by stored output"
  is not rendered the same as an ordinary idle cycle. A clock that rewinds
  is zero age, never an underflow.
- `ready` and `dispatchable` are different counts and are reported
  separately (`FrontierCounts::line` prints "N ready (M dispatchable)",
  plus the blocked count grouped by reason). The counts must reconcile
  (`ready == dispatchable + blocked.len()`); a frontier whose numbers do
  not reconcile is reporting a state that cannot exist.

Checkable in `autospec_core::stored_output` (`classify`, `release`,
`held_line`, `BlockerLedger`, `FrontierCounts`, `format_age`). Tests:
`crates/autospec-core/tests/stored_output.rs`.

## Environmental preconditions and load-aware selection (issue #4224)

A fix that rests on an environmental property — a homogeneous fleet,
identical context windows, a single gateway node — is only valid while the
property holds, and nothing enforces the "while". Gateway#21 made worker
selection load-aware by picking the worker with the most context; on a
homogeneous fleet that coincided with picking the least-loaded worker, and
the fix was right. A 32k-window worker joined a fleet of 256k-window ones,
and the same code became load-blind: it stacked requests on the big-window
worker and left the free 32k worker idle. The fix had silently reverted;
nothing warned, because nothing asserted the homogeneity it depended on.

- **Assert the property in code.** A fix that rests on an environmental
  property must carry a runtime check that warns loudly when the property
  no longer holds — `window_mismatches` reports every model whose workers
  report differing context windows, and its `WARN:` line names the model
  and every window observed. An assertion that cannot run is fail-closed
  (`Observation::Unrunnable`), never read as holding. A property that lives
  only in a comment is not asserted.
- **Regression tests run in the configuration the bug required.** The bug
  required a heterogeneous fleet (mixed context windows); on a homogeneous
  fleet every selection rule agrees, so a homogeneous test cannot see the
  bug. `tests/env_preconditions.rs` instantiates the mixed-window fleet the
  incident produced.
- **Capability filter before ranking filter.** "Eligible for this request"
  (context window ≥ the request requirement, compared against the
  requirement) runs before "best among candidates" (most free slots,
  compared among workers). A ranking filter over the whole candidate set
  silently re-asserts the assumption that all candidates are equivalent —
  the property that broke. The picker is total over answering workers:
  a fleet with no eligible worker still names the least-loaded one, flagged
  not-eligible (`Selection::Selected { eligible: false }`), and holding a
  zero-free-slot worker is the separate admission decision (`admit`,
  `Verdict::HeldSaturated` vs `HeldIncapable`).
- **Record the conditions when the issue closes.** A closeout for a fix
  that rests on an environmental property carries a `Valid while:` line per
  precondition, each naming the check that re-verifies it
  (`Precondition::line`). `Precondition::new` rejects a precondition with
  no named assertion: a precondition that no check re-verifies has no
  expiry, and it is how this incident happened.

Checkable in `autospec_core::env_preconditions` (`window_mismatches`,
`select_worker`, `admit`, `verdict`, `Precondition`, `PreconditionSet`,
`evaluate`). Tests: `crates/autospec-core/tests/env_preconditions.rs`.

## Work selection is part of the system, not the invocation (issue #4257)

A loop that runs every thirty minutes — "find the agent patches worth
converting" — never had its selection predicate written down, so it was
reconstructed from memory on every pass. Three reconstructions, three
distinct defects: the glob spanned four projects whose issue numbers
collide (`issue-14` is InferWeave, `issue-1` is the dispatcher — converting
by bare number would have opened InferWeave patches as autospec pull
requests); it counted issue *directories* rather than finished
`changes.patch` files, so in-flight work was offered as a candidate and a
whole pass printed `SKIP: no patch`; and it applied a filter from a
previous run that was never re-derived, reporting the backlog "drained" at
2 when it held 78. Each version was written in a hurry, looked right, and
produced a plausible number. None was reviewable, because none existed as
an artifact — they lived in shell history.

- **The predicate is an artifact, not an invocation.** A condition that
  decides what work to do is written down as a file: the scope (what the
  query may see at all) plus every exclusion, each with a short report
  label, a written condition, and a justification. Construction refuses an
  unnamed or unjustified condition — the comment block matters as much as
  the code, because every condition of the conversion selector is a bug
  that was actually shipped, and written down they stop being
  rediscoverable:

  ```
  #   1. scoped to ONE project's out/          (issue numbers collide across projects)
  #   2. a NON-EMPTY changes.patch exists      (the agent actually finished)
  #   3. no open or merged PR                  (CLOSED is an abandoned attempt, not a conversion)
  #   4. the issue is still OPEN               (a patch for a closed issue is moot)
  #   5. not already attempted                 (unless --retry-held)
  ```

- **The selector reports its denominator.** The pass prints
  `considered=423 finished_patches=423 have_pr=314 closed_issue=265
  attempted=232 -> candidates=0`: every exclusion reports how many items
  it removed, and the line always leads with `considered=`. A bare
  `candidates=0` is unfalsifiable — it cannot be told from "the query was
  never run" or "the filter is broken". With the denominator, a zero is
  *evidence the backlog is drained*, which is a different and much more
  useful statement (the #3992 rule applied to the selection step rather
  than the execution step). `considered=0` is neither: it is "the query
  never ran or the scope matched nothing", and it is never rendered as
  "drained".

- **Loop specs define the selection predicate as precisely as the
  action.** Every autonomous loop has a selection step and an execution
  step. Specs describe the execution step reliably — "convert each
  candidate patch to a PR" — and leave the selection step as an English
  phrase ("any new patches"), which is where the defects live, because the
  phrase is re-interpreted on every run. A spec for a loop must enumerate
  the conditions that decide what work to do, what is excluded and why, and
  it must require the implementation to report how many items each exclusion
  removed.

Checkable in `autospec_core::work_selection` (`SelectionSpec`, `Exclusion`,
`SelectionReport::line`, `SelectionReport::verdict`, `SelectionVerdict`).
Tests: `crates/autospec-core/tests/work_selection.rs`, including the
regression that reproduces the issue's report line verbatim and the three
shipped defects instantiated.

## Measured thresholds and capped destructive automation (issue #4276)

A watchdog built on a default constant and a "no output" signal killed 45%
of a fleet's healthy agents: the threshold (75 min) sat at the *median* of
measured run durations (4077 s; 190 of 418 runs longer than 75 min), not
the tail, because it was derived from the runner's `LIMIT:-2700` default —
while the dispatcher's call site passed `LIMIT=25200` (7 h) — and the "no
output for 15 min" signal is confoundable by buffered output (the same
family of error as #4259). A threshold at the median is not an outlier
detector: it is a coin flip applied to healthy work.

- **Measure the distribution before building the detector.** Compute the
  median and p90 of the metric the threshold applies to over real runs, and
  log the threshold against them before the detector is built. A threshold
  at or below the median is a defect, not a configuration choice
  (`ThresholdAudit::placement`, `is_outlier_threshold`). The regression
  test reconstructs the incident's distribution (418 runs, median 4077 s,
  p90 13939 s, max 25201 s, 190/418 > 75 min) and asserts the 45%.
- **Read the caller before trusting a default.** A value found in the
  callee's signature (`LIMIT:-2700`) is a default until a call site sets
  it. The operating value is what the caller passes; the default is
  operating only when no caller overrides it (`LimitProvenance`).
- **Prefer a detector that normal operation cannot confound.** "No output
  for N minutes" is confoundable — buffered output, slow writes, quiet
  phases all produce silence. "Past the limit its own dispatcher set and
  still in the call" is not, *given the limit is the operating value*, not
  a default the dispatcher overrides (`confoundable`).
- **Cap destructive automation, log it, and treat the cap as a
  measurement window.** Every kill is logged with its position in the cap
  (`KillLedger`); a zero cap is rejected. A cap hit whose kills sit at or
  below the p90 of normal work is a detector defect, not a fleet defect
  (`cap_verdict`): a detector calibrated on the tail should not exhaust its
  cap, and a cap hit is a signal to stop and re-measure — never a reason to
  raise the cap.
- **Asymmetric cost, asymmetric thresholds.** Killing a healthy agent costs
  a lost run; a wedged agent costs a slot for a bounded time. The
  threshold belongs far out in the tail — at the limit the system itself
  enforces, not a guess — and the build gate refuses a destructive detector
  whose threshold does not exceed the operating limit: a watchdog that
  sits below the limit its dispatcher sets is racing that timeout, and
  fires before the timeout's own verdict
  (`build_gate`, empty refusals = may build).

Checkable in `autospec_core::threshold_calibration` (`Distribution`,
`ThresholdAudit`, `LimitProvenance`, `confoundable`, `build_gate`,
`KillLedger`, `cap_verdict`). Tests: `crates/autospec-core/tests/threshold_calibration.rs`.

## Extract on the second implementation, not the fourth (issue #4289)

The same guard was written four times across sibling files — a timeout
wrapper in two conversion passes, a "the child owns the write" note in
two runners — and each copy was discovered in a separate debugging
session. The copies were siblings in every way that matters: names that
share a domain stem (`convpass`/`iwconv`), guard comments citing issue
numbers that appear in only one sibling, and shared strings that say the
same fact twice. Extraction happened only when the fourth copy forced
the argument; the rule is that the *second* implementation of a guard is
the extraction point, and deferral past it has a cost that is counted,
not assumed.

- **The second implementation is the extraction deadline.** A guard with
  two or more implementations is due for extraction
  (`extraction_due`, `SECOND_COPY`); the ledger names the deferral and
  its cost from the second copy on
  (`CopyLedger::line` → "extraction was due at copy 2 and N copy(s)
  were deferred at C each"). The deferral cost is `(copies − 2) ×
  copy_cost` — it multiplies the copy cost, never the extraction cost
  (`CostModel::deferral_cost`).
- **Sibling files are named, not assumed.** Two files are sibling
  candidates when their extension-stripped names share a common
  substring of at least 4 characters (`name_candidates`,
  `longest_common_substring_len`, `NAME_STEM_MIN_LEN`).
- **A guard comment citing an issue number in one sibling is a
  coverage gap in the others.** Citations are read from comment lines
  (`guard_citations`); an issue cited in at least one but not all files
  of a set is a gap naming every file that misses it (`GuardGap`,
  `guard_coverage`).
- **Grep the tree before writing a one-file fix.** A change that touches
  exactly one file while the tree contains sibling candidates is
  `SingleFile { siblings }` and renders as a `WARN:` naming the
  siblings that need the same fix (`fix_scope`); a change touching two
  or more files is `Broad` and passes.

Checkable in `autospec_core::guard_extraction` (`extraction_due`,
`CostModel`, `CopyLedger`, `name_candidates`, `longest_common_substring_len`,
`distinctive_tokens`, `sibling_pair`, `guard_citations`, `guard_coverage`,
`fix_scope`, `audit`). Tests: `crates/autospec-core/tests/guard_extraction.rs`.

## CI name drift: the steps are the contract, not the name (issue #4197)

A job's display name is documentation; its steps are the contract. The
incident: a conversion gate was built to "match CI" by reading a CI job's
name — `"Next.js baseline (lint / typecheck / build)"` — to learn what it
runs. The name enumerates three of the job's four steps and omits `test`, so
the gate derived from the name ran lint, typecheck and build and silently
skipped the test step: 68 tests never ran, and nothing warned, because
nothing compared the name to the steps it claimed to list.

- **Never derive behaviour from a label.** The only source of a job's
  commands is its `run:` steps, in order (`CiJob::command_list`). The name
  is documentation that may be wrong and is never parsed for commands.
- **A local gate is generated from the CI definition, not restated beside
  it.** `gate_matches_job(gate, job)` is the mechanical assertion "does my
  gate's command list equal the job's step list?". A gate missing a step is
  `GateDrift::OutOfSync` with the skipped step named; its line is a `WARN:`,
  so the check is an assertion, not a habit.
- **An enumerating name is tested against what it enumerates.**
  `name_enumeration` parses the last `(...)` group out of the name; `name_matches_steps`
  asserts the list matches the steps' names (case-insensitive, order-
  insensitive). A name that omits a step is `NameDrift::OutOfSync` with the
  omitted step named — the check that would have caught the incident.
- **When a gate is extended, re-derive the name — do not edit it.**
  `derive_name` regenerates the name from the steps, in order; `name_is_stale`
  is `true` when a hand-edited name no longer equals what the steps derive
  to. `audit(gate, job)` is the lint a gate-definition change triggers, and a
  drifted name is a `WARN:` on the same pass the breakage is introduced —
  never a silent re-version.
- **Regression tests run in the configuration the bug required.** The bug
  required an enumerating name that omits a step; on a job whose name
  matches its steps every check agrees and the bug is invisible. `tests/ci_name_drift.rs`
  instantiates the drifted-name job (the incident) and a matching-name
  control that proves the checks are not false-positiving.

Checkable in `autospec_core::ci_name_drift` (`CiJob::command_list`,
`gate_matches_job`, `name_enumeration`, `name_matches_steps`, `derive_name`,
`name_is_stale`, `audit`). Tests: `crates/autospec-core/tests/ci_name_drift.rs`.

## Name scope and collision (issue #4251)

An identifier is only meaningful inside the scope that issued it. Four
collisions made that concrete: a worker glob `qwen3.8-27b-*` that also
matched `qwen3.8-27b-vision-*`; a pipeline collecting `$LLM/*/out/issue-*`
across four projects where `issue-1` meant four different things; two
instances (edge and hive) both called `gateway`; and two components named
`gateway` in two repositories — one a directory, one a crate, one a service,
one a repository.

- **A glob collides when its literal part sits in one known identifier's
  name territory while reaching a different known identifier that continues
  it at a component-name boundary.** The boundary is the separator set
  (`NAME_SEPARATORS`: `-`, `_`, `/`, `.`), not a digit: `issue-1` vs
  `issue-14` is clean, `qwen3.8-27b` vs `qwen3.8-27b-vision` is not
  (`prefix_collisions`, parameterized separators).
- **A bare integer is never a cross-scope key.** Records keyed on a number
  flag unscoped records and numbers that appear under ≥2 distinct scopes —
  the four-project `issue-*` pipeline (`key_findings`). The qualified form
  is `Scope#14` (`ScopedId`, `parse_scoped_id` rejects a bare number, `qualify`).
- **A name used for ≥2 kinds of system object (directory, crate, service,
  repository) is probably two things** (`cross_kind_collisions`; two kinds
  is the sensitivity — one kind is just a name).
- **Evidence carried across instances must name its producer**
  (`CarriedClaim::verdict` — `Unattributed` is a finding, `Attributed` is
  clean; `Local` for in-instance use).
- **Specs encode scope explicitly.** A spec naming a component states where
  else that name is used (`unstated_name_uses`); a spec moving records
  between systems uses scope-qualified identifiers, never a bare key
  (`SpecMove::verdict` — `BareKey` over ≥2 systems is a finding).

Checkable in `autospec_core::name_scope` (`glob_matches`,
`prefix_collisions`, `ScopedId`, `key_findings`, `cross_kind_collisions`,
`unstated_name_uses`, `CarriedClaim`, `SpecMove`). Tests:
`crates/autospec-core/tests/name_scope.rs`.

## Self-gating of gatekeeping automation (issue #4263)

The watchdog reaped in queue order under a cap (two agents at 4h01m and
3h51m survived three sweeps while younger offenders were killed in each —
the cap was consumed by whoever came first in the `squeue` listing, and the
worst offenders were never at the front); the conversion selector keyed
"already attempted" on the **issue**, so a new patch for an issue whose
earlier patch had been attempted was never selected, and it counted **issue
directories** instead of patches, reporting "15" where the true population
was 11 issues / 14 patches / 15 directories. The predicate for all of it
lived in a one-line `jq` in a scratch-path script that died with the
session, and the whole stack was trusted because it had been running for a
while. Five rules, each checkable:

- **Destructive scripts need a dry-run mode, and the first production run
  needs a dry-run first.** `gate_destructive_run` refuses a destructive
  script that has no dry-run flag, and refuses the first production run
  until a dry run has been performed; a dry run itself is never refused.
- **Selection predicates report the denominator and live in a reviewable
  file.** `SelectionReport::line()` renders "N of M selected" — the
  denominator is never optional, and a numerator above the denominator is a
  counting bug, not an edge case. A report that is not written to a file
  has no home and is a finding.
- **Automation that decides work is an artifact.** `AutomationArtifact`
  requires a file (an inline predicate has no home), a comment per
  condition explaining the bug it encodes, and one fixture test; each
  missing element is a separate finding.
- **Cap the blast radius, and make the cap bind visibly.**
  `select_reaps(candidates, cap)` returns a `ReapPlan` that names every
  deferred offender; a zero cap is an error, not a silent pass, because a
  destructive reaper with nothing to fill is a configuration to surface.
  Selection is by severity — most seconds over the limit first — never by
  listing order, or the cap starves exactly the offenders it was meant to
  reach (queue order starved the two worst offenders across three sweeps;
  severity order selects them first).
- **Recency is not reliability.** `trust_verdict` returns true only for
  `ReliabilityEvidence::Gated`; `RanRecently { times }` is not trust
  evidence at any count, because a script that has been running for a
  while is exactly the one nobody re-reads.
- **"Attempted" keys on the patch, not the issue** (`AttemptLedger`). An
  attempt is a fact about the patch it was made on, not a verdict on the
  issue; a new patch for an attempted issue is still a candidate.
  `selector_line` reports "attempted N of M patches (K issues)" — patch
  count primary, issue count recorded separately, so a unit mix-up is
  visible in the line itself.
- **A directory is not a candidate** (`WorkState`). Directories are created
  when an agent *starts*; patches are what an agent may never produce.
  Only `HasPatch` is a conversion candidate — `InProgress` and `Converted`
  are not.

Checkable in `autospec_core::self_gate` (`gate_destructive_run`,
`SelectionReport`, `AutomationArtifact`, `select_reaps`, `trust_verdict`,
`AttemptLedger`, `WorkState`). Tests:
`crates/autospec-core/tests/self_gate.rs`.
## Swallowed arguments and env-scoped runs (issue #4292)

`convselect.sh` was scoped to a project through environment variables
(`R=` / `OUT=` / `SEEN=`) and its argument loop had no `*)` branch:
`convselect.sh iw` was accepted, ignored, and scoped to autospec anyway.
Both invocations printed byte-identical counts — `considered=100
finished_patches=3 closed_issue=0 candidates=0 retry-held=2` — and that
identity is what hid InferWeave's real candidate (issue #288): the number
was right and the input was wrong. A parser that swallows its arguments
reports the world of whatever scope it defaulted to, and nothing in its
output says which.

- **Every argument parser has a `*)` catch-all.** Unknown arguments are
  an error the caller turns into a non-zero exit, never an ignored value.
  `StrictParser::parse` returns `Err(Rejection)` on the first unknown
  argument — the shell equivalent is the `*) echo "…" >&2; exit 2;;`
  branch.
- **The rejection names the correct mechanism, not just the error.** A
  message that says "unknown argument 'iw'" tells the caller there is a
  problem; one that says "scope with R=/OUT=/SEEN= env vars, not
  positionally" tells them the fix. `validate_rejection_message` returns
  a finding for a message that omits either the offending argument or
  the mechanism, and `Rejection::line` builds the adequate one.
- **Identical output across distinct inputs is a defect.** Two runs that
  should differ — different projects, different scopes — producing
  byte-identical output proves one input did not reach the computation.
  `identical_output_pairs` flags runs under distinct scopes with equal
  output; identical output under the *same* scope is an idempotent
  re-run, not a finding. `per_project_findings` is the combined check
  worth an explicit run anywhere a tool is invoked per-project in a loop.
- **A scoped tool prints its scope on every run.** The scope is a fact
  about the output, not an assumption the reader makes about it.
  `EnvScope::line` renders `scope: K1=V1 K2=V2 …` (key-sorted, so
  deterministic) and `reported_in` / `unreported_scope_runs` check it
  appears on a line of the run's output.

Regression tests run in the configuration the incident required: the
per-project loop with a swallowed positional argument, byte-identical
counts, and no scope line — `tests/argument_scope.rs` reproduces the
loop and asserts the three findings (one identical-output pair, two
unreported scopes), and asserts the fixed loop (env-scoped, scope
printed, `candidates=1` with `288`) produces none. Checkable in
`autospec_core::argument_scope` (`StrictParser`, `Rejection`,
`validate_rejection_message`, `EnvScope`, `identical_output_pairs`,
`per_project_findings`). Tests:
`crates/autospec-core/tests/argument_scope.rs`.

## Merge-gate invariants for the converter (issue #4307)

The patch-to-PR converter (`convpass.sh`) ran a local gate — fmt, build,
clippy, test on one host — and merged on its result. The local gate covered
three of the six jobs the repository's `rust-suites` CI runs; it never
reached macOS, Windows or FreeBSD. A macOS-gated test file (#4306) with five
compile errors sat merged and hidden, because the workflow's pass rate on
`main` was 0/40 — a red gate read as a green one, and a merge record that
said nothing about the three jobs it did not check read as if it had.

- **The local gate is a pre-filter, not the gate.** The converter's merge
  requires the repository's real CI to have passed for the PR. A local-gate
  failure is a refusal (cheapest check first), but a local-gate pass is not a
  CI verdict: `Pending`, `Failed` and `NotRun` are all `CiNotPassed`, and
  only `Passed` approves. The fold "local pass ⇒ merge" is the incident, and
  it is now a named decision variant rather than an implicit default.
- **The merge record names what it did not check.** `Coverage` splits the
  workflow's jobs into `checked` and `unchecked` in workflow order, and
  `record_names_unchecked` refuses a merge record that does not name every
  unchecked job id verbatim. "Gate passed" with three jobs silently missing
  is a refusal, not a default — absence within the record is a gap, not an
  all-clear.
- **Refuse while the target workflow is red on the base branch.** A merge
  into a base where `rust-suites` is failing is `BaseBranchRed { failing_jobs }`
  — the failing jobs named — even when the local gate and the PR's own CI are
  green. The incident's 0/40 `main` would have blocked every conversion until
  fixed, instead of papering over it on every merge.
- **The pass rate is a number that is reported, not a feeling that is
  assumed.** `PassRate` carries the counters and renders `workflow on
  branch: N/M passed (P%)` via `line()`; `all_failing` flags the 0/40 state
  explicitly. A gate whose every run fails is flagged, never smoothed into
  "mostly passing", and a rate with no runs is not all-failing — there is
  nothing yet to fail.

Checkable in `autospec_core::merge_gate` (`decide`, `MergeDecision`,
`Coverage`, `record_names_unchecked`, `PassRate`). Tests:
`crates/autospec-core/tests/merge_gate.rs`.

## Prose closure safety (issue #4305)

A converter marked its PR `Refs #288 (does not close it)` — the trailer
said the issue stays open — but line 11 of the same body said `closed #288`
in prose, and GitHub closed the issue on merge. GitHub's closing semantics
fire on *any* closing keyword adjacent to a reference in the body, not only
on trailers; a trailer is a statement of intent, and prose is a live
directive. The body contradicted itself, and the contradiction was
machine-detectable before publish. Four invariants, one checkable each:

- **A closing keyword never sits adjacent to the reference in prose**
  (`prose_violations`). A closing directive is a closing verb from the
  shared `CLOSING_VERBS` list (`close`, `closes`, `closed`, `fix`, `fixes`,
  `fixed`, `resolve`, `resolves`, `resolved`) followed, after optional
  whitespace, by the reference to *this* issue — `find_closure_directives`
  finds every one in the body, with a digit-suffix guard so `closed #2883`
  is not a match for `#288`, a word-boundary guard so `unclosed #288` is
  not a match, and case-insensitive verb matching. The one sanctioned spot
  is a closing trailer on the final non-empty line; everything else is a
  violation.
- **Discussing closure means escaping the reference or using a full URL**
  (`escaped_reference`, `url_reference`). `&#35;288` and
  `https://github.com/owner/repo/issues/288` render as references to
  humans and carry no closing semantics to the platform; `ReferenceForm`
  classifies each directive `Bare` / `Escaped` / `Url` and only `Bare`
  (`is_live()`) is a live directive. Escaped-entity matching requires the
  semicolon (`&#3512` is entity 3512, not entity 35 plus `12`) and
  accepts hex (`&#x23;288`).
- **A `Refs` decision asserts the issue is still open after merge**
  (`verify_after_merge`). The converter's trailer is a claim about the
  issue's post-merge state; a `Refs` body with the issue observed `Closed`
  is `PostMergeAction::ReopenIssue`, not a silent success, and a `Closes`
  body with the issue still `Open` is `ReportTrackerLag` — each renders a
  `PostMergeCheck::line()` that fails loudly instead of aging into
  "fixed" (the merged-vs-fixed discipline of the Closeout report applied
  to issue state).
- **The contradiction is linted before publish** (`lint_refs_body`,
  `pre_publish_lint`). Any live closing directive in a body the converter
  marked `Refs` is rejected by `lint_refs_body`; `pre_publish_lint` gates
  both decisions — `Closes` requires the closing trailer and no prose
  violations, `Refs` requires no live closing directives at all — so the
  check runs before the body is written, not after the merge.

Checkable in `autospec_core::prose_closure` (`find_closure_directives`,
`ClosingDirective`, `ReferenceForm`, `prose_violations`,
`escaped_reference`, `url_reference`, `has_closing_trailer`,
`is_trailer_line`, `lint_refs_body`, `pre_publish_lint`,
`verify_after_merge`, `PostMergeCheck`). Regression tests reconstruct the
incident body (`Refs #288 (does not close it)` on line 1, `closed #288`
on line 11) and assert it: `crates/autospec-core/tests/prose_closure.rs`.

## Toolchain gate: the pin is the source of truth (issue #4303)

A CI step that passes `toolchain: stable` to
`actions-rust-lang/setup-rust-toolchain` does not select a toolchain: it
overrides the one the repository declared in `rust-toolchain.toml`. This
repository pins 1.91.0 — the only toolchain on the HPC cluster the agents run
on, and the version every lint in the gate was written against — while `stable`
floats past it. The incident: the gate ran on `stable`, where two lints that
do not exist in 1.91.0 (`manual_checked_division`,
`truncating_to_zero_length`) fired, and the gate was red on **every** input.
A gate that fails on every input is not measuring anything; it is a defect
report about the gate, and the standing instruction "ignore clippy failures"
was the diagnosis nobody filed. Five invariants, all checkable:

- **The pin is the single source of truth.** A workflow step that names a
  channel different from `rust-toolchain.toml` is `TOOLCHAIN_GATE_DRIFT`,
  reported with the workflow path, the step id, and both channels. A step
  that names the pinned version exactly is not drift — the pin says 1.91.0
  and the step says 1.91.0, so they agree. A step that names no channel
  inherits the pin and is clean.
- **Drift is directional.** A step on a newer channel than the pin is an
  *upgrade made by accident* — it changes the toolchain with no decision
  record and no re-validation of the gate's lint set. A step on an older
  channel is the drift the repository no longer declares. The two have
  different remediations and must be reported differently.
- **A gate that has never passed is a defect report, not a quality
  signal.** `gate_standing` classifies a gate's run history: zero passes in
  N runs is `DefectReport { runs: N }` — the gate is broken, file it — while
  any pass is a `QualitySignal` whose green rate is the measurement. An
  unobserved gate (no runs) is `Unobserved`, never `DefectReport`.
- **A pin bump is a recorded decision, not an observation.** Changing
  `rust-toolchain.toml` from 1.91.0 to 1.93.0 without a decision record
  (issue, PR discussion, or commit message saying why) is an
  `UnrecordedBump`: the toolchain moved, the gate's lint set moved with it,
  and nobody decided. `pin_change_verdict` distinguishes `Unchanged`,
  `RecordedBump`, and `UnrecordedBump { direction }` — an unchanged pin is
  not a decision, and a recorded bump is.
- **A standing instruction to ignore a gate is a bug report.** A workaround
  note ("clippy failures are expected; ignore") with no filed issue is
  `UnfiledBugReport` — the workaround is the diagnosis, and the fix is to
  file it. With a filed issue (e.g. `InferWeave/inferweave#323`), the
  workaround is `Filed` and clean.
- **A channel this cannot classify is refused, not guessed.**
  `parse_channel` accepts `major[.minor[.patch]]`, `stable`/`beta`/
  `nightly`, and a `-target-triple` suffix on any of them
  (`stable-x86_64-unknown-linux-gnu` — the triple is ignored, because it is
  not the toolchain). Anything else — `1.91.0-beta.2`, `1.9.1.0`, `weird` —
  is an error, because a wrong guess here is a wrong toolchain running the
  gate, which is the incident this module exists to prevent. Same refusal
  discipline as #4192: name what would tell you.

Checkable in `autospec_core::toolchain_gate` (`PinFile::from_toml`,
`parse_channel`, `pin_drift`, `drift_line`, `gate_standing`, `standing_line`,
`pin_change_verdict`, `workaround_verdict`, `audit`). Tests:
`crates/autospec-core/tests/toolchain_gate.rs`, including the regression
that reconstructs the incident end-to-end: the drifting step, the
never-passed gate, the unfiled workaround, and the clean post-fix audit.
## Platform gates: a red platform job cannot be ignored (issue #4312)

A platform-specific CI job (`macos-test`, `freebsd-test`) is
the sole verification of the code behind its `#[cfg]` gate: when it goes red,
every line behind the gate merges unverified, and a single-run view cannot
tell a job that broke moments ago from one that has been red for days. The
#4306 macOS breakage hid for a day and #4311 repeated the same shape on
Windows; each instance was fixed at the code level, and the class is the
tooling that makes a red platform job impossible to ignore.

- **A platform job's failure is a coverage loss, not a flaky check.**
  `classify_failure` maps a failing job to `CoverageLoss { job, platform }`
  when the job is the sole CI verification of a platform surface and to
  `Ordinary` otherwise. The two render differently and escalate differently:
  a red `macos-test` is a total loss of macOS coverage, never "probably
  flaky, re-run it".
- **The alarm fires on the rate, not on individual runs.** `rate_alarm` fires
  when the gate's pass rate for the default branch is all-failing (0/N, N >
  0) — conspicuous on its own, no state change required — and names the lost
  coverage when the platform is known. It reuses `merge_gate::PassRate`
  rather than duplicating it; the empty window is unknown, not red.
- **The local gate is not authority for surfaces it cannot compile.**
  `local_authority` scans the patch for `#[cfg(...)]` attributes and returns
  a `Hold` naming every surface the host cannot compile and the job that
  solely verifies it: the merge defers to the named CI jobs. Over-reporting
  (a cfg in a comment) is the safe direction — an unnecessary hold, never an
  unverified merge; `cfg!(...)` is a runtime branch and is not a surface.
- **The parser claims only what it can classify.** `target_os`,
  `target_family`, the bare `unix`/`windows` families, and a single
  `not(...)` around them; feature flags, `target_arch`, `any(...)`/`all(...)`
  and unknown values return `None` — absence is the honest encoding, not an
  invented semantics.

Checkable in `autospec_core::platform_gate` (`parse_platform_predicate`,
`patch_surfaces`, `local_authority`, `classify_failure`, `rate_alarm`).
Tests: `crates/autospec-core/tests/platform_gate.rs`, including the
regression that reconstructs the incident (an all-failing 0/40 rate, a red
platform job, and a macOS-gated patch on a Linux host) and the control that
a plain patch never holds.

## Restore-visibility contract

Restoring a file is not the same as making the restoration visible to an
mtime-based build system. `mv` (and `cp -p`/`-a`/`--preserve`, `install -p`,
`tar -x`, `git stash pop`) put a file back with the backup's timestamp; if the
backup predates the last build of that input, the rebuild declines to run and
the stale artefact is wrong rather than merely old — an `include_str!`-style
embedding ships the corrupted value as if it had been rebuilt (issue #3878).

- Any harness that corrupts and restores a tracked file (test fixtures,
  recovery traps, session restore) must `touch` the restored path in the same
  scope, **including trap paths** (inline `trap '...' EXIT` bodies and
  functions referenced by `trap NAME`). A content rewrite (`> "$f"`) also
  observes the restoration.
- Every rebuild a test depends on is followed by an artefact-content
  assertion. The mtime gate is the producer, not the proof.
- The deterministic ratchet `scripts/lint-restore-visibility.sh` makes an
  unobserved mtime-preserving restore site blocking (`STALE_RESTORE`, exit 1;
  `--list` audits existing sites). The per-site escape hatch
  `# restore-visibility:allow <reason>` (same line or the line above the
  restore) is for restores that genuinely do not feed a build and requires a
  reason — a bare marker is rejected.

## Generated-artifact consumer contract

A generated artefact with two consumers needs two checks: regenerating it
satisfies one and silently breaks the other (issue #3893 — a `pi` row existed
in `config/harness-runtime-aliases.tsv` but was never propagated to the
committed `templates/generated/` / `docs/generated/` artefacts, so
`tests/harness-runtime-alias-generation.bats` broke on `main` with no check
pointing at the cause).

- Every committed generated artefact is enumerated **in its generator
  script**: `# Consumers(<artefact_path>): <consumer1> <consumer2> …`. The
  enumeration is the single source of truth for who reads the artefact and it
  surfaces in the diff whenever the generator changes.
- Every generator that writes committed artefacts also writes or checks a
  digest pin manifest (`config/generated-artifact-integrity.sha256`,
  `sha256sum` format). `scripts/gen-harness-runtime-aliases.sh --check`
  re-renders every artefact plus the manifest and fails on any drift between
  `config/harness-runtime-aliases.tsv` and the committed files.
- The deterministic ratchet `scripts/lint-generated-artifacts.sh` is blocking
  (exit 1; `--list` audits, `--root DIR` retargets) with four rules:
  `UNLISTED_ARTIFACT` (generated file with no `# Consumers(...)` block in any
  candidate generator), `ORPHAN_ARTIFACT` (enumerated artefact no longer
  present), `STALE_REFERENCE` (a recorded sha256/sha512 of the artefact no
  longer matches it), `STALE_CONSUMER` (an enumerated consumer file is gone
  or no longer references the artefact — by path or by directory+stem
  prefix, so parameterised `$format` references count).
- Registered as `check_generated_artifact_integrity`
  (`tests/lint/test_generated_artifact_integrity_checker.bats`), which
  includes the AC3 negative case: corrupt one recorded digest and the ratchet
  fires.
- Adding a generator or an artefact: add the `# Consumers(...)` lines, emit
  the digest pin manifest as a generator output, and re-run the generator.
  Adding or removing a consumer: update the `# Consumers(...)` list in the
  same PR — the ratchet makes a forgotten consumer blocking, not silent.

## Closeout report contract

Every `auto-implement` agent ends an issue by emitting a **Closeout report** —
appended to the PR body and printed to the monitor log. It is the structured,
result-first summary the merge-gate and the done-challenge consume as *evidence*.
Keep it terse: a tight body, long only where a claim genuinely needs it.

Required fields (exact field names are gated by `autospec validate`):

- **Result** — one line, outcome first (what shipped), not a narration of the
  agent's own process. Open with the result, not "I'll" / "Let me".
- **Claims** — each load-bearing claim carries one label: `[verified]` (the agent
  checked it itself), `[assumed]` (inferred or taken from another agent's report),
  `[couldnt-verify]`, or `[likely-wrong]`. Unlabeled load-bearing claims are a
  defect.
- **Proof type** — for each `[verified]` claim, `runtime` or `static`. **Runtime
  claims need runtime proof, not just a build/read.** A `[verified]` runtime claim
  backed only by static/build evidence is downgraded to `[assumed]` by the
  consumer (see below).
- **Before/after** — the measurable delta this change produced (test count, perf
  number, error rate, …) or an explicit `n/a — <reason>`. A before/after is the
  marker of real work; the field is mandatory (the reason may be `n/a`).
- **Artifacts** — exact file paths and a re-runnable command a reviewer can
  execute to reproduce the proof.
- **Scoped git status** — the files this issue touched (scoped, not a raw global
  status dump).
- **One likely hidden failure** — the single most probable thing still wrong. Not
  optional; "none" is itself a claim to be challenged.

Post-merge observation fields (optional at parse time, mandatory at review time
for control-plane changes — `autospec-core::post_merge`): merged and fixed are
separate states. A change whose effect is only observable after deployment
(CI configuration, dispatch policy, merge automation) records its confirmation
as part of the change:

- **Post-merge observation** — `Post-merge observation: <query, log, or metric> —
  expected: <value>`, the specific observation that confirms the change took
  effect, recorded in the closeout so the confirmation cannot be lost with the
  session that made the change.
- **Follow-up check** — `Follow-up check: <what gets re-checked and when>`, the
  scheduled re-look. Control-plane changes with no follow-up check are flagged
  at review: the re-look is left to chance, which is exactly how a merged fix
  ages from incident into "fixed" without ever being checked.

While the observation is unrecorded the `Result:` line must say *merged*, not
*fixed* or *resolved* — the closeout validator rejects a fixed/resolved claim in
a closeout that declares a post-merge observation. The state becomes *fixed*
only once the observation is recorded and matches the expected value
(`post_merge::confirm_fixed`); a contradicting observation keeps it *merged*.
A fix that declares no observable effect at all is flagged at review, not
silently accepted.

### Consumer contract (critic / merge-gate)

The merge-gate and the autospec-run done-challenge treat the Closeout report as a
**claim, not proof**:

- Record the Closeout report as merge evidence (alongside the full-suite passing
  summary) — never merge on a closeout the agent did not actually emit.
- Re-read the cited artifacts; do not accept a closeout's word for them.
- **Reject (or downgrade to `[assumed]`) any `[verified]` runtime claim whose
  proof type is `static`/build-only.** This is the one machine-checkable critic
  predicate.

The judgment-bound discipline that cannot be gated, applied throughout an issue:
state the blast radius before any global/destructive action; stay in the issue's
scope and park unrelated findings as follow-up issues rather than expanding the
diff.

## Factual-claim lint

A comment that asserts an externally checkable fact — registry visibility,
package publicity, a host's or worker's capability, network reachability of a
registry or host — decays silently: it was true when written and nothing
re-checks it. A comment cannot fail, so a fact the code depends on must be one
of: executed (the same file carries a runtime check for that fact category),
dated and sourced (`# verified public 2026-09-06 by anonymous manifest GET
(200)`), or waived (`# linter:allow-FACTUAL_CLAIM <reason>`, reason mandatory).
Intent ("publish must not depend on an unproven property") stays true and
belongs in a comment; a bare assertion does not. Enforced by
`scripts/lint-factual-claims.sh` (no args scans `scripts/` and
`.github/workflows/`; exit code = finding count, capped at 64); fixtures and
suites live in `tests/fixtures/lint-factual-claims/` and
`tests/lint-factual-claims.bats`.

Agent reports separate **observed** from **reasoned** claims: any claim that
changes control flow (a gate, a dependency, a retry policy) must be in the
observed set — `[verified]` with `runtime` proof in the Closeout report —
while reasoned claims are `[assumed]` and never gate a decision.

## Test-failpoint scope lint

`scripts/lint-cfg-test-statics.sh` rejects new process-global mutable test
state in the `executor_bridge` tree (issue #3951): a `#[cfg(test)] static`
declared with `Atomic*`, `Mutex<T>` (T ≠ `()`), `Cell`, `RefCell`,
`UnsafeCell`, `OnceCell`, or `RwLock` is a finding, because parallel
`cargo test` threads collide on consume-once state. `thread_local!` statics,
`Mutex<()>` unit locks, and constants are exempt; each exemption and waiver
prints an `INFO:CFG_TEST_STATIC:...` audit line. Exit code = finding count
(capped at 64). Waiver: `linter:allow-CFG_TEST_STATIC <reason>` on the static
line or the line immediately before it — a bare marker is rejected.
Default scope is `crates/autospec-cli/src/commands/autonomous/executor_bridge*`;
explicit `.rs` files or directories can be passed as arguments. Bats:
`tests/lint-cfg-test-statics.bats`. Not yet wired into
`lint-implementation.sh` (follow-up).

## Scratch-promotion contract

Supervision tooling has a lifecycle problem: it accumulates in scratch paths
(`/tmp/…`) with no test, no owner, and no version control, then either rots or
gets re-derived on every session. The invariant (issue #3977): a helper that is
*invoked* from a scratch path more than twice is a tool, not a throwaway — it
must be promoted into the repo (gaining a test and an owner) or explicitly
discarded. The promote-or-discard decision is a process step a human or the
agent makes; the gate only reports the candidate, it never mutates.

`scripts/lint-scratch-promotion.sh` is the deterministic ratchet (RULE_ID
`SCRATCH_PROMOTION`). It is a read-only, static, token-level scanner over shell
and bats files. Default corpus is `scripts/` and `skills/`; pass explicit
`PATH…` args to sweep a supervision session log or any other tree. `tests/` is
excluded by default because its fixtures deliberately contain scratch
invocations. It counts, per distinct scratch tool path, how many times it is
*invoked* — in an invocation position (command start, or immediately after a
launcher: `bash sh zsh dash ksh ash python python2 python3 perl ruby node nodejs
source exec env sudo nohup xargs time nice stdbuf command .`). Scratch prefixes
are `/tmp/`, `/var/tmp/`, `/private/tmp/`, `${TMPDIR}/`, `$TMPDIR/`; recognized
tool extensions are `.sh .bash .py .bats .rb .pl .js .mjs .ts`. Paths carrying
`XXXXXX` or `$$` (mktemp templates) are exempt, and data-only references (`rm`,
`mv`, `>`) are not invocations. A tool invoked more than twice across the corpus
is a finding: `SCRATCH_PROMOTION:<file>:<line>: <tool> invoked <N>x from a
scratch path (first seen here): promote it into the repo or discard it`.
`--list` emits an `SCRATCH_TOOL:<tool>: <N>x` audit line per scratch tool and
always exits 0; blocking exit code = finding count (capped at 64).

The ratchet resolves no variables — `bash "$conv"` is invisible to it (static
text scan, not an interpreter); a tool must be invoked by a literal scratch path
to be counted. Waiver: `# linter:allow-SCRATCH_PROMOTION <reason>` (reason
mandatory) on the invocation line or the line immediately before it. Wired into
`scripts/self-enforce-qa.sh` as step 4 (WARN+skip if the script is absent), so
it enforces in the Phase 4 QA chain. Bats: `tests/lint-scratch-promotion.bats`,
fixtures under `tests/fixtures/lint-scratch-promotion/` (including the populated
#3793 case).

## Stdin-worklist loop lint (issue #3742)

A `while IFS= read -r x; do …; done < worklist` loop runs its entire body with
stdin pointed at the worklist file. Any command in the body that *reads stdin*
(`gh`, `ssh`, `cargo`, a `$(…)`, or `git`'s stdin-reading subcommands
`commit`/`apply`/`fast-import`/`hash-object`/`rebase`/`am`) consumes the file,
so the loop silently processes one candidate and exits 0. The fix is a
dedicated file descriptor: `while IFS= read -r x <&3; do …; done 3< worklist`.

`scripts/lint-stdin-worklist-loops.sh` is the deterministic ratchet (RULE_ID
`STDIN_WORKLIST_LOOP`). It is a read-only, single-pass, token-level AWK scanner
over shell files. Default corpus is `scripts/` and `skills/`; pass explicit
`PATH…` args, or `--root DIR`, to sweep another tree. For each `while`/`for`/
`until`/`select` loop whose `done` reads its worklist from FD 0 (`done <`, but
not `done 3<`/`done <&3`/`done < <(…)`), it scans the *direct* body for a
stdin-consuming command at command position — including inside `$(…)` (a command
substitution inherits the loop's stdin, verified empirically) and behind a
`GH_*`/`GIT_*`/`CARGO_*`/`SSH_*` variable alias or a `sudo`/`time`/`nice`/
`nohup` launcher. A nested loop with its own input is not charged to the outer
loop. A loop whose `git` subcommand does not read stdin (`log`, `ls-files`,
`rev-parse`, `ls-remote`, `grep`, …) is not a finding. Finding: `STDIN_WORKLIST_LOOP:<path>:<line>: <kind> loop (started line <N>) runs '<cmd>' at line <M> while reading its worklist from stdin; read the worklist on a dedicated FD (done 3< worklist, read <&3)`. `--list` emits an `FD0_LOOP:<path>:<line>: …` audit line per FD-0 loop and always exits 0; blocking exit code = finding count (capped at 64).

The counting half of the fix lives in the Rust conversion policy:
`autospec_core::execution::patch_pipeline::reconcile_phase_counts(candidates_produced, candidates_processed)` (rule #17 in the module) is the pure, testable primitive that the patch-to-PR conversion pass calls after its expensive-gating phase; it returns an Ok summary on equality and an `Err` (warn + non-zero exit at the caller) when processed ≠ candidates. Bats: `tests/lint-stdin-worklist-loops.bats`, fixtures under `tests/fixtures/lint-stdin-worklist-loops/`.

## Memory management scripts

Scripts for managing project memory files under `AUTOSPEC_MEMORY_DIR`
(`~/.claude/projects/-Users-<user>-IdeaProjects-autospec/memory/`).

| Script | Purpose | Flags |
|---|---|---|
| `scripts/memory-tags.yml` | Manifest mapping each `feedback_*.md` to 4–5 tags | — (data file) |
| `scripts/apply-memory-tags.sh` | Idempotent tagger: prepends `tags:` YAML frontmatter to each `feedback_*.md` | `--dry-run`, `--memory-dir DIR`, `--manifest FILE` |
| `scripts/install-implementer-precommit.sh` | Installs blocking pre-commit lint hook into an implementer worktree | `<worktree-path>` positional arg |
| `skills/autospec-shared/scripts/mempalace-compress.sh` | AAAK compression GC: wraps `mempalace compress`; no-op below LOC threshold | `--dir DIR`, `--threshold LOC`, `--dry-run`, `--quiet` |
| `skills/autospec-shared/scripts/mine-pr-history.sh` | Extract lessons from merged PR descriptions into `docs/memory/lesson_*.md` | `--repo OWNER/REPO`, `--output-dir DIR`, `--quiet` |
| `skills/autospec-shared/scripts/inject-relevant-memory.sh` | Grep/search `docs/memory/*.md` for keyword matches; emit top-k context block for skill prompt injection | `--context KEYWORDS`, `--top-k N`, `--memory-dir DIR` |

`AUTOSPEC_MEMORY_DIR` — override for the memory directory path (default: auto-detected
from `$HOME/.claude/projects/`).

## Auto context rollover skills

### autospec-session launcher (`scripts/autospec-session`)
Starts the `autospec_context_monitor` daemon in the background for a given tmux session.
The daemon monitors context percentage and fires compact/handoff/clear/resume actions automatically.
Stop it with `kill $(cat ~/.autospec/context-monitor.pid)` or by ending the tmux session.

- `/autospec-rollover-status` — reports current context % and last rollover event for the active session (see [`docs/specs/2026-05-31-auto-context-rollover-design.md`](docs/specs/2026-05-31-auto-context-rollover-design.md)).

`AUTOSPEC_COMPRESS_THRESHOLD` — LOC threshold for `mempalace-compress.sh` (default: `5000`).
`AUTOSPEC_COMPRESS_EVERY` — invoke compress every N calls to `auto-init-memory.sh` (default: `10`).
`AUTOSPEC_MINE_PR_HISTORY` — set to `1` to enable PR history mining in `auto-init-memory.sh` (off by default; bandwidth-heavy).
`AUTOSPEC_MINE_MIN_BODY` — minimum PR body length for `mine-pr-history.sh` (default: `200`).
`AUTOSPEC_MINE_LIMIT` — max PRs to scan per `mine-pr-history.sh` run (default: `200`).

## Pre-commit lint hook

`scripts/install-implementer-precommit.sh <worktree>` writes `.git/hooks/pre-commit`
into the given worktree. The hook runs `lint-implementation.sh --pre-commit --staged`
on `git diff --cached` and blocks commits containing RULE_ID violations.

`lint-implementation.sh` extended flags:
- `--pre-commit` / `--staged` — read staged diff (`git diff --cached`) instead of a PR diff
- `--directives` — reformat each finding as `Fix RULE_ID: <imperative action>` for use in implementer retry prompts

## CI-wait sentinel

Replaces synchronous `gh pr checks --watch` with a fire-and-forget background poller.

| Script | Purpose | Flags |
|---|---|---|
| `scripts/ci-wait.sh` | Spawns background CI poller; returns immediately | `<PR>`, `--timeout SECONDS`, `--required-only` |
| `scripts/ci-wait-poll.sh` | Reads sentinel; returns state as exit code | `<PR>` |
| `scripts/ci-wait-cleanup.sh` | Kills poller; removes sentinel files | `<PR>` |

Signal file: `~/.autospec/ci-state/<PR>.signal` — JSON `{pr, state, checks, settled_at}`.
State values: `pending | pass | fail | stalled`.
Exit codes from `ci-wait-poll.sh`: 0=pass, 1=fail/stalled, 2=pending, 3=no sentinel.

## Batch size policy

Default `AUTOSPEC_BATCH_SIZE=1`; force batch=1 when the next ready issue is `reasoning:deep` (high blast-radius work runs one-at-a-time per monitor session). This only ends the current monitor batch: `autospec-run` must automatically relaunch fresh monitor batches until the queue is `ALL_DONE`.

## Memory inventory

Persistent cross-session memory lives at [`docs/memory/`](docs/memory/).
Index: [`docs/memory/MEMORY.md`](docs/memory/MEMORY.md).

Memory types (mempalace wings):
- **semantic** — codebase facts, architecture, conventions
- **episodic** — session diary (`docs/memory/diary/`), in-flight project status
- **procedural** — playbooks, runbooks, recipes (also see SKILL.md files)
- **synthesis** — lessons learned (feedback patterns, anti-patterns, gotchas)

Read memories relevant to your task at session start. Write new memories by adding/editing files in `docs/memory/` and updating the index. Mempalace MCP layer (`mempalace search`, `mempalace traverse`, `mempalace kg_query`) is available if your tool supports MCP.

## Git hygiene (agents)

These rules apply to every autospec skill that mutates the repository (run
implementers, define spec-PRs, doc regenerate commits, explore sandbox,
release). The enforcement tool is `scripts/worktree-guard.sh`.

### Primary checkout is read-only for agents

Agents MUST NOT `cd`, `git checkout`, or `git commit` in the primary checkout.
Operator dirt in it is never touched and never matters. Every git-mutating step
happens in a linked worktree, never in the primary checkout directory.

### Fetch-before-branch

Always run `git fetch origin` (with one automatic retry) before creating or
adopting a branch. `worktree-guard.sh create` handles this automatically;
callers that bypass `create` must fetch explicitly.

### Fresh-or-verified-clean worktrees only

Use `worktree-guard.sh create` to obtain a worktree. Before any edit or commit,
call `worktree-guard.sh assert`; it MUST exit 0. A non-zero exit means the
worktree is dirty, primary, or stale — stop work, comment on the issue, and
restore `auto-implement`. Never force-reuse a dirty worktree.

### PR-aware ladder (standard branch-exists behavior)

Before creating a new worktree, call `worktree-guard.sh resolve-branch`:

- `open-pr` — a PR already exists for this branch: validate + merge the
  existing PR; skip re-implementation (#886 recovery).
- `branch-only` — branch exists on origin but no open PR: adopt it in a fresh
  worktree and continue (#917 recovery).
- `fresh` — nothing exists: `worktree-guard.sh create` off `origin/main`.

### Resetting a worktree (guarded, detached)

Never run a bare `git -C <wt> checkout ... && git -C <wt> reset ... && git -C <wt> clean ...` sequence by hand: an unguarded chain plows on after the first failure (e.g. `checkout -f main` fails when a sibling worktree holds `main`) and can move a local branch off its base commit (#3653). Use `worktree-guard.sh reset --path <wt> [--base <ref>] [--clean]` instead. It fetches, parks the worktree DETACHED at the base tip via `git checkout --detach` (it never names a shared branch, so a sibling holding `main` cannot fail it), checks every step and stops at the first failure (exit 7 mid-unit), and asserts the detached HEAD state after each step. Exit codes: 3 primary, 4 dirty (without `--clean`), 5 unknown/stale base ref, 7 mid-unit failure.

### Cleanup after merge + prune

After a PR is confirmed merged, remove the worktree and prune the git metadata:

```bash
git worktree remove /tmp/wt-<branch> 2>/dev/null || true
git worktree prune
```

Never leave stale worktrees; the watchdog GC (`scripts/autospec-watchdog.sh`)
sweeps orphans as a safety net, but proactive cleanup is required.

The watchdog cross-checks the GitHub `autospec-run-state` comment before releasing
any `claimed` heartbeat — a live sibling's claim is never reclaimed on local age
alone. The default `claimed` threshold is **1800s**; override with
`AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS`. See SKILL.md §"Running concurrent workers"
for the full concurrency model and tuning table.

### Pointer to enforcement tool

`scripts/worktree-guard.sh` (installed to `~/.autospec/scripts/worktree-guard.sh`)
implements `assert`, `resolve-branch`, `create`, and `reset` (`--path`, `--base`, `--clean`). See
`docs/specs/2026-06-03-worktree-guard-design.md` §D1 for the full contract and
pinned exit codes.

## Repository retirement & cross-cutting invariants

Working software is a specification with no readers and an expiry date. Code
survives archival; the program (phase order, topology, deferred gaps,
rationale) does not. Full checklist: [`docs/runbooks/repository-retirement.md`](docs/runbooks/repository-retirement.md).

- **Extract before archiving.** Retiring a repository has a mandatory
  checklist step: program state, deployment topology, deferred decisions and
  rationale are relocated to a named home *first*; the archive (a gated
  destructive action) happens only after.
- **Specify from a running original.** When a second implementation is
  planned, the first implementation's design decisions are written down
  **while the original still runs**, each entry citing the running system as
  evidence (component, revision, probe/command).
- **Rationale belongs where the decision is used.** Cross-cutting invariants
  have a home outside the file that implements them —
  [`docs/invariants.md`](docs/invariants.md) — and every entry names the
  components it binds. A new component bound by an invariant is added to that
  invariant's components-bound list in the same PR that introduces the
  component.

## Memoized decisions are keyed on the input (issue #4260)

The conversion/selector loop kept a set of issues whose patches had "already
been attempted" and used it to exclude future candidates — keyed on the issue
identifier. When an agent was killed and redispatched, it produced a
brand-new patch for the same issue, and the filter saw "already attempted"
and excluded the fresh work forever. The denominator made the defect look
healthy: `attempted=236 -> candidates=0`. The denominator proves the filter
ran; it does not prove the filter is correct.

- **Key the exclusion on the input, not the subject.** An attempt only
  excludes the artifact it was recorded against (content hash or mtime
  stamp), never other artifacts for the same subject
  (`input_keyed_excluded`). A memo entry that never matched the current
  input must not skip it; the subject-keyed filter is kept as a named
  reference for the failure mode (`subject_keyed_excluded`).
- **Record what was attempted alongside the fact.** `AttemptRecord` carries
  the input key with the attempt. A record with no captured key is legacy
  and never excludes — re-attempting a patch is cheap, losing a fresh one
  is not.
- **Recency overrides history.** An artifact whose mtime is within the
  fresh window (`DEFAULT_FRESH_MIN`) is a candidate even when its key
  matches a recorded attempt; a future mtime (clock skew) is fresh, not an
  error. The override and the stale-memo set are first-class report
  dimensions (`fresh=N`, `stale=N`), and a `SelectionReport` whose buckets
  do not reconcile is reporting a state that cannot exist
  (`reconciles()`).
- **Per-stage counters are not end-to-end evidence.** Work completed within
  the reconciliation window (`DEFAULT_RECONCILE_WINDOW`) must appear either
  as a PR or as a held line; `reconcile` asserts that invariant and names
  — with age — every piece of work that reached neither. A PR takes
  precedence over a held line.

Checkable in `autospec_core::memo_key` (`AttemptRecord`,
`subject_keyed_excluded`, `input_keyed_excluded`, `is_fresh`,
`select_candidates`, `SelectionReport`, `reconcile`, `ReconcileReport`).
Tests: `crates/autospec-core/tests/memo_key.rs`, including the regression
case that reconstructs the incident: 62 fresh patches for 62
"already attempted" issues — the subject-keyed filter yields
`candidates=0`, the input-keyed filter yields `stale=62 ... candidates=62`.
