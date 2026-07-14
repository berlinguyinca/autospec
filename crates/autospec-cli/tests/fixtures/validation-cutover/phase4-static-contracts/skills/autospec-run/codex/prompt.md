<!-- guardian-block:begin -->
guardian body
<!-- guardian-block:end -->

Issue start summary
[monitor] starting #$ISSUE:
[monitor] goal:
[monitor] smoke:
[monitor] scope:

Immediate next-issue pickup: NO SLEEP after process(ISSUE)
fresh queue scan can pick any issue unblocked

BATCH_COMPLETE is a continuation signal, not a terminal state
reasoning:deep may reduce a single monitor batch to one issue
the orchestrator MUST relaunch automatically until ALL_DONE
Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE
Codex native subagents with explicit `agent_type`, `model`, or `reasoning_effort` MUST use a bounded handoff, not a full-history fork
RETRY-LOOP:begin MAX_IMPL_RETRIES directive_context Retry attempt
Implementer hit max retries; manual intervention needed
auto-implement-active

Full test suite gate AUTOSPEC_FULL_TEST_COMMAND autospec validate
If the full suite fails, fix the failure, recommit, rerun the full suite, and repeat
Do NOT dispatch LGTM review
Do NOT run `gh pr merge`
Record the exact full-suite command and passing output summary

data-scope invariant lens
empty optional filters reject unless documented
unsupported-filter empty-filter job-only sample-only job+sample

AUTOSPEC_BATCH_SIZE:-1
Fresh-subagent-per-issue (canonical Phase 4 path, formerly single-agent absorbed-discipline)
The orchestrator NEVER implements in its own context
the default is 1 (one issue per subagent)
reasoning:deep` force-to-1 rule is retained
> > 8. SUCCESS
<!-- token-report:begin -->
post-token-report.sh
<!-- token-report:end -->
> > 9. FAILURE

_REGEN resolveAutoRegenerate
<!-- docs-drift-gate:begin -->
doc-orchestrator.mjs
docs: regeneration skipped (auto_regenerate=false)
<!-- docs-drift-gate:end -->
