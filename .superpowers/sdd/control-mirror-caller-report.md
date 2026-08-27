# project-board-control-mirror.sh caller — report

## The seam chosen

`scripts/lib/autospec-loop.sh`, `autospec_conductor_run()`, new **Step 1c**,
inserted immediately before the existing **Step 2: Tier-0 control-channel
poll** (`bash "$_control_ch" ...`) and after Step 1b (persona mining).

Why this seam and not another: the conductor's per-cycle sequence is
Step 1 (resilience heartbeat) → Step 1b (persona) → Step 2 (Tier-0
control-channel poll, which reads reserved labels off the CURRENT repo and
can immediately `break` the loop on `graceful-stop`/`pause`) → Step 2b
(readiness) → Step 3 (waterfall) → Step 4/5 (drain). Step 2 is the only
place a repo ever reads reserved control labels. Mirroring must land labels
on each fleet repo's own marker issue strictly before that repo's own Step 2
runs, or the signal is invisible for that whole cycle and only takes effect
next cycle. Step 1c is the last possible point before Step 2, so it gives
same-cycle propagation — the strongest timing guarantee available without
restructuring Step 2 itself.

I considered a second seam — after Step 2, right before Step 3 (waterfall) —
which would still preserve every safety property but would delay the board
signal by exactly one full cycle for the repo currently running (Step 2 for
*this* cycle would already have executed). Rejected: it is strictly worse on
timing with no compensating benefit, since Step 1c has no dependency on
anything Step 2 produces.

## How config reaches it

No new `AUTOSPEC_*` operator-facing env var. Step 1c calls
`autospec autonomous project-board-config --repo-dir <repo_root>` — the same
Rust/shell bridge every other `project_board.*` consumer already uses
(`autonomous-promote-open-issues.sh`, `autonomous-spend-ledger.sh`). The
bridge binary is resolved via `AUTOSPEC_PROJECT_BOARD_CONFIG_BIN`, falling
back to the conductor's own already-resolved `_queue_bin`, matching the
existing convention.

**Bridge extended**: `format_project_board_config` in
`crates/autospec-cli/src/commands/autonomous.rs` did not previously emit
`control_issue` at all, even though `ProjectBoardConfig.control_issue` was
already parsed from `.autospec/autonomous.yml`. Added a `"control_issue"`
key (string or `null`) to the JSON, positioned right after `"allowlist"`.
Updated all 6 existing formatter unit tests for the new field and added a
dedicated test (`control_issue_flows_through_alongside_a_configured_url_and_allowlist`)
proving it is independent of the `url`/`repo_allowlist` gate.

Step 1c reads two fields from that JSON via `jq`:
- `.control_issue` — if empty/`null`, the mirror script is **never invoked**
  (not even a subprocess spawn) — the opt-in no-op.
- `.allowlist` (the board's `repo_allowlist`, joined with `,`) — used as
  **both** `--repos` and `--allowlist` to the mirror script. The fleet repos
  eligible to receive a project-level control signal are exactly the
  operator's own `repo_allowlist`; nothing wider is ever passed.

The mirror script itself (`scripts/project-board-control-mirror.sh`,
unmodified) still independently re-validates that the control issue's own
repo is inside that same allowlist, and skips wholesale (zero `gh` calls)
if not — Step 1c does not duplicate that gate, it relies on the script's
own enforcement, matching the "config-driven only" and "never a new
env var" constraints.

Advisory contract: `bash "$_pb_mirror_sh" ... >/dev/null 2>&1 || true` — the
trailing `|| true` is commented in place as the deliberate contract (mirror
failure must never fail/delay/alter the cycle), following the codebase's
established pattern for advisory calls.

## Timing guarantee

**Same cycle.** Step 1c runs to completion (or times out/fails silently)
strictly before Step 2's `bash "$_control_ch" ...` call in the same
iteration of the cycle loop. Proven directly in test 5 below: labels
mirrored by a stubbed-but-stateful `gh` in Step 1c are read back by the
REAL `autonomous-control-channel.sh` later in the *same* `autospec_conductor_run`
invocation (`CONDUCTOR_MAX_CYCLES=1`), producing `DECISION:graceful-stop`
in that single cycle's output.

## Red/green proofs

New file: `tests/autospec/project-board-control-mirror-caller.bats` (5 tests,
all green):

1. `control_issue set invokes the mirror once with control-issue/repos/allowlist`
2. `control_issue unset makes zero mirror invocations and zero gh calls`
3. `a mirror failure does not fail, delay, or alter the cycle`
4. `an out-of-allowlist control issue makes no gh call naming it`
5. `a mirrored autospec:stop label is honored in the same cycle`

RED proofs (each via `cp` to a temp backup, in-place break, `bats` run,
restore via `cp` + `diff` back to byte-identical):

- Break A: gated the whole Step 1c block behind `if false && ...` →
  tests 1, 3, 5 went RED (`not ok`, `[ -f "$MIRROR_LOG" ]` /
  `DECISION:graceful-stop received` assertions failed); tests 2 and 4
  stayed green (expected — a fully-disabled feature is indistinguishable
  from "correctly opted out" or "correctly gated" for those two negative
  assertions alone).
- Break B: forced `if true; then` around the control_issue check (removed
  the opt-in guard) → test 2 went RED (asserted `false`, i.e. a mirror
  invocation / gh call happened when none should have).
- Break C: appended a hardcoded `,outside/repo` onto the allowlist passed
  to the mirror script (simulating a caller bug that widens the allowlist
  beyond the board's own `repo_allowlist`) → test 4 went RED.
- After each break, `cp /tmp/autospec-loop.sh.bak scripts/lib/autospec-loop.sh`
  followed by `diff` confirmed byte-identical restoration before the next
  break or the final run. `git checkout --` was never used.

Final state: `diff /tmp/autospec-loop.sh.bak scripts/lib/autospec-loop.sh`
→ identical; `md5` matched pre- and post-break/restore.

## Test summary (final, foreground, real numbers)

- `cargo test -p autospec-cli project_board_config_tests` → **9 passed, 0 failed**
  (6 pre-existing formatter tests, updated for the new field, + 1 new
  dedicated `control_issue` test + 2 pass-through/independent-of-`url` tests
  already present).
- `bats tests/autospec/project-board-control-mirror.bats` → **23 passed**
  (script itself untouched; unaffected by this change — the brief's "expect
  20" was an earlier estimate, the suite has since grown to 23).
- `bats tests/autospec/project-board-config-wiring.bats` → **8 passed**
  (unaffected; JSON field-list is not order/shape-asserted there).
- `bats tests/autospec/project-board-control-mirror-caller.bats` → **5 passed**
  (new).
- `bats tests/autospec/test_conductor_wiring.bats` → **34 passed** (no
  regressions from the Step 1c insertion).
- `bats tests/autospec/test_conductor_accountability.bats` → **2 passed**
  (no regressions).
- `bash -n scripts/lib/autospec-loop.sh` and `bash -n scripts/project-board-control-mirror.sh`
  → both clean.
- `cargo build -p autospec-cli` → succeeds (only pre-existing, unrelated
  dead-code warnings in `claim.rs`).

## Safety properties verified

- **Opt-in**: `control_issue` unset → Step 1c never invokes the mirror
  script at all (test 2; also RED-proven via Break B).
- **Allowlist-gated**: the control issue's own repo and every target repo
  must be inside `project_board.repo_allowlist`, enforced by the (unmodified)
  mirror script; Step 1c never widens or substitutes that set (test 4;
  RED-proven via Break C).
- **Never blocks the conductor**: `|| true` on the mirror invocation, proven
  by test 3 — a mirror script that always exits 1 still lets the cycle reach
  the premerge gate and complete with exit status 0.
- **Additive only**: unchanged — Step 1c passes arguments straight through
  to the existing, unmodified `project-board-control-mirror.sh`, whose own
  `--add-label`-only design (never `--remove-label`) is untouched by this
  change.

---

## Addendum: `--repos` was the allowlist, not the fleet (fixed)

A reviewer probe found that the original Step 1c passed the board's
`repo_allowlist` (which may contain glob patterns, e.g. `"InferWeave/*"`,
the canonical example in this codebase) as **both** `--repos` and
`--allowlist` to the mirror script. `--repos` needs concrete `owner/name`
slugs; a glob pattern there makes the mirror try to operate on a literal
repo named e.g. `InferWeave/*`, which fails against the real GitHub API —
so the feature looked wired (a `gh` call happens, `--control-issue` is
correct) but silently mirrored to **nothing**, and a board-level
`autospec:stop` still would not reach any real fleet repo.

### Where the repo list now comes from

`--repos` is now sourced from resolving the board itself — the actual
repo set the board's items live in — never from echoing the allowlist
back. New helper `_autospec_conductor_pb_repos()` in
`scripts/lib/autospec-loop.sh` (placed beside the existing
`_autospec_conductor_board_state()` helper) calls
`scripts/project-board-resolve.sh --url <board-url> --emit repos`, which
returns the board's real repos as a JSON array (already the promoter's own
route to the same information via `--emit plan`, just the lighter `repos`
mode: no per-repo closed-issue join, only the field-list + item-list `gh`
call pair). The board URL comes from `.url` in the same
`autospec autonomous project-board-config` bridge JSON Step 1c already
parses for `control_issue` and `allowlist` — no new config surface.

`--allowlist` is unchanged: still the board's `repo_allowlist` verbatim
(globs and all), passed straight to the mirror script, which independently
re-checks the control issue's own repo AND every resolved repo against it
via its own `allowed()` gate. Step 1c does not duplicate that gate — it
relies on the mirror script's existing enforcement, same as before.

If the board has no `.url` configured (control_issue set on its own, no
board), `_autospec_conductor_pb_repos` returns empty immediately (no
resolver invocation, no `gh` calls) and Step 1c treats that exactly like
"no repos known this cycle" — it does **not** fall back to treating
allowlist entries as a repo list, even for allowlist entries that happen
to be glob-free literal names. That fallback was considered and rejected:
it would reintroduce a second, narrower version of the exact bug just
fixed (an unvalidated assumption that an allowlist string is a repo name),
for a use case (`control_issue` configured with no board at all) the brief
never asked for and this session has no test coverage proving safe.

### Caching decision and per-cycle cost

`_autospec_conductor_pb_repos` caches the resolved repo array on disk,
under `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/board-cache/<url-shasum>-repos.json`,
with the SAME TTL semantics `autonomous-promote-open-issues.sh`'s
`board_plan()` already uses (`project_board.ttl` from the bridge, default
300s; same URL-shasum keying scheme; same atomic per-process-tmp + `mv`
write pattern to avoid multi-worker races at a shared TTL boundary).

I deliberately did **not** try to opportunistically read the promoter's
own plan cache (`<url-shasum>.json`, which also carries a `.repos` field)
instead of resolving separately. Reusing it would make Step 1c's
correctness depend on whether Tier 1.5 promotion happened to already run
earlier in the *same* cycle and populate that cache fresh — an incidental
scheduling fact, not an explicit contract, and one that varies by whether
a given fleet even runs board-driven promotion at all. A dedicated
`-repos.json` cache file keeps Step 1c self-contained (its own cache
independent of another subsystem's read/write timing) while still sharing
the same directory and TTL *policy*, so an operator running both
subsystems on one board only has one TTL number to reason about, not two
different cache lifetimes.

Per-cycle cost:
- **Cache hit** (the overwhelming majority of cycles once warm): one
  `stat` + one `jq` parse of a small cached JSON array. No `gh` calls, no
  resolver subprocess exec.
- **Cache miss** (first cycle after start, or once every TTL seconds —
  default 300s / 5 minutes): exactly the two `gh` calls
  `project-board-resolve.sh --emit repos` documents (project field-list +
  item-list). It does not pay the `--emit plan` mode's O(repos) per-repo
  closed-issue join, which is gated on `--emit plan` only.

So a conductor polling every 60s (`CONDUCTOR_POLL_INTERVAL` typical value)
pays board-resolution network cost roughly once per 5 cycles, not once per
cycle — the same amortization ratio the promoter already relies on for its
own board reads.

### New red/green proofs

`tests/autospec/project-board-control-mirror-caller.bats` grew from 5 to
**8** tests. All safety properties from the first pass still hold and are
still covered (opt-in, mirror-failure-never-blocks, same-cycle visibility);
three tests are new or rewritten specifically for this bug:

1. **`a glob allowlist resolves through the board to concrete repos, never
   a literal glob`** (rewritten from the original "invoked once with
   right args" test) — allowlist `["o/*"]`, board resolves to
   `["o/a","o/b"]`. Asserts (via the REAL, unmodified
   `project-board-control-mirror.sh` and a logging `gh` stub) that no `gh`
   call anywhere contains a literal `*`, and that `o/a`/`o/b` are both
   named — end-to-end proof the glob never reaches the API and the real
   resolved repos do.
2. **`the control issue's own out-of-allowlist repo makes no gh call
   naming it`** (kept from the first pass, now exercised with a live
   resolve returning real in-scope repos so the mirror script's own
   `allowed(ctl_repo)` gate is what's actually being tested, not an
   incidentally-empty repo set).
3. **`a board-resolved repo outside the allowlist is skipped while allowed
   repos are still reached`** (new) — allowlist `["o/a"]`, board resolves
   to `["o/a","o/b"]`. Proves `o/b` (a genuine board repo the operator
   never allowlisted) never appears in any `gh` call while `o/a` does.
4. **`a resolver failure makes zero mirror invocations and leaves the
   cycle unaffected`** (new) — resolver exits 1; asserts zero
   `project-board-control-mirror.sh` invocations and the gate still runs.
5. **`an empty resolved repo set makes zero mirror invocations and leaves
   the cycle unaffected`** (new) — resolver returns `[]`; same assertions.

RED proof: reverted `--repos "$_pb_repos_csv"` back to
`--repos "$_pb_allowlist_csv"` (the exact bug reported) via a scripted
in-place edit, ran the suite:

```
not ok 1 conductor: a glob allowlist resolves through the board to concrete repos, never a literal glob
not ok 7 conductor: a resolver failure makes zero mirror invocations and leaves the cycle unaffected
not ok 8 conductor: an empty resolved repo set makes zero mirror invocations and leaves the cycle unaffected
```
(tests 2–6 stayed green — expected, since those properties are independent
of where `--repos` is sourced from). Restored via
`cp /tmp/autospec-loop.sh.bak3 scripts/lib/autospec-loop.sh` followed by
`diff` confirming byte-identical restoration. `git checkout --` was never
used at any point in this session.

### Final test numbers (foreground, real)

- `tests/autospec/project-board-control-mirror-caller.bats`: **8 passed**
- `tests/autospec/project-board-control-mirror.bats`: **23 passed**
  (unmodified script; confirmed independently grown to 23, matching the
  reviewer's note — not the original brief's estimate of 20)
- `tests/autospec/project-board-config-wiring.bats`: **8 passed**
- `tests/autospec/test_conductor_wiring.bats`: **34 passed** (no
  regressions)
- `tests/autospec/test_conductor_accountability.bats`: **2 passed** (no
  regressions)
- `bash -n` clean on `scripts/lib/autospec-loop.sh`,
  `scripts/project-board-control-mirror.sh`, and
  `scripts/project-board-resolve.sh`
