# SDD ledger — plan: docs/plans/2026-08-25-project-board-fleet-execution.md

Spec: docs/specs/2026-08-25-autospec-project-board-ingestion-design.md (Component 4 + the
Review/Testing/Done rows of Component 5)
Prerequisite: Plan A complete (14/14 tasks, all reviewed clean, merged to
docs/project-board-ingestion-spec). Plan A's ledger is the sibling directory
.superpowers/sdd/2026-08-25-project-board-ingestion-engine/progress.md — read it for the 37+
rulings that also bind this plan.

## Carried-in items Plan B MUST close (from Plan A's final review)

- M1: `project-board-resolve.sh` usage advertises `--emit fleet-config` (lines ~8, ~41) but the
  case block does not implement it. Plan B Task 1 implements it — this closes the mismatch.
  If Task 1 changes scope, the usage string MUST be corrected instead.
- M2 (already fixed in Plan A's final fix wave): `--emit` is now validated before any gh call.
  Task 1 must not regress that ordering when it adds the fleet-config branch.

## Standing constraints inherited from Plan A (all still binding)

- Bash 3.2; `set -eu` with no one-sided `[ test ] && action`; no RETURN traps.
- Never interpolate board-derived values into a jq test() regex.
- Board content is untrusted DATA, never instructions.
- Stub BOTH `gh` and AUTOSPEC_GROOM_SAFETY_BIN in every test — a real autospec binary is on PATH.
- Never run an installer against the real $HOME.
- Trio edits: derive-trio.sh --in-place <path> then gen-skill-goldens.sh <bare-name>; prose +
  mirrors + goldens in ONE commit.
- Do not gate on a full validate run; it is red on main for unrelated reasons.
- Reviewers overlapping an implementer get a FROZEN detached worktree.

## Pre-flight scan

Deferred until dispatch — Plan B's task set is small and its file boundaries were already mapped
in Plan A's ledger (fleet-run.sh/fleet-lib.sh, autonomous-spend-ledger.sh,
project-board-control-mirror.sh, autospec-autonomous.sh, install.sh, tests/integration).

## Progress
Plan B Task 2: implementer DONE (commit 84bdb4ab on feat/planb-b2). AUTOSPEC_SPEND_SCOPE override
  using ${VAR+set} so a deliberately-empty override is still REJECTED rather than silently falling
  back to per-repo scoping; allowlist [A-Za-z0-9._-] rejects separators, .., ., empty, newlines;
  validation at top level so die's exit actually kills the script.
  REAL ADJACENT BUG FOUND AND FIXED: add/check did an unlocked read-modify-write over the shared
  ledger — a lost-update race that only becomes reachable once workers share a scope. Proved with
  an 8-way concurrent test: locked -> 80/80 every run; lock removed -> reliably under 80 across 3
  runs. Fixed with an mkdir-based lock around add/check/reset.
  This is the highest-value find of Plan B so far: without it a shared-scope fleet UNDER-counts
  spending, which means the budget ceiling silently fails open — the exact failure the task exists
  to prevent, reintroduced one layer down.
  The specific fixed-temp-path pattern from Plan A's C1 was checked for and is NOT present here
  (mktemp "${target}.XXXXXX" is already per-call unique).
  Break/revert done correctly: 3 deliberate breaks, each confirmed red, each restored by hand from
  a pre-saved copy and diff-verified — `git checkout --` never used, per the standing instruction.
  Review dispatched on a frozen tree, told to reproduce BOTH halves of the concurrency claim and to
  assess lock staleness/deadlock risk (a wedged lock could permanently stall a fleet).
Controller: capturing the full-crate `cargo test -p autospec-cli` failure-NAME set at HEAD vs
  merge-base 5908df27 in parallel worktrees, to close the one Plan A claim left unverified.
Plan B Task 1: implementer DONE (commit 25b83313 on feat/planb-b1). 33/33 resolve, 148/148 board.
  Self-disclosed a bats trap it hit in its OWN red/green pass: a bare `! cmd` used as a NON-FINAL
  statement in a @test block does not fail the test, which masked a real break.

CONTROLLER SWEEP triggered by that disclosure — this is the kind of defect that is never confined
  to the file that noticed it. I scripted every board/integration/install/listener/ledger bats file
  for non-final bare negations and then PROVED the semantics empirically rather than reasoning
  about them:
      @test "non-final" { ! true; [ 1 -eq 1 ]; }  -> PASSES (wrong)
      @test "final"     { [ 1 -eq 1 ]; ! true; }  -> fails (correct)
  Found exactly 3, all in tests/autospec/project-board-writeback.bats (lines 59, 67, 99) — and all
  three are `! grep -q 'item-edit' "$GH_CALLS"`, i.e. the assertions proving the ONLY mutating
  script in this feature issues NO mutation when it skips (no token scope, no state field, no
  matching option). Those safety proofs have been silently vacuous the whole time.
  The SCRIPT is correct — the Task 9 reviewer verified each behavior by hand — so this is not a
  live bug; it is a missing guard against a future regression on the most dangerous file we have.
Ruling: fix by converting to an explicit `run` + status assertion rather than by reordering the
  statements so the negation lands last. — Why: reordering would fix these three by accident and
  leave the identical trap armed for the next person to edit the file. — Cost if wrong: slightly
  more verbose assertions.
Ruling: have the fixer audit the WHOLE file by hand rather than trusting my script's 3 hits.
  — Why: my detector only catches one shape of vacuous assertion; unchecked pipelines and
  discarded greps would not show up in it. — Cost if wrong: a few extra minutes on one file.

CONTROLLER ERROR (mine, recorded because it nearly cost prior-session work): I invoked task-brief
  from the MAIN checkout's cwd with a worktree-relative plan path. That created a stray
  .superpowers/ in the main checkout, and I removed it with `rm -rf .superpowers` — but that
  directory ALSO held two PREVIOUS plans' git-TRACKED workspaces
  (2026-08-14-darwin-autonomous-ownership-recovery, 2026-08-14-portable-autonomous-runtime).
  Recovered in full via `git checkout -- .superpowers`; main checkout is back to its exact
  session-start state (one pre-existing untracked file) at 5908df27, verified.
Ruling: never `rm -rf` a shared-name directory to clean up my own stray artifact — delete only the
  specific path I created, and always run workspace tooling from the worktree that owns the plan.
  — Why: the SDD skill is explicit that another plan's workspace is never mine to write, let alone
  delete; only the fact that those files were committed saved them. — Cost if wrong: unrecoverable
  loss of another session's working record.
Plan B Task 1: implementer DONE (25b83313 fleet-config) + (550b069f assertion fix). 33/33 resolve,
  148/148 board. The assertion fixer audited all 19 @test blocks by hand, found exactly the 3
  I had scripted and no others, converted every bare negation to the explicit run+status form for
  uniformity, and proved each of the 3 goes RED by inserting a real item-edit into the skip branch.
  It also caught a secondary bug in its OWN fix: `run` overwrites $output, which broke a later
  output-dependent check until it reordered them. Review dispatched.
Plan B Task 2: fix round 1 dispatched — lock staleness (a SIGKILLed worker currently wedges the
  ledger permanently for every future worker, confirmed by reproduction), plus leading-dash and
  length bounds on the scope validator.
Plan B Task 1: complete (commits d5480a10..550b069f, review clean — 0 Critical, 0 Important).
  YAML-injection probe table clean across 10 hostile repo names (quotes, newline, #, leading -,
  colon-space, {}[], 3000-char, and a name that is itself valid YAML): every case either dropped by
  the strict owner/name filter or safely quoted via tojson; NO case ever injected a key.
  fleet-config makes exactly 2 gh calls (field-list + item-list) — correctly skips the node-id and
  closed-issue join that only --emit plan needs.
  Independent bats audit of all 222 lines confirmed zero remaining bare negations, all 16 run+grep
  pairs followed by an explicit status check, no $output ordering hazards. The reviewer also
  independently reproduced the red/green proof by inserting a real item-edit into each of the 3
  skip branches. Merged.
Plan B Task 2: fix round 1 DONE (commit 654c9937). Found a genuine SECOND-ORDER race a naive fix
  would have shipped: verify-then-mv is two syscalls, so two workers could both pass the
  is-this-stale check before either acted, and the loser's stale-approved mv could blind-steal a
  lock the winner had already re-acquired LIVE. Serialized the reclaim DECISION behind a second
  mkdir mutex; 100 stress runs clean before porting, then 8 clean full-suite runs. Also disclosed
  the suite was flaky ~4/25 during development due to that real race. 15/15 now.
  Re-review dispatched with the question I most want answered: if the RECLAIM MUTEX itself is
  orphaned by a kill, has the original wedge just moved up one level? Plus a mandate to run the
  suite 10+ times — a concurrency fix validated once is not validated.
Plan B Task 3: implementer DONE (add238e4) 8/8; review APPROVED, 0 Critical, 0 Important.
  Dry-run inertness proven BEHAVIOURALLY (a logging stub's spawn log stayed zero-byte across every
  dry-run, including with a missing checkout) rather than by matching printed text.
  Liveness: new fleet-worker.json marker justified — worker-liveness.sh is hard-typed to a
  host:user:harness:pid shape that cannot survive the --detach fork boundary, and
  autospec-repo-lock.sh is documented opt-in short-critical-section only. Verified live marker
  skips, a marker past the staleness window does NOT skip forever, and a corrupt/empty marker
  causes no crash and no wedge.
  Injection table clean: space, quotes, `;rm -rf`, $(touch), backticks all blocked at config-lint;
  a leading-dash slug reaches fleet-run but is passed as a literal argv value, never eval'd.
  Capacity caps verified incl. node-cap + --parallel taking the tightest min.
  Minor (deferred): fleet_worker_live checks marker MTIME only, so a 0-byte marker with a fresh
  mtime reads as live. No crash or wedge risk; note for the final review.
  NOTE (out of scope, worth filing): tests/unit/test_doc_drift_fleet_scope.bats HANGS — the
  reviewer's full legacy run stopped at 44/50 because of it. Unrelated to this diff.
CONTROLLER SCOPING ERROR: I limited Task 3's touch list to 3 files, but replacing the one-shot
  command with a conductor necessarily invalidates 3 pre-existing tests that assert the OLD text.
  Ruling: dispatch a follow-up to update those tests to the new contract rather than leaving a red
  suite. — Why: a red suite is not shippable, and the tests are stale expectations, not defects.
  I required the fixer to work out what property each test protected and preserve it, explicitly
  forbidding weakening an assertion to a substring that always matches. — Cost if wrong: coverage
  silently lost in tests I did not read myself.
Plan B Task 4 dispatched (control mirroring) with an open design question flagged: the brief's
  sketch would label the SAME issue NUMBER in each target repo, which may not exist there or may be
  an unrelated issue. Told it not to ship anything that could slap autospec:stop on a random issue.

CONTROLLER SWEEP #2 — vacuous bats assertions, wider class. The legacy-test agent independently hit
  a SECOND shape of the same bug and reported it. I verified empirically rather than accepting it,
  and the truth is sharper than either of us had it:
      non-final `[[ a == b ]]` false -> test PASSES  (vacuous)
      final     `[[ a == b ]]` false -> test fails   (correct)
      non-final `[ a = b ]`    false -> test fails   (correct)
  So the trap is specific to `[[ ]]` and `!`; plain `[ ]` is safe in any position. bash 3.2.57.
  Scripted every board/fleet/ledger/listener bats file: 22 vacuous-position assertions total.
  17 are PRE-EXISTING debt in files this branch does not own (test_fleet_gui.bats x15,
  test_autospec_fleet_url.bats x2) — recorded, not fixed here, worth a separate sweep issue.
  3 were the writeback ones, already fixed on the merged branch (verified clean there).
  2 are NEW, in Task 3's own tests/fleet/project-board-fleet.bats (lines 103, 111) — and they are
  exactly the NEGATIVE SAFETY assertions proving a repo that failed to spawn, or whose checkout is
  missing, was NOT launched. Both inert. Task 3's reviewer did not catch them.
Ruling: expand the legacy-test agent's touch list to include Task 3's test file and fix both there
  rather than opening another round with the Task 3 agent. — Why: that agent already has live
  context on this exact bug class in this exact worktree, and the fix is identical in shape to the
  one it just made; a separate dispatch would rebuild context for two lines. — Cost if wrong: one
  commit touches three test files instead of two.
Ruling: do NOT fix the 17 pre-existing vacuous assertions in test_fleet_gui.bats /
  test_autospec_fleet_url.bats in this branch. — Why: they are inherited debt, unrelated to board
  ingestion, and touching 17 assertions in files this feature does not own would bloat the diff and
  risk breaking suites I have no reason to be in tonight. File them as a follow-up sweep.
  — Cost if wrong: those tests stay decorative until someone does that sweep.
Plan B Task 2: fix round 1/5 re-review -> NOT READY, send back. Findings 2 and 3 clean. The core
  lost-update fix is genuinely correct and was verified hard: live owner's lock never reclaimed
  regardless of age, dead owner's lock reclaimed and logged to stderr, unset-scope byte-identical
  to d5480a10, all traversal probes rejected with nothing on disk, 12/12 clean suite runs, 8-way
  concurrency exactly 80 in 5/5 reps.
  NEW CRITICAL, and it is precisely the question I told the reviewer to attack — "if the reclaim
  mutex itself is orphaned, has the wedge just moved up a level?" Answer: yes, AND it got worse.
  ledger_lock_acquire's retry loop `continue`s without incrementing `waited` or sleeping when a
  stale lock is seen but the reclaim mutex cannot be won. Reproduced: pre-stage an orphaned
  <lockdir>.reclaiming, run add -> hangs indefinitely (timeout 5 killed it, exit 124) even with
  LOCK_MAX_WAIT_ITER=20, burning CPU and printing nothing.
  Before the fix: bounded, loud failure after ~10s. After: unbounded, silent, CPU-spinning hang.
  For an unattended multi-week fleet that is a regression, not a fix.
  Worth recording WHY it slipped: the happy paths got 100 stress runs; this failure path was
  REASONED about rather than executed, and the report asserted it "degrades to the existing
  LOCK_MAX_WAIT_ITER timeout" — false as implemented.
Ruling: fix round 2 rather than reverting the reclaim mutex. — Why: the mutex is what prevents the
  blind-steal race the implementer correctly identified; removing it would trade a bounded hang for
  silent budget under-counting, which is the worse of the two. Keep the mutex, bound its failure
  path. — Cost if wrong: one more round on a task already at round 2 of 5.
Ruling: require the implementer to ENUMERATE every exit of the retry loop and test each with
  `timeout`, not just the reported case. — Why: the bug came from reasoning about a failure path
  instead of executing it; enumerating exits is the generalisation of that lesson. — Cost if wrong:
  a handful of extra tests on a lock that guards real spend caps.
Plan B Task 4: implementer DONE (8b09cd31), review APPROVED with 2 Important, 0 Critical.
  It REJECTED the brief's flawed sketch (which would have labeled the same issue NUMBER in every
  target repo, potentially an unrelated issue) and instead finds-or-creates a dedicated marker
  issue per repo with a constant title never derived from board input. Reviewer confirmed a closed
  set of exactly 4 gh sites (2 read, 2 write), no --remove-label anywhere in code, zero
  out-of-allowlist create/edit under 6 probes, and every crafted near-miss label rejected
  (autospec:stop-not-really, Autospec:Stop, leading-dash, space). All 5 negative assertions in
  final position — no vacuous-position trap in this file.
  IMPORTANT 1: a transient search failure or malformed output is treated as "marker not found" and
  CREATES another. On the live API this is not theoretical — GitHub's search index lags creation by
  seconds to minutes, so the marker just created may not appear next cycle, and over a multi-week
  unattended run duplicates accumulate in the operator's repos.
  IMPORTANT 2: the "marker exists -> edit" branch has ZERO test coverage; every stub returns an
  empty list, so only the create path is exercised. Idempotence is real in code but unguarded.
Ruling: on an uncertain find, SKIP rather than create. — Why: creating on uncertainty is the wrong
  default for a mutating operation; a missed mirror cycle is recoverable, a pile of duplicate issues
  in someone's repo is not. Distinguish "searched and found nothing" (may create) from "could not
  determine" (must not). — Cost if wrong: control mirroring lags by a cycle during API flakiness.
Plan B Task 3b: legacy tests updated + both new vacuous assertions fixed (5d0c25f9). 27/27 fleet,
  3/3 scheduler, 2/2 dry-run. The agent reported HONESTLY that one fixed assertion is still
  structurally TAUTOLOGICAL — the stub refuses to log anything containing o/a, so the log can never
  gain that string no matter what production does. It documented this instead of fabricating a red
  run. I dispatched a follow-up to fix the FIXTURE (log-then-fail, splitting "attempted" from
  "succeeded") so the assertion becomes genuinely falsifiable.
Ruling: fix the fixture rather than accept a tautological assertion. — Why: an assertion that
  executes but cannot fail is only marginally better than a vacuous one, and it actively misleads
  the next reader into thinking the case is covered. — Cost if wrong: two lines of stub.
Plan B Task 3: complete and MERGED (ef1aa26b). Production code reviewed and approved earlier
  (0 Critical, 0 Important); commits 5d0c25f9 and 6a2410bd are test-only follow-ups.
  The fixture split ("attempted" vs "succeeded" logs) made the attempted-assertion genuinely
  falsifiable — proven red via a scheduler bug that silently drops o/a before trying it.
Ruling: ACCEPT the residual on the not-in-success-log clause rather than pushing a third round.
  The agent showed, correctly, that this clause CANNOT be independently falsified for the
  "fleet-run swallows a real spawn failure" class: the stub decides from the same literal substring
  the assertion greps, and writes (or withholds) its log entry BEFORE fleet-run ever observes the
  exit code — so nothing fleet-run does afterwards can change what the external process already
  logged. The clause states a true fact but does not discriminate that bug. That class IS covered
  by the adjacent, live code_health:fleet_worker_spawn_failed assertion, and the new attempted-log
  assertion covers the scheduler-drop class. — Why accept: the coverage exists via neighbours, and
  redesigning the fixture further is diminishing returns at this hour. — Cost if wrong: one clause
  in one test is decorative; two adjacent live assertions still guard the behaviour.
Ruling: fold verification of the two test-only commits (5d0c25f9, 6a2410bd) into Plan B's final
  whole-branch review rather than running a dedicated re-review round now. — Why: they touch only
  test files, I ran the suites myself before merging (fleet 27/27, scheduler 3/3, dry-run 2/2), and
  I personally scripted and empirically verified the vacuous-assertion sweep that drove them.
  — Cost if wrong: a test-file defect reaches the final review instead of being caught a round
  earlier — which is exactly what the final review is for.
Plan B Task 4: fix round 1 re-review — all 4 findings ADDRESSED. The --search -> --label change was
  validated against the exact regression I feared: create applies --label "$MARKER_LABEL"
  unconditionally (traced live), and MARKER_LABEL is not in RESERVED so it can never be mirrored or
  confused with a control label. All 6 lookup-failure rows verified with full gh.log inspection;
  deterministic lowest-number tie-break re-ran identically; reserved-label near-miss table still
  correct; no markers created when there is nothing to mirror.
  TWO NEW Important:
  (a) test 15 never asserts the create call carries --label MARKER_LABEL. Code is right; nothing
      pins it. If a future edit dropped that flag the marker would be unfindable forever and the
      script would create a new one EVERY cycle — infinite duplicates, suite green. Exactly the
      failure this fix round existed to prevent, left unguarded.
  (b) the label-only lookup adopts ANY open issue wearing the marker label — demonstrated live, a
      foreign issue #12 received `issue edit 12 --add-label autospec:stop`. This is the ORIGINAL
      hazard returning through a different door: the marker design existed precisely because the
      brief's sketch could stamp control labels on an arbitrary unrelated issue, and the --label
      lookup reopened that path.
Ruling: require a TITLE cross-check before adopting a labelled candidate, and on label-matches-but-
  title-does-not, SKIP with a distinct reason rather than adopt or create. — Why: adopting an
  impostor is what caused (b); creating on ambiguity is what fix round 1 removed. Skipping is the
  only option that does neither. — Cost if wrong: control mirroring skips a repo whose marker a
  human renamed, until they rename it back or the operator intervenes — visible and recoverable.
Plan B Task 5: implementer returned BLOCKED, correctly, with no code changed. My plan named the
  WRONG FILE. Verified myself: scripts/autospec-autonomous.sh only sources the loop and calls
  autospec_conductor_run; the real sites are in scripts/lib/autospec-loop.sh —
  `gh pr view ... statusCheckRollup` (~631), `gh pr merge --admin --squash` (~712),
  `gh pr close` (~837). It refused both bad options: faking a hook into the read-only status-display
  path, and unilaterally expanding into the file that issues the real admin merge. That is exactly
  the judgement I want from a blocked implementer.
  It also surfaced a genuine DESIGN defect in my plan: those calls operate on a ROLLUP PR off an
  integration branch, not a per-issue PR (per-issue PR creation lives in
  autonomous-integration-branch.sh / worktree-guard.sh). So the plan's row "a PR is opened for the
  issue" describes mechanics that do not exist in this conductor.
Ruling: map the three states onto the ROLLUP lifecycle applied to each rolled issue —
  rolled into the integration branch / rollup PR exists -> Review; that PR's checks running ->
  Testing; rollup merged and issue closed -> Done. — Why: an issue genuinely IS in review once an
  open rollup PR carries it, and in testing while that PR's checks run, so this reports real state
  rather than inventing a per-issue PR that does not exist. The loop already tracks the rolled-issue
  set (a `rolled_issues` parameter ~line 570), so the carrier exists. — Cost if wrong: board states
  are coarser than a per-issue view would be — several issues share a rollup's Review/Testing —
  which is accurate, just less granular.
Ruling: re-scope Task 5 to scripts/lib/autospec-loop.sh with a STRICTER merge-safety constraint
  than the original brief carried, since this is the file that issues `gh pr merge --admin
  --squash`. Zero new failure modes on the merge path; no board configured must be a zero-cost
  silent no-op. — Cost if wrong: a decorative board update could perturb the most dangerous code
  path in the repo, which is why the constraint is absolute rather than best-effort.
Plan B Task 2: fix round 2/5 (1 addressed, 0 open; commits 654c9937..6c30e533). Re-review PASS.
  Pre-fix reproduced at parent: exit 124 after 8s — the test has real power. Post-fix, all 5
  loop-exit paths bounded with clear stderr and none hitting the timeout kill:
    orphaned mutex + live holder -> exit 1, 1298ms
    orphaned mutex + dead holder -> exit 1, 1449ms
    orphaned mutex + no lock     -> exit 0, 38ms
    both orphaned, then recovers -> exit 1 (1464ms) then exit 0 (118ms)
    mutex won, lock vanishes     -> exit 0, 1134ms
  Reviewer read every branch itself and confirmed the implementer's list of 5 exits was complete.
  Concurrency 8-way x5: 80, 80, 80, 80, 80 — exact every time. Live owner's lock never reclaimed
  even backdated ~999999s. Suite 10/10 runs at 20/20 = 100%.
  AUTOSPEC_SPEND_LOCK_TEST_STALL confirmed genuinely inert when unset (single guard, defaults
  empty, no other reference) — the test seam cannot alter production behaviour.
Plan B Task 2: complete (commits d5480a10..6c30e533, review clean) and MERGED.
Ruling: HOLD Task 6 (installer registration) until Task 4 merges. — Why: Task 6's test asserts that
  every registered script exists on disk, and scripts/project-board-control-mirror.sh currently
  lives only on feat/planb-b4, which is still in fix round 2. Dispatching Task 6 now would either
  register a nonexistent file (red test) or register an incomplete set (the exact clean-install
  crash Plan A's Task 13 existed to prevent). — Cost if wrong: Task 6 starts a few minutes later.
  Worktree pb-b6 is pre-staged off the current head and its brief is written, ready to dispatch the
  moment Task 4 lands.
Ruling: Task 7 (multi-repo e2e) also waits on Task 4, for the same reason — it exercises the whole
  fleet path including control mirroring.
Plan B Task 4: fix round 2/5 re-review — both findings ADDRESSED, verdict approved.
  Mutation-proven in BOTH directions: dropping --label MARKER_LABEL turns test 15 red; forcing
  adopt-on-label-alone reproduces the impostor hazard (tests 19+20 red); forcing the owned pool
  empty turns 16/17/18 red, so the positive path is not vacuous either.
  TITLE-DRIFT ANSWER (the question I asked because each prior round opened a new door): it SKIPS
  FOREVER and does not create a duplicate — `marker_label_title_mismatch`, zero edit, zero create.
  The third door stayed closed, consistent with "uncertainty skips, never creates".
  Comparison is strict literal equality — whitespace, case, prefix and superstring titles all
  correctly rejected; jq's == does no trimming, folding or normalisation. Mixed pools adopt only
  the correct candidate, deterministically.
  Documented side effect, accepted: a repo whose marker title drifted has control mirroring
  silently and permanently disabled until a human fixes it — no self-healing, no alerting beyond
  the `skipped` array. Worth surfacing to the operator; not a defect given the alternatives are
  adopting an impostor or creating duplicates.
  NEW Minor: the vacuous-position trap appeared a FIFTH time, at line 215 in the brand-new test 19.
  Currently inert (mutations die on an earlier jq -e assertion before reaching it), so no coverage
  is lost today.
Ruling: fix the fifth instance anyway rather than deferring it as a redundant Minor. — Why: this
  trap has silently voided a real safety proof twice on this branch already, and leaving a known
  instance in freshly-written code — after converting every other instance — means it returns the
  moment someone reorders assertions. — Cost if wrong: one line and one round-trip.
Plan B Task 7 dispatched off feat/planb-b4 (so the control-mirror script is present), with the
  framing that this test IS the feature's central claim and every assertion must be provably
  falsifiable.
Plan B Task 4: complete (commits 2a530cd0..b836d6c6, review clean) and MERGED (9c42b7c3).
  The fifth vacuous-assertion instance was fixed AND proven independently exploitable, not merely
  "safer": the agent injected a probe firing `issue edit 12` while still satisfying the earlier
  jq -e check, so test 19 went red specifically at the new [ "$status" -ne 0 ] line.
Plan B Task 7: implementer DONE (commit 6442adde on feat/planb-b7), 182/182, 44s wall.
  All 5 load-bearing claims manually broken, confirmed RED, restored byte-identical:
  cross-repo blocking, allowlist gate, deps_unresolvable ordering, shared-budget truncation,
  no-mutation-without---apply.
Plan B Task 6 dispatched (worktree pb-b6b) now that Task 4 is merged and all 5 board scripts exist.
Note: the merged-tree health check now exceeds a 2-minute foreground budget (Task 7's file alone is
  44s), so it runs in background from here.

## PLAN B FINAL WHOLE-BRANCH REVIEW (opus) — DO-NOT-MERGE-as-is, 1 Critical

C1 CRITICAL: fleet_worker_command spawned `autospec-autonomous start --detach ...`, but the
  conductor's parser has NO --detach case — catch-all is `die "unknown argument"`. Every live spawn
  failed, emitted code_health:fleet_worker_spawn_failed, wrote no marker, retried forever. NO
  CONDUCTOR EVER RAN. I verified independently before acting.
  WHY 309 TESTS MISSED IT — the important part: every test stubbed autospec-autonomous with an
  argument-blind `printf '%s' "$*"` that accepts anything. An argument-blind stub CANNOT catch a
  rejected argument. This exact failure mode is in the project's own memory
  (feedback_codex_exec_needs_skip_git_repo_check).
BUILT-BUT-UNWIRED cluster — the same class Plan A hit, found again only by a whole-branch pass:
  I1 nothing sets AUTOSPEC_SPEND_SCOPE -> a 6-repo board still gets 6x the budget, the exact
     failure Task 2 existed to prevent. Inert exactly like Plan A's ProjectBoardConfig was.
  I2 --emit fleet-config has no production consumer; the emit path also skips repo_allowlist, so
     wiring it naively later would admit every board repo.
  I3 project-board-control-mirror.sh has NO caller; its envelope is write-only. An operator
     applying autospec:stop to the control issue reaches nothing.
  I6 nothing clones checkouts, so the launcher reports "checkout not found; skipping launch".
  I7 skills/autospec-fleet/install.sh shipped NO fleet scripts — Task 3's work was not on the
     operator's machine at all.
  I9 both skills' prose contradicted the built behaviour again.
Seam-break audit HELD for: fleet-config->YAML->fleet-run (hostile names filtered + tojson),
  write-back states present in both boards' candidate tables, .project.id/.fields surviving
  normalize->deps, board-cache keys agreeing between promoter and loop. No path found where board
  data influences execution.

FIX WAVE (merged e4c83f63): C1 fixed at BOTH call sites (display string and the real live-spawn
  invocation were both wrong); the argument-blind stub replaced with one that validates like the
  real parser, proven RED 5/9 against the old shape and GREEN 9/9 after. I7 fleet scripts derived
  empirically and registered in a new FLEET_SCRIPT_FILES group, verified by isolated-HOME install.
  I4 mirror now separates `failed` (gh attempted, non-zero) from `skipped` (pre-flight decision) so
  a failed stop can never report as propagated. I9 both trios corrected + re-derived + re-goldened.
  109/109 on targeted suites.
  MY OWN VERIFICATION: the produced command is now accepted by the real parser (rc=0) and --detach
  is still rejected — proving the old shape was genuinely broken, not a stub artifact.
Ruling: fix C1/I4/I7/I9 now but explicitly DEFER I1/I2/I3/I6. — Why: the deferred four need design
  decisions (where the conductor exports the scope, who consumes fleet-config under an allowlist,
  where the mirror is called in the tier cycle, and who owns cloning), not patches. Guessing at 4am
  would produce exactly the kind of plausible-but-unwired code this review just caught. — Cost if
  wrong: multi-repo shipping stays manual until those four are wired; single-repo board-driven
  promotion works today.
