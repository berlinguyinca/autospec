# autospec configuration reference

The canonical operator configuration file is `.autospec/autospec.yml`. Runtime
scripts read that file first, then fall back to legacy `AUTOSPEC_*` environment
variables, then built-in or auto-discovered defaults. Use env vars only for
one-off compatibility overrides when no config key is present.

Behavior toggles that are flag-files rather than env vars live in [`FLAGS.md`](FLAGS.md).

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

## Issue intent safety

`safety.issue_intent_gate` configures deterministic issue screening. Missing or invalid config falls back to conservative built-in defaults. `block_patterns` and `ambiguous_patterns` add regex rules. `trusted_actors` can pass scoped test/dev cleanup but cannot bypass secret exfiltration, production data destruction, instruction bypass, backdoors, or CI/review bypass.

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

## Model tiers & dispatch
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_REVIEWER_TIER` | tier-policy default | Override the model tier for the LGTM/guardian reviewer. |
| `AUTOSPEC_HARNESS_DISPATCHER` | auto-detected | Force a harness dispatcher (claude/opencode/codex). |
| `AUTOSPEC_NO_GUARDIAN` | (unset) | Disable the guardian RULE_ID pass (not recommended). |

## Testing & QA
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_FULL_TEST_COMMAND` | repo-detected | Override the full test command. |
| `AUTOSPEC_TEST_STACK` | auto-detected | Force the test stack (node/python/go/…). |
| `AUTOSPEC_QA_STRICT` | (unset) | Stricter autospec-qa gating. |
| `AUTOSPEC_CUSTOM_VERIFY_CMD` / `_SNAPSHOT_CMD` / `_RESTORE_CMD` | (unset) | Custom verify/snapshot/restore hooks for E2E reset. |
| `AUTOSPEC_WITH_DOCS` | (unset) | Include the docs dimension in sweeps/reviews. |

## Crash-resume & watchdog
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_RESUME_COMMAND` | derived | Command the usage-limit supervisor relaunches on reset. |

## Security (autospec-secaudit)
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_SEC_MAX_ROUNDS` | `3` | Phase 4 security gate fix/re-scan loop cap. |
| `AUTOSPEC_SKIP_ENSURE_TOOL` / `_<TOOL>` | (unset) | Disable scanner auto-install (all, or one tool). |

## Explore (autospec-explore RSI)
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_EXPLORE_LEDGER` | `.autospec/explore-ledger.jsonl` | Outcome-ledger path. |
| `AUTOSPEC_EXPLORE_MIN_CONFIDENCE` | `0.3` | Constitution confidence floor (D2). |
| `AUTOSPEC_EXPLORE_CONSTITUTION` | `.autospec/explore-constitution.md` | Proposal-constitution path. |

For automation toggles (skip-review, stop, no-heal, …) see [`FLAGS.md`](FLAGS.md).
This reference covers the operator-facing knobs; run `grep -rho 'AUTOSPEC_[A-Z0-9_]*' skills scripts | sort -u` for the exhaustive internal set.
