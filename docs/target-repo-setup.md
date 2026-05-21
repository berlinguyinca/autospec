# Target repo setup

This guide is for repository operators who want to run autospec
(`/autospec-define` + `/autospec-run`) against their own codebase. It
documents the two pre-merge gates that close the cross-session CI rot
documented in [issue #307] and the design spec
[`docs/specs/2026-05-17-cross-session-ci-rot-design.md`].

[issue #307]: https://github.com/berlinguyinca/autospec/issues/307
[`docs/specs/2026-05-17-cross-session-ci-rot-design.md`]: ../docs/specs/2026-05-17-cross-session-ci-rot-design.md

Both gates are opt-in. autospec keeps working against repos that have
not enabled them — you just lose the cross-session safety net.

## 1. Required branch protection on `main`

The Phase 4 implementer's rebase-and-retest gate
(`docs/specs/2026-05-17-cross-session-ci-rot-design.md` § Change α)
re-proves each PR against post-merge `main` before admin-merging. Pair
it with branch protection so GitHub also refuses to merge a stale
branch even if `--admin` is used: belt-and-suspenders.

Required settings on `main`:

- **Require status checks to pass before merging** — enabled.
- **Require branches to be up to date before merging** — enabled.
  Forces GitHub to refuse merges of stale branches even when an admin
  override is in play. This is the kernel of the cross-session
  protection.
- **List required status checks** — every check that must pass for an
  admin-merge to proceed. Project-specific; common examples:
  `build`, `unit-tests`, `lint`, `integration-tests`.

The protection is wired via GitHub's branch-protection API — the
operator-facing flag is `required_status_checks.strict = true` plus the
list under `required_status_checks.contexts`. The verification one-liner
below queries both.

## 2. Migration-replay test convention

The Phase 4 implementer's migration-replay hook
(`docs/specs/2026-05-17-cross-session-ci-rot-design.md` § Change β,
landed as PR #315) runs your target repo's migration-replay test before
opening a PR whose diff touches a migration path. If the test exits
non-zero, the implementer comments the failure on the issue and aborts
before `gh pr create` — the regression must be fixed first.

Opt in by providing exactly **one** of the following test invocations.
The implementer probes them in this order and uses the first hit:

1. **`make migrate-test`** — if `Makefile` exists and contains a
   `^migrate-test:` target.
2. **`npm run migrate:test`** — if `package.json` exists and
   `.scripts."migrate:test"` is non-null.
3. **`bin/migrate-test`** — if the script exists at `bin/migrate-test`
   and is executable.
4. **`pytest tests/migrations`** — if a `tests/migrations/` directory
   exists; invoked as `pytest tests/migrations -x`.

The test you provide MUST:

- Exit non-zero on regression.
- Run **all** migrations on a fresh database (not against a
  pre-migrated state). Replay-order regressions only surface when the
  full migration chain is exercised end-to-end.

Recommended skeletons:

- **Postgres + Goose**: spin a throwaway container, run `goose -dir
  <migrations_dir> up`, run a schema or smoke assertion, then
  `goose -dir <migrations_dir> reset`.
- **Postgres + Alembic**: same pattern with `alembic upgrade head`
  followed by `alembic downgrade base` (or a fresh DB per run).
- **Node + Knex**: `knex migrate:latest` on a throwaway DB, then assert
  the resulting schema.

If none of the four conventions is detected, the implementer logs an
informational line and continues without the replay check — the gate
is voluntary.

## 3. Why this matters

The cross-session CI rot incidents documented in [issue #307] are not
detectable by per-PR CI alone. Two PRs can each pass independently
against pre-merge `main` and still leave `main` broken after both
squash-merge:

- **Incident #1** (duplicate `:=`): stale branch at merge — `main`
  moved underneath PR B between its CI and its squash. PR B's CI was
  never run against PR A's already-merged state.
- **Incident #2** (`CHECK` constraint allow-list dropped by a
  later-timestamped migration): semantic conflict invisible to static
  per-PR CI. The two migration files don't textually conflict; the
  regression only appears when Goose runs them in timestamp order on a
  fresh DB.
- **Incident #3** (settings-tabs refactor + unrelated failing test): a
  duplicate of #1 in test-fixture territory. The refactor and the
  broken tests live in the same suite, but no single PR ever ran with
  both diffs applied.

Branch protection + the rebase-and-retest gate close #1 and #3. The
migration-replay hook closes #2.

## 4. Verification

After enabling the settings above, paste this one-liner to confirm both
gates are wired:

```bash
# Branch protection check — exits 0 only when strict=true AND at least
# one required context is configured. `--jq` evaluates a predicate and
# returns the boolean as text; gh exits non-zero when --jq's expression
# raises, but `select` may emit nothing without raising, so we test the
# stdout instead.
strict_ok=$(gh api repos/<owner>/<repo>/branches/main/protection \
  --jq '(.required_status_checks.strict == true) and ((.required_status_checks.contexts | length) > 0)' \
  2>/dev/null)
if [ "$strict_ok" = "true" ]; then
  echo "branch protection OK"
else
  echo "branch protection NOT OK (need strict=true AND at least one required context)" >&2
fi

# Migration-replay target check. Use metadata-only inspections — never
# execute the migration test as part of detection. `npm run ... --dry-run`
# does NOT mean "don't execute the script" — it still invokes node and
# may fire the script's side effects. Inspect package.json with jq instead.
{
  ( [ -f Makefile ]    && grep -qE '^migrate-test:' Makefile ) \
  || ( [ -f package.json ] && command -v jq >/dev/null 2>&1 \
       && [ "$(jq -r '.scripts."migrate:test" // empty' package.json)" != "" ] ) \
  || [ -x bin/migrate-test ] \
  || [ -d tests/migrations ];
} && echo "migration-replay target detected" \
  || echo "migration-replay target NOT detected (target repo has not opted in)" >&2
```

The first block exits the `strict_ok == true` branch only when
`required_status_checks.strict` is `true` AND at least one required
context is listed. The second block uses metadata-only inspections
(`grep`, `jq`, `test -x`, `test -d`) — it never invokes `make`, `npm`,
or `pytest`, so running it against a target repo is side-effect-free.

Once both lines print their `OK`/`detected` strings, autospec's
Phase 4 implementer will run both gates against PRs targeting your
`main`.

## How to enable autospec-test

`autospec-test` gates every Phase 4 PR on unit + E2E coverage and
auto-heals gaps within a 60-minute coding budget. Enabling it requires
three steps: install Playwright (if not already present), write
`.autospec/test.yml`, and optionally run the wizard.

### 1. Install Playwright in your target repo

```bash
npm init playwright@latest
# Accept defaults; choose TypeScript if your repo uses it.
# Commit playwright.config.ts and the generated test skeleton.
```

If your repo uses a different test framework for E2E (e.g. Cypress),
the skill gates on Playwright only. Cypress repos should set
`forbidden_url_patterns_intentionally_empty: true` and omit E2E stage
until a Playwright suite is added.

### 2. Write `.autospec/test.yml`

Create `.autospec/test.yml` in your target repo with at minimum:

```yaml
# .autospec/test.yml
mode: strict_isolation          # default; safe for all repos

e2e:
  clone_url_env: E2E_BASE_URL   # env var pointing at your isolated clone
  forbidden_url_patterns:
    - "^https?://app\\.example\\.com"      # your production URL(s)
    - "^https?://.*\\.prod\\.example\\.internal"

budgets:
  coding_time_minutes: 60       # LLM coding time; test runtime is unbounded
  max_loop_iterations: 5
  same_error_halt_threshold: 3
```

**Fail-closed rule:** if `forbidden_url_patterns` is missing or empty
(without `forbidden_url_patterns_intentionally_empty: true`), the skill
refuses to run and labels the PR `e2e:contract-error`. Always declare
at least one forbidden pattern covering your production URL.

Commit `.autospec/test.yml` to your repo's default branch.

### 3. Run the wizard (optional but recommended)

```bash
/autospec-test --init
```

The wizard walks through mode selection, detects backup drivers for
Mode II (scoped production), prints a dry-run preview of constraints,
and requires you to type `I UNDERSTAND` before writing any files.

### What the labels mean

| Label | Meaning |
|---|---|
| `e2e:passed` | Both stages green; auto-merge proceeds |
| `e2e:healed` | Gate was red; self-heal loop fixed it; auto-merge proceeds (subject to assertion-shift guardrail) |
| `e2e:blocked` | Loop exhausted budget or iteration cap; needs human review |
| `e2e:stuck-error` | Same error in 3 consecutive iterations; needs human review |
| `e2e:refused` | Skill refused to run (missing contract or forbidden URL check); fix `.autospec/test.yml` |
| `e2e:contract-error` | Contract validation failed (missing `forbidden_url_patterns`, Playwright not found, etc.) |
| `e2e:assertion-loosening` | Loop weakened a test assertion; needs human review before merge |
| `needs-human-review` | Combined with any of the above blocked states |
| `e2e:scoped-prod` | Mode II run (touches production under declared scope tokens) |
