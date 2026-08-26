# SDD ledger — plan: docs/plans/2026-08-25-project-board-ingestion-engine.md

Spec: docs/specs/2026-08-25-autospec-project-board-ingestion-design.md (reachable, read)
Worktree: .claude/worktrees/project-board-spec, branch docs/project-board-ingestion-spec
Merge base: 5908df27

## Pre-flight conflict scan

### Shared file / interface pairs

| Tasks | Produces → Consumes | Finding |
|---|---|---|
| T1 → T3,T5,T14 | pinned fixtures → bats `$FIX` paths | Clean. All three reference `tests/fixtures/project-board/p{1,2}-{items,fields}.json`. |
| T2 → T3 | `scripts/project-board-resolve.sh`, `parse_identity`, bats setup() | Clean. T3 appends to T2's file and reuses `$TMP/bin`, `$SCRIPT`, `$FIX` from T2's setup(). |
| T3 → T4 | board plan `.items[].labels` | Clean. |
| T3 → T9 | `.fields.autospec_state.{id,options}` | Clean. Same shape produced and consumed. |
| T3 → T14 | plan shape | Clean; T14 rebuilds the shape from fixtures via `plan_from`, not via the resolver. Intentional (offline CI). |
| T4 → T5 | plan passthrough with `.normalized` added | Clean. T5 does not read `.normalized`. |
| T5 → T6 | `.items[].blocked_by` | Clean. Same key, same `{repo,number}` element shape. |
| T6 → T8 | `.items[].ready`, `.cycles` | Clean. |
| T7 → T8,T10 | `ProjectBoardConfig` field names vs YAML/env keys | **FINDING A** — see rulings. |
| T8 → T10 | `.board.{ready,promotable,out_of_scope,demoted}`, `$_board_cache` | **FINDING B** — see rulings. |
| T9 → T10 | `--plan/--item/--state` CLI | Clean. |
| T2..T10 → T13 | script files → `AUTONOMOUS_SCRIPT_FILES` | **FINDING C** — see rulings. |
| T11 → T12 | `/autospec-project ship` route target | Clean. |
| T11,T12 → goldens | trio derive + goldens | Clean; both tasks bundle prose+derive+goldens in one commit per the constraint. |

### Per-task internal agreement

| Task | Finding |
|---|---|
| T1 | Clean. |
| T2 | Clean. |
| T3 | Clean. |
| T4 | **FINDING D** — first test opens with a nonsense `run bash -c ... \| true` line that asserts nothing. |
| T5 | Minor noise: the 78/80 test calls `project-board-resolve.sh --help >/dev/null` for no reason. Harmless; strip. |
| T6 | **FINDING E** — Step 3's wiring instruction is prose ("change the final line"), not code. **FINDING F** — `cyclic` flags dependents-of-cycles as cycle participants. |
| T7 | **FINDING G** — assumes `AutonomousConfig::parse` is the real API name; unverified. |
| T8 | Clean apart from Finding B. |
| T9 | Clean. |
| T10 | Clean. |
| T11 | Clean. |
| T12 | Clean. |
| T13 | See Finding C. |
| T14 | **FINDING H** — asserts `cycles == 0` on real board data; unverified assumption. |

## Pre-flight rulings

Ruling: FINDING A — T7 defines Rust `ProjectBoardConfig` but T8/T10 read env vars (`AUTOSPEC_PROJECT_BOARD_URL`, `_ALLOWLIST`, `_TTL`), never the Rust config. Decided: the env vars are the shell-side contract and the Rust config is the parse/validate authority that the conductor exports into them. T8's dispatch will say so explicitly; no shell task reads YAML directly. — Why: keeps the security validation (required allowlist) in one typed place while leaving the shell scripts testable without a Rust build. — Cost if wrong: a second config read path has to be added later, localized to the conductor entry point.

Ruling: FINDING B — T8's snippet references `$repo`, `$apply`, and `$_board_cache` as if they already exist in autonomous-promote-open-issues.sh. Decided: the implementer must read the real script first and bind to its actual variable names, and must export the cache path as `_board_cache` if no equivalent exists. — Why: the plan cannot know the host script's locals. — Cost if wrong: T10's write-back call gets an empty plan path and silently no-ops (fail-open), caught by T10's tests.

Ruling: FINDING C — T13 registers only `autonomous-promote-open-issues.sh` and `list-groomable.sh`, but the promoter also shells out to classify-model-fit.sh, promote-eligibility.sh, groom-fill.sh, and grooming-config.sh. Decided: T13 registers the full grooming dependency set, not just the two named. — Why: registering a partial set still leaves the Tier 1.5 board stage crashing on a clean install, which is the exact bug the task exists to prevent. — Cost if wrong: a few extra files ship that were already expected to be present.

Ruling: FINDING D — strip the nonsense first line of T4's first test. — Why: a line that asserts nothing is a test-hygiene defect the reviewer would flag anyway. — Cost if wrong: none.

Ruling: FINDING E — T6 Step 3 will be dispatched with an explicit instruction to restructure the script as: extraction jq writes to stdout, then `if [ "$resolve" -eq 1 ]; then resolve_stage; else cat; fi` consumes it as a pipeline stage. — Why: prose-only steps are a plan defect; the implementer needs the shape. — Cost if wrong: implementer picks a different but equivalent pipeline arrangement; tests still gate it.

Ruling: FINDING F — a node whose blocker sits in a cycle is reported as a cycle participant. Decided: keep it. — Why: such a node is genuinely unschedulable, so marking it not-ready is fail-closed and correct; only the `reason` string is imprecise. — Cost if wrong: an operator reading the digest sees "dependency cycle" on an item that is merely downstream of one, and looks in a slightly wrong place.

Ruling: FINDING G — T7's implementer must read crates/autospec-core/src/autonomous/config.rs and bind to the real parse entry point rather than the assumed `AutonomousConfig::parse`. — Why: the plan asserted an API name it did not verify. — Cost if wrong: compile error, caught immediately.

Ruling: FINDING H — T14 asserts `cycles == 0` on live board data. Decided: if the real boards contain a cycle, the implementer reports it as a finding and does NOT weaken the assertion. — Why: a cycle on a real delivery board is a genuine discovery worth surfacing, not a test to relax. — Cost if wrong: T14 fails and needs one ruling round.

## Progress

Task 1: implementer DONE (commit aca4e224); review dispatched.
Task 1: note — review package is 698KB (four single-line JSON captures). Reviewer was
  directed to verify by jq query instead of reading the blob. Later tasks do not re-add
  fixtures, so their packages will be small.
Task 1: complete (commits c7a7d41..aca4e224, review clean — spec OK, quality approved)
Task 1: Ruling: reviewer's Important finding ("AutoSpec state exists on p2 but not p1") is a
  correct observation about LATER tasks, not a defect in Task 1's deliverable — Task 1's job was
  to capture reality and it did. No fix dispatched. Decided: carry it into the Task 3 and Task 9
  dispatches, where the "board has no AutoSpec state field" branch is therefore a REAL execution
  path that must be tested, not defensive dead code. — Why: the spec already specifies skip-with-
  one-warning for a missing field; the finding proves that branch is load-bearing on Project 1.
  — Cost if wrong: write-back silently no-ops on Project 1 until the operator adds the field,
  which is the specified behavior but must be surfaced to the operator.
Task 1: minor (deferred): p2 raw "Blocked by:" count is 79; 78 after filtering the one "none".

## Controller discovery while ruling on Task 1's finding

I listed p1's field names to size the finding and found the divergence is WIDER than the
reviewer reported. The two boards disagree on field NAMES, not just label taxonomy:

  p2: Workflow | Priority | Risk | Area | AutoSpec state | Implementer model |
      Reviewer model | Dependencies | Context budget
  p1: Delivery status | Priority | Area | Context fit | Reasoning fit |
      Parent/tracker | Depends on | Evidence status | Release gate

So p1's analogues are `Delivery status` (≈ AutoSpec state) and `Depends on` (≈ Dependencies).

Ruling: the spec hardcodes the literal field names "AutoSpec state" and "Dependencies", which
CONTRADICTS its own Global Constraint "No target-board specifics hardcoded" — and the plan
inherited the contradiction. The spec's global constraint is the binding authority, so the
literal names lose. Decided: add a `field_map` to the project_board config with two keys
(`state_field`, `dependencies_field`), each defaulting to an ordered candidate list
—- state: ["AutoSpec state","Delivery status"]; dependencies: ["Dependencies","Depends on"] —-
resolved by probing which candidate actually exists on the board. Folded into Task 3 (field map),
Task 5 (dependency field source), Task 7 (config), Task 9 (write-back lookup).
— Why: without it, write-back and the field-based dependency source both silently no-op on
Project 1, i.e. the multi-repo board this feature was requested for.
— Cost if wrong: one extra config key and a probe loop that nobody needs; strictly additive,
and the single-name behavior remains reachable by pinning the config.

Ruling: verifying the field_map ruling surfaced a second layer — the two boards' state OPTION
names diverge as well, not only the field names:
    p2 "AutoSpec state": Blocked, Ready, Planning, Implementation, Review, Testing, Done
    p1 "Delivery status": Backlog, Ready, In progress, In review, Verify, Blocked, Done
Only Blocked / Ready / Done are shared. Decided: extend the SAME ordered-candidate resolution
one level down, from field names to option names, with a canonical-state -> candidates table:
    Blocked -> [Blocked]; Ready -> [Ready]; Done -> [Done];
    Implementation -> [Implementation, "In progress"]; Review -> [Review, "In review"];
    Testing -> [Testing, Verify]
Folded into Task 9. — Why: without it, write-back on Project 1 silently skips 3 of its 6 states
(Task 9 already skips an unknown option name by design), so the board would show Ready and Done
but never the middle of the lifecycle. This is one mechanism applied twice, not a new one.
— Cost if wrong: a canonical state maps onto a board option the operator meant differently;
visible on the board immediately and correctable by pinning the candidate list in config.

Task 2: review NOT approved — Critical: `shift 2` on a missing option value aborts under
  `set -eu` with rc=1 instead of the contracted rc=2. Defect originates in the PLAN's own code,
  not the implementer's transcription. Important: `/projects/N/views/M` rejected. Minor: leading
  zero emits `"number":02` (invalid JSON).
Task 2: Ruling: accept an optional `/views/<N>` suffix on the URL. — Why: it is the shape GitHub
  produces when a user copies from the board UI, so rejecting it fails the most likely real
  invocation; the spec says "accepts a project URL" without restricting to the bare form.
  — Cost if wrong: one extra accepted URL shape that normalizes to the same identity.
Task 2: Ruling: fold the Minor leading-zero finding into fix round 1 rather than deferring it.
  — Why: it lives in the same function as the Critical fix and costs one line; the script's
  contract is "emits JSON" and `02` is not valid JSON, so every downstream jq consumer inherits
  malformed input. — Cost if wrong: negligible; one extra line of normalization.
Task 2: fix round 1/5 dispatched to original implementer; returned 10/10 (commits 7fa5a4eb..30f36aa0).
  Scoped re-review dispatched. Controller flagged one specific risk for verification: the
  implementer normalized leading zeros "via arithmetic expansion", and bash arithmetic treats a
  leading-zero literal as OCTAL — so /projects/010 may yield 8 and /projects/08 may error.
Task 2: fix round 1/5 (3 addressed, 0 open; commits 7fa5a4eb..30f36aa0). Octal risk checked and
  clean — implementer used `$((10#$n))`, which forces base 10; 08 -> 8, 010 -> 10, all valid JSON.
Task 2: complete (commits aca4e224..30f36aa0, review clean)

Task 3: implementer DONE 17/17 (commit 46b932e9), reported that `content.state` is absent from
  both fixtures so the CLOSED branch has no coverage. I verified the root cause myself:
  `gh project item-list --format json` returns `.content` with ONLY {body, number, repository,
  title, type, url}. There is NO issue open/closed state anywhere in the board payload. The only
  doneness signal is the board's own `status` column ("Done" on 1 of 80 p2 items).

Ruling: this is a CRITICAL defect in MY SPEC, not in the implementation. The spec's Component 2
  says "An edge is satisfied when the referenced issue is closed and its linked PR merged", and
  Task 6's DAG satisfies an edge only on `state == "closed"` — but the resolver can never emit
  `closed` from board data, so on real data every blocker stays open forever, nothing ever
  promotes, and the whole feature is inert while its tests stay green. This is precisely the
  self-consistent-fixture failure mode: the tests construct `state` by hand, so they cannot catch
  that production never sets it.
  Decided: the resolver must join real issue state from a SECOND source — one
  `gh issue list --repo R --state closed --json number` per distinct board repo (<=6 calls,
  cached with the existing board TTL) — and set `.items[].state` from that join, NOT from the
  board payload. Board `status == "Done"` is NOT sufficient on its own: it is an
  operator-maintained column that lags reality. Folded into Task 3's fix round.
  — Why: correctness on real data outranks a passing suite; a per-repo closed-list is O(repos)
  not O(items), so it is affordable inside the existing cache.
  — Cost if wrong: one extra gh call per repo per TTL, and a stale-by-TTL view of closedness,
  which is fail-closed (a just-closed blocker promotes one cycle later, never early).
Task 3: Ruling: the spec text itself is now known-wrong on edge satisfaction, so the Task 3 fix
  round must amend BOTH the code and the spec's Component 1/2 wording in the same commit, rather
  than me editing the spec from the controller session. — Why: a controller-authored spec edit
  skips review, and the spec is the authority the reviewer gates against; they must move together.
  — Cost if wrong: the fix commit is slightly larger and touches a doc.
Task 3: review — spec OK, quality approved with the Critical tracked. Reviewer independently
  confirmed `[.items[].state] | unique == ["open"]` across all 80 p2 items, and confirmed the
  candidate-list resolution works on both boards (p2 -> "AutoSpec state", p1 -> "Delivery status",
  env override re-orders correctly). 10 Task-2 test names diffed byte-identical — none weakened.
Task 3: Ruling: truncation of the per-repo closed-issue list must NOT exit 4, unlike item-list
  truncation. — Why: an unlisted closed issue is read as open, so a dependent stays blocked; that
  delays a promotion but can never cause a wrong one. Item truncation is the opposite (a partial
  board could promote something whose blocker is merely unlisted), so it stays fail-closed at 4.
  Conflating the two would either break real repos with many closed issues or open a wrong-promotion
  hole. — Cost if wrong: a promotion can lag by one cache TTL.
Task 3: Ruling: fold the Minor `PATH=/usr/bin:/bin` test-hardening into fix round 1. — Why: on
  Debian/Ubuntu `gh` installs to /usr/bin, where the test would silently stop testing its own
  premise; a test that quietly stops testing is worse than a missing one. — Cost if wrong: none.
Task 3: fix round 1/5 dispatched (state join + test hardening + spec amendment).
Task 3: fix round 1/5 (3 addressed, 0 open; commits 46b932e9..35e9239a). Re-reviewer proved the
  two truncation paths live: item-list LIMIT=2 -> exit 4, no plan; closed-list omission -> exit 0,
  full 80-item plan, the omitted issue degrades to "open". 6 distinct p1 repos -> exactly 6
  closed-list calls (asserted against the stub call log, not the result). `bash -n` and
  `shellcheck -s bash` clean. Spec amended in the same commit.
Task 3: minor (deferred): the closed-list query shares AUTOSPEC_PROJECT_BOARD_LIMIT with the
  item-list cap and has NO truncation detection or warning of its own, so a silently-truncated
  closed list delays promotions with zero operator signal. Safe-direction only. Point the final
  whole-branch review at this — it is an observability gap, not a correctness one.
Task 3: Ruling: defer that Minor rather than folding it into the loop, unlike the two Minors I
  folded earlier. — Why: those two sat inside the exact function already being fixed and cost one
  line; this one needs its own warning path and test, and Minors are not supposed to extend the
  loop. — Cost if wrong: an operator debugging "why is nothing promoting" lacks one stderr line.
Task 3: complete (commits 30f36aa0..35e9239a, review clean)

Task 4: review spec ❌ — the brief's "never fails, exit 0 always" contract is unmet. 5 of 8
  degenerate inputs crash with jq exit 5: non-JSON stdin; JSON with no .items; item missing
  `labels`; `labels: null`; wrong-shape --label-map (list or scalar). Malformed-YAML map DOES
  degrade correctly. Defect originates in the PLAN's jq (unguarded `.items |= map(...)`), not the
  implementer's transcription. Injection surface verified CLOSED across 5 hostile label probes and
  regex-metachar map keys. Real fixtures: 0 null priorities on either board, no taxonomy gap.
Task 4: fix round 1/5 dispatched.
Task 4: fix round 1/5 (6 addressed, 0 open; commits 8e52046c..a29b36d8). Re-reviewer confirmed
  exit 0 comes from narrow shape guards, NOT blanket `|| true` / `set +e` suppression. Bash 3.2
  verified by actually running under /bin/bash 3.2.57. Injection surface still closed.
Task 4: two reviewers DISAGREED on P1 null-priority count; I verified directly. Re-reviewer correct:
  p1 has 30 of 80 items with no priority-family label (labels are priority:p0=30, priority:high=11,
  priority:p1=9 -> critical:30 + high:20 + null:30). p2 has 0. Normalization is right; the nulls are
  real missing labels, not a taxonomy gap. Carry into Task 8: ranking on Project 1 must have a
  defined behavior for 30 unprioritized items rather than assuming every item carries a priority.
Task 4: complete (commits 35e9239a..a29b36d8, review clean)

Task 5: implementer DONE 16/16 (52/52 across the board suite), commit 442e7704. Surfaced that one
  p2 item declares its dependency in PROSE with no #N syntax and therefore parses as unblocked.
  I confirmed exactly two p2 items have a "Blocked by" phrase with no parseable ref:
    #1  IW-WB-000 Bootstrap  -> "Blocked by: none."  (legitimately unblocked root)
    #80 Phase 5.5 audit      -> "Blocked by the implementation and acceptance portfolio
                                 IW-WB-001 through IW-WB-078."  (blocked by ALL 78, in prose)
Ruling: the script is RIGHT to refuse to parse prose — issue bodies are untrusted DATA and
  inferring edges from free text would be both unsafe and unreliable. But silently treating an
  unparseable declaration as "no blockers" is the wrong default: on this very board it would
  promote the FINAL AUDIT issue first, ahead of the 78 issues it audits. Decided: when the
  `## Dependencies` section contains a "Blocked by" phrase, does NOT say "none", and yields ZERO
  parseable refs, mark the item as having an UNRESOLVABLE declared dependency -> fail closed
  (not ready, needs human), never unblocked. Generalizes past this fixture; folded into Task 5's
  fix round and consumed by Task 6's readiness rules.
  — Why: "I detected a dependency I could not understand" must fail closed, exactly like the
  unresolvable-reference case the spec already specifies; under-promotion is recoverable, a
  wrong promotion of an audit ahead of its subjects is not.
  — Cost if wrong: an item whose body merely mentions "blocked by" in passing needs an operator to
  either add a real ref or say "none" before it can promote. Visible and one edit to fix.
Task 5: review spec OK, quality approved with 1 Critical + 1 Important + 3 Minor.
  CRITICAL: p1 yields 0/80 edges — it writes `Depends on issue #N` (112 occurrences, zero uses of
  "blocked by"). I verified p1 bodies DO carry a proper `## Dependencies` section; only the marker
  phrase differs. Fix: configurable marker-phrase SET (Blocked by, Depends on) via
  AUTOSPEC_PROJECT_BOARD_DEP_MARKERS, section scoping unchanged.
  Confirmed on real p2: exactly 78/80 have edges, the two without are #1 and #80.
Task 5: Ruling: fold Minors 3-5 (code-fence refs, HTML-comment refs, second `## Dependencies`
  section ignored) into fix round 1. — Why: all three live in the same parse function being
  rewritten for the Critical; splitting them into a deferred list would mean re-opening the same
  code later. — Cost if wrong: a slightly larger fix diff.
Task 5: Ruling: bare `#N` keeps resolving to the item's OWN repo even though p1 may intend a
  tracker repo. — Why: guessing a different target repo risks fabricating a WRONG edge that could
  later read as satisfied; an edge pointing at an issue absent from the board is already handled
  downstream as unresolvable and fails closed. — Cost if wrong: some p1 cross-repo edges land as
  unresolvable and need an operator to write `owner/repo#N` explicitly. Fail-closed either way.
Task 5: fix round 1/5 dispatched (5 findings).

## Execution shape change (operator asked for completion tonight)

Ruling: switch from strictly-serial to parallel-on-disjoint-file-sets, fold same-file tasks into
one dispatch, and overlap reviews with the next implementation. — Why: 16 tasks remained and the
serial loop would not finish tonight; implementers cannot share a worktree (concurrent git commit
corrupts the index) but CAN run in dedicated worktrees when their file sets are disjoint, and
reviewers are read-only so they never contend. — Cost if wrong: merge conflicts between parallel
branches, which are visible and recoverable; and a slightly higher chance two agents duplicate a
small piece of work.
Ruling: do NOT reduce review depth to save time. — Why: the loop has caught a real defect in 4 of
the 5 tasks so far, including the two that would have made the feature useless on real data (inert
`closed` state; final-audit-promoted-first). Cutting the gate to hit a deadline is how a green
suite ships an inert feature. — Cost if wrong: we finish later than the operator wants.
Parallel track opened: worktree .claude/worktrees/pb-rust, branch feat/project-board-rust-config,
based on 442e7704, scoped to crates/** only. Task 7 dispatched there.
Batching plan for the remainder: 5+6 to one agent (same file), 9+10 to one, 11+12+13 to one,
14 alone. Plan A drops from 10 remaining dispatches to ~5.
Task 5: fix round 1/5 returned 28/28 deps, 64/64 suite (commits 442e7704..8d16fcb5). p1 edges
  0 -> 54/80; p2 unchanged 78/80; #1 deps_unresolvable=false vs #80 true. Implementer self-reported
  finding and fixing two bugs it introduced mid-round (a jq index(.) scoping bug that escaped every
  char, and an apostrophe inside the single-quoted jq program) — re-reviewer told to verify both.
  New schema fields: deps_unresolvable, deps_reason (fixed sentence, deliberately not echoing
  untrusted body text).
Task 5: re-review running in FROZEN detached worktree .claude/worktrees/pb-review @ 8d16fcb5.
Ruling: reviewers get a frozen detached worktree whenever they overlap an implementer touching the
  same files. — Why: my overlap plan would otherwise have a reviewer running tests against a tree
  another agent is mid-edit on, which yields false failures and false passes (the known
  no-tree-mutation-during-background-validate hazard). — Cost if wrong: one extra worktree per
  overlapped review, and ~30s of checkout.
Task 6: dispatched to the SAME agent as Task 5 (same file, hot context) instead of a fresh
  implementer. Carried three rulings: consume deps_unresolvable as never-ready; explicit pipeline
  wiring shape (brief's step 3 was prose); keep the cycle-imprecision deliberately. Also told to
  measure the recursive jq's runtime and swap to an iterative fixpoint if pathological.
Task 5: fix round 1/5 (5 addressed, 0 open; commits 442e7704..8d16fcb5). Re-review PASS, all
  numbers independently re-measured. Both self-reported mid-round bugs verified genuinely gone.
  p1's 26 edgeless items explained and benign: 10 declare "none", 16 are tracker/epic issues with
  no `## Dependencies` heading (they use a `## Protocol issue tree` checkbox list). Reviewer
  independently grepped every p1 Dependencies section — no third unparsed marker phrasing exists.
Task 5: complete (commits a29b36d8..8d16fcb5, review clean)
Task 7: implementer DONE (commit c0725735 on feat/project-board-rust-config). 7/7 new suite; two
  OTHER crate failures claimed pre-existing (macOS /var vs /private/var TMPDIR) — reviewer told to
  verify that claim against the parent commit rather than trust it. Notable catch by the
  implementer: `control_issue: owner/repo#1` is truncated by config.rs's top-level `#` comment
  stripper, so it used the tier4-style whitespace-guarded stripper instead.
Task 9: dispatched in parallel, dedicated worktree .claude/worktrees/pb-wb, branch
  feat/project-board-writeback, scoped to two NEW files only (writeback script + its bats) so it
  cannot conflict with Task 6 (deps) or the upcoming Task 8 (promoter), both of which touch other
  files. Carried the degenerate-input lesson from Tasks 4 and 5 up front to avoid a third
  predictable fix round.
Ruling: Tasks 8 and 10 both modify scripts/autonomous-promote-open-issues.sh, so they must NOT run
  in parallel with each other; they will be batched into a single dispatch once Task 6 lands.
  — Why: two agents editing one file on one branch is a guaranteed conflict, and batching them
  also saves a full review cycle. — Cost if wrong: one larger review surface for that dispatch.
Task 7: complete (commit c0725735 on feat/project-board-rust-config, review clean — 0 findings at
  any severity). Reviewer independently checked out parent 442e7704 and reproduced the two failing
  suites there, confirming they are pre-existing macOS TMPDIR symlink failures, not a regression.
  Security gate probed hard: [""], non-list scalar, blank url — all correctly rejected.
  `control_issue: owner/repo#1` round-trips intact while trailing real comments still strip.
Task 7: MERGE PENDING — feat/project-board-rust-config is clean but must not be merged into
  docs/project-board-ingestion-spec while the Task 6 agent is working in that worktree. Merge as
  soon as Task 6 lands.
Task 9: implementer DONE 17/17 + shellcheck clean (commit 49b910df on feat/project-board-writeback);
  review dispatched with a mutation-safety audit (it is the only mutating script in the feature).
Note: new worktrees need .superpowers copied in BEFORE dispatch — the Task 7 reviewer had to
  recover its brief from the sibling worktree. Copy first next time.
Task 6: implementer DONE 41/41 deps, 77/77 suite (commit 9f4cb03e). Replaced the brief's recursive
  cyclic() with an iterative topological fixpoint (bounded N+1 rounds); p1 0.469s, p2 0.329s.
  REAL DATA: p1 ready=26 blocked=54 unresolvable=0 cycles=0; p2 ready=1 blocked=79 unresolvable=1
  (#80, correctly not ready) cycles=0. Zero cycles on both real boards — no finding to surface,
  and p2's ready=1 matches the spec's premise that the DAG, not the queue, is what gates work.
  Self-reported and fixed two more mid-round bugs (unconditional overwrite of blocked_by without a
  dep source; a second `.`-rebinding jq bug in index(key(.))). Review dispatched on frozen tree.
Task 9: complete (commit 49b910df, review approved — 0 Critical, 0 Important). gh call sites
  enumerated: exactly 2, exactly 1 write (`project item-edit`). No field-create/item-add/
  item-delete/issue-edit anywhere. Fail-open, scope gate, idempotence, and 9 degenerate inputs all
  confirmed; no silent-swallow path; shellcheck clean.
Task 9: minor (folded into the 8+10 dispatch): the idempotence test passed for the WRONG reason —
  its shared fixture lacks `Blocked` as an option, so it succeeded via the no-matching-option skip
  instead of the real idempotence branch. Ruling: fix it rather than defer. — Why: a test that
  passes for the wrong reason is the exact failure mode that hides real bugs behind a green suite;
  same category as the Task 3 "missing gh" test I folded earlier, so precedent is consistent.
  — Cost if wrong: three lines of fixture.
Task 9: minor (deferred): a board-supplied id starting with `-` reaches `gh --id` as a value;
  quoting stops shell injection and pflag should bind it positionally, but this was only proven
  against the stub, never real gh. Flag for the final review.
MERGED into docs/project-board-ingestion-spec: feat/project-board-rust-config (f2022521) and
  feat/project-board-writeback (bb64968e). 94/94 board tests green on the merged tree.
Tasks 8+10: dispatched as ONE agent (both edit autonomous-promote-open-issues.sh), carrying five
  rulings incl. null-priority ranking (30/80 p1 items have no priority label) and
  deps_unresolvable => never promotable.
Task 6: review spec OK, quality approved with 1 Important. Reviewer reproduced every requested
  graph probe (2-cycle, 3-cycle, self-loop, 20-deep chain, diamond, downstream-of-cycle) and
  confirmed nothing schedulable is wrongly withheld and no cyclic item escapes. Timings actually
  BETTER than claimed (p1 0.332s, p2 0.251s). Both self-reported mid-round bugs verified fixed.
  IMPORTANT: `--resolve` exits 5 when `blocked_by` is present but not an array and the item has no
  dep source to re-derive from — i.e. the script's own idempotent re-pipe path. I reproduced it
  directly (rc=5, "Cannot iterate over string"). Not covered by the 41-test suite.
Task 6: minor (deferred): `.cycles` lumps true members and downstream items into one array rather
  than per connected component. Adequate for per-item gating; inadequate for a future "list
  distinct cycles" view. Flag for the final review.
Task 6: fix round 1/5 dispatched to a FRESH implementer in worktree .claude/worktrees/pb-deps,
  branch fix/project-board-deps-guard.
Ruling: use a fresh implementer in a new worktree rather than resuming the original Task 5/6 agent.
  — Why: resuming it would put it back in .claude/worktrees/project-board-spec, which the Tasks
  8+10 agent currently occupies; two agents in one worktree corrupt each other's index. The fix
  touches project-board-deps.sh only, which 8+10 does not touch, so the branches merge cleanly.
  — Cost if wrong: the fresh agent lacks the original's context and may need the repro spelled out,
  which I supplied.
Task 6: fix returned 51/51 deps, 104/104 suite (commit 2012de49 on fix/project-board-deps-guard).
  Implementer chose option (b) and justified it: a malformed `blocked_by` is an UNRESOLVABLE
  declared dependency (never ready), while explicit null/absent still means "no blockers" and can
  be ready. That split is consistent with the script's existing fail-closed rule for unparseable
  prose. Real-fixture numbers reported unchanged. Re-review dispatched on frozen tree, told to
  scrutinize the MIXED array case (one valid edge + one malformed element) since silently honoring
  the valid one while dropping the malformed one would be an inconsistency.
Note: the "apostrophe inside the single-quoted jq program" bug has now appeared THREE times in this
  one file's history (twice self-caught by implementers, once here). Told the re-reviewer to grep
  the shipped file for it. Worth a lint rule — candidate follow-up issue after this branch lands.
Task 6: fix round 1/5 (1 addressed, 0 open; commits bb64968e..2012de49). Re-review APPROVED.
  Mixed array (one valid edge + one malformed element) is all-or-nothing unresolvable — no silent
  element dropping. blocked_by null/absent still ready. Real-fixture numbers identical. 41 prior
  test names byte-identical. Apostrophe grep clean. Guards are structural (is_edge_obj,
  sanitize_blocked_by), not suppression.
Task 6: complete (commits 8d16fcb5..2012de49, review clean)
Task 6: MERGE PENDING — fix/project-board-deps-guard cannot merge while the 8+10 agent occupies
  the project-board-spec worktree. Merge once Task 10 lands.
Task 8: landed as 296f56f4 (agent now on Task 10 in the same dispatch).
Tasks 11 and 13: dispatched in parallel worktrees pb-skill (feat/project-board-skill) and
  pb-install (feat/project-board-install), both off 296f56f4. File sets are disjoint from each
  other and from 8+10: 11 owns skills/autospec-project/** + its goldens, 13 owns
  skills/autospec-autonomous/install.sh + tests/install/. Task 12 (autospec-listen route) is held
  back deliberately — it regenerates autospec-listen goldens and I want Task 11's trio proven first.
Ruling: expand Task 13 beyond its brief to register the FULL transitive grooming dependency set,
  determined empirically from the script's own seam variables rather than the brief's partial list,
  with grooming-config.sh going in the SHARED-library group because it lives under
  skills/autospec-shared/scripts. — Why: registering a partial set still crashes Tier 1.5 on a
  clean install, which is the exact bug the task exists to prevent, and mis-grouping a shared
  helper is a known past failure. — Cost if wrong: a few already-present files get re-copied.
Ruling: Task 13 must run the installer only against an isolated temporary HOME, never the real one.
  — Why: the operator has a live autospec install; an unisolated installer run would overwrite it.
  — Cost if wrong: none; if isolation is not achievable the check is skipped and reported.
Tasks 8+10: implementer DONE 122/122 (commits 296f56f4 Task 8, bab5c258 Task 10, 0f25e29a the
  write-back idempotence-fixture fix). All 94 board + 14 promoter tests pass unchanged.
  MERGED fix/project-board-deps-guard -> afeb697e; 132/132 on the merged tree.
Tasks 8+10: two self-reported gaps, both real:
  (a) board promotion is NOT subject to budget.max_issues_per_cycle. I confirmed in the source:
      line 136 computes the budget (default 10), line 295 passes it ONLY to the grooming path.
      Consequence: a board with many simultaneously-ready items could promote all of them into an
      admin-auto-merge queue in one cycle, bypassing the admission control every other path
      respects. Reviewer asked to rate severity independently; I expect this becomes a fix round.
  (b) my brief's test scaffolding omitted an AUTOSPEC_GROOM_SAFETY_BIN stub, and a REAL autospec
      binary is on PATH here — so the given tests could have reached a live safety authority. The
      implementer caught it and added a stub matching the existing promoter test pattern. That was
      a genuine safety hole in my brief, not in their work.
Ruling: reviewers and implementers must stub BOTH `gh` and AUTOSPEC_GROOM_SAFETY_BIN from now on.
  — Why: a real autospec binary on PATH plus an unstubbed safety seam means a test can perform a
  real promotion against a real repo. — Cost if wrong: none; stubbing is free.
Task 13: implementer DONE (commit a87f212d). Registered 9 repo-root scripts + grooming-config.sh in
  a NEW SHARED_LIB_SCRIPT_FILES group. Key finding beyond registration: the promoter resolves
  grooming-config.sh via a RELATIVE path ($SCRIPT_DIR/../skills/autospec-shared/scripts), not the
  flat AUTOSPEC_SCRIPTS_DIR convention — so a flat copy would ship the file yet leave it
  UNFINDABLE at runtime. Installed to the nested path the consumer actually computes. Verified
  against an isolated mktemp HOME, never the real one. Tests derive expected sets from live
  sources (glob + seam grep), not hardcoded lists.
  MERGED -> 584b31c2. Its "writeback has no caller" concern was an artifact of branching before
  Task 10; I verified the wiring exists on the merged tree (promoter lines 76, 230-234, 545, 551).
Task 13: review dispatched on a SECOND frozen worktree (pb-review2 @ 584b31c2) because pb-review is
  still occupied by the running 8+10 review — re-pointing a tree out from under a live reviewer
  would corrupt its run.
Tasks 8+10: review spec OK, quality APPROVED with one Important. Every safety property proven not
  assumed: dry-by-default verified with a PATH-shadowed fake autospec that logged if invoked (it
  wasn't); allowlist literal-match probed with o.*, o(a|b)/z — regex would match, literal didn't;
  deps_unresolvable verified using the VERBATIM real p2 #80 body through the real
  normalize->deps->promoter path (#80 ready:false excluded, #1 ready:true included); envelope keys
  unchanged; resolver failure/malformed/unset all isolated; board path reuses the same
  review_admitted_issue/record_rust_safety_result authority — no second mutation path.
  IMPORTANT confirmed: no budget gate on board promotions. Fix round 1 dispatched (shared budget
  across both paths, deterministic top-of-rank truncation, truncation reported not silent).
  Minor folded in: the deps_unresolvable test used PARAPHRASED #80 text instead of the fixture's
  verbatim wording — pointed at the real text so it cannot drift from reality.
Tasks 8+10: minor (deferred): concurrent multi-worker TTL-cache writes not stress-tested (atomic mv
  suggests safety but is unproven under concurrency). Flag for the final review.
Task 13: complete (commit a87f212d, merged 584b31c2, review clean — 0 Critical, 0 Important).
  Reviewer independently traced every seam variable and derived the SAME 9+1 script set that was
  registered; no unregistered runtime-reachable script exists. Path-resolution claim verified by
  installing to an isolated temp HOME and computing the promoter's relative path from the installed
  location — the file is exactly there, and a flat install would indeed have missed it.
  Failing-test-first verified: 5/6 genuinely fail at parent 296f56f4.
Task 14: dispatched in worktree pb-e2e, branch test/project-board-e2e.
Task 12 (autospec-listen route) intentionally still held until Task 11's trio derive+goldens is
  proven — no point regenerating a second skill's goldens until the tooling path is known good.
Task 11: implementer stalled waiting on a full `autospec validate` notification with all work
  UNCOMMITTED on disk. I redirected it to targeted checks and a commit.
Ruling: do not gate any task on a full validate run. — Why: validate is red on main here for
  unrelated reasons, so its raw output cannot say whether OUR change is clean; making it meaningful
  needs a failure-SET diff against a clean origin/main worktree, which costs more than these tasks
  warrant. The targeted gates (derive idempotence, golden sync, injection greps, structural
  sections, trio bats) are the real signal. — Cost if wrong: a lockstep break that only the full
  validate would catch slips to the final review or to CI.
Task 11: complete pending review (commit f1851e22, one commit with prose + both mirrors + all 3
  goldens). Derive idempotent, goldens stable on re-run, no {FEATURE_DESCRIPTION} heredoc, no
  inlined $1/$2/$3, structural sections all present, 24/24 trio+golden tests.
Task 11: FINDING carried forward — `scripts/project-board-resolve.sh` usage advertises
  `--emit identity|plan|fleet-config|repos` (lines 8 and 41) but the case block implements only
  identity/plan/repos. I confirmed it directly. `fleet-config` is Plan B Task 1's deliverable.
Ruling: leave the mismatch for Plan B Task 1 to close rather than patching the usage string now.
  — Why: Plan B is queued for tonight and will implement the mode; patching the string now would
  just be reverted. — Cost if wrong: if Plan B slips, Plan A ships with a --help that advertises a
  mode returning "unsupported". MUST be re-checked at the final review, and the usage string
  corrected if Plan B has not landed by then.
Task 12: dispatched in pb-skill on top of Task 11's commit, so the route can reference the real
  skill. Told to leave scripts/listener-match.sh alone unless genuinely required — it is a shared
  deterministic classifier and an unnecessary edit risks other routes.
Task 14: implementer DONE 7/7 e2e + 118/118 board suite (commit 07a1885a on test/project-board-e2e).
  All measured baselines reproduced exactly. Ran a deliberate-failure check (flipped p2 #80 to
  ready, confirmed red, reverted). Directly proved p1's `Depends on issue #N` phrasing is
  load-bearing: 54 edges with the default marker set, 0 edges if only `Blocked by` is recognized.
  Live-network steps from the brief were removed from scope by me and NOT run.
Task 14: review dispatched with a MUTATION-TEST mandate — for each of the 7 tests, break the thing
  it claims to check and confirm it goes red, then revert.
Ruling: review a test-only change by mutation, not by reading. — Why: the question for a test file
  is not "does it pass" but "would it fail if the feature broke"; a test that cannot fail
  manufactures confidence, and this plan has already hit two tests that passed for the wrong reason
  (Task 3's missing-gh test, Task 9's idempotence test). — Cost if wrong: the review costs more
  tokens and briefly mutates a frozen worktree, which is restored and verified clean.
Tasks 8+10: fix round 1 returned 136/136 (commit de922740). Shared budget:
  board_budget_remaining = budget - grooming_promoted_count, floored at 0, computed after the
  grooming loop; cap keeps the TOP of the existing ranking; withheld items get no mutation and no
  write-back and are counted in a new .board.truncated field. Finding 2 fixed: the test now pulls
  #80's verbatim body from the fixture at runtime via jq. Re-review dispatched, told to probe the
  shared-budget property at N=0, 0<N<budget, and N>=budget.
Task 11: review spec OK, quality approved with one IMPORTANT — the skill contradicts itself about
  `ship`. SKILL.md:117 says "Do not describe ship as launching workers unattended", but the
  frontmatter description (line 3), the Invocation quick-reference (line 47) and README:31 all
  promise "conductors, unattended". I confirmed all four locations directly. The frontmatter is the
  most-visible summary AND drives skill matching, so it is the version an operator acts on.
  Fix dispatched in worktree pb-skillfix, branch fix/project-board-skill-prose.
Task 11: FINDING (pre-existing, not Task 11's fault) — project-board-resolve.sh validates --emit
  AFTER making gh network calls, so an unsupported mode returns exit 3 (auth/scope) instead of the
  contracted exit 2 (usage), and burns two API calls doing it. Confirmed: `--emit fleet-config`
  gives "gh project field-list failed", rc=3.
Ruling: fold the --emit validation-ordering fix into Plan B Task 1, which already edits that exact
  case block to implement fleet-config. — Why: same file, same function, and fixing it now in a
  separate branch would collide with Plan B's edit. — Cost if wrong: if Plan B slips, an invalid
  --emit keeps returning the wrong exit code and makes needless API calls. Re-check at final review
  alongside the fleet-config --help mismatch already logged.
Task 12: implementer DONE (commit e4d10615). Touched scripts/listener-match.sh — justified: the
  acceptance phrase contains the bare word "autospec" which would otherwise have won the old
  generic branch, and SKILL.md prose cannot add URL detection or branch priority. Probes: ship+URL
  -> autospec-project/project-ship; bare URL and "what's on <url>" -> project-resolve, never ship;
  "ship this feature for me" (no URL) -> unchanged autospec-run/ship; /views/3 recognized.
  110/110 classifier tests. Review dispatched with a REGRESSION-DIFF mandate over a broad corpus.
Task 11: fix DONE (commit b4ca726f). I independently verified its one questionable claim: I probed
  derive-trio.sh with a sentinel and confirmed it regenerates only the mirror BODY and preserves
  each mirror's own frontmatter (documented at its lines 14, 17-18). So hand-editing opencode's
  frontmatter description is the intended workflow, not a violation — the never-hand-edit rule
  governs bodies. Worktree restored clean after the probe.
Task 14: complete (commit 07a1885a, merged; review clean — 0 Critical, 0 Important).
  MUTATION TABLE: every one of 5 mutations broke the expected test — flipped #80 readiness (test 4
  red), dropped `Depends on` from the marker set (tests 3 and 6 red), mapped p0 out of vocabulary
  (test 5 red), injected a #1<->#2 cycle (tests 2, 4, 7 red), off-by-one repo count (test 3 red).
  No test stayed falsely green. Offline-ness proven with a PATH-shadowed gh stub that exits 99 —
  never invoked. Fixture provenance checked: plan_from is a pure structural reshape, not a
  re-derivation of the logic under test. ~4.4s, per-test mktemp, no order dependence.
Task 11: fix round 1/5 (1 addressed, 0 open; commits f1851e22..b4ca726f). Re-review ADDRESSED.
  All 9 remaining 'unattended' hits now state the launch is NOT built; none promise it. New
  description explicitly flags the gap rather than silently omitting it. opencode frontmatter
  matches SKILL.md byte-for-byte; codex has no frontmatter block. derive --check exit 0, goldens
  already matching, one commit, 24/24.
Task 11: complete (commits 296f56f4..b4ca726f, review clean)
Tasks 8+10: fix round 1/5 (2 addressed, 0 open; commits 0f25e29a..de922740). Re-review APPROVED.
  Shared-budget probe (2 board-ready items, budget=3):
    N=0 -> remaining 3, both promoted, truncated 0
    N=1 -> remaining 2, both promoted, truncated 0
    N=3 -> remaining 0, ZERO board promotions, truncated 2
    N=5 -> remaining 0 (floored), ZERO board promotions, truncated 2
  Budget check sits AFTER the needs-human / in-progress / not-ready short-circuits, so
  budget-truncation and deps_unresolvable-exclusion never conflate in the envelope. Ranking
  preserved under truncation (ranked top [2,3] survives vs arbitrary slice [1,2]). Withheld items
  proven to get zero mutation and zero write-back from the gh/safety call logs. 136/136, no
  pre-existing test names removed or renamed.
Tasks 8+10: complete (commits bb64968e..de922740, review clean)
Task 12: review spec OK, quality approved with 1 Important. Regression hunt over a 50-input corpus
  found that ONLY Projects-URL inputs changed decision; every other route byte-identical
  (design/spec, implement/build/ship, refine/optimize/tune, run variants, review, explore/discover,
  fix + suppressors, bare autospec, update/self-update, stop). Ship/no-ship asymmetry holds:
  ship|implement|build + URL -> project-ship (0.85); bare URL / "what's on" / "show me" /
  "look at" -> project-resolve (0.7), never ship; "don't ship yet" -> match:false, no route at all.
  Non-Projects GitHub URLs (issue, PR, repo, /repositories) all correctly unmatched. Injection
  probes (regex-metachar URL, 100k-char input, trailing )/") no crash, correct capture boundaries.
  tests/lint 124/125 with the one failure independently reproduced at parent f1851e22.
  IMPORTANT: the two new classifier branches have ZERO tests while every other branch has 110+.
Task 12: fix round 1/5 dispatched — add classifier tests for both new branches plus regression
  guards for the no-URL routes, and prove they can fail by breaking URL detection.
Ruling: require the new branches to be test-locked before merge rather than deferring. — Why: this
  is a SHARED classifier every listen route depends on, project-ship triggers autonomous work on a
  real repo, and an untested branch is one careless edit away from being hijacked or dropped
  silently. — Cost if wrong: one extra fix round on an otherwise-approved task.
MERGED TREE HEALTH at 9145e423: 149 board/promoter/integration/install tests + 24 trio/golden,
  0 failures. 29 commits on branch. main still untouched at 5908df27.
Task 12: fix round 1/5 (1 addressed, 0 open; commits e4d10615..09142460). Re-review PASS.
  Mutation 1 reproduced independently (narrow the Projects-URL regex): exactly tests 71-80 red
  (all 6 project-ship + all 4 project-resolve), all 117 other tests green.
  Mutation 2 (my requested independent one — force ship-verb detection false so ship+URL should
  downgrade to project-resolve): tests 71-76 red, everything else green, and the mutated binary was
  manually confirmed to return trigger:"project-resolve". The ship/no-ship safety asymmetry IS
  covered — that was the gap I was most worried about.
  All positive tests assert BOTH skill and trigger, never merely `match`. No missing required case.
Task 12: complete (commits f1851e22..09142460, review clean)

## PLAN A COMPLETE — all 14 tasks closed, every one reviewed clean.

## FINAL WHOLE-BRANCH REVIEW (opus) — verdict: merge after fixing one Critical

Found what 14 green per-task reviews structurally COULD NOT: three seam breaks where a downstream
stage reads a key no upstream stage ever writes. 236/236 bats passing is exactly why they slipped —
nothing asserted the seams. Rust: 8 failures at HEAD, IDENTICAL set reproduced at merge-base in an
isolated worktree, so zero Rust regressions (pre-existing macOS TMPDIR issue verified, not trusted).

VERIFIED INTACT (the things I most wanted confirmed):
- Load-bearing safety property holds end-to-end through the real promoter with --apply and full
  stubs: promoted:[1], #80 NOT promoted, exactly one safety-authority call, PATH-shadowed real
  autospec invoked 0 times.
- Untrusted-data containment CLEAN against a hostile plan carrying payloads in project id, field id,
  option id, item id (leading -), repo name, comma-forged labels, and body: zero artifacts created,
  no execution-influence path. Label comma-injection can only cause SKIPS, never promotions.
  Allowlist is fail-closed when unset and backed by an independent .repo == $repo gate that holds
  even if the allowlist were "*".

C1 CRITICAL: board_plan() writes a FIXED "$_cache.tmp"; 6 concurrent workers -> 1 rc=0 and 5 rc=1
  with zero stdout. board_plan runs BEFORE the grooming loop, so the ordinary non-board Tier 1.5
  pass dies too and the conductor reads it as "dry" — a multi-worker fleet silently stops grooming.
  The mv is atomic; the SOURCE path is shared. My earlier deferral of item 6 said "atomic mv
  suggests safety" — that was WRONG, and the final review caught my error.
I1: write-back 100% inert — resolver emits project:{owner,kind,number}, write-back needs
  .project.id. 0 item-edit calls in a full --apply cycle, silent because it is fail-open.
I2: idempotence check unreachable — reads .items[].autospec_state, which nothing writes.
I3: scope probed PER ITEM — 80 `gh auth status` calls in one p2 cycle; spec says once.
I4: ProjectBoardConfig has NO consumer — /autospec-autonomous with project_board: configured reads
  no board at all, silently. SKILL.md:76 asserts the opposite as fact.
I5: dependency sources 1 and 2 are dead — resolver projects neither dependencies nor parent_issue.
I6: cycle handling stubbed — .cycles computed then dropped; 3 of 4 code_health markers nonexistent.

Ruling: ONE fix wave, split across TWO agents on disjoint file sets (A: resolver+writeback, I1/I2/
  I3/I5/M2. B: promoter+conductor, C1/I4/I6/M4) rather than one agent or one-fixer-per-finding.
  — Why: the rule against per-finding fixers exists to stop context rebuild and suite re-runs; two
  agents split by file boundary keeps that benefit while halving wall-clock, and the operator asked
  for completion tonight. — Cost if wrong: a merge conflict between two branches, which is visible.
Ruling: re-triage deferred item 6 from "acceptable" to MUST-FIX. — Why: I had accepted the
  implementer's "atomic mv suggests safety" reasoning; the final reviewer disproved it empirically.
  — Cost if wrong: none, the fix is a one-line per-process temp name.
Final fix A: DONE (commit 3b735c6c on fix/final-fixA). 52/52 in its own files, 141 across the
  broader board surface, no regressions. I1 resolver now fetches the real PVT_ node id via
  `gh project view` and emits .project.id (verified at line 191). I2 projects each item's current
  resolved-state into .items[].autospec_state. I5 projects .dependencies (candidate-list field-name
  resolution) and .parent_issue. I3 caches the auth-scope probe. M2 validates --emit before any gh
  call. 5 NEW SEAM TESTS — including a real-resolver -> real-writeback test asserting an actual
  item-edit occurs, which is precisely the test whose absence let I1 ship. Each new test was
  deliberately broken, confirmed red, restored byte-identical (diff-verified), confirmed green.
  No spec correction needed: the spec already said .project.id and "probed once" — these were
  implementation gaps, not spec errors.
  I checked two things myself: the resolver remains a PURE READER (only gh reads, no file writes),
  and the new scope-probe cache degrades to re-probing when it cannot write the file rather than
  failing — fail-soft, consistent with the script's fail-open contract.
Final fix B: DONE (commit 64bca103). C1 CRITICAL fixed (per-process temp path; concurrency test
  confirmed red pre-fix, re-run 3x with no flake), I6 cycles consumed + code_health marker emitted +
  participants labeled needs-human under --apply only, M4 .board.promotable no longer overstates.
  144/144. I4 correctly reported BLOCKED rather than freehanded — it found the shell conductor never
  parses autonomous.yml at all, so there was no pattern to follow, and it refused to reimplement the
  Rust repo_allowlist security gate in shell. It fixed SKILL.md's false claim and re-derived the
  trio + goldens in the same commit. That was the right call, not a failure.
MERGED both fix branches -> bc34e9eb. Combined tree: 0 failures across board, promoter, integration,
  install, and listener suites.

Ruling: I4 is a RESIDUAL LOAD-BEARING finding, so per the breaker rule I ruled on the smallest
  change that unblocks it rather than parking it. Fix B's BLOCKED reasoning was sound but scoped to
  what it could see: `AutonomousConfig` has no consumer in autospec-core, but the CLI already has
  `load_autonomous_config()` at crates/autospec-cli/src/commands/autonomous.rs:1970 which parses
  .autospec/autonomous.yml via repository_config_path(). So the bridge is a small CLI subcommand,
  not new infrastructure. Dispatched in worktree pb-wire.
  — Why: without it /autospec-autonomous with project_board: configured does NOTHING silently,
  which is the difference between an operator re-running /autospec-project sync by hand forever and
  the conductor actually driving the board. That is the autonomy the feature exists to provide.
  — Cost if wrong: a new CLI subcommand that nothing ends up using, removable in one commit.
Ruling: require JSON output parsed with jq over shell `export` lines. — Why: eval-ing Rust-produced
  text is an injection surface and these values flow into a promoter that acts on repositories.
  — Cost if wrong: marginally more shell parsing code.
Final fix I4: DONE but returned UNCOMMITTED, and the agent disclosed a real close call — it twice
  ran `git checkout -- <file>` to revert a deliberate test break, which discards uncommitted work
  rather than undoing the break, and reapplied the lost edits from memory before diff-verifying
  against a saved copy.
Ruling: verify independently rather than trust, then commit on the agent's behalf. — Why: "reapplied
  from memory" on uncommitted work is exactly where a silent divergence hides, and committing is
  bookkeeping, not authoring, so it does not violate the no-controller-fixes rule. — Cost if wrong:
  I would be committing subtly wrong code, which is why I probed the security gate myself first.
  MY OWN VERIFICATION (not the agent's): a project_board block with a url and NO repo_allowlist
  produces exit=2 and stdout_bytes=0 — no url can leak to the promoter. With an allowlist it emits
  the expected JSON. All 3 wiring tests pass; 0 failures across the whole board surface.
  Committed as 7a50ff49, merged -> bf20c280.
  Known scope limit, documented not hidden: ProjectBoardConfig has no schema fields for TTL or
  label_map, so the bridge emits those as null and the shell keeps its own defaults.
Scoped re-review of the ENTIRE fix wave (5a9663ff..bf20c280) dispatched on opus, including an
  explicit mandate to reproduce the 6-worker concurrency crash, attack the I4 security gate along
  9 named paths, and judge the restored I4 code for coherence as if freshly written.

## Scoped re-review of the final fix wave (opus) — DO-NOT-MERGE-as-is -> MERGE after 3 small fixes

ALL NINE findings ADDRESSED (C1, I1, I2, I3, I4, I5, I6, M2, M4). 253/253 bats (up from 236).
C1 concurrency REPRODUCED both ways: pre-fix at 5a9663ff gave 1 rc=0 and 5 rc=1 with 0 bytes and
  the exact `mv: ...json.tmp: No such file or directory`; post-fix 6/6 rc=0 with valid plans,
  re-run 3x = 24/24 clean, no flake.
I4 security gate attacked along 18 probes — malformed YAML, [], [""], scalar allowlist,
  whitespace-only url, missing allowlist all fail at RUST PARSE TIME with exit 2 and zero stdout;
  garbage/partial JSON, missing binary, and --repo-dir outside a repo all degrade to "no board";
  operator env still wins. Invariant HOLDS.
All previously-verified safety properties re-confirmed intact: ordering (#80 held, #1 promoted,
  shadow autospec invoked 0 times), untrusted-data containment (zero /tmp/PWN_* artifacts, every
  hostile payload a literal in argv), dry-by-default, literal allowlist matching, repo scoping,
  shared budget, deps_unresolvable never promotable.
I4 restored-code coherence judged CLEAN — every new symbol occurs once, mirrors main_health's
  dispatch idiom, doc-comment claims all true of the code, nothing half-restored. My concern about
  the `git checkout --` close call was checked and is unfounded.
Reviewer also CORRECTED an earlier under-report: the real Rust failure count is 55 at HEAD vs 54 at
  base across six targets, not the "20" the I4 report claimed — and the failure-NAME set diff is
  exactly ONE test.

THREE NEW Important, nothing Critical:
  NEW-1 the auth-scope cache is written beside the PERSISTENT board cache, so probing once before
    the token has `project` scope caches 0 forever — write-back never runs again on that board on
    that host and never re-probes, with zero operator signal. Reintroduces the I1 silent-inertness
    class that this whole wave existed to kill.
  NEW-2 fixB's skill prose says the conductor does NOT read project_board:, but the I4 bridge landed
    two commits later and made it do so. The shipped skill now asserts the opposite of shipped
    behavior — the exact defect I4 was filed for, inverted.
  NEW-3 `legacy_validation_surfaces_are_absent_from_tracked_files` fails at HEAD because MY plan
    docs (provenance c7a7d416) contain literal `bash scripts/validate.sh`. Red CI here, green on
    main. My own text is the merge blocker.

Ruling: run one more small fix dispatch despite the "no second fix wave" rule. — Why: that rule
  exists to stop endless fix loops on contested findings; these three are uncontested, precisely
  located, and one is a hard CI blocker introduced by my own plan prose. Shipping a red test that is
  green on main, plus a skill that documents the inverse of its behavior, is not a defensible
  hand-off. — Cost if wrong: one more review cycle before merge.
Final blockers: ALL THREE DONE.
  e2febee4 NEW-3 — dead `scripts/validate.sh` references replaced with `autospec validate`;
    `legacy_validation_surfaces_are_absent_from_tracked_files` now PASSES (merge blocker cleared).
  fb2f907d NEW-2 — skill prose corrected to say the conductor DOES consume project_board: via the
    bridge, with mirrors + goldens re-derived in the same commit.
  d5480a10 NEW-1 — auth-scope probe cache moved from a persistent on-disk file to an IN-PROCESS
    shell variable. Deliberate-failure check done properly: stashed the fix and the new test went
    genuinely red against the old code (auth_calls expected 2, actual 1). Foreground evidence:
    run 1 -> 1 `gh auth status`; run 2 in a fresh process against the SAME persistent board-cache
    path -> cumulative 2, proving a fresh run re-probes. No *authscope* file left anywhere.
  255/255 bats (253 baseline + 2 new), 24/24 trio+goldens, 0 regressions.
  Agent honestly flagged that the full-crate cli failure-set diff was not re-run because I had
  scoped it to the single blocking test. I am running that diff myself now rather than leaving the
  claim unverified.

## PLAN A: 14/14 tasks + final review + 3-fix wave + 3 blockers — ALL CLOSED.
