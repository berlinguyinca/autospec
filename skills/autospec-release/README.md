# autospec-release

Use `autospec-release` when you want one repo-level answer to the question:
"Is this ready to ship?"

It is a wrapper over the working autospec skills. It runs the release-readiness
loop across sweep, review, implementation, tests, QA proof artifacts,
documentation drift, and legacy cleanup.

## Use it when

- You are done with feature work and want a release gate.
- You want specs, docs, tests, QA, and code reality checked together.
- You want stale legacy code and stale spec references removed or tracked.
- You want a clear `PASS`, `PARTIAL`, or `FAIL` verdict.

## What it runs

1. Preflight: branch, dirty files, issues, PRs, and repo state.
2. `/autospec-sweep`: recurring spec/docs/tests/code drift.
3. Spec/docs/story sync: stale claims and legacy references.
4. `/autospec-review`: spec-vs-issue regression gaps.
5. `/autospec-classify` and `/autospec-run`: remaining implementation queue.
6. `/autospec-test`: release test gates.
7. `/autospec-qa`: running-app proof, controls, validation, no-mock smoke.
8. Legacy cleanup: dead code, stale docs, old config, old infra.
9. Final verdict: `PASS`, `PARTIAL`, or `FAIL`.

## Install

From this checkout:

```bash
./skills/autospec-release/install.sh --harness all
```

Through the top-level installer:

```bash
./install.sh --skill autospec-release --harness all
```

## Output

The final report lists what ran, what passed, what changed, and what still
blocks release. It must separate proven evidence from `NOT TESTED` gaps.
