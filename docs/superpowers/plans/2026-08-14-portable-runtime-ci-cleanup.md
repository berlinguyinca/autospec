# Portable Runtime CI Cleanup Plan

## Goal

Make PR #3146 satisfy every non-advisory CI gate without weakening the reviewed
Linux, macOS, Windows, or FreeBSD ownership and heartbeat contracts.

## Locked behavior

Before refactoring, preserve these green regression suites:

- portable heartbeat publication and retirement: 15 tests;
- portable executor reconciliation and no-replay: 10 tests, one helper ignored;
- process-tree ownership: 10 tests;
- draft-release recovery: 7 tests;
- workflow behavior contract: one Bats test;
- strict workspace Clippy, workspace build, and Linux/Windows/FreeBSD target checks.

The CI-specific Linux and Windows failures receive focused failing regressions or
native reproductions before their production fixes.

## Cleanup lanes

1. **CI and integration-test scope**
   - Make `run_exact` preserve and print a failing test status instead of exiting
     inside command substitution.
   - Narrow Windows compile checking to the Autospec binary; native Windows unit
     behavior remains executed explicitly.
   - Remove the branch-only Unix crate attributes from integration tests so
     existing oversized test files no longer grow.

2. **Portable heartbeat**
   - Fix the native Windows no-replace publication failure proven by CI.
   - Split the 2,695-line module into sub-600-line platform, publication,
     retirement, and test modules while preserving a small API facade.
   - Keep descriptor/handle-relative access, exclusive publication, immutable
     identity, and FreeBSD linked-alias recovery unchanged.

3. **Executor portability and process ownership**
   - Split portability tests and process-owner backends into sub-600-line sibling
     modules.
   - Keep quarantine admission fail-closed and retain OS ownership resources.

4. **Executor bridge extraction**
   - Move portable file I/O, path resolution, process identity, reviewer capture,
     and other branch-added blocks from the oversized bridge monolith into
     cohesive sibling modules.
   - Shrink the original file to at most its merge-base line count.

5. **Incidental oversized-file growth**
   - Relocate branch-added helpers from autonomous, drain, trusted-git,
     resilience, claim, runtime, ready-queue, state, and validation monoliths into
     existing or focused sibling modules.
   - Each changed file above 600 lines must finish no larger than its merge-base
     version.

6. **Security findings**
   - Attach invariant proofs directly to the two newly detected unsafe boundaries
     or replace them with safe test/process APIs where feasible.
   - Re-run the repository security workstream; do not suppress findings through
     workflow policy changes.

## Verification and stop condition

Run the file-size ratchet locally against `origin/main`, the security workstream,
formatting, strict Clippy, workspace build, focused behavior suites, workflow
Bats, and all available target checks. Obtain a fresh independent `LGTM`, push
the correction commits, and merge only after every non-advisory PR check passes.
