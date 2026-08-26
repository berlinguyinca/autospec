# Project Board Fleet Execution Implementation Plan (Plan B of 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn a resolved multi-repo board plan into actually-running per-repo conductors that share one budget, obey one project-level control channel, and report PR lifecycle back to the board.

**Architecture:** The board resolver (Plan A) already computes cross-repo readiness before promotion, so repo workers stay ignorant of each other. This plan supplies what is missing underneath: fleet materialization from the board, a fleet runner that actually spawns processes instead of printing them, a board-scoped spend ledger so N repos do not multiply the budget N times, control-label mirroring from one designated control issue, and the `Review`/`Testing`/`Done` half of write-back.

**Tech Stack:** Bash 3.2-compatible shell, `gh` CLI, `jq`, `yq`, bats-core.

**Spec:** `docs/specs/2026-08-25-autospec-project-board-ingestion-design.md` (Component 4, and the `Review`/`Testing`/`Done` rows of Component 5)

**Prerequisite:** Plan A (`docs/plans/2026-08-25-project-board-ingestion-engine.md`) must be complete. Tasks here consume `project-board-resolve.sh`, `project-board-writeback.sh`, and `ProjectBoardConfig`.

## Global Constraints

Every constraint from Plan A applies verbatim. Additionally:

- **One board, one budget.** A fleet of N repos must never consume N× the configured lifetime token cap. This is a correctness requirement, not a nicety.
- **Control mirroring is additive only.** Project-level labels are applied to repos that lack them and never removed from a repo that set them locally. A locally-paused repo must not be un-paused by the board.
- **Fleet spawn is idempotent.** Re-running `fleet-run` while a worker is live must not start a second conductor for the same repo. The existing repo lock and heartbeat are the authority.
- **Never mutate a tree while a background `validate.sh` runs.** Use a dedicated detached worktree and confirm the gate's own final OK line — a backgrounded `cmd | tail; echo` reports exit 0 even when the gate failed.

---

## File Structure

**Modify:**
- `scripts/project-board-resolve.sh` — add `--emit fleet-config`
- `skills/autospec-fleet/scripts/fleet-run.sh` — actually launch; spawn conductors
- `skills/autospec-fleet/scripts/fleet-lib.sh` — conductor spawn command builder
- `scripts/autonomous-spend-ledger.sh` — scope override
- `scripts/autonomous-control-channel.sh` — mirror source
- `scripts/autospec-autonomous.sh` — PR lifecycle write-back hooks

**Create:**
- `scripts/project-board-control-mirror.sh`
- `tests/fleet/project-board-fleet.bats`, `tests/autospec/spend-ledger-scope.bats`, `tests/autospec/project-board-control-mirror.bats`, `tests/integration/project-board-multirepo.bats`

---

### Task 1: `--emit fleet-config` from a board plan

**Files:**
- Modify: `scripts/project-board-resolve.sh`
- Test: `tests/autospec/project-board-resolve.bats`

**Interfaces:**
- Consumes: `.repos` from the Plan A board plan.
- Produces: `--emit fleet-config` → an `autospec-fleet.yml` document on stdout conforming to `schemas/autospec-fleet.schema.json`.

- [ ] **Step 1: Write the failing test**

```bash
cat >> tests/autospec/project-board-resolve.bats <<'BATS'

@test "fleet-config lists every board repo as an enabled entry" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  [ "$status" -eq 0 ]
  echo "$output" | yq -e '.repos | length == 6'
  echo "$output" | yq -e '.repos[0].enabled == true'
  echo "$output" | yq -e '.version == 1'
}

@test "fleet-config repo urls are clone-ready https urls" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  run bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
  echo "$output" | yq -e '.repos[] | select(.url == "https://github.com/InferWeave/inferweave-protocol.git")'
}

@test "fleet-config validates against the fleet schema" {
  stub_gh "$FIX/p1-fields.json" "$FIX/p1-items.json"
  bash "$SCRIPT" --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config > "$TMP/fleet.yml"
  run bash "${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-config-lint.sh" --config "$TMP/fleet.yml"
  [ "$status" -eq 0 ]
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/autospec/project-board-resolve.bats -f fleet-config`
Expected: FAIL — "unsupported --emit: fleet-config".

- [ ] **Step 3: Write minimal implementation**

Add to the `case "$emit"` block in `scripts/project-board-resolve.sh`:

```bash
    fleet-config)
        printf '%s\n' "$plan" | jq -r '
          "version: 1",
          "workspace: .autospec-fleet/repos",
          "parallel_repos: " + (env.AUTOSPEC_PROJECT_BOARD_PARALLEL // "2"),
          "repos:",
          (.repos[] | "  - url: https://github.com/" + . + ".git\n    enabled: true")'
        ;;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/autospec/project-board-resolve.bats`
Expected: PASS (12 tests)

- [ ] **Step 5: Commit**

```bash
git add scripts/project-board-resolve.sh tests/autospec/project-board-resolve.bats
git commit -m "feat(project-board): emit a lint-clean fleet config from a board plan"
```

---

### Task 2: Board-scoped spend ledger

The ledger is path-scoped per repo slug today, so six repos silently get six budgets. A board-scoped override makes one board equal one budget.

**Files:**
- Modify: `scripts/autonomous-spend-ledger.sh`
- Test: `tests/autospec/spend-ledger-scope.bats`

**Interfaces:**
- Produces: `AUTOSPEC_SPEND_SCOPE=<slug>` overrides the per-repo ledger directory. Unset preserves today's per-repo behavior exactly.

- [ ] **Step 1: Write the failing test**

```bash
cat > tests/autospec/spend-ledger-scope.bats <<'BATS'
#!/usr/bin/env bats
# A fleet sharing one board must share one budget, not multiply it per repo.

setup() {
  TMP="$(mktemp -d)"; export HOME="$TMP"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/autonomous-spend-ledger.sh"
  mkdir -p "$TMP/a" "$TMP/b"
  (cd "$TMP/a" && git init -q . && git remote add origin https://github.com/o/a.git)
  (cd "$TMP/b" && git init -q . && git remote add origin https://github.com/o/b.git)
}
teardown() { rm -rf "$TMP"; }

@test "without a scope override two repos keep separate ledgers" {
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 100'
}

@test "a shared scope accumulates both repos into one ledger" {
  AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run env AUTOSPEC_SPEND_SCOPE=board-inferweave-2 bash "$SCRIPT" status --repo-dir "$TMP/a"
  echo "$output" | jq -e '.tokens == 200'
}

@test "a shared scope parks both repos once the shared cap is hit" {
  AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/a" >/dev/null
  AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" add --tokens 100 --repo-dir "$TMP/b" >/dev/null
  run env AUTOSPEC_SPEND_SCOPE=board-x AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=150 \
    bash "$SCRIPT" check --repo-dir "$TMP/b"
  echo "$output" | grep -q '^park'
}

@test "a scope containing a path separator is rejected" {
  run env AUTOSPEC_SPEND_SCOPE='../../etc' bash "$SCRIPT" status --repo-dir "$TMP/a"
  [ "$status" -ne 0 ]
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/autospec/spend-ledger-scope.bats`
Expected: FAIL — the shared-scope tests each see 100, not 200.

- [ ] **Step 3: Write minimal implementation**

In `scripts/autonomous-spend-ledger.sh`, where the ledger path is derived from the repo slug, insert the override ahead of it:

```bash
# A board-scoped fleet shares one budget. Without this, N repos silently get
# N× the configured lifetime cap.
resolve_scope() {
    _scope="${AUTOSPEC_SPEND_SCOPE:-}"
    if [ -n "$_scope" ]; then
        # The scope becomes a directory name; reject traversal outright.
        case "$_scope" in
            */*|..*|"") printf 'spend-ledger: invalid AUTOSPEC_SPEND_SCOPE: %s\n' "$_scope" >&2; exit 2 ;;
        esac
        printf '%s\n' "$_scope"
        return 0
    fi
    repo_slug_for "$1"      # existing per-repo derivation, unchanged
}
```

Replace the existing slug call sites with `resolve_scope "$repo_dir"`.

- [ ] **Step 4: Run test to verify it passes**

```bash
bats tests/autospec/spend-ledger-scope.bats
bats tests/autospec/test_autonomous.bats
```
Expected: PASS — including the pre-existing ledger tests, which must be unchanged.

- [ ] **Step 5: Commit**

```bash
git add scripts/autonomous-spend-ledger.sh tests/autospec/spend-ledger-scope.bats
git commit -m "feat(autonomous): allow a shared spend-ledger scope so one board is one budget"
```

---

### Task 3: Make `fleet-run.sh` actually launch workers

Today both the dry-run and live branches `printf 'fleet: launch …'`. It is a planner that has never started a process.

**Files:**
- Modify: `skills/autospec-fleet/scripts/fleet-run.sh:135-145`, `skills/autospec-fleet/scripts/fleet-lib.sh`
- Test: `tests/fleet/project-board-fleet.bats`

**Interfaces:**
- Consumes: `autospec-fleet.yml` (Task 1).
- Produces: `fleet_worker_command <profile> <worker_id> <repo> <checkout>` in `fleet-lib.sh`, returning a conductor spawn command. `--dry-run` prints; without it, the command executes detached.

- [ ] **Step 1: Write the failing test**

```bash
mkdir -p tests/fleet
cat > tests/fleet/project-board-fleet.bats <<'BATS'
#!/usr/bin/env bats
# fleet-run must actually start workers, not print what it would have started.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin" "$TMP/ws"
  RUN="${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-run.sh"
  export FLEET_SPAWN_LOG="$TMP/spawn.log"; : > "$FLEET_SPAWN_LOG"
  # Stub the conductor so no real process is started.
  cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FLEET_SPAWN_LOG"
SH
  chmod +x "$TMP/bin/autospec-autonomous"
  cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
case "$*" in *"queue ready"*) printf '{"batch":[{"number":1}]}' ;; *) printf '' ;; esac
SH
  chmod +x "$TMP/bin/autospec"; export PATH="$TMP/bin:$PATH"
  cat > "$TMP/fleet.yml" <<'YML'
version: 1
workspace: WORKSPACE
parallel_repos: 2
repos:
  - url: https://github.com/o/a.git
    enabled: true
  - url: https://github.com/o/b.git
    enabled: true
YML
  sed -i.bak "s#WORKSPACE#$TMP/ws#" "$TMP/fleet.yml"
  mkdir -p "$TMP/ws/o/a" "$TMP/ws/o/b"
}
teardown() { rm -rf "$TMP"; }

@test "dry-run prints and starts nothing" {
  run bash "$RUN" --config "$TMP/fleet.yml" --dry-run
  [ "$status" -eq 0 ]
  [ ! -s "$FLEET_SPAWN_LOG" ]
}

@test "a live run actually spawns one worker per eligible repo" {
  run bash "$RUN" --config "$TMP/fleet.yml"
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq 2 ]
}

@test "the spawned command is a conductor, not a one-shot autospec-run" {
  run bash "$RUN" --config "$TMP/fleet.yml"
  grep -q 'start' "$FLEET_SPAWN_LOG"
  grep -q -- '--repo-dir' "$FLEET_SPAWN_LOG"
  grep -q -- '--repo o/a' "$FLEET_SPAWN_LOG"
}

@test "parallel_repos caps the number of spawned workers" {
  sed -i.bak 's/parallel_repos: 2/parallel_repos: 1/' "$TMP/fleet.yml"
  run bash "$RUN" --config "$TMP/fleet.yml"
  [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq 1 ]
}

@test "a repo with a live worker is not spawned twice" {
  bash "$RUN" --config "$TMP/fleet.yml" >/dev/null
  before="$(wc -l < "$FLEET_SPAWN_LOG")"
  bash "$RUN" --config "$TMP/fleet.yml" >/dev/null
  [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq "$before" ]
}

@test "a spawn failure quarantines that repo and continues to the next" {
  cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
case "$*" in *"o/a"*) exit 1 ;; *) printf '%s\n' "$*" >> "$FLEET_SPAWN_LOG" ;; esac
SH
  chmod +x "$TMP/bin/autospec-autonomous"
  run bash "$RUN" --config "$TMP/fleet.yml"
  [ "$status" -eq 0 ]
  grep -q 'o/b' "$FLEET_SPAWN_LOG"
  echo "$output" | grep -q 'code_health:fleet_worker_spawn_failed'
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/fleet/project-board-fleet.bats`
Expected: FAIL — the spawn log is empty on a live run; `fleet-run.sh` only prints.

- [ ] **Step 3: Write minimal implementation**

Add to `skills/autospec-fleet/scripts/fleet-lib.sh`:

```bash
# A fleet worker is a perpetual conductor, not a one-shot batch run: "ship this
# project" means keep draining until the board is done.
fleet_worker_command() {
    _profile="$1"; _worker_id="$2"; _repo="$3"; _checkout="$4"
    printf 'autospec-autonomous start --detach --repo-dir %s --repo %s' \
        "$(shell_quote "$_checkout")" "$(shell_quote "$_repo")"
}

# The repo lock and heartbeat are the liveness authority; never a PID guess.
fleet_worker_live() {
    _repo="$1"
    _hb="${HOME}/.autospec/process-heartbeats/$(printf '%s' "$_repo" | tr '/' '-')"
    [ -d "$_hb" ] && [ -n "$(ls -A "$_hb" 2>/dev/null)" ]
}
```

Replace the print-only block at `fleet-run.sh:135-145`:

```bash
    if [ "$dry_run" -eq 1 ]; then
        printf 'fleet: %s: cd %s && %s\n' "$normalized" "$checkout_path" "$command"
    elif fleet_worker_live "$normalized"; then
        printf 'fleet: %s: worker already live; skipping\n' "$normalized"
    else
        printf 'fleet: launch %s\n' "$normalized"
        # A single repo failing to start must not abort the fleet.
        if ! ( cd "$checkout_path" && eval "$command" ); then
            printf 'code_health:fleet_worker_spawn_failed repo=%s\n' "$normalized" >&2
        fi
        scheduled=$((scheduled + 1))
    fi
```

Change the `command=` assignment to use `fleet_worker_command "$profile" "$worker_id" "$normalized" "$checkout_path"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/fleet/project-board-fleet.bats && bats tests/fleet`
Expected: PASS — new tests plus the pre-existing fleet suite.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-fleet/scripts/fleet-run.sh skills/autospec-fleet/scripts/fleet-lib.sh \
        tests/fleet/project-board-fleet.bats
git commit -m "feat(fleet): launch per-repo conductors instead of printing the command"
```

---

### Task 4: Project-level control mirroring

**Files:**
- Create: `scripts/project-board-control-mirror.sh`
- Test: `tests/autospec/project-board-control-mirror.bats`

**Interfaces:**
- Consumes: `ProjectBoardConfig.control_issue` and `.repo_allowlist` (Plan A Task 7).
- Produces: `project-board-control-mirror.sh --control-issue owner/repo#N --repos a,b --allowlist 'o/*'`. Additive only. Emits `{"mirrored":[{"repo","label"}],"skipped":[...]}`.

- [ ] **Step 1: Write the failing test**

```bash
cat > tests/autospec/project-board-control-mirror.bats <<'BATS'
#!/usr/bin/env bats
# Control mirroring is ADDITIVE ONLY: a locally-paused repo must never be
# un-paused by the board.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/project-board-control-mirror.sh"
  export GH_CALLS="$TMP/gh.log"; : > "$GH_CALLS"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_CALLS"
case "$*" in
  *"issue view"*"--json labels"*) printf '%s' "${GH_CONTROL_LABELS:-[]}" ;;
  *"issue list"*)                 printf '%s' "${GH_REPO_LABELS:-[]}" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}
teardown() { rm -rf "$TMP"; }

@test "a project-level pause is mirrored into every fleet repo" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a,o/b --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored | length == 2'
  grep -q 'o/a' "$GH_CALLS"
  grep -q 'o/b' "$GH_CALLS"
}

@test "mirroring never removes a label a repo set for itself" {
  export GH_CONTROL_LABELS='[]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  ! grep -q -- '--remove-label' "$GH_CALLS"
}

@test "a control issue outside the allowlist disables mirroring" {
  export GH_CONTROL_LABELS='[{"name":"autospec:stop"}]'
  run bash "$SCRIPT" --control-issue evil/ctl#1 --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | grep -q 'code_health:project_board_repo_out_of_scope'
  ! grep -q -- '--add-label' "$GH_CALLS"
}

@test "an unset control issue is a no-op, not an error" {
  run bash "$SCRIPT" --repos o/a --allowlist 'o/*'
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.mirrored == []'
}

@test "only the four reserved labels are mirrored" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"},{"name":"bug"},{"name":"autospec:steer"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a --allowlist 'o/*'
  echo "$output" | jq -e '[.mirrored[].label] | sort == ["autospec:pause","autospec:steer"]'
  ! grep -q -- '--add-label bug' "$GH_CALLS"
}

@test "a target repo outside the allowlist is skipped" {
  export GH_CONTROL_LABELS='[{"name":"autospec:pause"}]'
  run bash "$SCRIPT" --control-issue o/ctl#1 --repos o/a,evil/b --allowlist 'o/*'
  echo "$output" | jq -e '.skipped | length == 1'
  ! grep -q 'evil/b' "$GH_CALLS"
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/autospec/project-board-control-mirror.bats`
Expected: FAIL — script does not exist.

- [ ] **Step 3: Write minimal implementation**

```bash
cat > scripts/project-board-control-mirror.sh <<'SH'
#!/usr/bin/env bash
# scripts/project-board-control-mirror.sh — mirror project-level Tier-0 control
# labels from one designated control issue into each fleet repo.
#
# ADDITIVE ONLY. A label is applied to a repo that lacks it and NEVER removed,
# so a locally-paused repo cannot be un-paused by the board.
#
# Usage:
#   project-board-control-mirror.sh [--control-issue owner/repo#N] \
#       --repos a,b --allowlist 'pat,pat'

set -eu

RESERVED="autospec:stop autospec:pause autospec:priority autospec:steer"

control=""; repos=""; allowlist=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --control-issue) control="${2:-}";   shift 2 ;;
        --repos)         repos="${2:-}";     shift 2 ;;
        --allowlist)     allowlist="${2:-}"; shift 2 ;;
        --help|-h) printf 'project-board-control-mirror.sh [--control-issue o/r#N] --repos a,b --allowlist pat\n'; exit 0 ;;
        *) printf 'project-board-control-mirror: unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
done

# Literal prefix compare, never a regex: repo names are board-controlled.
allowed() {
    _r="$1"; _old_ifs="$IFS"; IFS=','
    for _pat in $allowlist; do
        case "$_pat" in
            *\*) case "$_r" in "${_pat%\*}"*) IFS="$_old_ifs"; return 0 ;; esac ;;
            *)   if [ "$_r" = "$_pat" ]; then IFS="$_old_ifs"; return 0; fi ;;
        esac
    done
    IFS="$_old_ifs"; return 1
}

if [ -z "$control" ]; then
    printf '{"mirrored":[],"skipped":[]}\n'; exit 0
fi

ctl_repo="${control%%#*}"
ctl_num="${control##*#}"

if ! allowed "$ctl_repo"; then
    printf 'code_health:project_board_repo_out_of_scope control_issue=%s\n' "$control" >&2
    printf '{"mirrored":[],"skipped":[]}\n'; exit 0
fi

labels_json="$(gh issue view "$ctl_num" --repo "$ctl_repo" --json labels 2>/dev/null || printf '[]')"

mirrored='[]'; skipped='[]'
_old_ifs="$IFS"; IFS=','
for repo in $repos; do
    IFS="$_old_ifs"
    if ! allowed "$repo"; then
        skipped="$(printf '%s' "$skipped" | jq --arg r "$repo" '. + [{repo:$r,reason:"out-of-scope"}]')"
        IFS=','; continue
    fi
    for label in $RESERVED; do
        has="$(printf '%s' "$labels_json" | jq -r --arg l "$label" \
            'if type=="array" then . else (.labels // []) end | map(.name) | index($l) // "no"')"
        if [ "$has" != "no" ]; then
            gh issue list --repo "$repo" --label "$label" >/dev/null 2>&1 || true
            gh issue edit "$ctl_num" --repo "$repo" --add-label "$label" >/dev/null 2>&1 || true
            mirrored="$(printf '%s' "$mirrored" | jq --arg r "$repo" --arg l "$label" '. + [{repo:$r,label:$l}]')"
        fi
    done
    IFS=','
done
IFS="$_old_ifs"

printf '{"mirrored":%s,"skipped":%s}\n' "$mirrored" "$skipped"
SH
chmod +x scripts/project-board-control-mirror.sh
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/autospec/project-board-control-mirror.bats`
Expected: PASS (6 tests)

- [ ] **Step 5: Commit**

```bash
git add scripts/project-board-control-mirror.sh tests/autospec/project-board-control-mirror.bats
git commit -m "feat(project-board): mirror project-level control labels into fleet repos"
```

---

### Task 5: PR lifecycle write-back (`Review`, `Testing`, `Done`)

Plan A wired `Blocked` and `Ready` at promotion time. The remaining three states are driven by the conductor's PR lifecycle.

**Files:**
- Modify: `scripts/autospec-autonomous.sh`
- Test: `tests/autospec/project-board-writeback.bats`

**Interfaces:**
- Consumes: `project-board-writeback.sh` (Plan A Task 9).

| Conductor event | State |
|---|---|
| PR opened for the issue | `Review` |
| PR checks running | `Testing` |
| PR merged and issue closed | `Done` |

- [ ] **Step 1: Write the failing test**

```bash
cat >> tests/autospec/project-board-writeback.bats <<'BATS'

@test "an issue with an open PR maps to Review" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Review
  [ "$status" -eq 0 ]
  # Review has no option on this fixture board → skipped, never invented.
  ! grep -q 'field-create' "$GH_CALLS"
}

@test "a merged PR maps to Done and writes the Done option id" {
  run bash "$SCRIPT" --plan "$TMP/plan.json" --item PVTI_a --state Done
  [ "$status" -eq 0 ]
  grep -q 'opt_done' "$GH_CALLS"
}

@test "the lifecycle state map covers every conductor event" {
  MAP="${BATS_TEST_DIRNAME}/../../scripts/autospec-autonomous.sh"
  for s in Ready Implementation Review Testing Done Blocked; do
    grep -q "$s" "$MAP" || { echo "unmapped lifecycle state: $s"; false; }
  done
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/autospec/project-board-writeback.bats -f lifecycle`
Expected: FAIL — `autospec-autonomous.sh` mentions no lifecycle states.

- [ ] **Step 3: Write minimal implementation**

In `scripts/autospec-autonomous.sh`, add a helper and call it at the three existing lifecycle points (PR creation, CI wait entry, post-merge):

```bash
# Board write-back is advisory: never let a board mutation failure affect the
# merge path. `|| true` is the contract.
board_state() {
    _issue="$1"; _state="$2"
    _plan="${AUTOSPEC_STATE_DIR:-$HOME/.autospec}/board-cache/current.json"
    [ -f "$_plan" ] || return 0
    _item="$(jq -r --arg r "${CONDUCTOR_REPO:-}" --argjson n "$_issue" \
        '.items[] | select(.repo == $r and .number == $n) | .item_id' "$_plan" 2>/dev/null || true)"
    [ -n "$_item" ] || return 0
    bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/project-board-writeback.sh" \
        --plan "$_plan" --item "$_item" --state "$_state" >/dev/null 2>&1 || true
}
```

Call `board_state "$issue" Review` after PR creation, `board_state "$issue" Testing` before the CI wait, and `board_state "$issue" Done` after a successful merge.

- [ ] **Step 4: Run test to verify it passes**

Run: `bats tests/autospec/project-board-writeback.bats && bats tests/autospec/test_autonomous.bats`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/autospec-autonomous.sh tests/autospec/project-board-writeback.bats
git commit -m "feat(project-board): write PR lifecycle states back to the board"
```

---

### Task 6: Register Plan B scripts and run the full gate

**Files:**
- Modify: `skills/autospec-autonomous/install.sh:44`, `tests/install/project-board-install.bats`

- [ ] **Step 1: Extend the install test**

```bash
cat >> tests/install/project-board-install.bats <<'BATS'

@test "the control mirror script is registered" {
  grep -q 'project-board-control-mirror.sh' "$INSTALL"
  [ -x "$REPO/scripts/project-board-control-mirror.sh" ]
}
BATS
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats tests/install/project-board-install.bats`
Expected: FAIL — the mirror script is unregistered.

- [ ] **Step 3: Register it**

Append `project-board-control-mirror.sh` to `AUTONOMOUS_SCRIPT_FILES` at `skills/autospec-autonomous/install.sh:44`.

- [ ] **Step 4: Run the full gate against a clean baseline**

`validate.sh` is red on `main`, so compare failure **sets**, not counts. Run it in a dedicated detached worktree — mutating the tree mid-run corrupts the checkout and produces false "required file missing" errors.

```bash
git worktree add /tmp/as-baseline origin/main
( cd /tmp/as-baseline && bash scripts/validate.sh 2>&1 | tee /tmp/baseline.log | tail -5 )
bash scripts/validate.sh 2>&1 | tee /tmp/head.log | tail -5
diff <(grep -oE '^(FAIL|ERROR):.*' /tmp/baseline.log | sort) \
     <(grep -oE '^(FAIL|ERROR):.*' /tmp/head.log | sort)
git worktree remove /tmp/as-baseline
```

Expected: the diff shows **no new failures**. Confirm the gate's own final OK/summary line — a backgrounded `cmd | tail; echo` reports exit 0 even when the gate failed.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-autonomous/install.sh tests/install/project-board-install.bats
git commit -m "fix(install): ship the project-board control mirror"
```

---

### Task 7: Multi-repo end-to-end proof

The claim this whole plan exists to support: repo B does not start until repo A's blocker merges.

**Files:**
- Create: `tests/integration/project-board-multirepo.bats`

- [ ] **Step 1: Write the test**

```bash
cat > tests/integration/project-board-multirepo.bats <<'BATS'
#!/usr/bin/env bats
# The load-bearing claim: a cross-repo blocker holds repo B until repo A closes.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  S="${BATS_TEST_DIRNAME}/../../scripts"
  PROMOTE="$S/autonomous-promote-open-issues.sh"
  export AUTOSPEC_BOARD_NORMALIZE_SCRIPT="$S/project-board-normalize.sh"
  export AUTOSPEC_BOARD_DEPS_SCRIPT="$S/project-board-deps.sh"
  export AUTOSPEC_BOARD_RESOLVE_SCRIPT="$TMP/resolve.sh"
  export AUTOSPEC_PROJECT_BOARD_URL="https://github.com/orgs/o/projects/1"
  export AUTOSPEC_PROJECT_BOARD_ALLOWLIST='o/*'
  export AUTOSPEC_PROJECT_BOARD_TTL=0
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
case "$*" in *"issue list"*) printf '[]' ;; *) printf '' ;; esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
}
teardown() { rm -rf "$TMP"; }

board() {
  printf '#!/usr/bin/env bash\ncat <<%s\n%s\n%s\n' "'JSON'" "$1" "JSON" > "$TMP/resolve.sh"
  chmod +x "$TMP/resolve.sh"
}

UPSTREAM_OPEN='{"project":{},"fields":{},"repos":["o/up","o/down"],"items":[
 {"item_id":"PVTI_up","repo":"o/up","number":1,"state":"open","labels":[],"body":"Blocked by: none."},
 {"item_id":"PVTI_dn","repo":"o/down","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: o/up#1.\n"}]}'

UPSTREAM_CLOSED='{"project":{},"fields":{},"repos":["o/up","o/down"],"items":[
 {"item_id":"PVTI_up","repo":"o/up","number":1,"state":"closed","labels":[],"body":"Blocked by: none."},
 {"item_id":"PVTI_dn","repo":"o/down","number":5,"state":"open","labels":[],"body":"## Dependencies\n- Blocked by: o/up#1.\n"}]}'

@test "the upstream repo is ready first" {
  board "$UPSTREAM_OPEN"
  run bash "$PROMOTE" --repo o/up
  echo "$output" | jq -e '.board.ready == 1'
}

@test "the downstream repo is held while the upstream issue is open" {
  board "$UPSTREAM_OPEN"
  run bash "$PROMOTE" --repo o/down
  echo "$output" | jq -e '.board.ready == 0'
}

@test "the downstream repo becomes ready once the upstream issue closes" {
  board "$UPSTREAM_CLOSED"
  run bash "$PROMOTE" --repo o/down
  echo "$output" | jq -e '.board.ready == 1'
}

@test "an out-of-scope repo is never promoted even when unblocked" {
  board '{"project":{},"fields":{},"repos":["evil/x"],"items":[
   {"item_id":"PVTI_e","repo":"evil/x","number":1,"state":"open","labels":[],"body":"Blocked by: none."}]}'
  run bash "$PROMOTE" --repo evil/x
  echo "$output" | jq -e '.board.ready == 0'
  echo "$output" | jq -e '.board.out_of_scope | length == 1'
}
BATS
```

- [ ] **Step 2: Run the test**

Run: `bats tests/integration/project-board-multirepo.bats`
Expected: PASS (4 tests)

- [ ] **Step 3: Run the whole board suite together**

```bash
bats tests/autospec/project-board-*.bats tests/fleet/project-board-fleet.bats \
     tests/integration/project-board-*.bats tests/install/project-board-install.bats
```
Expected: PASS.

- [ ] **Step 4: Live dry-run against the real multi-repo board**

```bash
bash scripts/project-board-resolve.sh --url https://github.com/orgs/InferWeave/projects/1 --emit fleet-config
```
Expected: six enabled repo entries, lint-clean. Read-only.

- [ ] **Step 5: Commit**

```bash
git add tests/integration/project-board-multirepo.bats
git commit -m "test(project-board): prove cross-repo blockers gate downstream promotion"
```

---

## Self-Review

**Spec coverage (Plan B's share):**

| Spec section | Task(s) |
|---|---|
| Component 4 — fleet materialization | 1 |
| Component 4 — prerequisite: fleet-run does not launch | 3 |
| Component 4 — prerequisite: spawn conductors, not batch runs | 3 |
| Component 4 — prerequisite: shared spend ledger | 2 |
| Component 4 — project-level control channel | 4 |
| Component 5 — `Review`/`Testing`/`Done` | 5 |
| Security — allowlist on the control issue | 4 |
| Security — additive-only mirroring | 4 |
| Installer completeness | 6 |
| Testing — multi-repo cross-repo blocker | 7 |

Combined with Plan A, every spec section maps to at least one task.

**Placeholder scan:** No "TBD", no "add appropriate error handling", no "similar to Task N". Every code step is runnable.

**Type consistency:** `fleet_worker_command` / `fleet_worker_live` (Task 3) are defined in `fleet-lib.sh` and called from `fleet-run.sh` under those exact names. `AUTOSPEC_SPEND_SCOPE` (Task 2) is the same name used by the fleet spawn path. `board_state` (Task 5) consumes the `--plan/--item/--state` interface defined in Plan A Task 9. The `.board.{ready,out_of_scope}` keys asserted in Task 7 match Plan A Task 8's producer.

**Known residual risk:** Task 4's mirroring writes control labels onto the *control issue number* in each target repo, which assumes that issue number exists there. If a target repo has no such issue, the `gh issue edit` fails silently under `|| true` and the pause does not propagate. Harden this before relying on project-level pause in production — the first implementer of Task 4 should surface it as a follow-up issue rather than widening this task.
