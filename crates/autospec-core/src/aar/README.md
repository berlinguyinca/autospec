# Adaptive Agent Runtime (AAR)

Provider-neutral contract for running planner, scout, builder, test, and
reviewer roles as supervised agent sessions. Design:
[`docs/specs/2026-09-02-adaptive-agent-runtime-design.md`](../../../../docs/specs/2026-09-02-adaptive-agent-runtime-design.md).

## Module map

- `classify/` — issue classification and model-fit rubric.
- `context.rs` — context policy (full history excluded by default).
- `dispatch_fit.rs` — slot/endpoint fit for an issue's context needs.
- `escalation.rs` — model fallback that re-checks separation of duties.
- `guards.rs` — edit and reasoning budgets.
- `inferweave.rs` — fleet probing and liveness checks.
- `memory.rs` — durable per-task memory files.
- `outcome.rs` — structured task outcomes.
- `pi.rs` — Pi harness adapter: session specs, event folding, working rules.
- `policy.rs` — `RolePolicy` per `AgentRole`; only `Builder` mutates.
- `profile.rs` — sampling/reasoning profiles.
- `reasoning.rs` — reasoning token budgets (tiny 512 … exceptional 8192).
- `telemetry.rs` — token accounting (prompt = cached + new prefill).
- `topology.rs` — session topology and `enforce_separation`.

## Session isolation (`isolation.rs`, issue #3324)

Planner, builder, and reviewer run as **distinct sessions**; read-only lanes
may share a worktree in parallel, while every mutating (`Builder`) session
requires **exclusive worktree ownership**.

- `SessionIsolation::grant(SessionGrant)` — admit a session. Fails closed on
  `EmptyField`, `DuplicateSession`, `PolicyMismatch`, `WorktreeCollision`
  (a second writer on one worktree), and `UnknownSession`.
- `SessionIsolation::finish(session_id, ObservedEdits)` — settle a session.
  A read-only session that reports filesystem edits is a `ReadOnlyBreach`
  and fails closed. Writer claims release on finish.
- `ReviewVerdict::parse` — reviewer output validates as exactly one of
  `approve | changes_required | uncertain`.
- `SessionArtifact` — structured plans, diffs, test receipts, and review
  verdicts passed between sessions; JSON wire format tagged by `kind`.
- `ObservedEdits` — observed filesystem edits, with
  `From<&PiSessionResult>` so Pi event folds feed the check directly.

Coverage: `tests/aar_session_isolation.rs` (cargo) and
`tests/pi-multi-role-isolation.bats` (repo root); structural gate:
`scripts/validate-aar.sh`.
