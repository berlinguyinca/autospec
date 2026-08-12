# autospec configuration reference

The canonical operator configuration file is `.autospec/autospec.yml`. Runtime
scripts read that file first, then fall back to legacy `AUTOSPEC_*` environment
variables, then built-in or auto-discovered defaults. Use env vars only for
one-off compatibility overrides when no config key is present.

Behavior toggles that are flag-files rather than env vars live in [`FLAGS.md`](FLAGS.md).

## Runtime resource isolation

Manifest `version: 2` is the canonical repository configuration for worktree resources.
`resources.maven.isolation: split-local` requires Maven 4 and owns only
`<effective-local-repository>/autospec/<AGENT_ENV_ID>`. `resources.compose` declares `files`,
`exports`, `preserve_volumes`, and exact `shared_resources`; every other Compose container,
network, volume, project name, and published port is broker-owned.

| Environment value | Effect |
|---|---|
| `AGENT_ENV_STATE_ROOT` | Overrides the private state root; Unix directories/files are forced to `0700`/`0600`. |
| `AUTOSPEC_MAVEN_ISOLATION=off` | Bypasses Maven isolation and exports `AUTOSPEC_ISOLATION_BYPASSED=1`. |
| `AUTOSPEC_COMPOSE_ISOLATION=off` | Bypasses Compose isolation and exports `AUTOSPEC_ISOLATION_BYPASSED=1`. |
| `AUTOSPEC_ENV_DISABLE=1` | Bypasses broker provisioning for a direct child and exports `AUTOSPEC_ISOLATION_BYPASSED=1`. |

An opt-out downgrades an isolation claim from verified. Cleanup rejects symlinked state or
session roots with `RUNTIME_STATE_SYMLINK_REJECTED`; use `gc` or `down --purge-maven` rather
than deleting state by hand. Compose migrations use `normalize-compose --check` followed by
the fingerprint-bound `--apply` command.

## Autonomous runtime

```yaml
autonomous:
  concurrency:
    batch_size: auto
    max_concurrent_repo_workers: auto
  claims:
    lease_seconds: 10800
    settle_seconds: 0.2
    confirm_reads: 5
  watchdog:
    stale_secs: 1800
    reclaim_secs: 10800
    claimed_timeout_secs: 1800
    nudge_cooldown_secs: 900
  drain:
    stall_secs: 1800
    poll_secs: 15
```

`auto` discovers a conservative local worker cap from CPU count, clamped to 1-4.
For a larger workstation or cluster, set `max_concurrent_repo_workers` explicitly.

| Config key | Legacy env fallback | Effect |
|---|---|---|
| `autonomous.repo_dir` | `AUTOSPEC_REPO_DIR` | Checkout used by autonomous drain commands. |
| `autonomous.repo` | `AUTOSPEC_REPO` / `AUTOSPEC_WATCHDOG_REPO` | GitHub `owner/name` when not inferable. |
| `autonomous.heartbeat_dir` | `AUTOSPEC_HEARTBEAT_DIR` / `AUTOSPEC_WATCHDOG_DIR` | Shared worker heartbeat root. |
| `autonomous.concurrency.batch_size` | `AUTOSPEC_BATCH_SIZE` | Queue snapshot size requested by the conductor. |
| `autonomous.concurrency.max_concurrent_repo_workers` | `AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS` | Repo-wide active worker cap. |
| `autonomous.claims.lease_seconds` | `AUTOSPEC_CLAIM_LEASE_SECONDS` | Cross-machine claim lease TTL. |
| `autonomous.claims.settle_seconds` | `AUTOSPEC_CLAIM_SETTLE_SECONDS` | Delay before confirming a GitHub claim. |
| `autonomous.claims.confirm_reads` | `AUTOSPEC_CLAIM_CONFIRM_READS` | Claim ownership confirmation reads. |
| `autonomous.watchdog.stale_secs` | `AUTOSPEC_WATCHDOG_STALE_SECS` | Heartbeat age before nudging. |
| `autonomous.watchdog.reclaim_secs` | `AUTOSPEC_WATCHDOG_RECLAIM_SECS` | Stale claim reclaim age. |
| `autonomous.watchdog.claimed_timeout_secs` | `AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS` | Claimed-step release threshold. |
| `autonomous.watchdog.nudge_cooldown_secs` | `AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS` | Minimum time between watchdog nudges. |
| `autonomous.watchdog.state_file` | `AUTOSPEC_WATCHDOG_STATE_FILE` | Nudge cooldown state file. |
| `autonomous.watchdog.gc_dir` | `AUTOSPEC_WATCHDOG_GC_DIR` | Orphan worktree GC root. |
| `autonomous.watchdog.gc_heartbeat_fresh_secs` | `AUTOSPEC_WATCHDOG_GC_HEARTBEAT_FRESH_SECS` | Fresh-heartbeat guard for GC. |
| `autonomous.drain.stall_secs` | `AUTOSPEC_AUTONOMOUS_DRAIN_STALL_SECS` | No-output timeout for one drain run. |
| `autonomous.drain.poll_secs` | `AUTOSPEC_AUTONOMOUS_DRAIN_POLL_SECS` | Drain output poll interval. |

### Repository-local Rust mainline health

Rust autonomous health admission reads a separate, repository-owned file at
`<checkout-root>/.autospec/autonomous.yml` on every `main-health` or
`run-foreground` invocation. When `--repo-dir` names a subdirectory of a Git
checkout, Rust resolves its checkout root first; outside Git it uses
`<repo-dir>` literally. This is deliberately not `.autospec/autospec.yml`, and
it is a strict supported subset rather than a general YAML policy file:

```yaml
main_health:
  branch: master_ai
  ignore_checks:
    - "Unit Tests"
```

`main_health.branch` is optional and nonempty. The Rust CLI resolves a branch
in this order: explicit `--branch`, then `main_health.branch`, then the GitHub
default branch. `ignore_checks` is an optional list of nonempty, exact,
case-sensitive check names. A matching health observation remains persisted as
evidence but is marked advisory; it no longer blocks mainline health. Unmatched
failed or pending checks remain required and block admission.

Each appended `main-health-observations.jsonl` record includes an
`effective_policy_digest`. The digest is canonical over the resolved branch and
the sorted, exact `ignore_checks` names: it changes when either effective value
changes, but not when YAML formatting or unrelated configuration changes. Rust
reloads the repository file and records the resulting digest on every evaluated
`run-foreground` invocation, including invocations that return retained
conductor state. When GitHub supplies no default branch, the typed
`default-branch-missing` observation uses the reserved, invalid-ref identity
`autospec:unresolved-default-branch` so the failed evaluation is still bound to
the effective policy.

A missing file preserves existing behavior. An unreadable file or malformed,
duplicate, unknown, incorrectly indented, or wrongly typed field inside
`main_health` fails closed with a diagnostic before foreground lease, queue,
claim, state, or executor mutation. Rust does not read or export
`AUTOSPEC_MAIN_HEALTH_*`; this setting does not relax premerge, safety, claims,
or merge policy.

### Self-originated integration branch

Config contract for the autonomous self-originated integration-branch design
(`docs/specs/2026-07-10-autonomous-integration-branch-design.md`). Resolved
via `autospec_runtime_config_get` (`scripts/autospec-runtime-config.sh`), no
legacy env fallback — these are new keys. The consumer scripts
(`scripts/autonomous-integration-branch.sh`, the `self-originated` subcommand
of `scripts/autonomous-guardrails.sh`) land in separate follow-up issues
(#1740, #1742, and a conductor-wiring child); this section documents the
config contract those consumers must honor once landed.

```yaml
autonomous:
  self_originated:
    integration_branch_prefix: "autospec/autonomous-"
    max_open_prs: 20
    max_age_days: 14
    max_diff_lines: 5000
    allow_direct_merge: false
```

| Config key | Type | Default | Consumer | Effect |
|---|---|---|---|---|
| `autonomous.self_originated.integration_branch_prefix` | string | `autospec/autonomous-` | `scripts/autonomous-integration-branch.sh` (`ensure`, `reset`) | Prefix used to derive the shared integration branch name from the parent branch. |
| `autonomous.self_originated.max_open_prs` | int | `20` | `scripts/autonomous-integration-branch.sh status` | Cap on open worker PRs accumulated on the integration branch before the conductor parks self-originated tiers. |
| `autonomous.self_originated.max_age_days` | int | `14` | `scripts/autonomous-integration-branch.sh status` | Cap on the integration branch's age before the conductor parks self-originated tiers. |
| `autonomous.self_originated.max_diff_lines` | int | `5000` | `scripts/autonomous-integration-branch.sh status` | Cap on cumulative diff size vs. the parent before the conductor parks self-originated tiers. |
| `autonomous.self_originated.allow_direct_merge` | bool | `false` | `scripts/autonomous-guardrails.sh self-originated` (premerge gate) | When `false`, a self-originated PR based directly on a protected parent branch is blocked (`block self_originated_direct_merge`) instead of merged; flipping to `true` requires a trusted-actor marker recorded in the same config commit that changes the value, so the bypass itself is attributable and reviewable, not a casual toggle. |

Any of the three caps (`max_open_prs`, `max_age_days`, `max_diff_lines`) being
exceeded parks only the self-originated tiers and sends a notification;
operator-originated tiers keep draining unaffected. Provenance and roll-up
state carry two GitHub labels: `origin:self` (issue was filed by the
autonomous system itself) and `approved-by-operator` (a trusted-actor approval
marker that flips an issue's provenance from `self` to `operator` pre-dispatch).

The remaining tables list older operator-facing env knobs that do not yet have
dedicated config keys.

## Root helper wrapper policy

The supported command surface is the Rust `autospec` binary plus helpers
installed into `~/.autospec/scripts` by skill installers. Root-level
`scripts/` shell files are repository source files, not a generated wrapper API:
autospec does not support, regenerate, or install ad-hoc root-level wrapper
copies. Restoring stale helper copies into root `scripts/` is disallowed unless
the copy is tracked source or has a tracked canonical skill/shared script source.

Validation enforces this policy with `check_root_helper_wrapper_policy` in
`autospec validate`: untracked root `scripts/` files without a matching tracked
source under `skills/*/scripts/` or `skills/autospec-shared/scripts/` fail the
run. This keeps old stashes from silently reviving a second shell-wrapper
surface; add new operator behavior to the Rust CLI or a skill-owned installer
instead.

Audit note: the named dropped stash
`codex-preserve-main-worktree-before-autonomous-merge` was not present in
`git stash list` during the issue-1901 implementation. The fallback audit below
uses root helper names present in tracked repository evidence and records whether
they have a skill/shared canonical source.

| Helper name | Canonical tracked path | Skill/shared canonical? | Policy result |
|---|---|---:|---|
| `accessibility-workstream.sh` | `scripts/accessibility-workstream.sh` | No | Tracked repo source, not an install wrapper |
| `apply-memory-tags.sh` | `scripts/apply-memory-tags.sh` | No | Tracked repo source, not an install wrapper |
| `architecture-fitness.sh` | `scripts/architecture-fitness.sh` | No | Tracked repo source, not an install wrapper |
| `assemble-impl-prompt.sh` | `scripts/assemble-impl-prompt.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-control-channel.sh` | `scripts/autonomous-control-channel.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-guardrails.sh` | `scripts/autonomous-guardrails.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-integration-branch.sh` | `scripts/autonomous-integration-branch.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-persona-mine.sh` | `scripts/autonomous-persona-mine.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-persona-sources.sh` | `scripts/autonomous-persona-sources.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-persona-synth.sh` | `scripts/autonomous-persona-synth.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-premerge-gate.sh` | `scripts/autonomous-premerge-gate.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-prioritize.sh` | `scripts/autonomous-prioritize.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-priority-match.sh` | `scripts/autonomous-priority-match.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-promote-open-issues.sh` | `scripts/autonomous-promote-open-issues.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-provenance.sh` | `scripts/autonomous-provenance.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-resilience.sh` | `scripts/autonomous-resilience.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-self-improvement.sh` | `scripts/autonomous-self-improvement.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-spend-ledger.sh` | `scripts/autonomous-spend-ledger.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-usage-governor.sh` | `scripts/autonomous-usage-governor.sh` | No | Tracked repo source, not an install wrapper |
| `autonomous-waterfall.sh` | `scripts/autonomous-waterfall.sh` | No | Tracked repo source, not an install wrapper |

## Issue intent safety

`safety.issue_intent_gate` configures deterministic issue screening. The conservative built-in defaults apply only when the implicit `.autospec/autospec.yml` path is absent; an explicit, unreadable, or malformed config fails closed. The built-in policy is evaluated natively in Rust: duplicate built-in entries are accepted, while a custom regex that the evaluator cannot represent fails closed with `invalid-policy-regex` rather than being ignored. `trusted_actors` can pass scoped test/dev cleanup but cannot bypass secret exfiltration, production data destruction, instruction bypass, backdoors, or CI/review bypass. `autospec queue review-safety --limit N [--issue N]` and `autospec claim acquire` use this same policy; review requires an explicit positive bound, and `--issue N` targets one admitted issue without scanning the queue. Neither adds a configuration key.

## Core paths & repo
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_STATE_DIR` | `~/.autospec` | Root for run-state, heartbeats, flag files. |
| `AUTOSPEC_DIR` | `~/.autospec` | Legacy alias for the state dir in some scripts. |

## Batch, caps & budgets
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_AUTONOMOUS_ISSUE_CAP` | (unset) | Max issues an autonomous run will process before stopping. |
| `AUTOSPEC_AUTONOMOUS_TOKEN_CAP` | (unset) | Token ceiling for an autonomous run. |
| `AUTOSPEC_LOOP_TOKEN_CAP` | (unset) | Token ceiling for loop entrypoints (`--loop`, explore). |
| `AUTOSPEC_LOOP_TIME_CAP` | (unset) | Wall-clock ceiling for loop entrypoints. |
| `AUTOSPEC_GAP_MAX_ROUNDS` | `2` | Phase 5.5 gap-remediation round cap. |
| `AUTOSPEC_HEAL_MAX_ROUNDS` | (skill default) | autospec-qa self-heal round cap. |
| `AUTOSPEC_RESUME_MAX_ATTEMPTS` | `3` | Consecutive crash-resume attempts before giving up. |

## Autonomous review-policy learning

`scripts/autonomous-self-improvement.sh candidates` reads attributed review
outcomes and Phase 5.5 gaps from `.autospec/review-outcomes.jsonl` and
`.autospec/gaps.json` by default. The following CLI overrides are intended for
isolated runners and tests:

| Option | Default | Effect |
|---|---|---|
| `--review-outcomes PATH` | `.autospec/review-outcomes.jsonl` | Append-only attributed review observations and canary outcomes. |
| `--gaps PATH` | `.autospec/gaps.json` | Attributed escaped-defect evidence clustered into candidates. |
| `--learning-ledger PATH` | `.autospec/review-learning.jsonl` | Append-only repetition/frequency observations. |
| `--lifecycle-ledger PATH` | `.autospec/review-policy-lifecycle.jsonl` | Candidate, shadow, canary, promoted, held, and rolled-back events. |

`evaluate --candidate FILE --rollback-digest DIGEST` advances one experiment.
Promotion requires the declared sample floor, zero high-severity escapes,
non-regressing total escapes and cost, provider-diverse review, and the exact
prior-policy rollback digest. Validation evidence selects the closed
`git-diff-check` recipe; proof JSON never supplies executable argv. The evaluator
reproduces that recipe against the named commit, derives protected-boundary
changes from the commit diff, and tests the rollback patch against an archive of
that exact commit. `AUTOSPEC_SELF_IMPROVEMENT_APPLY=1` still requires
the explicit `apply --apply` pair before strengthening or neutral candidates
are filed with `needs-classify` and `origin:self`; weakening stays report-only.
Superseded outcomes and rows missing complete PR, commit, receipt, reviewer,
reasoning, diversity, or risk attribution are excluded from both discovery and
promotion samples.

## Model tiers & dispatch
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_REVIEWER_TIER` | tier-policy default | Override the model tier for the LGTM/guardian reviewer. |
| `AUTOSPEC_HARNESS_DISPATCHER` | auto-detected | Force a harness dispatcher (claude/opencode/codex). |
| `AUTOSPEC_HANDOFF_DISPATCHER_KIND` | initiating session | Select the native autonomous executor (`claude`, `codex`, or `opencode`); overrides Codex, Claude, and OpenCode runtime markers. Without a marker, executors are PATH-probed in alias-table order. |
| `AUTOSPEC_HARNESS_RUNTIME_ALIASES` | `$AUTOSPEC_CONFIG_DIR/harness-runtime-aliases.tsv`, otherwise `~/.autospec/config/harness-runtime-aliases.tsv` | Override the installed four-column TSV mapping harness kind, executable alias, approval alias, and display name. |
| `AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER` | auto-discovered (`~/.autospec/scripts/lib/opencode-containment-adapter.sh`) | Absolute containment adapter required before the native executor may launch mutating OpenCode work; auto-discovered from the install tree or checkout when unset, and fails closed with `executor_harness_uncontained` when absent. |
| `AUTOSPEC_NO_GUARDIAN` | (unset) | Disable the guardian RULE_ID pass (not recommended). |

## Local model supply discovery
`scripts/discover-model-supply.sh` measures what this host can actually run and
writes a capability document consumed by `/autospec-run`'s profile auto-init.

| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_MODEL_CAPABILITY` | `~/.autospec/model-capability.json` | Capability-document path. |
| `AUTOSPEC_OLLAMA_HOST` | `127.0.0.1:11434` | Ollama host:port probed for local models. |

The document is cached on a hardware fingerprint (accelerator identity + VRAM +
the installed model set) plus a TTL; a fingerprint change forces a re-probe, and
the model set is **replaced** rather than merged so stale entries cannot persist.

Two fields decide whether a local model is usable, and both fail closed:

- `accelerator.usable` / `accelerator.reason` — a present `nvidia-smi` and a
  present `/dev/nvidia0` do *not* mean a usable GPU. When `usable` is `false`,
  `reason` names the cause (`nvml_driver_library_mismatch`,
  `nvidia_smi_query_failed`, `no_accelerator_detected`, …). Surface it: a driver
  mismatch is a quick fix, whereas a silent fall back to CPU inference is not.
- `local_models[].dispatch_recommended` — `false` means the model was found but
  the host cannot run it usefully (no usable accelerator, or weights exceeding
  the memory budget). `--profiles` emits those entries commented out.

Context ceilings come from `ollama show`, never from the model name: a tag can
misreport its parameter count, and two tags can share one set of weights.

`--profiles` prints a paste-able `model-profiles.yml` fragment, and
`--profiles --only <profile>` narrows it to a single entry. Prefer **one** local
profile: a local GPU is capacity-1, so a second local profile at a different size
buys no parallelism and alternating between them evicts and reloads weights
between dispatches. `--only` filters the *output*; the probe still discovers
every model, because hiding models from discovery is the blindness this tool
exists to fix. Naming a profile the probe did not find is an error rather than an
empty mapping, so a typo cannot masquerade as "nothing found".

The emitted fragment carries no prices, and `routing-cost.sh` refuses a profile
with no cost keys. Add `cost_minute:` (USD-equivalent per GPU-minute) — and
optionally `max_wall_clock_ms:` — before a local profile can be routed to.

## Evidence-based model routing
`scripts/routing-ledger.sh` records what each dispatch cost and how it turned out;
`scripts/routing-cost.sh` scores candidate profiles on measured **effective** cost;
`scripts/route-decide.sh` makes the call.

| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_ROUTING_LEDGER` | `.autospec/routing-ledger.jsonl` | Append-only outcome-ledger path. |
| `AUTOSPEC_ROUTING_POLICY` | `auto` | `off` = always the baseline (no ledger read); `auto` = override only when strictly cheaper; `on` = override whenever a candidate is eligible. |
| `AUTOSPEC_ROUTING_ALPHA` | `5` | Bayesian smoothing pseudo-count. Larger keeps rates closer to their priors. |
| `AUTOSPEC_ROUTING_MIN_SAMPLES` | `10` | Dispatches in a cell before a profile may win it. |
| `AUTOSPEC_ROUTING_FIRST_PASS_FLOOR` | `0.6` | Quality floor; the cheapest profile *clearing this* wins, not the cheapest outright. |
| `AUTOSPEC_ROUTING_CACHE_BETA` | `0.5` | Strength of the prompt-cache penalty. |
| `AUTOSPEC_ROUTING_EXPLORE_PCT` | `0` (off) | Cold-start exploration percent. Confined to the lowest-stakes cell (`ctx:32k` + `reasoning:shallow`). |
| `AUTOSPEC_ROUTING_PREFIX_TOKENS` | `0` (unknown) | Prefix size this dispatch will stage, tested against each profile's `cache_min_tokens`. `0` fails open and scores exactly as before. |

**Prompt-cache minimums (`cache_min_tokens`).** A prompt cache only engages above a
per-model token floor, and the floors differ sharply — Haiku 4.5 needs **4096**
tokens where Opus 5 needs **512** — which makes the cheapest per-token profile the
easiest one to fall under. When `AUTOSPEC_ROUTING_PREFIX_TOKENS` is below a
profile's `cache_min_tokens`, its measured `cache_hit_ratio` is zeroed (and
`cache_floor_unmet` is reported), because a hit ratio recorded under a larger prefix
must not be credited to a dispatch that cannot cache at all. Without this, Haiku
looks cheaper than it is on short prefixes.

**Effort (`effort`).** An optional per-profile reasoning tier, surfaced by
`route-decide.sh --print-effort` and `select-model-profile.sh --print-effort`. It is
often a better dial than swapping models, because **switching model invalidates the
whole prompt cache** across all three tiers while raising effort on the same model
keeps the cached prefix intact. It is *reported*, never modelled as a cost
multiplier: two profiles differing only in effort are separate catalog rows, so the
ledger measures the difference instead of the scorer guessing a factor. A profile
that states no effort exits 3, so the caller keeps its own default rather than being
handed a guess — and on an override, effort follows the winning profile, never the
baseline it replaced.

**Which dispatch kinds may be re-routed.** `route-decide.sh` holds an
**allowlist**, so a kind added to the ledger vocabulary later is baseline-only
until someone deliberately opens it:

| Kind | Re-routable | Why |
|---|---|---|
| `implementer` | yes | Works from a Tier-A contract; output is gated by tests, lint, and review. |
| `explore-researcher` | yes | High fan-out read-and-report; findings are re-checked by a later gate. |
| `refine-lens` | yes | Same shape as explore. |
| `qa-sweep` | yes | Same shape as explore. |
| `lgtm-reviewer` | no | Reviewer tier must stay ≥ implementer tier — a cheap model LGTM'ing its own tier's output degrades quality invisibly, and the ledger records that as a first-pass success, *rewarding* the pairing. |
| `verify-voter` | no | Voter independence is a vendor question, not a cost one (see `verify-voter-vendor.sh`). Cost-ordering voters converges them onto one model, which is exactly what a second vote is supposed to rule out. |
| `secaudit-pass` | no | Safety gate. Never local, never downgraded. |
| `spec-decompose` | no | Spec quality is the upstream bottleneck; a cheap model here costs N implementer cycles correcting it. |
| `growth-lens` | no | No ledger evidence yet. |

### Cross-vendor verify voters
`scripts/verify-voter-vendor.sh --proposer <vendor>` names the vendor for the next
verify voter. Two dispatches to the same model family share training data and
failure modes, so they tend to be wrong together — the one case a second vote
exists to catch — and the script therefore never returns the proposer's own vendor.

| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_VOTER_VENDORS` | host PATH probe of `claude`/`codex`/`opencode` | Explicit candidate list, e.g. `claude,codex`. An unknown name is an error, not a smaller fleet. |

It narrows in four steps: installed vendors → minus any `--unavailable` vendor →
minus the proposer's vendor → least ledger spend of what remains. **Step 2 is the
load-bearing mechanism.** `scripts/usage-observe.sh` reports `observable=false`
for all three harnesses, so remaining quota is not measurable — a 429 is ground
truth, while ledger spend is an estimate that is wrong by however much the
operator used that harness outside autospec. Treat the spend comparison as a
tiebreak between vendors that are both fine.

Exit 3 means no *independent* vendor is available (single-harness host, or
failover exhausted the alternatives). It fails closed to the caller's current
same-vendor `TIER_B` voter rather than printing the proposer's vendor, which would
claim an independence the host cannot provide.

The voter's *tier* is not chosen here — it is the selected harness's own `TIER_B`.
`verify-voter` is deliberately absent from `route-decide.sh`'s overridable
allowlist: cost-ordering voters converges them onto one cheapest model, which is
exactly the correlation this script exists to break.

Two properties are deliberate and worth relying on:

- **No data means no change.** With an empty or thin ledger, `route-decide.sh` prints
  exactly what `select-model-profile.sh` prints. A router that silently re-routed on a
  host with no telemetry would be worse than no router.
- **Cheap per token is not cheap per merged PR.** A local model that fails often and
  escalates costs *more* than one dispatch of a reliable cheaper-tier cloud model. Cost
  is scored as
  `unit × (1 + E[retries]) × cache_penalty + P(escalate)×advisor + P(fail)×fallback`,
  and unproven profiles shrink toward **pessimistic** priors so no-data never looks cheap.

Profiles carry their own prices in `model-profiles.yml`: `cost_in`/`cost_out` (USD per
Mtok) for cloud, `cost_minute` for local. A profile with no cost keys is never chosen
over the baseline — routing fails closed rather than guessing a price.

Only `implementer` dispatches are re-routable. The reviewer keeps its own tier, which
also keeps every safety gate and all spec/decompose work on its existing tier.

## Local model dispatch
`scripts/local-dispatch.sh` runs a dispatch on a local model via Codex CLI's native
`--oss --local-provider` support. It is chosen over a bespoke HTTP client because
autospec already depends on Codex for peer review.

| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_LOCAL_PROVIDER` | `ollama` | Local provider (`ollama` or `lmstudio`). |
| `AUTOSPEC_LOCAL_TIMEOUT_SECS` | `600` | Wall-clock ceiling per local dispatch. |
| `AUTOSPEC_LOCAL_LOCK_DIR` | `~/.autospec/locks` | Lock dir serializing the capacity-1 GPU. |

It exits **3** — meaning *keep the cloud tier* — when Codex is absent, when Codex does
not advertise `--oss` (an older build would ignore the flag and silently bill a **paid**
cloud model, the most expensive possible failure), when the capability probe reports the
model is not `dispatch_recommended`, or when no wall-clock bound can be applied. Exit
**4** means the dispatch hit its ceiling, kept distinct from a wrong answer.

## Automation lifecycle toggles
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_NO_AUTOMERGE_SPEC` | `0` | Set to `1` to skip admin auto-merge for spec PRs; the skill pauses for a manual merge before continuing. |
| `AUTOSPEC_NO_SELF_UPDATE` | `0` | Set to `1` to skip the once-per-24h startup self-update preflight for multi-harness skills. |
| `AUTOSPEC_PR_ADVISORY_CHECKS` | `AUTOSPEC_MAIN_HEALTH_IGNORE_CHECKS` or `^$` | Regex for PR check names/contexts treated as advisory during auto-merge; matching checks may be pending or failing once local validation is green. It is not consumed by Rust mainline health; `main_health.ignore_checks` does not alter premerge or auto-merge behavior. |

## Testing & QA
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_FULL_TEST_COMMAND` | repo-detected | Override the full test command. |
| `AUTOSPEC_TEST_STACK` | auto-detected | Force the test stack (node/python/go/…). |
| `AUTOSPEC_QA_STRICT` | (unset) | Stricter autospec-qa gating. |
| `AUTOSPEC_CUSTOM_VERIFY_CMD` / `_SNAPSHOT_CMD` / `_RESTORE_CMD` | (unset) | Custom verify/snapshot/restore hooks for E2E reset. |
| `AUTOSPEC_WITH_DOCS` | (unset) | Include the docs dimension in sweeps/reviews. |

## Implementation size gates
`scripts/lint-implementation-gates.sh` wraps `lint-implementation.sh` and corrects its
size rules; the implementer pre-commit hook calls the wrapper, not the linter directly.
The file-LOC rule is a ratchet — an oversized file may be edited and shrunk but not
grown — and `PR_SIZE`'s changed-line cap is waived for a change that removes at least as
many lines as it adds. A new file holding relocated code is reported without blocking,
since rejecting an extraction would preserve the larger original. `PR_SIZE`'s unit count
excludes doc files, because `DOC_OUT_OF_SYNC` compels a public-surface change to touch
one; see `docs/API_REFERENCE.md`.
The relocation test counts **code** lines only. An extraction cannot avoid adding a module
declaration, an import and a header explaining the split, so counting those made every honest
extraction read as net growth and forfeited the waiver written for it — measured at +31 raw
and 0 code lines when the harness cluster left `executor_bridge.rs`. Comments, blank lines and
`mod`/`use`/`import` declarations are excluded; statements are not, which is what stops the
waiver laundering a feature through a refactor.
`.github/workflows/file-size-ratchet.yml` applies the same ratchet in CI against the
merge base, which is where growth actually enters: the rule previously ran only as a
local hook, so files grew freely through the merge path.

`COMPLEXITY` findings are **advisory by default**: they print as `INFO:` and do not block.
As a veto the limits froze oversized files against even a one-line safe edit — the ratchet
waives file-LOC for a file that does not grow, but a long function or a file-level
cyclomatic score is neither waived nor suppressible (`line: -` defeats
`linter:allow-COMPLEXITY`), so `fleet-gui-server.py` could not accept a three-line fix at
all (#2961). Enforcement in CI is unaffected: the file-size ratchet workflow is a separate
implementation and still blocks new oversized files and any growth of an existing one.

| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_COMPLEXITY_ENFORCE` | `0` | `1` makes `COMPLEXITY` findings blocking again — every threshold below, plus nesting depth and duplicate function names. The thresholds themselves apply either way; this decides whether exceeding one is a veto or a note. Read by **both** linters: `scripts/lint-implementation.sh` and the Rust `autospec lint implementation`, which the Phase 4 executor uses to admit a diff. They disagreed for one release, so the autonomous path still vetoed while the pre-commit path did not (#3056). |
| `AUTOSPEC_MAX_FILE_LOC` | `600` | File-length limit the ratchet enforces. Rough guidance meant to stop multi-thousand-line files, not to police a modest overage. Keep in step with `MAX_LOC` in the CI workflow. |
| `AUTOSPEC_MAX_CYCLOMATIC` | `10` | Cyclomatic-complexity limit, measured per function. |
| `AUTOSPEC_MAX_FUNC_LOC` | `50` | Per-function length limit. |

## UI evidence tests
`.github/workflows/ui-evidence-tests.yml` runs the `node:test` suites for the runtime UI
gates — `ui-motion-evidence`, `ui-device-evidence`, `ui-keyboard-evidence`,
`ui-liveregion-evidence`, `ui-liveregion-induce` and `ui-a11y-baseline` — on any PR touching
those scripts or their tests. It globs `ui-*.test.mjs`, the directory's whole naming
convention, after a narrower `ui-*-evidence.test.mjs` pattern silently excluded a new suite
and ran 65 of 76 tests while reporting green. A floor on the suite count fails the job if
fewer are found than expected. Nothing ran
them before: no workflow invoked `node --test` and no validate script did, so the
assertions existed without ever executing on the merge path.

The job installs Playwright and then asserts chromium launched. Each suite falls back to
its pure assertions when no browser is present and still reports green, so a missing
install would look identical to a pass while leaving the browser half unrun — which is the
half that has historically caught the defects, since a rendered control's size and a focus
ring's computed style cannot be reached by source analysis.

| Var | Default | Effect |
|---|---|---|
| `PLAYWRIGHT_CHROMIUM_PATH` | (unset) | Launch this chromium binary instead of the bundled one. Honoured by every `ui-*-evidence.mjs` script; useful when a system browser is already present. |

## Crash-resume & watchdog
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_RESUME_COMMAND` | derived | Literal resume command persisted by `autospec run` and reused by supervisors on reset. |

## Memory management
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_COMPRESS_THRESHOLD` | `5000` | LOC threshold passed to `mempalace-compress.sh`; below the threshold compression is a no-op. |
| `AUTOSPEC_MINE_PR_HISTORY` | `0` | Set to `1` to enable bandwidth-heavy PR history mining during `auto-init-memory.sh`; off by default. |

## Security (autospec-secaudit)
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_SEC_MAX_ROUNDS` | `3` | Phase 4 security gate fix/re-scan loop cap. |
| `AUTOSPEC_REQUIRED_SYSTEM_TOOLS` | core tools + `npm` | Override required core installer commands; the immutable executor scanner set is always added and verified. |

**`rg` (ripgrep) is an optional-but-recommended dependency of the reuse lens**, and
deliberately *not* in `AUTOSPEC_REQUIRED_SYSTEM_TOOLS` — a required tool that is
missing aborts the install, and the lens fails open rather than failing hard, so a
missing `rg` must degrade the lens rather than block installation. `REINVENT_REPO_UTIL` and
`NEW_ABSTRACTION_SINGLE_CALLER` shell out to it and **fail open** when it is absent —
non-blocking by design, but the lens then detects nothing. That is now announced as
`INFO:REUSE_LENS_DISABLED` once per run, because a silently inert lens is
indistinguishable from a repo with no reuse problems: build-vs-buy BLOCKs simply stop
being raised and the precision ledger records nothing rather than a miss.

**Dogfood detector adapters exit 0 by contract.** `scripts/dogfood-detectors.sh`
decides PASS/FAIL by diffing each adapter's findings against its allowlist under
`tests/dogfood/allowlist/`, so an adapter that exits non-zero is a bug in the adapter,
not a finding. When an adapter's underlying engine fails it must print the error and
still exit 0 — a bare non-zero exit gives the driver nothing to report.

Check with `command -v rg` **in a non-interactive shell**. Several agent harnesses
inject `rg` as a *shell function* proxying to a bundled copy, and shell functions are
not exported to child processes — so `rg` can work at your prompt and be missing
inside every script autospec runs.
| `AUTOSPEC_SKIP_ENSURE_TOOL` / `_<TOOL>` | (unset) | Disable scanner auto-install (all, or one tool); required scanners remain verified and fail closed by exact name. |

## Explore (autospec-explore RSI)
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_EXPLORE_LEDGER` | `.autospec/explore-ledger.jsonl` | Outcome-ledger path. |
| `AUTOSPEC_EXPLORE_MIN_CONFIDENCE` | `0.3` | Constitution confidence floor (D2). |
| `AUTOSPEC_EXPLORE_CONSTITUTION` | `.autospec/explore-constitution.md` | Proposal-constitution path. |

For automation toggles (skip-review, stop, no-heal, …) see [`FLAGS.md`](FLAGS.md).
This reference covers the operator-facing knobs; run `grep -rho 'AUTOSPEC_[A-Z0-9_]*' skills scripts | sort -u` for the exhaustive internal set.
