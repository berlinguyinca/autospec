---
name: feedback_claim_session_binding_and_parallel_safe
description: One claim per harness session_id (sidecar fails closed), and queue parallel_safe=true with paths=[] is not proven disjointness
metadata:
  type: feedback
---

Two Phase 4 dispatch constraints discovered empirically on 2026-09-02, both invisible
until you trip them:

**1. `autospec claim acquire` allows exactly ONE in-flight issue per `--session-id`.**
The first successful claim writes an immutable session sidecar at
`~/.autospec/process-heartbeats/<slug>/sessions/<hex-of-session-id>.json` bound to that
issue+worker+branch+claim. A second claim reusing the same `session_id` for a different
issue fails closed with `{"claimed":false,...,"reason":"heartbeat_write_failed"}`.
It is NOT a lost race (rc 2) and NOT a config error — it is the sidecar refusing an
identity mismatch. The failed attempt leaves the issue's labels untouched, so no cleanup
is needed.

**Why:** this is what enforces the SKILL's "ONE issue at a time, sequential" monitor rule.
In-session parallelism via the claim path is therefore impossible by design.

**How to apply:** for real parallelism, run separate harness sessions each with a distinct
session id (and distinct `AUTOSPEC_WORKER_ID`), which is the documented concurrent-worker
path. Do not try to fan out multiple claims from one orchestrator session — and do not
misread `heartbeat_write_failed` as a transient fault worth retrying. See
[[feedback_per_session_worktree_isolation]] and [[reference_harness_session_id_envs]].

**2. `autospec queue ready` reports `parallel_safe: true` on issues with `paths: []`.**
That flag reflects ABSENT path data, not proven disjointness. On the self-originated
`code_health` (STRING_MATCH_DOMAIN_LOGIC) backlog, many issues target the SAME file:
#2599 and #2600 both target `crates/autospec-cli/tests/autonomous_conductor_commands.rs`;
#2602 and #2603 both target `crates/autospec-cli/tests/cli_commands.rs`.

**How to apply:** before any parallel dispatch, extract the real target from the issue body
marker rather than trusting the flag:
`jq -r '.ready[]? | "\(.number)\t\(.body|capture("path=(?<p>[^ ]+)").p)"'`
then group by path and serialize within each group. claim-guard's `acquire` is the
last-line defense, not the first. Relates to [[feedback_self_consistent_test_fixtures_mask_bugs]].
