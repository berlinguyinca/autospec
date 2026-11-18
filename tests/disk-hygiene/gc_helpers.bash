# tests/disk-hygiene/gc_helpers.bash — shared real-dependency fixtures for the
# watchdog GC tests. Real git (a hardlinked clone of this repo posing as
# berlinguyinca/autospec), real linked worktrees, the real watchdog, the real
# autospec runtime-cleanup adapter, the real hostname and the REAL gh (read-only
# calls against github.com). Scratch lives under the gitignored .test-tmp/.
#
# Stable GitHub facts the tests rely on (old, closed, never reopened):
#   issue #297   CLOSED
#   PR #303      CLOSED (unmerged), head dca761db33f1e6c3d00e73b719ad0a3c8cc494c8,
#                branch feat/297-harness-detection-classify-review
#   PR #1        MERGED, head d8cfb9d6603f896eeb06bcf016b77baff70bb98c,
#                branch improve-spec-to-roadmap-governance

ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
WATCHDOG="$ROOT/scripts/autospec-watchdog.sh"
DH="$ROOT/scripts/disk-hygiene.sh"
REAL_REPO="berlinguyinca/autospec"
CLOSED_ISSUE=297
CLOSED_PR_BRANCH="feat/297-harness-detection-classify-review"
CLOSED_PR_OID="dca761db33f1e6c3d00e73b719ad0a3c8cc494c8"
MERGED_PR_BRANCH="improve-spec-to-roadmap-governance"
MERGED_PR_OID="d8cfb9d6603f896eeb06bcf016b77baff70bb98c"

require_real_gh() {
    command -v gh >/dev/null 2>&1 || skip "gh is not installed: these tests call the real GitHub API"
    gh auth status >/dev/null 2>&1 || skip "gh is not authenticated: these tests call the real GitHub API"
    gh api "repos/$REAL_REPO" --silent >/dev/null 2>&1 || skip "GitHub API unreachable or $REAL_REPO not readable"
}

require_real_autospec() {
    command -v "${AUTOSPEC_BIN:-autospec}" >/dev/null 2>&1 || skip "autospec binary not installed (real runtime-cleanup adapter)"
}

# gc_setup [real]: scratch dir, env, and a repo whose origin URL is
# .../berlinguyinca/autospec.git so the watchdog treats its worktrees as its own.
gc_setup() {
    mkdir -p "$ROOT/.test-tmp"
    TEST_TMP="$(mktemp -d "$ROOT/.test-tmp/gc.XXXXXX")"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    unset CARGO_TARGET_DIR CARGO_INCREMENTAL AUTOSPEC_DISK_HYGIENE AUTOSPEC_DISK_CLEANUP \
          AUTOSPEC_DISK_PREFLIGHT GIT_SSH_COMMAND GIT_SSH GIT_SSH_VARIANT
    export AUTOSPEC_WATCHDOG_DIR="$TEST_TMP/heartbeats"
    export AUTOSPEC_WATCHDOG_REPO="$REAL_REPO"
    export AUTOSPEC_WATCHDOG_GC_ONLY="1"
    export AUTOSPEC_WATCHDOG_GC_DIR="$TEST_TMP/wt"
    export AUTOSPEC_HOST="$(hostname)"
    export AUTOSPEC_CONFIG_FILE="$TEST_TMP/missing-autospec.yml"
    export AUTOSPEC_HERDR_ENG="$TEST_TMP/no-such-herdr-eng"
    export AUTOSPEC_RUNTIME_WORKTREE_CLEANUP="$ROOT/scripts/autospec-runtime-worktree-cleanup.sh"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/berlinguyinca__autospec" "$AUTOSPEC_WATCHDOG_GC_DIR" "$TEST_TMP/berlinguyinca"
    ORIGIN="$TEST_TMP/berlinguyinca/autospec.git"
    PRIMARY="$TEST_TMP/primary"
    git clone -q --bare "$ROOT" "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY" 2>/dev/null
    BASE="origin/$(git -C "$PRIMARY" rev-parse --abbrev-ref HEAD)"
}

gc_teardown() { rm -rf "$TEST_TMP"; }

# make_worktree <name> <branch> [<start-point>] in the primary clone.
make_worktree() {
    local dir="$AUTOSPEC_WATCHDOG_GC_DIR/$1"
    git -C "${REPO_FOR_WT:-$PRIMARY}" worktree add -q -b "$2" "$dir" "${3:-$BASE}"
    printf '%s' "$dir"
}

commit_unpushed() { echo w > "$1/w.txt"; git -C "$1" add w.txt; git -C "$1" commit -q -m w; }

# Extract a function definition from the watchdog (the script is not sourceable).
load_watchdog_fn() {
    local f
    for f in "$@"; do eval "$(sed -n "/^$f()/,/^}/p" "$WATCHDOG")"; done
}
