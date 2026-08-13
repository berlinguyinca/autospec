# Review governance PR #3010 supersession matrix

## Result

PR #3010 and the dirty local follow-up are fully superseded by changes already merged into `main`. No runtime code should be ported from either archive. The archived refs remain the recovery record:

- `archive/review-governance-3010-committed` → `31844f894945d2bdf090abf73bb390588c1faf78`
- `recovery/review-governance-3010-dirty` → `9d7e571301dc876a0b2836a747eb1337c2c7bb41`

The matrix was evaluated against `origin/main` at `060440db` after a fresh fetch. A `missing` disposition requires a failing current-main test or a concrete absent contract; none was found.

## Committed PR #3010 capabilities

| Capability | Disposition | Replacement evidence on `main` | Current proof surface |
|---|---|---|---|
| Risk-aware review policy | integrated | #3017 `73910085`, #3018 `af16d0f5` | `crates/autospec-core/src/autonomous/review_policy.rs`; `review_governance_tests.rs` |
| Deterministic evidence inventory | integrated | #3019 `f292d87f`, #3020 `2e1fceda` | `executor_bridge/review_evidence.rs`; `review_evidence/integration.rs` |
| Provider diversity and identity | integrated | #3021 `bf39a25d`, #3023 `8070ad44` | `review_provider.rs`; `review_provider/review_binding.rs` |
| Independent dispatch | integrated | #3022 `ff1640c1`, #3032 `13b0a52a`, #3033 `981cd7b5` | `review_provider/review_dispatch.rs`; `independent-review-adapter.sh`; harness-neutral Bats coverage |
| Exact-commit receipt binding | integrated | #3022 `ff1640c1`, #3024 `012451f4`, #3026 `0a48a0e4` | `review_receipt.rs`; `review_receipt/parse.rs`; `structured_review_receipt.rs` |
| Structured verdict validation | integrated | #3025 `d53c61e1`, #3042 `74c639c4` | `structured_review.rs`; `tests/structured_review.rs`; active executor bridge wiring |
| Executable integration smoke evidence | integrated | #3027 `ad17914b` | `tests/integration_smoke.rs` |
| Experiment governance | integrated | #3028 `df014117`, #3029 `4298d0ee` | `scripts/autonomous-self-improvement-evaluate`; `scripts/autonomous-self-improvement.sh`; `test_self_improvement_candidates.bats` |
| Rollback reproduction and lifecycle proof | integrated | #3030 `ece9d964` | `tests/autonomous/test_review_escape_learning.bats` exercises promotion, rejection, and rollback lifecycle |
| Review fixture migration | integrated | #3037 `bfe9384b` | `tests/result_reviewer.rs`; `tests/reviewer_automatic.rs`; `tests/reviewer_runtime.rs` |
| Review coverage registration | integrated | #3038 `341c06fc`, #3039 `fc1e1626` | `tests.rs`; `tests/attempt_generation.rs`; `tests/codex_sandbox.rs`; `tests/support_invocation.rs`; `tests/support_launch.rs`; `tests/terminal_label.rs` |
| Legacy reviewer fixture retirement | integrated | #3040 `35468c03` | `tests/base_merge.rs`; `tests/merged_reconciliation.rs`; removal of `tests/ready_harness.rs` |
| Runtime selection context | integrated | #3041 `4e96791a`, #3042 `74c639c4` | `autonomous.rs`; `executor_bridge.rs` |
| Gap attribution | integrated | #3043 `14477ff6`, #3044 `51c93757` | `emit-gaps.sh`; `gap-json-lib.sh`; attribution tests |
| Autonomous policy advancement | integrated | #3046 `5905d961` | `scripts/lib/autospec-loop.sh`; conductor-wiring test |

## Dirty local follow-up dispositions

| Recovery cluster | Disposition | Main evidence |
|---|---|---|
| Claim lease linearization | integrated | #3016 `b918ec44`; later claim fixes #3050, #3052, #3086 and #3114 |
| Queue GET retries and shared read helper | integrated | #3031 `0dc8039e`, #3035 `1c3a8952`, #3036 `1b47cc2f`; later #3063 and #3074 |
| Blocked-cycle extraction and recovery | integrated | #3070, #3072, #3073; `blocked_cycle.rs` exists on main |
| Program/selector module extraction | integrated | `program.rs` and `one_shot_selector.rs` exist on main; later fixes #2998 and #3011 build on them |
| Executor harness support split | obsolete | Current executor fixtures were migrated by #3037–#3040 and subsequently changed; the archived helper shape is not a missing contract |
| One-file `review_evidence.rs` edit | obsolete | Replacement evidence implementation landed via #3019/#3020 and later active wiring via #3042 |
| Repo-quality audit pruning | integrated | #2999 `8d5366e0`; script and `repo-quality-audit-prune.bats` exist on main |
| Dogfood allowlist/driver updates | integrated | #2998 `6bf101a4` and subsequent main allowlist maintenance |

## Verification

- All replacement PRs #3016–#3046 that exist are merged; the sequence intentionally omits #3034 and #3045.
- `git log -- <path>` confirms the current implementation history cited above.
- Current `main` contains every named runtime module and regression surface required by the archived design.
- A baseline `cargo build --workspace` passed on the docs recovery branch.
- The workspace test run exposed existing `branch_predecessor` failures before it was stopped; this documentation-only change neither touches nor claims to fix them.

## Terminal decision

Tasks 3–5 of the recovery plan require a `missing` matrix row plus a failing current-main regression. There are no such rows. Those implementation tasks therefore terminate as `not applicable`; reconstructing any archived code would duplicate or regress the reviewed replacement series.
