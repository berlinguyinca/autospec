# autospec configuration reference

The operator-relevant `AUTOSPEC_*` environment variables, grouped by area. (Internal
plumbing vars such as `AUTOSPEC_SCRIPTS_DIR`, `AUTOSPEC_REPO_ROOT`, and test-only
`AUTOSPEC_TEST_*` overrides are intentionally omitted.) Behavior toggles that are
flag-files rather than env vars live in [`FLAGS.md`](FLAGS.md).

Set them in your shell, an `.env`, or inline for one run:
```bash
AUTOSPEC_BATCH_SIZE=1 /autospec-run        # one issue per subagent
```

## Core paths & repo
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_STATE_DIR` | `~/.autospec` | Root for run-state, heartbeats, flag files. |
| `AUTOSPEC_DIR` | `~/.autospec` | Legacy alias for the state dir in some scripts. |
| `AUTOSPEC_REPO` | gh context | `owner/name` slug when not inferable from the checkout. |

## Batch, caps & budgets
| Var | Default | Effect |
|---|---|---|
| `AUTOSPEC_BATCH_SIZE` | `1` | Issues processed per subagent. `reasoning:deep` issues force-stay at 1. |
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
| `AUTOSPEC_WATCHDOG_STALE_SECS` | (skill default) | Age after which a heartbeat is considered stale. |
| `AUTOSPEC_WATCHDOG_RECLAIM_SECS` | (skill default) | Age after which a stale claim lease may be reclaimed. |
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
