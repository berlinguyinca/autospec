<!-- autospec-test-report-marker -->
## autospec-test — ❌ Blocked

**Mode:** scoped-production
**Stage 1 (unit):** passed
**Stage 2 (E2E):** failed

### Why blocked
- e2e:scope-violation — test mutated an out-of-scope row (family_id outside allowed values)
- Restore invoked: ✅ succeeded

### Next steps for human reviewer
1. Fix the test to only access rows matching the allowed scope token (family_id = test-family-fixture-01).
2. Re-run after fixing the scope violation.
