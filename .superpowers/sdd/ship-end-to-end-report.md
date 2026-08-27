# `/autospec-project ship <board-url>` end-to-end wiring

## Division: prose vs shell

`bare`, `sync`, and `status` stay pure prose (a sequence of bash snippets an
agent runs directly) because each is a straight-line composition of
already-independent, already-tested scripts with no cross-cutting invariant
that has to hold identically on every call.

`ship` is different: it has a hard security boundary (the repo allowlist)
that must be enforced identically and first, before any board read or
mutation, and a per-repo failure-isolation contract (one repo's provisioning
or launch failure must never abort the others) that spans six steps. Both
of those are exactly the kind of thing that rots if re-derived by an agent
from prose each time. So `ship` is now backed by one script,
`scripts/project-ship.sh`, and the `SKILL.md` section for `ship` was
rewritten to describe what the script does rather than re-implementing the
chain in prose. This mirrors the existing precedent in this codebase
(`fleet-init.sh`, `fleet-run.sh`, `autonomous-promote-open-issues.sh` are
all scripts, not prose, for the same reason — a security-relevant loop with
per-item isolation belongs in one tested place).

`project-ship.sh` does not shell out to `fleet-init.sh` as a subprocess. It
sources `fleet-lib.sh` directly and calls `fleet_provision_repo` itself,
the exact function `fleet-init.sh` uses internally — this means `ship` gets
the same tested clone/fetch/dirty/non-ff logic without adding a second
install dependency on `fleet-init.sh` (which, as an aside, is *not*
currently registered in `skills/autospec-fleet/install.sh`'s
`FLEET_SCRIPT_FILES` — an existing gap in a script this task's own
provenance note said was "real, idempotent, never-destroys-local-work,"
left alone since fixing an unrelated skill's installer was out of scope
here). `project-ship.sh` does still shell out to the real `fleet-run.sh`
for the launch step, since that script owns capacity, liveness, and queue
probing — duplicating that would be exactly the kind of re-derivation this
design avoids.

`skills/autospec-project/install.sh` was updated to register
`project-ship.sh` (repo-root `scripts/`, so also auto-copied by the
top-level `install.sh`'s wildcard `scripts/*.sh` copy) plus `fleet-lib.sh`,
`fleet-run.sh`, and `fleet-config-lint.sh` — mirroring
`autospec-fleet/install.sh`'s own `FLEET_SCRIPT_FILES` set — so a fresh
`autospec-project`-only install lands every file `ship` needs. Verified
with a real isolated-`$HOME` install run (see Red/Green proofs below).

## Per-repo reporting shape

Every line `project-ship.sh` prints is one of exactly three shapes, so an
operator (or the calling skill) can `grep` for ground truth instead of
reading prose:

```
project-ship: repo=<owner/repo> allowlisted=no action=skipped reason=not-allowlisted
project-ship: repo=<owner/repo> allowlisted=yes provision=<ok|skipped:dirty|skipped:not-fast-forward|failed>
project-ship: repo=<owner/repo> allowlisted=yes launch=<launched|skipped:checkout-not-found|skipped:no-ready-work-or-capacity|failed>
```

A dry-run substitutes a single `action=plan-provision checkout=<path>` line
per allowlisted repo instead of the provision/launch pair, and never writes
or spawns anything. `fleet-run.sh`'s own raw output is also echoed
verbatim beneath the summary lines, so nothing is hidden — the summary
lines are a derived, greppable index into it, not a replacement for it.

Note on the dirty/non-fast-forward distinction: `fleet_provision_repo`
(fleet-lib.sh) returns 0 (success) for both those cases — it did the right
thing by not touching the checkout, but a naive `if fleet_provision_repo;
then ok; else failed; fi` would misreport "dirty, left alone" as
"provisioned." `project-ship.sh` instead captures the function's combined
stdout+stderr and pattern-matches on the specific diagnostic text
(`"local changes present"` / `"would not fast-forward"`) to report the
correct status. This was caught by the mutation-testing pass (see below) —
an earlier draft reported `provision=ok` for a dirty checkout, and the
dirty-checkout test (RED as designed) caught it immediately.

## Prose: before vs after

**Before**, the `ship` section said (verbatim): *"it still does not clone
or sync checkouts, so a board repo with no local checkout is skipped with
'checkout not found', not launched"* and *"checkout cloning is a separate
follow-up plan; until it lands…"* — both false as of `fleet-init.sh`
landing, and actively misleading since the skill never even invoked it.

**After**, the section describes the real six-step chain (allowlist gate →
resolve → filter → write fleet config → provision → launch), states the
exact per-repo status vocabulary, and the closing paragraph explicitly
says: *"`ship` genuinely is the unattended multi-repo pipeline now: given a
board and an allowlist, it clones what's missing, updates what exists
(never destructively), and launches a conductor per eligible repo in one
call — there is no more 'clone it yourself first' step."*

What is *not* overclaimed, stated explicitly in "Current scaffold status":
`ship` launches conductors, it does not babysit them (liveness/health
monitoring is `autospec-autonomous`'s own surface), and any live-server-
dependent metric is out of scope for board ingestion entirely.

## Non-negotiables, how each is enforced

- **Allowlist is the security boundary.** Checked first (`project-ship.sh`
  Step 1), before the board is even resolved. Empty/unset allowlist → exit
  3, zero `git`/`gh`/`autospec-autonomous` calls. A resolved-but-
  non-allowlisted repo is filtered out (Step 3, same prefix-or-equality
  match as the promoter's `board_stage()`) before the fleet config is
  written, before provisioning, before launch — it is never passed as an
  argument to `git` or `autospec-autonomous`. Proven in
  `tests/fleet/project-ship.bats` by asserting the denied repo string is
  absent from the git-call log and the spawn log, not just absent from
  printed text.
- **Never destroy local work.** `project-ship.sh` calls
  `fleet_provision_repo` (the same function `fleet-init.sh` uses) with no
  new code path around it — the dirty/non-ff refusal in `fleet-lib.sh` is
  untouched.
- **Failure is per-repo.** Every fallible call inside the provisioning and
  reporting loops is a deliberate `if/then`, never a one-sided `&&`, since
  the script runs under `set -euo pipefail`. Proven by a mutation that
  removed the `|| provision_rc=$?` fallback (test 2 went RED as expected).
- **Honest reporting.** See "Per-repo reporting shape" above.

## Red/Green proofs

All 7 new tests in `tests/fleet/project-ship.bats`, plus the 6 pre-existing
`tests/fleet/*.bats` files, were run together; every new test was proven to
independently catch a real regression by sabotaging `project-ship.sh`,
confirming RED, then restoring via `cp` from a saved copy and `diff`-
confirming byte-identical restoration before re-running GREEN:

| Mutation | Test(s) that went RED |
|---|---|
| `select(allowed(.))` → `select(true)` (bypass allowlist filter entirely) | test 1 (cross-contamination) and test 5 (empty-intersection no-op) |
| drop `\|\| provision_rc=$?` (one-sided failure handling under `set -e`) | test 2 (per-repo isolation) |
| `"local changes present"` case → `provision_status="ok"` | test 3 (dirty-checkout honesty) |
| `if [ "$allowlist_count" -eq 0 ]` → `if false` (skip the refuse gate) | test 4 (clean no-op) |
| dry-run `continue` → `true` (fall through into real provisioning) | test 6 (dry-run isolation) |
| `if [ "$resolve_rc" -ne 0 ]` → `if false` (swallow resolve failure) | test 7 (resolve-failure propagation) |

Each mutation was applied with `sed`/`python3`, confirmed RED with
`bats tests/fleet/project-ship.bats`, then reverted with
`cp <scratchpad>/project-ship.sh.orig scripts/project-ship.sh` and verified
`diff` showed zero output before re-running GREEN. Final state committed is
identical to the diff-verified original.

Additionally, a real (non-mocked) `skills/autospec-project/install.sh
--harness claude` run against an isolated `$HOME` in `/tmp` confirmed
`project-ship.sh`, `fleet-lib.sh`, `fleet-run.sh`, and
`fleet-config-lint.sh` all land in the installed layout and that
`project-ship.sh --help` resolves its dependencies correctly from that
flattened directory (the installed-layout fallback path, not just the
dev-tree fallback exercised by the bats suite).

## Full test run (real numbers)

- `bats tests/fleet/project-ship.bats` — 7/7 pass (new)
- `bats tests/fleet/` (full directory, includes the above) — 48/48 pass
- `bats tests/autospec/project-board-*.bats` — 186/186 pass, exit 0
- `bats tests/derive-trio.bats tests/gen-skill-goldens.bats` — 24/24 pass

No skips, no failures, in any of the above.
