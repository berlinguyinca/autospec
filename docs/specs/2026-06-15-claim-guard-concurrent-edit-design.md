# claim-guard — concurrency-safe pre-edit gate for parallel sessions

## Summary

When multiple autospec or claude sessions run against one repo (multiple
worktrees on a box, or several machines), they edit overlapping skills/files
with no coordination and stomp each other. Because the trio + golden + validate
machinery makes every skill edit an **atomic multi-file unit**
(`SKILL.md` + `codex/prompt.md` + `opencode/agent.md` + `tests/fixtures/skill-goldens/<skill>.*`
+ `validate.sh` gates), two sessions touching one skill = guaranteed
lockstep/golden conflict and rework.

The repo already ships the two outer layers of the fix; this spec adds the
missing inner one:

1. **Worktree-first + stale-base** — already provided by `worktree-guard.sh
   assert` (exit `3 in_primary_checkout` / `4 dirty` / `5 stale_base`). Make
   passing it a **precondition to editing**, not just to sandbox commits.
2. **Pre-flight overlap scan** — new: before editing, check open PRs + other
   `git worktree list` branches + live claims for in-flight edits to the same
   files/skill.
3. **File/skill-granular claim lease** — new: `claim-guard.sh`, an atomic
   path/skill lease with heartbeat-TTL reclaim, composing with (not duplicating)
   the issue-level lease from
   `docs/specs/2026-06-02-atomic-cross-machine-issue-claim-design.md`.

`claim-guard` is the fine-grained complement to the issue claim: the issue
claim says "I own issue #N"; claim-guard says "I own `skills/autospec-explore/*`
right now" — which matters for interactive sessions with no issue and for a
single issue that legitimately edits several skills.

## Problem statement

Observed 2026-06-15: an explore-trio edit was made on an in-flight feature
branch while `main` advanced 15 commits (refactoring the same weights file) and
two other live worktrees (`fix/explore-codebase-signals-precision`,
`fix/installer-ships-runtime-libs`) edited explore-adjacent code. Result: a
conflicted PR and full rework. No mechanism warned that another session owned
that surface. `worktree-guard.sh` existed but is only invoked at sandbox-commit
time, and its scope is the *whole worktree*, not the *files being edited*.

## Team personality

- **Selected team:** Reliability/SRE + backend developer + technical writer.
- **Why this team fits:** the feature is concurrency control — atomicity, TTL
  reclaim, and never stealing a live lease dominate; the writer keeps the
  operator-facing conflict messages actionable.
- **Risks this team will notice:** deadlock on multi-path acquire, a too-short
  TTL reclaiming a slow-but-live editor, claims leaking after a crash, the
  store becoming a single point of failure, lock granularity so coarse it
  serializes unrelated work.
- **Carry into child issues:** all-or-nothing multi-path acquire in sorted
  order; never steal a live claim within TTL; degrade to no-op (never block) if
  the store is unwritable; skill is the atomic lock unit.

## Review counter-team

- **Selected counter-team:** Concurrency + portability.
- **What they should challenge:** can two sessions both win the same lock under
  a race (TOCTOU on acquire)? Does a stale-reclaim ever fire against a live
  editor? Does it work across machines, or silently only same-box? Does it
  leak claims when a session is `kill -9`'d? Is the session identity stable
  under tool-call subprocesses (no TTY)?

## Architecture (where the code lives)

- `scripts/claim-guard.sh` — the lease CLI (`acquire` / `assert` / `refresh` /
  `release` / `status` / `scan`). Mirrors `worktree-guard.sh` style: `usage()`/
  `die()` helpers, stable `code_health:` identifiers on stderr, no RETURN
  traps, `if/then/fi` for one-sided conditionals (repo bash rules), bash 3.2
  safe.
- Claim store: `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/edit-claims/<repo-slug>/`
  — **path-scoped by repo slug subdir** to avoid the cross-repo collision the
  heartbeat dir had ([[feedback_heartbeat_cross_repo_collision]]). One JSON file
  per lock key.
- Session identity: `CLAUDE_CODE_SESSION_ID` with the documented fallback chain
  ([[reference_harness_session_id_envs]]); PPID fallback is unreliable.
- Heartbeat reuse: claim freshness is the same `updated_at`-vs-`now − ttl`
  model the issue claim and `heartbeat-write.sh` already use — no new schema.

## API shape

```
claim-guard.sh acquire <path|skill>...   # atomic, all-or-nothing; 0 ok / 6 claim_conflict
claim-guard.sh assert  <path|skill>...   # read-only check; 0 if free/mine, 6 if held by other
claim-guard.sh refresh                   # bump updated_at on my claims (heartbeat)
claim-guard.sh release [<path|skill>...] # drop my claims (default: all mine in this repo)
claim-guard.sh status                    # list live claims for this repo
claim-guard.sh scan    <path|skill>...   # pre-flight: warn on overlapping open PRs / worktrees / claims
```

Strictness via `AUTOSPEC_CLAIM_GUARD=off|warn|strict` (default `warn`: log the
conflict and proceed; `strict`: refuse with exit 6). `off` or an unwritable
store ⇒ no-op (never block work).

Canonical caller pattern (composes the three layers):

```bash
bash scripts/worktree-guard.sh assert || exit $?         # layer 1: worktree + stale base
bash scripts/claim-guard.sh scan "$TARGETS" || true       # layer 2: advisory overlap warning
bash scripts/claim-guard.sh acquire $TARGETS || exit 6    # layer 3: take the lease
# ...edit... (refresh on the existing heartbeat tick)
bash scripts/claim-guard.sh release $TARGETS
```

## Lock-unit + overlap semantics

- **Lock unit = skill directory.** Any path under `skills/<name>/`, its goldens
  `tests/fixtures/skill-goldens/<name>.*`, or its `skills/<name>/tests/…`
  resolves to lock key `skill:<name>` — because the trio+golden+validate set is
  atomic. Non-skill paths resolve to `path:<normalized-glob>`.
- **Overlap** = two claims whose lock-key sets intersect, OR whose resolved
  path sets intersect. An overlapping live claim owned by a different session
  blocks `acquire`/`assert`.
- **Atomic acquire** via `mkdir`(2) of a per-key `.lock` dir (POSIX-atomic,
  bash-3.2 safe) before writing the JSON; multi-path acquire sorts keys and
  takes them in order (deadlock-free), releasing already-taken keys on any
  conflict (all-or-nothing).

## Data model

`edit-claims/<repo-slug>/<lock-key>.json`:

```json
{
  "lock_key": "skill:autospec-explore",
  "paths": ["skills/autospec-explore/", "tests/fixtures/skill-goldens/autospec-explore.*"],
  "owner_session": "<CLAUDE_CODE_SESSION_ID>",
  "host": "<hostname>",
  "pid": 12345,
  "branch": "fix/explore-7-researchers",
  "worktree": "/tmp/autospec-explore-pr",
  "acquired_at": "<iso>",
  "updated_at": "<iso>",
  "ttl_seconds": 1800
}
```

## Error handling + reclaim

- **Conflict** → exit 6, emit
  `code_health:claim_conflict key=<k> owner_session=<s> host=<h> branch=<b>` so
  the operator/other session can coordinate or pick different work.
- **Stale reclaim** → only when `updated_at` is older than `now − ttl_seconds`.
  Within TTL a slow-but-live editor is **never** reclaimed (mirrors the issue
  claim and autospec-resume's "don't steal a genuinely-live worker"). Reclaim
  is itself atomic (re-`mkdir` the `.lock`).
- **Crash leak** → covered by TTL: a `kill -9`'d session's claim ages out and
  becomes reclaimable; `status` flags claims whose `updated_at` is stale.
- **Cross-machine** → v1 targets same-filesystem multi-session (the common
  multi-worktree case). For distributed runs, the GitHub-issue-comment lease
  (issue-claim spec, which already carries `paths[]`) remains the
  cross-machine authority; claim-guard records `host` so a shared-FS deployment
  works directly and a non-shared one degrades to warn.

## Integration

- **autospec-run Phase 4 implementer**: before editing, `acquire` the
  skill(s)/files it will touch; `refresh` on the existing heartbeat tick;
  `release` on PR open. Sits *inside* the issue claim it already holds.
- **worktree-guard**: unchanged; claim-guard is invoked after its `assert`.
- **Interactive sessions**: documented manual `acquire`/`release`; an optional
  `Edit`/`Write` PreToolUse assert hook is **out of scope v1** (noted as the
  enforcement path).

## Testing (validation-via-shell)

- `tests/test_claim_guard_acquire.bats` — acquire/release round-trip; `status`
  lists the live claim.
- `tests/test_claim_guard_atomicity.bats` — two concurrent `acquire`s of the
  same key: exactly one wins, the other exits 6 (no double-grant).
- `tests/test_claim_guard_overlap.bats` — `assert` of a skill golden path
  conflicts with a held `skill:<name>` claim; a disjoint path does not.
- `tests/test_claim_guard_stale_reclaim.bats` — a claim with `updated_at` past
  TTL is reclaimable; one within TTL is NOT.
- `tests/test_claim_guard_session.bats` — same session re-acquiring its own
  claim is a no-op success; different session is blocked.
- `tests/test_claim_guard_degrade.bats` — `AUTOSPEC_CLAIM_GUARD=off` and an
  unwritable store both no-op without blocking.

## Acceptance criteria

- [ ] `scripts/claim-guard.sh` ships with `acquire`/`assert`/`refresh`/
      `release`/`status`/`scan`, stable `code_health:` identifiers, and bash 3.2
      safety.
- [ ] Claims live under `${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/edit-claims/<repo-slug>/`,
      path-scoped by repo slug; one JSON per lock key matching the data model.
- [ ] Skill directory is the atomic lock unit; skill `SKILL.md`/codex/opencode,
      its goldens, and its tests all resolve to `skill:<name>`.
- [ ] Multi-path `acquire` is atomic all-or-nothing and deadlock-free
      (sorted-order acquisition).
- [ ] Concurrent acquire of one key grants exactly one owner (atomicity test).
- [ ] Stale reclaim fires only past TTL; a live claim within TTL is never
      stolen.
- [ ] `AUTOSPEC_CLAIM_GUARD=off`/`warn`/`strict` behave as specified; unwritable
      store degrades to no-op (never blocks).
- [ ] `scan` reports overlapping open PRs + worktrees + live claims for the
      given targets.
- [ ] autospec-run Phase 4 acquires/releases claims around its edit step,
      refreshing on the existing heartbeat tick, nested inside the issue claim.
- [ ] `scripts/validate.sh` gains `check_claim_guard_contract()` (script present
      + bash-valid + bats suite green); all bats fixtures pass.

## Decomposition into child issues

Aiming for 3 children plus an umbrella.

1. **Issue A — `claim-guard.sh` core**: data model, store layout, lock-unit
   resolver, atomic `acquire`/`assert`/`refresh`/`release`/`status`, TTL
   reclaim, strictness env + degrade-to-no-op. Bats for acquire/atomicity/
   overlap/stale/session/degrade. Files: ~3.
2. **Issue B — `scan` + validate gate**: pre-flight overlap scan across open
   PRs (`gh`) + `git worktree list` + live claims; `check_claim_guard_contract()`
   in `validate.sh`. Depends on A. Files: ~2.
3. **Issue C — autospec-run integration**: Phase 4 implementer acquires/releases
   claims around the edit step and refreshes on the heartbeat tick; trio
   lockstep prose + goldens for the autospec-run change. Depends on A. Files: ~5.

Total: 3 children + 1 umbrella.

## Out of scope (defer to v2)

- A `PreToolUse` `Edit`/`Write` enforcement hook (v1 is caller-invoked +
  autospec-run integration; the hook is the eventual hard-enforcement path).
- Cross-machine claim authority beyond the existing issue-comment lease.
- Lock units finer than a skill directory (e.g. per-section).
- A live operator dashboard of who-owns-what (v1 ships `status`).
- Auto-rebase/auto-merge of conflicting work — claim-guard prevents the
  collision; it does not resolve one after the fact.
