# Atomic cross-machine issue claim — design spec

**Date:** 2026-06-02
**Status:** Phase 2 design (autonomous)
**Repo:** berlinguyinca/autospec
**Author:** autospec Phase 2 design-spec author (autonomous mode)

---

## Problem statement

Multiple `autospec-run` monitors running on different machines pick up and
implement the **same** `auto-implement` GitHub issue, producing duplicate
branches and duplicate PRs that close the same issue.

The concrete TOCTOU (time-of-check / time-of-use) window lives in the hot
monitor loop in `skills/autospec-run/SKILL.md` (lines 479–496, byte-mirrored in
`skills/autospec-run/opencode/agent.md` 478–489 and
`skills/autospec-run/codex/prompt.md` 474–484):

```
CURRENT_LABELS=$(gh issue view ISSUE --json labels --jq -r '.labels[].name')   # CHECK
if ! echo "$CURRENT_LABELS" | grep -q "^auto-implement$"; then ... continue; fi
gh label create in-progress-by-bot --color ededed --force
if ! gh issue edit ISSUE --remove-label auto-implement --add-label in-progress-by-bot; then # USE
  ... skipping
fi
```

`gh issue edit` is an unconditional REST PATCH on the issue's label set — there
is no `If-Match`/ETag precondition — and the label swap (remove
`auto-implement`, add `in-progress-by-bot`) is **idempotent**. Two monitors on
two hosts that both observe `auto-implement` inside the same window therefore
both execute the swap, both see it "succeed", and both proceed to implement.
The only liveness record that survives the swap — the heartbeat at
`~/.autospec/process-heartbeats/<owner_repo>/<issue>.json`
(`scripts/heartbeat-write.sh`, schema
`{"issue","branch","step","ts","pr","repo"}`) — is machine-**local** and is
never read cross-machine, so it cannot break the tie.

## Root cause

The claim is **check-then-act, not compare-and-swap**. The hot loop reads the
label, then blindly writes the label, with no shared, monotonic arbiter
deciding which of two concurrent claimants actually owns the issue. A
GitHub-visible, cross-machine compare-and-swap substrate already ships
(`skills/autospec-run/scripts/claim-issue.sh` +
`skills/autospec-run/scripts/run-state.sh`, which store an
`autospec-run-state` lock JSON inside a marked issue comment), and AGENTS.md
(lines 350–385) even documents routing through it — but the inline hot loop
never calls it and the documentation explicitly calls the unsafe inline swap
the "fallback". Worse, the existing helper's tiebreak is **array-order
dependent, not deterministic earliest-comment-id** (see Data model → Critical
gap), so even routing through it today does not fully resolve a contended
claim.

## Goal

Route the hot monitor loop through a single GitHub-shared compare-and-swap
claim path so that, for any set of monitors on any set of machines contending
for one `auto-implement` issue, **exactly one** monitor wins the claim and
proceeds to implement; every other monitor deterministically detects it lost,
self-cleans, and continues with no branch and no PR. Crashed winners must have
their lease reclaimed cross-machine after a bounded TTL. All existing label
semantics, heartbeat schema, and reclaim windows are preserved.

---

## Team personality

**Reliability / backend team.** This is a distributed mutual-exclusion problem
expressed entirely in shell over the GitHub REST API — no database, no real
lock service, only issue comments and labels as the shared substrate. The team
that fits:

- **Backend developer** — owns the shell claim/run-state code paths and the
  exact `gh`/`gh api` call shapes; keeps the win/lose contract clean.
- **Platform engineer** — owns the SKILL trio wiring and the
  `AUTOSPEC_SCRIPTS_DIR` resolution / fallback so the hot loop actually calls
  the helper on every host.
- **SRE** — owns the stale-lease reclaim, TTL semantics, and the operational
  story when a winner crashes mid-implement; cares that the system self-heals
  on the next loop tick.
- **Security advisor** — owns "can a malicious or buggy worker steal a live
  lease?" and the integrity of the marker comment.
- **Distributed-systems / concurrency specialist** — owns the CAS correctness
  argument itself: linearization point, monotonic arbiter, what happens under
  GitHub eventual consistency.

**Risks this team should notice:** lost-update races on the comment-create
path; clock skew (must never trust a local clock for reclaim); a winner that
crashes after winning the label swap but before writing full run-state; the
helper returning an exit code the SKILL loop cannot branch on; lock-step drift
across the harness trio; the new validate.sh check producing false negatives
(passing while the unsafe pattern is still present).

**Emphasis to carry into child issues:** every claim decision must reduce to a
single deterministic, server-side, monotonic value (earliest marked comment
id); never a local timestamp; one and only one claim code path.

## Review counter-team

**Security + operations + data-integrity counter-team.** Roles:

- **Security reviewer** — challenge: can worker B reclaim worker A's lease
  while A is still alive? Is the TTL large enough that a slow-but-live
  implement is never reclaimed out from under itself?
- **Operations reviewer** — challenge: what does an operator see when a claim
  is lost vs reclaimed? Are both logged distinguishably (`claim lost` vs
  `stale lease reclaimed`)? Does a transient `gh` 5xx look like a lost race
  (safe) or like a false win (catastrophic)?
- **Data-integrity reviewer** — challenge: the CAS-correctness assumptions —
  is "earliest marked comment id wins" actually a total order under concurrent
  comment creation? Does the loser delete only **its own** comment and never
  the winner's? Can the dedup step ever delete the winning comment?

**Assumptions the counter-team must challenge:** (1) comment-id ordering is
monotonic and global per issue; (2) read-back-after-write observes the winner's
comment despite replication lag; (3) reclaim keyed on server `updated_at` can
never fire against a live lease within TTL; (4) `claim-issue.sh` exit codes are
sufficient for the loop to branch on.

**Staying in-scope:** the counter-team reviews CAS correctness, race safety,
and reclaim safety only. It does **not** expand scope into reaping
pre-existing duplicate branches/PRs, into eliminating the extra read-back API
call, or into GitHub replication-lag elimination (all out of scope below).

---

## Architecture (where the code lives)

| File | Role | Change |
|---|---|---|
| `skills/autospec-run/scripts/run-state.sh` | run-state lock-comment read/upsert/clear over `gh api .../comments` | Make `state_comment_ids` / `state_comment_id` / `state_comment_body` sort by **numeric comment id ascending** (deterministic earliest-id), not array order. Add an `--expect-worker` guard for safe reclaim. |
| `skills/autospec-run/scripts/claim-issue.sh` | CAS claim: label swap → upsert run-state → read-back verify; returns win/lose JSON + exit 0/2 | Add earliest-comment-id tiebreak (loser self-deletes its own losing comment); add cross-machine stale-lease reclaim keyed on server-side `updated_at`. Keep exit-0/exit-2 contract. |
| `skills/autospec-run/SKILL.md` (+ `opencode/agent.md`, `codex/prompt.md`) | hot monitor loop | Delete the inline blind label-swap (lines 479–496 / mirrors); **redirect to `claim-issue.sh`** as the sole claim path; branch on its exit code. Byte-identical across the trio (lock-step). |
| `autospec validate` | shell-only validation gate | New `check_claim_cas_guard` (fails if any harness file contains the unguarded check-then-act add-label pattern not wrapped in a CAS read-back path); lock-step diff already covered by `check_lockstep`; `bash -n` on both scripts; file-presence. |

`AUTOSPEC_SCRIPTS_DIR` resolution and the `[ -x "$COORD_CLAIM" ] || COORD_CLAIM=skills/...` fallback (AGENTS.md 356–362) are reused unchanged so the
installed copy is preferred and the in-repo copy is the fallback.

## Interactivity / API shape

**Claim contract (sole entry point):**

```bash
claim-issue.sh --issue <N> --repo <owner/repo> \
               --worker-id <id> [--branch <branch>]
```

- **Exit 0** → claim won. stdout JSON: `{"claimed":true,"issue":N,"repo":...,"worker_id":...,"branch":...}`.
- **Exit 2** → claim lost / not-ready / conflict. stdout JSON:
  `{"claimed":false,...,"reason":"not_auto_implement"|"label_mutation_failed"|"claim_lost"}`.
- **Exit 1** → hard usage error (bad args). Never a race signal.

> Verified against the real file: `skills/autospec-run/scripts/claim-issue.sh`
> lines 13–16 (documented exit codes), 54–59 (not-auto-implement → exit 2),
> 62–66 (label-mutation-failed → exit 2), 76–88 (read-back verify
> `worker_id`+`state`, `claim_lost` → exit 2), 90–95 (win → exit 0). **The
> win/lose exit-code interface the SKILL loop branches on already exists and is
> usable as-is.** The only additions needed are inside the verify/reclaim
> logic, not to the exit-code contract.

**GitHub calls used:**
- `gh issue view <N> --repo <r> --json labels` — read labels (check half).
- `gh label create in-progress-by-bot --color ededed --force` — ensure label.
- `gh issue edit <N> --remove-label auto-implement --add-label in-progress-by-bot` — label swap (kept; semantics unchanged, backward compat).
- `gh api repos/<r>/issues/<N>/comments` — list lock comments (now read with
  `created_at`/`id` so the earliest id can be selected and `updated_at` read
  for reclaim).
- `gh issue comment <N> --body-file` / `gh api repos/<r>/issues/comments/<id> -X PATCH|DELETE` — upsert / dedup / loser self-clean.

**SKILL loop branch:**

```bash
claim_json="$("$COORD_CLAIM" --issue "$ISSUE" --repo {repo} \
  --worker-id "${AUTOSPEC_WORKER_ID:-<derived>}" --branch "$BRANCH")" && claim_rc=0 || claim_rc=$?
if [ "$claim_rc" -ne 0 ]; then
  echo "[monitor] claim lost for #$ISSUE (rc=$claim_rc); refreshing queue"
  ready=($(printf '%s\n' "${ready[@]}" | grep -v "^${ISSUE}$" || true))
  continue
fi
# exit 0 only: this monitor owns #$ISSUE; proceed to process(ISSUE)
```

## Data model

**Lock-comment marker** (unchanged format, `run-state.sh` 6–7):
```
<!-- autospec-run-state:begin -->
{ ...state json... }
<!-- autospec-run-state:end -->
```

**run-state JSON** (`run-state.sh` 147–159, unchanged schema):
```json
{ "schema":1, "repo":"owner/repo", "issue":N, "worker_id":"host:user:...",
  "state":"claimed", "branch":"", "pr":"", "step":"claimed",
  "paths":[], "claimed_at":"<iso>", "updated_at":"<iso>", "ttl_seconds":10800 }
```

**Comment-id tiebreak.** GitHub comment `id`s are globally monotonic per issue
in creation order. The deterministic arbiter is: **the lowest (earliest) marked
comment id is the single owner.** This is the CAS linearization point.

> **Critical gap found in the current code (must be fixed by this spec).**
> `run-state.sh` selects the "winning" comment by **array order**, not by id:
> `state_comment_id()` (lines 91–93) returns `state_comment_ids | sed -n '1p'`
> and `state_comment_body()` (lines 95–100) returns `.[0].body`; the dedup loop
> (lines 174–176) deletes `state_comment_ids | sed '1d'`. The GitHub comments
> endpoint returns ascending `created_at` today, but ordering is not contracted
> and array position is not a guaranteed monotonic key. The fix makes
> `state_comment_ids` emit ids **sorted numeric-ascending** (`jq 'sort_by(.id)'`
> before `.[].id`), so `state_comment_id` / `state_comment_body` / dedup all key
> off the lowest id deterministically.

**Reclaim key.** Staleness is judged **only** on the lock comment's
**server-side `updated_at`** (from `gh api .../comments`, never a local clock)
versus `ttl_seconds` (default `AUTOSPEC_WATCHDOG_RECLAIM_SECS=10800`). A lock
whose server `updated_at` is older than now − ttl is reclaimable; a fresh lock
is not.

## Error handling

- **Loser path (lost race).** A worker whose read-back shows a different
  `worker_id` as owner (or whose own comment is not the lowest id) **deletes
  its own losing lock comment** (`gh api .../comments/<own-id> -X DELETE`),
  logs `claim lost`, returns exit 2, and continues with no branch/PR. It never
  deletes the winner's (lower-id) comment.
- **Win-then-crash.** A worker that wins the label swap and posts its lock
  comment, then crashes before writing full run-state, still leaves a comment
  whose server `updated_at` ages out. After TTL, the next monitor's reclaim
  fires: it treats the stale lock as reclaimable, upserts its own
  `worker_id`/fresh `updated_at`, read-back-verifies, and proceeds.
- **Stale reclaim.** Only when server `updated_at` is older than now − ttl.
  Within ttl, a live-but-slow implement is never reclaimed. Reclaim re-runs the
  same read-back verify so two simultaneous reclaimers still resolve via
  lowest-comment-id.
- **Transient `gh` errors.** Any `gh`/`gh api` failure on the claim path is
  treated as **lost race (exit 2), never a win.** Reuses
  `gh_api_retry` (`run-state.sh` 65–82) for idempotent reads/patches. A failed
  label swap already returns exit 2 (`label_mutation_failed`,
  `claim-issue.sh` 62–66). Fail-closed: ambiguity → do not implement.

## Testing (validation-via-shell only)

No language test runner. Every change ships a `autospec validate` extension
that passes after the change:

1. **`check_claim_cas_guard` (new).** Fails if any of the three harness files
   (`SKILL.md`, `opencode/agent.md`, `codex/prompt.md`) still contains the
   unguarded check-then-act pattern — i.e. a `gh issue edit ... --add-label
   in-progress-by-bot` that is **not** inside a `claim-issue.sh`-routed block.
   Implementation: `grep` for the add-label line in each trio file and assert
   it does not appear except within the documented `claim-issue.sh` call site /
   inside `claim-issue.sh` itself. Also assert each trio file references
   `claim-issue.sh` in the claim section.
2. **`check_lockstep`** (existing, lines 55–67) covers the trio byte-identity
   of the rewritten claim prose automatically.
3. **`bash -n`** on `claim-issue.sh` and `run-state.sh` (existing
   `check_bash_syntax`, lines 81–87).
4. **File presence + executable** for both scripts.
5. **Contended-claim simulation (mocked `gh`).** A shell harness that puts a
   fake `gh` on `PATH` returning a canned comments list with **two** marked
   lock comments (ids `100` and `101`, different `worker_id`s) and asserts:
   - `run-state.sh read` returns the body of id `100` (lowest id), not array
     order;
   - the worker owning id `101` gets exit 2 `claim_lost` and issues a DELETE
     for `101` only;
   - the dedup never DELETEs `100`.
6. **Stale-reclaim simulation (mocked `gh`).** Fake `gh` returns one marked
   comment with `updated_at` older than now − ttl; assert a new worker reclaims
   (exit 0) and updates `updated_at`. A second fixture with fresh `updated_at`
   asserts no reclaim (exit 2).

### Critical-improvement fold-in (highest-risk failure beyond the obvious tests)

**The run-state comment-create call itself races.** When no lock comment exists
yet, both racing workers reach the `else` branch in `run-state.sh` (lines
171–173, `gh issue comment ... --body-file`) and **both POST a begin/end
comment near-simultaneously.** Earliest-comment-id still resolves this *only if*
selection keys off numeric id — which today it does not (the Critical gap
above). Therefore:

- **Folded into Acceptance criteria + Testing:** the contended-claim simulation
  (test 5) MUST cover the *both-create* case (two distinct marked comments, no
  pre-existing lock), not only the both-patch case, and MUST assert the
  lower-id worker wins and the higher-id worker self-deletes **its own**
  comment and returns exit 2.
- **Loser detection vs genuine stale reclaim must be disambiguated:** a worker
  whose comment is the higher id concludes `claim lost` (delete own, exit 2)
  **regardless of `updated_at`** — it must NOT treat the lower-id winner's
  fresh comment as a "stale lease" and reclaim it. Reclaim is only ever
  considered when the lowest-id lock's server `updated_at` is older than
  now − ttl. The order of evaluation is fixed: (1) determine lowest-id owner;
  (2) if I am not the lowest-id owner and the lowest-id lock is fresh → lost,
  self-clean, exit 2; (3) only if the lowest-id lock is stale → reclaim.
- **Exit-code sufficiency confirmed:** `claim-issue.sh` already returns exit 0
  (win) / exit 2 (lost) with parseable JSON (verified, lines 13–16, 90–95), so
  the SKILL loop can branch without new exit codes. The additions are confined
  to selection-by-id and reclaim logic inside the helper; **no new exit codes
  are required.**

## Acceptance criteria

- [ ] The hot monitor loop in all three harness files calls `claim-issue.sh`
      as the **sole** claim path; the inline blind `gh issue edit --add-label
      in-progress-by-bot` is removed from the loop.
- [ ] The three harness files are byte-identical in the claim section
      (`check_lockstep` passes).
- [ ] `claim-issue.sh` returns exit 0 on win and exit 2 on lost/conflict, with
      JSON `{"claimed":true|false,...}` on stdout; the SKILL loop branches on
      the exit code and `continue`s on exit 2.
- [ ] `run-state.sh` selects the winning lock comment by **lowest numeric
      comment id** (not array order) in read, upsert dedup, and clear.
- [ ] Two simultaneous claimants (both-create and both-patch) resolve to
      exactly one winner; the loser deletes only its own comment and exits 2.
- [ ] A lock whose server-side `updated_at` is older than
      `AUTOSPEC_WATCHDOG_RECLAIM_SECS` (10800) is reclaimable; a fresher lock
      is not. Reclaim never keys on a local clock.
- [ ] Any transient `gh` error on the claim path yields exit 2 (lost), never a
      false win.
- [ ] Heartbeat schema `{"issue","branch","step","ts","pr","repo"}` and
      `in-progress-by-bot` label semantics are unchanged.
- [ ] `autospec validate` gains `check_claim_cas_guard` which fails on the
      unguarded check-then-act pattern, plus the two mocked-`gh` simulations;
      `validate.sh` passes end-to-end after the change.
- [ ] The 300s watchdog claimed-timeout is untouched.

## Out of scope

- Reaping pre-existing duplicate branches/PRs created **before** this fix
  (separate cleanup issue).
- Eliminating GitHub REST eventual-consistency / replication lag — brief
  ambiguity is accepted; comment-id ordering self-heals on the next loop tick.
  Read-back-after-write mitigation is allowed but not required.
- Eliminating the extra read-back GET/POST per claim attempt (one added call
  per claim is acceptable; no caching layer).
- Reworking the 300s watchdog claimed-timeout itself.
- Any change to the heartbeat schema or `in-progress-by-bot` label semantics.

## Decomposition preview

**Parent tracker:** "Atomic cross-machine issue claim" — route the hot monitor
loop through a single GitHub-shared CAS claim path; close when all four
children merge and `validate.sh` passes.

Ordered children (each ≤3 files, harness trio byte-identical where touched):

1. **Route hot loop through `claim-issue.sh`.** Replace the inline blind
   label-swap (SKILL.md 479–496 + mirrors) with a `claim-issue.sh` call that
   branches on exit 0/2. Files: `SKILL.md`, `opencode/agent.md`,
   `codex/prompt.md`. Depends-on: none.
2. **Earliest-comment-id CAS tiebreak + loser self-clean.** Make
   `run-state.sh` select by lowest numeric comment id (read/dedup/clear); make
   `claim-issue.sh` loser delete only its own comment and return exit 2. Files:
   `run-state.sh`, `claim-issue.sh`. Depends-on: #1.
3. **Cross-machine stale-lease reclaim on server `updated_at`.** Add
   reclaim-when-stale to `claim-issue.sh`/`run-state.sh` keyed on server
   `updated_at` vs `AUTOSPEC_WATCHDOG_RECLAIM_SECS`; evaluation order
   lowest-id-owner → lost-if-fresh → reclaim-if-stale. Files: `claim-issue.sh`,
   `run-state.sh`. Depends-on: #2.
4. **`validate.sh` guard + lock-step propagation.** Add `check_claim_cas_guard`
   + the two mocked-`gh` contended/reclaim simulations; confirm `check_lockstep`
   covers the new claim prose. Files: `autospec validate` (+ any mock
   fixtures). Depends-on: #1, #2, #3.

## Autonomous assumptions

> **AUTONOMOUS ASSUMPTION:** GitHub issue-comment `id`s are globally monotonic
> in creation order per issue (lower id = created earlier). This is the
> deterministic arbiter; it is consistent with observed REST behavior but is
> not a contracted ordering guarantee.

> **AUTONOMOUS ASSUMPTION:** the `comments` REST list returns a per-comment
> `updated_at` server timestamp usable for staleness, and that it advances on
> every PATCH to the lock comment (so heartbeat upserts refresh the lease).

> **AUTONOMOUS ASSUMPTION:** `AUTOSPEC_WATCHDOG_RECLAIM_SECS` (default 10800)
> is the correct reclaim window to reuse for stale-lease detection; it is
> larger than any single legitimate implement, so a live worker is never
> reclaimed within TTL.

> **AUTONOMOUS ASSUMPTION:** the `--worker-id` derivation in `claim-issue.sh`
> (lines 48–52, `host:user:shell:pid:epoch`) is unique enough across machines
> that two distinct workers never collide on the same id.

> **AUTONOMOUS ASSUMPTION:** mocking `gh` via a `PATH`-shadow shell stub is an
> acceptable validation technique in this repo (consistent with the
> "validation-via-shell, no language test runner" rule in AGENTS.md line 10).
