# Agent Integration Contracts

## Version

V67

## Objective

Normalize handoff and result handling across Codex, Claude Code, Fable, and generic agent runners.

## Scope

- Agent runner trait.
- Codex handoff prompt.
- Claude/Fable handoff prompt.
- Generic runner contract.
- Output normalization.
- Validation result ingestion.
- Safe mode for destructive operations.

## Non-Goals

- No hosted orchestration service.
- No hidden telemetry.

## Dependencies

- `v66-autonomous-execution-queue`

## Files To Create/Modify

- Create: `crates/autospec-core/src/agent/mod.rs`
- Create: `crates/autospec-core/src/agent/contract.rs`
- Create: `crates/autospec-core/tests/agent_contracts.rs`
- Create: `prompts/agent-handoff/codex.md`
- Create: `prompts/agent-handoff/claude.md`
- Create: `prompts/agent-handoff/fable.md`
- Create: `schemas/autospec-agent-result.schema.json`
- Modify: `docs/architecture.md`

## Implementation Steps

1. Define normalized `AgentTask` and `AgentResult`.
2. Define required output fields: result, files changed, validation, blockers, handoff.
3. Add handoff prompt templates for Codex, Claude/Fable, and generic runners.
4. Implement safe-mode checks for destructive operations before runner invocation.
5. Add tests for result normalization and unsafe-operation rejection.

## Acceptance Criteria

- [ ] Agent result schema validates sample Codex/Claude/Fable outputs.
- [ ] Safe mode blocks destructive operations by default.
- [ ] Validation results can update queue state.
- [ ] Handoff prompts cite spec id and validation command.

## Validation Commands

```bash
cargo test --all agent_contracts
bash scripts/validate.sh --fast
```

## Expected Outputs

- Normalized `AgentResult` JSON ingested into run state.

## Rollback/Handoff Notes

If Fable-specific details are unknown, implement Fable as the generic runner template with a named adapter placeholder and explicit limitations.
