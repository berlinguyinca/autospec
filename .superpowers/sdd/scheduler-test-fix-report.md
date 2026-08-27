# Scheduler test fix report

## What was wrong

`fleet-run.sh` was migrated from printing a one-shot `/autospec-run --profile
... --worker-id ...` command to launching a perpetual conductor
(`autospec-autonomous start --repo-dir <checkout> --repo <slug>`). A
`--detach` flag was briefly part of that command and then removed in
`fleet-lib.sh` (`fleet_worker_command`, ~line 111) because the real
`autospec-autonomous` binary's parser has no `--detach` case and rejects it.
`tests/fleet/*.bats` was updated for the new contract at the same time, but
`tests/unit/test_autospec_fleet_scheduler.bats` was missed and still
asserted the stale `--detach` command, so it could never pass.

## What each test protects, and how the fix preserves it

- **Test 1 — "fleet-run dry-run emits autospec-autonomous conductor commands
  for eligible repos"** (renamed from "...autospec-run commands..." to match
  the current command family; title only, no behavior change). Protects: a
  dry run emits one launch line per eligible repo, with the correct
  checkout path and repo slug wired into the command. Old assertions
  independently grepped for `--detach`, `--repo org/repo-a`, `--repo
  org/repo-b`, `org__repo-a`, `org__repo-b` — a scattered set of substrings
  that would still all match even if flags were reordered or extra
  (wrong) flags were injected. New assertions grep for the two full command
  lines verbatim:
  `autospec-autonomous start --repo-dir /tmp/fleet/repos/org__repo-a --repo org/repo-a`
  and the `-b` equivalent — derived directly from `fleet_worker_command` in
  `fleet-lib.sh` and `repo_checkout_path`/`repo_slug`, using the
  `workspace: /tmp/fleet/repos` from the test's `fleet-node.yml`. This is
  stricter: it fails if `--detach` (or any other flag) reappears anywhere in
  the line, not just if it's missing from one specific grep.

- **Test 2 — "fleet-run caps output at parallel_repos 2"**. Protects: dry-run
  output is capped at `parallel_repos` (here 2) even though 3 repos are
  eligible, and repo-c (the 3rd) never appears. Old assertion counted
  occurrences of `autospec-autonomous start --detach` (which never matches,
  since the string doesn't exist — this test was actually counting zero
  matches against an expected 2, an unconditional failure) and separately
  checked repo-c is absent. New assertion counts occurrences of the full
  command shape (`autospec-autonomous start --repo-dir ... --repo
  org/repo-[ab]`) via `grep -c -E`, still requiring exactly 2, and keeps the
  repo-c absence check unchanged.

- **Test 3 — "fleet-run skips repos with profiles unavailable on the
  node"**. Untouched. Confirmed it passes for the right reason: the
  node-config here only allows `claude-sonnet-cloud`, so
  `node_allows_profile` rejects every repo in `fleet.yml` (all configured
  for `qwen3-6-35b-a3b-laptop`) before `queue_has_work` or
  `fleet_worker_command` is ever reached — output is empty by construction,
  not by coincidence.

## Other stale references found

None beyond the `--detach` string and the test-1 title. Grepped the file
for `--profile`, `--worker-id`, and `/autospec-run` (remnants of the old
one-shot command) — the only match was the test-1 title text itself
("...emits autospec-run commands..."), which was cosmetic and has been
renamed. No other assertion in the file referenced the retired command
shape.

## Red/green proof

**Green (current `fleet-lib.sh`, current test file):**
```
$ bats tests/unit/test_autospec_fleet_scheduler.bats
1..3
ok 1 fleet-run dry-run emits autospec-autonomous conductor commands for eligible repos
ok 2 fleet-run caps output at parallel_repos 2
ok 3 fleet-run skips repos with profiles unavailable on the node

$ bats tests/fleet/
1..28
ok 1..28  (all pass; 28 tests total in tests/fleet/, not 41 — see note below)
```

**Red (temporarily reintroduced `--detach` in `fleet_worker_command`,
`fleet-lib.sh` line 111 changed to
`printf 'autospec-autonomous start --detach --repo-dir %s --repo %s\n' ...`):**
```
$ bats tests/unit/test_autospec_fleet_scheduler.bats
1..3
not ok 1 fleet-run dry-run emits autospec-autonomous conductor commands for eligible repos
# `printf '%s\n' "$output" | grep -q -- 'autospec-autonomous start --repo-dir /tmp/fleet/repos/org__repo-a --repo org/repo-a'' failed
not ok 2 fleet-run caps output at parallel_repos 2
# `count="$(printf '%s\n' "$output" | grep -c -E -- '...')"' failed
ok 3 fleet-run skips repos with profiles unavailable on the node
```
This confirms tests 1 and 2 now catch the exact regression that previously
shipped to main (`--detach` reappearing in the conductor command).

`fleet-lib.sh` was restored from a copy (`cp` + edit back, never `git
checkout --`), and `diff` against the pre-edit copy confirmed byte-identical
restoration. `git status --short` after restoration shows only
`tests/unit/test_autospec_fleet_scheduler.bats` modified.

**Note on the "41" expected count for `tests/fleet/`:** actual count is 28
(`test_fleet_gui_skeleton.bats`: 2, `test_fleet_gui.bats`: 17,
`project-board-fleet.bats`: 9 = 28). All 28 pass both before and after this
change; `fleet-lib.sh` was untouched in the final state, so this is not a
regression introduced here — the 41 figure in the task brief did not match
what `bats tests/fleet/` actually collects on this branch.

## Safety

`autospec-autonomous` was stubbed at `/tmp/stubbin/autospec-autonomous`
(prints a warning and exits 99) and placed first on `PATH` for every bats
invocation in this session, so no test could spawn a real conductor or
reach GitHub. All tests in both suites are `--dry-run`/mocked and never
exercised the live spawn path with the stub in place.
