<!-- autospec-test-report-marker -->
## autospec-test — ❌ Blocked

**Mode:** strict-isolation
**Stage 1 (unit):** passed
**Stage 2 (E2E):** failed

### Why blocked
- assertion_shift: LOOSENING — peak_detector test tolerance widened without JUSTIFICATION

### Next steps for human reviewer
1. Add `JUSTIFICATION: <reason>` to the commit message for the assertion change, or revert the loosening.
