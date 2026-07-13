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

Full test suite gate AUTOSPEC_FULL_TEST_COMMAND scripts/validate.sh
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
