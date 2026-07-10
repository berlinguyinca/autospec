# Growth Conductor Fold-In Implementation Plan (Plan 5 of 5, revised)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fold the growth flywheel into the existing `autospec-autonomous` conductor as capability-gated tiers (no new skill), and fix the live content-quality-gate bypass for `growth:artifact` issues.

**Architecture:** Two parts. **Part A** routes `growth:artifact` issues through `/autospec-run`'s existing Phase-4 label router so the content-quality gate fires on every drain path. **Part B** appends three growth tiers to the conductor (`autonomous-waterfall.sh` + `scripts/lib/autospec-loop.sh`), gated on `.autospec/growth.yml` presence, invoking the already-shipped `grow-define`/`grow-run`. Growth artifact *implementation* already competes in Tier 1 (the issues carry `auto-implement`); only the growth *meta-work* (discovery, outbound, measure) is new tier work, appended after Tier 4 so existing tier numbers are unchanged.

**Tech Stack:** bash 3.2 + `jq` + `gh`; the autospec trio toolchain (`derive-trio.sh`, `gen-skill-goldens.sh`, `validate.sh`).

## Global Constraints

- bash 3.2 compatible; scripts `set -uo pipefail` (match each edited file's existing `set` line — `autonomous-waterfall.sh` uses `set -u`, `fab-route.sh` uses `set -uo pipefail`).
- Whole-label matching only (no substring, no regex on labels).
- Capability-gated: growth tiers are inert unless `.autospec/growth.yml` exists AND validates via `validate-growth-config.sh`. A repo without it must produce **byte-identical** waterfall decisions and conductor behavior to pre-change `main`.
- New growth tiers are **appended after Tier 4** in the waterfall cascade (before `idle-rescan`); do NOT renumber Tiers 0–4 (tests/digests assert those numbers).
- Every tier dispatch follows the existing template: `AUTOSPEC_<CAP>_CMD` env seam → real skill invocation → graceful no-op JSON `{"dry":true,"filed":0,"reason":"..."}`; parse `.dry`/`.filed`; increment `_tierN_dry_cycles`; set `_work_done=1` on real output.
- Never auto-post (Plan 4 invariant) — Tier G2 produces packages, never publishes.
- Editing any `SKILL.md` requires re-deriving its trio (`derive-trio.sh skills/<skill> --in-place`) AND regenerating goldens (`gen-skill-goldens.sh <skill>` — **bare name**, not a path) in the SAME task.
- Shared budget: growth uses the one conductor quota; do NOT add a growth spend seam.

## File Structure

Modified:
- `skills/autospec-run/scripts/fab-route.sh` — add the `growth` route.
- `skills/autospec-run/scripts/fab-route.bats` (or the existing suite) — growth cases.
- `skills/autospec-run/SKILL.md` (+ trio + goldens) — `GATE=growth` branch.
- `skills/autospec-grow-run/SKILL.md` (+ trio + goldens) — R1 delegates the gate.
- `scripts/autonomous-waterfall.sh` — three growth actions + args + capability gate.
- `scripts/lib/autospec-loop.sh` — three dispatch branches + growth detection + flag threading.
- `skills/autospec-autonomous/SKILL.md` (+ trio + goldens) — document the growth tiers.
- `scripts/validate.sh` — extend the conductor/waterfall check for the growth-tier contract.
- Test files under `tests/` for the waterfall growth cases and loop dispatch.

No new skills, no new conductor scripts.

---

## Task 1: Route `growth:artifact` through fab-route.sh

**Files:**
- Modify: `skills/autospec-run/scripts/fab-route.sh`
- Test: `skills/autospec-run/scripts/fab-route.bats` (extend; if the suite lives elsewhere, find it with `grep -rl fab-route tests skills`)

**Interfaces:**
- Produces: `fab-route.sh --labels "<csv>" | --stdin` now prints one of `fab|growth|default`. Precedence `fab` > `growth` > `default`. `growth` when any label equals `growth:artifact` (whole-label).

- [ ] **Step 1: Write the failing tests**

Add to the existing bats suite:

```bash
@test "growth:artifact routes to growth" {
  run bash "$SCRIPT" --labels "auto-implement,growth:artifact,growth:content"; [ "$status" -eq 0 ]; [ "$output" = "growth" ]
}
@test "fab wins over growth when both present" {
  run bash "$SCRIPT" --labels "growth:artifact,area:fab"; [ "$output" = "fab" ]
}
@test "growth:artifactx does not route to growth (whole-label)" {
  run bash "$SCRIPT" --labels "growth:artifactx"; [ "$output" = "default" ]
}
@test "plain auto-implement still default" {
  run bash "$SCRIPT" --labels "auto-implement,ctx:64k"; [ "$output" = "default" ]
}
```
(Set `SCRIPT="$BATS_TEST_DIRNAME/fab-route.sh"` in `setup` if the suite doesn't already.)

- [ ] **Step 2: Run to verify failure**

Run: `bats skills/autospec-run/scripts/fab-route.bats`
Expected: the growth cases FAIL (route returns `default`).

- [ ] **Step 3: Add the growth route**

In `fab-route.sh`, add a growth-labels constant next to `FAB_LABELS`:

```bash
# Labels that route an issue to the growth content-quality gate.
GROWTH_LABELS="growth:artifact"
```

In `route_for_labels()`, keep the existing fab loop first (fab precedence), then add a growth pass before the final `default`. Replace the single fab loop with two ordered passes over the parsed labels:

```bash
    # Pass 1: fab wins (highest precedence).
    for label in "$@"; do
        label="${label#"${label%%[![:space:]]*}"}"; label="${label%"${label##*[![:space:]]}"}"
        [ -n "$label" ] || continue
        for fab in $FAB_LABELS; do
            if [ "$label" = "$fab" ]; then printf 'fab\n'; return 0; fi
        done
    done
    # Pass 2: growth.
    for label in "$@"; do
        label="${label#"${label%%[![:space:]]*}"}"; label="${label%"${label##*[![:space:]]}"}"
        [ -n "$label" ] || continue
        for g in $GROWTH_LABELS; do
            if [ "$label" = "$g" ]; then printf 'growth\n'; return 0; fi
        done
    done
    printf 'default\n'
    return 0
```

Update the header usage comment to list `growth`.

- [ ] **Step 4: Run to verify pass**

Run: `bats skills/autospec-run/scripts/fab-route.bats`
Expected: PASS (existing fab/default cases + 4 new).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/scripts/fab-route.sh skills/autospec-run/scripts/fab-route.bats
git commit -m "fix(run): route growth:artifact issues to a growth gate in fab-route"
```

---

## Task 2: `/autospec-run` Phase-4 `GATE=growth` branch

**Files:**
- Modify: `skills/autospec-run/SKILL.md` (+ derived `codex/prompt.md`, `opencode/agent.md`, goldens)

**Interfaces:**
- Consumes: `fab-route.sh` now emitting `growth` (Task 1); `growth-content-quality-precheck.sh` (Plan 4, installed in `~/.autospec/scripts`).
- Produces: `/autospec-run` runs the content-quality gate for `GATE=growth` issues before the standard gates.

- [ ] **Step 1: Locate the routing prose**

Read `skills/autospec-run/SKILL.md` around the "Fab implementer routing (label → gate)" section (the `GATE=fab` / `GATE=default` bullets). The new branch goes alongside them.

- [ ] **Step 2: Add the `GATE=growth` branch (prose)**

After the `GATE=fab` bullet and before `GATE=default`, add:

```markdown
- `GATE=growth` — the issue carries `growth:artifact`. Keep the **standard
  implementer**, and add one **content-quality gate** to Phase 4 before the
  standard reviewer + `growth-ethics` + `autospec-secaudit` gates: run
  `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/growth-content-quality-precheck.sh`
  on the changed content (deterministic pre-checks: keyword-density ceiling,
  FTC-disclosure presence, citation presence), then a `TIER_A` reviewer for
  E-E-A-T / brand-voice, wrapped in the standard 5-attempt adaptive-retry loop
  that feeds findings back as directives. A failing gate blocks merge
  (fail-closed); it never ships unreviewed growth content. This makes the gate
  fire for every path that reaches a `growth:artifact` issue — the autonomous
  Tier-1 drain and `/autospec-grow-run` R1 alike.
```

Ensure the `GATE=default` bullet still reads "every **other** issue".

- [ ] **Step 3: Re-derive trio + regenerate goldens**

```bash
scripts/derive-trio.sh skills/autospec-run --in-place
scripts/gen-skill-goldens.sh autospec-run     # BARE NAME — not a path (exit 0 + "wrote ..." lines)
scripts/derive-trio.sh skills/autospec-run --check   # expect exit 0
```
Confirm `gen-skill-goldens.sh` printed `wrote .../autospec-run.*.sha256` and `git status tests/fixtures/skill-goldens/autospec-run.*` shows them changed.

- [ ] **Step 4: Verify**

Run:
```bash
grep -qF 'GATE=growth' skills/autospec-run/SKILL.md && echo prose-ok
scripts/derive-trio.sh skills/autospec-run --check && echo lockstep-ok
```
Expected: `prose-ok`, `lockstep-ok`.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/SKILL.md skills/autospec-run/codex/prompt.md skills/autospec-run/opencode/agent.md tests/fixtures/skill-goldens/autospec-run.*
git commit -m "feat(run): content-quality gate for growth:artifact issues (GATE=growth)"
```

---

## Task 3: Simplify `grow-run` R1 to delegate the gate

**Files:**
- Modify: `skills/autospec-grow-run/SKILL.md` (+ trio + goldens)

**Interfaces:**
- Consumes: Task 2's `GATE=growth` behavior in `/autospec-run`.
- Produces: `grow-run` R1 no longer describes running the content-quality gate itself; it states that `/autospec-run` now gates `growth:artifact` issues (single-sourced).

- [ ] **Step 1: Read R1**

Read `skills/autospec-grow-run/SKILL.md` `## R1 — Artifact drain`. It currently layers `growth-content-quality-precheck.sh` + a TIER_A reviewer + 5-attempt retry *after* `/autospec-run`.

- [ ] **Step 2: Replace the R1 gate prose with a delegation note**

Change R1 so it invokes `/autospec-run` per `growth:artifact` issue and states that the content-quality gate now runs **inside** `/autospec-run` (via `GATE=growth`, Task 2) — R1 no longer re-runs the precheck/reviewer itself. Keep the ledger outcome recording (`merged_clean`/`failed`) and the "drain continues on per-issue failure" behavior. One concise paragraph replaces the duplicated gate steps. Do NOT change R2/R3/R4.

- [ ] **Step 3: Re-derive trio + regenerate goldens**

```bash
scripts/derive-trio.sh skills/autospec-grow-run --in-place
scripts/gen-skill-goldens.sh autospec-grow-run    # BARE NAME
scripts/derive-trio.sh skills/autospec-grow-run --check
```
Confirm goldens changed in `git status` and lock-step exits 0. (The `check_grow_run_contract` in `validate.sh` still requires the R1/R2/R4 headings — keep the `## R1 — Artifact drain` heading.)

- [ ] **Step 4: Verify**

Run:
```bash
grep -qF '## R1 — Artifact drain' skills/autospec-grow-run/SKILL.md && echo heading-ok
scripts/derive-trio.sh skills/autospec-grow-run --check && echo lockstep-ok
```
Expected: `heading-ok`, `lockstep-ok`.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-grow-run/SKILL.md skills/autospec-grow-run/codex/prompt.md skills/autospec-grow-run/opencode/agent.md tests/fixtures/skill-goldens/autospec-grow-run.*
git commit -m "refactor(grow-run): R1 delegates content-quality gate to /autospec-run (single-sourced)"
```

---

## Task 4: Add growth tiers to the waterfall

**Files:**
- Modify: `scripts/autonomous-waterfall.sh`
- Test: `tests/autonomous/test_waterfall_growth.bats` (new; if the autonomous waterfall tests live elsewhere, put it beside them — `grep -rl autonomous-waterfall tests`)

**Interfaces:**
- Consumes: caller-injected growth state via new flags (all default empty/0 → growth inert).
- Produces: when `--growth-enabled 1`, appends three actions AFTER Tier 4 and BEFORE idle-rescan:
  - `service-growth-outbound` (tier 5) when `--growth-outbound-pending N` > 0.
  - `run-growth-define` (tier 6) when `--growth-backlog N` < `--growth-backlog-floor M` (default 3) and `--tierg-dry-cycles` < threshold.
  - `run-growth-measure` (tier 7) when `--growth-measure-due 1`.
  When `--growth-enabled` is unset/0, NONE of these emit (byte-identical to today).

- [ ] **Step 1: Write the failing tests**

```bash
# tests/autonomous/test_waterfall_growth.bats
setup() { SCRIPT="$BATS_TEST_DIRNAME/../../scripts/autonomous-waterfall.sh"; }

# Force all code tiers dry so the cascade reaches the growth section.
DRY="--dry-cycles 9 --tier15-dry-cycles 9 --tier2-dry-cycles 9 --tier3-dry-cycles 9 --tier4-dry-cycles 9 --backlog-count 0 --open-issue-count 0"

@test "growth disabled: never emits a growth action (regression)" {
  run bash "$SCRIPT" $DRY
  [ "$status" -eq 0 ]
  [[ "$output" != *growth* ]]
  [[ "$output" == *idle-rescan* ]]
}
@test "outbound pending -> service-growth-outbound (tier 5)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 2
  [[ "$output" == *service-growth-outbound* ]]; [[ "$output" == *'"tier":5'* ]]
}
@test "backlog below floor -> run-growth-define (tier 6)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 1 --growth-backlog-floor 3
  [[ "$output" == *run-growth-define* ]]; [[ "$output" == *'"tier":6'* ]]
}
@test "measure due -> run-growth-measure (tier 7)" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 5 --growth-backlog-floor 3 --growth-measure-due 1
  [[ "$output" == *run-growth-measure* ]]; [[ "$output" == *'"tier":7'* ]]
}
@test "outbound outranks define" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 1 --growth-backlog 0 --growth-backlog-floor 3
  [[ "$output" == *service-growth-outbound* ]]
}
@test "no growth work -> idle-rescan even when enabled" {
  run bash "$SCRIPT" $DRY --growth-enabled 1 --growth-outbound-pending 0 --growth-backlog 5 --growth-backlog-floor 3 --growth-measure-due 0
  [[ "$output" == *idle-rescan* ]]
}
@test "growth does not preempt a non-empty code backlog" {
  run bash "$SCRIPT" --backlog-count 3 --growth-enabled 1 --growth-outbound-pending 5
  [[ "$output" == *'"tier":1'* ]]; [[ "$output" != *growth* ]]
}
```

- [ ] **Step 2: Run to verify failure**

Run: `bats tests/autonomous/test_waterfall_growth.bats`
Expected: growth cases FAIL (unknown args).

- [ ] **Step 3: Add the args + defaults**

In `scripts/autonomous-waterfall.sh`, add defaults near the other vars (after line 38):

```bash
GROWTH_ENABLED="${AUTOSPEC_GROWTH_ENABLED:-0}"
GROWTH_OUTBOUND_PENDING=""
GROWTH_BACKLOG=""
GROWTH_BACKLOG_FLOOR="${AUTOSPEC_GROWTH_BACKLOG_FLOOR:-3}"
GROWTH_MEASURE_DUE="0"
TIERG_DRY_CYCLES=0
```

Add to the arg parser (before the `-h|--help` arm):

```bash
        --growth-enabled)          GROWTH_ENABLED="$2"; shift 2 ;;
        --growth-outbound-pending) GROWTH_OUTBOUND_PENDING="$2"; shift 2 ;;
        --growth-backlog)          GROWTH_BACKLOG="$2"; shift 2 ;;
        --growth-backlog-floor)    GROWTH_BACKLOG_FLOOR="$2"; shift 2 ;;
        --growth-measure-due)      GROWTH_MEASURE_DUE="$2"; shift 2 ;;
        --tierg-dry-cycles)        TIERG_DRY_CYCLES="$2"; shift 2 ;;
```

- [ ] **Step 4: Add the growth cascade (after Tier 4, before idle-rescan)**

Immediately before the final never-idle `emit 4 "idle-rescan" ...` block (the comment at line ~237 and `emit` at ~243), insert:

```bash
# ── Growth tiers (capability-gated on .autospec/growth.yml; the caller sets
# --growth-enabled). Appended after Tier 4 so Tiers 0–4 keep their numbers.
# growth:artifact IMPLEMENTATION already competes in Tier 1 (auto-implement);
# these tiers are the growth META-work (outbound service, candidate research,
# measurement) that fills otherwise-idle cycles.
if [ "$GROWTH_ENABLED" = "1" ]; then
    if [ -n "$GROWTH_OUTBOUND_PENDING" ] && [ "$GROWTH_OUTBOUND_PENDING" -gt 0 ] 2>/dev/null; then
        emit 5 "service-growth-outbound" "growth: $GROWTH_OUTBOUND_PENDING outbound draft/approval item(s) pending"
        exit 0
    fi
    if [ -n "$GROWTH_BACKLOG" ] && \
            [ "$GROWTH_BACKLOG" -lt "$GROWTH_BACKLOG_FLOOR" ] 2>/dev/null && \
            [ "$TIERG_DRY_CYCLES" -lt "$DRY_CYCLES_THRESHOLD" ] 2>/dev/null; then
        emit 6 "run-growth-define" "growth: artifact backlog=$GROWTH_BACKLOG below floor=$GROWTH_BACKLOG_FLOOR; researching candidates (tierg-dry-cycles=$TIERG_DRY_CYCLES)"
        exit 0
    fi
    if [ "$GROWTH_MEASURE_DUE" = "1" ]; then
        emit 7 "run-growth-measure" "growth: measure interval elapsed; measuring and re-weighting"
        exit 0
    fi
fi
```

Also add the six new flags + the growth env vars to the `usage()` text (mirror the existing option lines).

- [ ] **Step 5: Run to verify pass**

Run: `bats tests/autonomous/test_waterfall_growth.bats`
Expected: PASS (7/7). Also run the existing waterfall suite (`grep -rl autonomous-waterfall tests` → run each) to confirm no regression.

- [ ] **Step 6: Commit**

```bash
git add scripts/autonomous-waterfall.sh tests/autonomous/test_waterfall_growth.bats
git commit -m "feat(conductor): growth tiers (outbound/define/measure) in the waterfall, capability-gated"
```

---

## Task 5: Wire the growth dispatch branches into the loop

**Files:**
- Modify: `scripts/lib/autospec-loop.sh`
- Test: `tests/autonomous/test_loop_growth_dispatch.bats` (new) — or extend the existing conductor dispatch test found via `grep -rl 'run-architecture-improvement' tests`.

**Interfaces:**
- Consumes: the three growth actions from Task 4; the growth skills.
- Produces: growth detection (sets growth-enabled when `.autospec/growth.yml` exists+valid), threads `--growth-*` flags into the waterfall call, and three `elif` dispatch branches with `AUTOSPEC_GROWTH_{DEFINE,OUTBOUND,MEASURE}_CMD` seams. A growth tier failure never stops the loop.

- [ ] **Step 1: Write the failing tests**

Model on the existing conductor-dispatch test (the one exercising `run-architecture-improvement` via `AUTOSPEC_ARCHITECTURE_IMPROVEMENT_CMD`). Drive the dispatch with each growth `_action` and a mock CMD seam, asserting the mock is invoked and a failing mock does not abort. Example shape:

```bash
@test "run-growth-define dispatches AUTOSPEC_GROWTH_DEFINE_CMD" {
  export AUTOSPEC_GROWTH_DEFINE_CMD="echo '{\"dry\":false,\"filed\":2}'"
  # invoke the dispatch harness the existing test uses for one cycle with
  # _action=run-growth-define; assert _work_done reflects filed>0.
}
@test "growth tier failure does not abort the loop" {
  export AUTOSPEC_GROWTH_DEFINE_CMD="exit 1"
  # assert the cycle records a dry/error result and the loop continues.
}
```
(Match the harness/entrypoint the neighbouring loop-dispatch bats file uses; read it first.)

- [ ] **Step 2: Run to verify failure**

Run the new bats file. Expected: FAIL (no growth branches).

- [ ] **Step 3: Growth capability detection**

In `autospec_conductor_run()`, near where other per-cycle state resolves (mirror the persona-staleness / config reads), compute growth-enabled once per cycle:

```bash
        local _growth_enabled=0
        if [ -f "${_repo_root}/.autospec/growth.yml" ]; then
            local _gv="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/validate-growth-config.sh"
            [ -f "$_gv" ] || _gv="${_sdir}/validate-growth-config.sh"
            if [ -f "$_gv" ] && bash "$_gv" "${_repo_root}/.autospec/growth.yml" >/dev/null 2>&1; then
                _growth_enabled=1
            fi
        fi
```

- [ ] **Step 4: Thread the growth flags into the waterfall invocation**

Where the loop calls `bash "$_waterfall" ...` (around line 1013), add the growth flags when `_growth_enabled=1`. Compute the growth state cheaply (mirror the existing `open_issue_count` gh query pattern; guard each `gh` call so a failure yields 0 / not-due, never aborts):

```bash
        local _growth_flags=""
        if [ "$_growth_enabled" = "1" ]; then
            local _g_backlog _g_outbound _g_measure_due
            _g_backlog="$(gh issue list --repo "$_repo" --state open --label growth:artifact --json number --jq 'length' 2>/dev/null || echo '')"
            _g_outbound="$(gh issue list --repo "$_repo" --state open --label growth/needs-draft --json number --jq 'length' 2>/dev/null || echo 0)"
            # measure-due: compare now vs last growth measure ledger line against grow.measure_interval; default not-due on any error.
            _g_measure_due="$(bash "${_sdir}/growth-measure-due.sh" "$_repo_root" 2>/dev/null || echo 0)"
            _growth_flags="--growth-enabled 1 --growth-outbound-pending ${_g_outbound:-0} --tierg-dry-cycles ${_tierg_dry_cycles:-0}"
            [ -n "$_g_backlog" ] && _growth_flags="$_growth_flags --growth-backlog $_g_backlog"
            [ "$_g_measure_due" = "1" ] && _growth_flags="$_growth_flags --growth-measure-due 1"
        fi
```
Add `local _tierg_dry_cycles=0` beside the other `_tierN_dry_cycles` locals (near line 785). Append `$_growth_flags` (unquoted, word-split intentional) to the existing `_waterfall` invocation's argument list.

> **Note on `growth-measure-due.sh`:** this tiny helper (repo-root → reads the growth ledger, prints `1` if `now - last_measure_ts >= grow.measure_interval`, else `0`; fail-closed to `0`) does not exist yet. Create it in this task under `skills/autospec-shared/scripts/growth-measure-due.sh` with its own bats (`tests/unit/growth-measure-due.bats`): build ledger via real `growth-ledger.sh --append`; assert due when interval elapsed (using `GROWTH_NOW_EPOCH`), not-due otherwise, `0` on missing/unreadable ledger. It installs via `SHARED_LIB_SCRIPT_FILES` wherever the growth scripts are already listed (grep the grow-run install.sh) — but since the CONDUCTOR (repo-root scripts) calls it, also ensure it ships with the core install: add it to the root install's `skills/autospec-shared/scripts` copy set if that isn't automatic (the root install `cp -R`s that dir, so it is automatic).

- [ ] **Step 5: Add the three dispatch branches**

After the `run-explore-once`/`run-explore-once-internet` branch (or beside the Tier 3 branch), add three `elif` branches modeled EXACTLY on the Tier-3 architecture branch (`autospec-loop.sh:1290-1335`): resolve the seam → fallback → dry/error JSON → parse `.dry`/`.filed` → `_work_done=1` or increment the counter. Fallbacks:

```bash
        elif [ "$_action" = "run-growth-define" ]; then
            local _gd_cmd="${AUTOSPEC_GROWTH_DEFINE_CMD:-}"
            [ -n "$_gd_cmd" ] || _gd_cmd="autospec-grow-define run-this-cycle"
            # ... dry-run guard + bash -c "$_gd_cmd" with graceful {"dry":true,"filed":0,"reason":"growth-define-error"} ...
            # parse .dry/.filed; _work_done=1 on filed>0 else _tierg_dry_cycles=$((_tierg_dry_cycles+1))

        elif [ "$_action" = "service-growth-outbound" ]; then
            local _go_cmd="${AUTOSPEC_GROWTH_OUTBOUND_CMD:-}"
            [ -n "$_go_cmd" ] || _go_cmd="autospec-grow-run outbound-only"
            # servicing outbound is always "work"; on non-error set _work_done=1 (no dry-cycle counter)

        elif [ "$_action" = "run-growth-measure" ]; then
            local _gm_cmd="${AUTOSPEC_GROWTH_MEASURE_CMD:-}"
            [ -n "$_gm_cmd" ] || _gm_cmd="autospec-grow-run measure-only"
            # measure is cadence-gated; on non-error set _work_done=1
```

Use the exact dry-run/error-JSON/`jq`-parse idiom copied from the Tier-3 branch for each. (`outbound-only` / `measure-only` / `run-this-cycle` are the grow skills' existing invocation phrases — confirm the exact operator string each grow skill accepts by reading its SKILL.md front matter/usage; if the skill takes no sub-verb, invoke it bare and let its own phase detection run.)

- [ ] **Step 6: Run to verify pass**

Run the new loop-dispatch bats + `bash -n scripts/lib/autospec-loop.sh`.
Expected: PASS; syntax clean.

- [ ] **Step 7: Commit**

```bash
git add scripts/lib/autospec-loop.sh skills/autospec-shared/scripts/growth-measure-due.sh tests/unit/growth-measure-due.bats tests/autonomous/test_loop_growth_dispatch.bats
git commit -m "feat(conductor): growth dispatch branches + capability detection + measure-due helper"
```

---

## Task 6: Document the growth tiers + validate wiring + regression

**Files:**
- Modify: `skills/autospec-autonomous/SKILL.md` (+ trio + goldens)
- Modify: `scripts/validate.sh`
- Test: the full gate + a growth-disabled regression assertion.

**Interfaces:**
- Produces: green `validate.sh`; a documented, capability-gated growth surface on the conductor; a regression guard that growth-disabled behavior is unchanged.

- [ ] **Step 1: Document the growth tiers (prose)**

In `skills/autospec-autonomous/SKILL.md`, extend the tier documentation (and the description line's tier enumeration) to add the three capability-gated growth tiers: they activate only when `.autospec/growth.yml` is present+valid, append after Tier 4, invoke `/autospec-grow-define` (research) / `/autospec-grow-run` outbound (never auto-posts) / measure, and compete under the one shared conductor quota. Keep it concise; do not restructure existing tier prose.

- [ ] **Step 2: Re-derive trio + regenerate goldens**

```bash
scripts/derive-trio.sh skills/autospec-autonomous --in-place
scripts/gen-skill-goldens.sh autospec-autonomous    # BARE NAME
scripts/derive-trio.sh skills/autospec-autonomous --check
```
Confirm goldens changed + lock-step exit 0.

- [ ] **Step 3: Wire validate.sh**

Find the existing conductor/waterfall check in `scripts/validate.sh` (grep for `autonomous-waterfall` / `check_conductor` / `test_conductor`). Add `bash -n` for the edited scripts and register the two new bats suites (`tests/autonomous/test_waterfall_growth.bats`, `tests/autonomous/test_loop_growth_dispatch.bats`, `tests/unit/growth-measure-due.bats`) in `main`'s run list (gate-atomicity). Mirror the exact helper names the file already uses.

- [ ] **Step 4: Regression assertion (growth-disabled = unchanged)**

Confirm the Task-4 regression test (`growth disabled: never emits a growth action`) is in the validate run set, and add one asserting the waterfall's decision for a representative dry state is identical with and without the growth flags absent:

```bash
@test "growth-absent waterfall decision unchanged vs baseline" {
  a="$(bash "$SCRIPT" --dry-cycles 9 --tier15-dry-cycles 9 --tier2-dry-cycles 9 --tier3-dry-cycles 9 --tier4-dry-cycles 9 --backlog-count 0 --open-issue-count 0)"
  [[ "$a" == *idle-rescan* ]]; [[ "$a" != *growth* ]]
}
```

- [ ] **Step 5: Full gate + root install**

Run:
```bash
scripts/validate.sh            # expect: validate: OK — all validation checks passed.
TH="$(mktemp -d)"; HOME="$TH" bash install.sh --hook-mode claude >/dev/null 2>&1 && echo root-ok; rm -rf "$TH"
```
Expected: `validate: OK …` and `root-ok` (no new skill pair; existing pairs unaffected).

- [ ] **Step 6: Commit**

```bash
git add skills/autospec-autonomous/SKILL.md skills/autospec-autonomous/codex/prompt.md skills/autospec-autonomous/opencode/agent.md tests/fixtures/skill-goldens/autospec-autonomous.* scripts/validate.sh
git commit -m "docs(conductor): document growth tiers + validate wiring + regression guard"
```

---

## Self-Review

**Spec coverage:**
- Part A route fix → Task 1 (fab-route `growth`), Task 2 (`GATE=growth` in /autospec-run), Task 3 (grow-run R1 delegates). ✓
- Part B fold-in → Task 4 (waterfall growth tiers), Task 5 (loop dispatch + capability detection + measure-due helper). ✓
- Capability-gated / regression (growth-disabled byte-identical) → Task 4 regression test + Task 6 assertion. ✓
- Shared budget → no spend seam added anywhere (constraint honored); growth meta-work appended after Tier 4, artifacts compete in Tier 1. ✓
- Docs + validate + gate → Task 6. ✓

**Placeholder scan:** the deterministic inserts (fab-route, waterfall cascade, args) carry complete code. Task 5's three dispatch branches are specified as exact mirrors of the existing Tier-3 branch (`autospec-loop.sh:1290-1335`) with the fallback commands given; the plan directs reading that template rather than repeating ~120 lines three times — acceptable because the template is a single named, quoted source in-repo. `growth-measure-due.sh` has an explicit contract + test spec.

**Type/name consistency:** actions `run-growth-define` / `service-growth-outbound` / `run-growth-measure` and seams `AUTOSPEC_GROWTH_{DEFINE,OUTBOUND,MEASURE}_CMD` and flags `--growth-{enabled,outbound-pending,backlog,backlog-floor,measure-due}` / `--tierg-dry-cycles` are used identically in Tasks 4, 5, 6. Tier numbers 5/6/7 are append-only.

## Notes for the executor

- Tasks 1–3 (Part A) are independent of Tasks 4–6 (Part B) and could be reviewed/merged first, but ship as one branch per the operator's "one combined plan" choice.
- The highest-risk edits are Tasks 4–5 (a working 1889-line conductor). The regression guard (growth-disabled → byte-identical waterfall) is the safety net — keep it green at every step.
- Confirm the exact operator sub-verb each grow skill accepts (Task 5 fallbacks) by reading `skills/autospec-grow-define/SKILL.md` and `skills/autospec-grow-run/SKILL.md`; if they take none, invoke bare.
- Any SKILL.md touch → re-derive trio + `gen-skill-goldens.sh <bare-name>` in the same task (validate fails closed otherwise; the bare-name-vs-path gotcha bit Plan 4).
