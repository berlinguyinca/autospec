#!/usr/bin/env bash
# Verifies the Phase 4 implementer's migration-replay pre-PR hook
# (Change β from docs/specs/2026-05-17-cross-session-ci-rot-design.md).
#
# Strategy: this is a prompt-text test (the hook is executed by the LLM
# following documented shell). We assert:
#   1. The v2-flow prompt's Finalize section contains the migration-replay
#      keywords and the 4 detection targets in the right order, before
#      "Run the project's test command".
#   2. The detection shell extracted from the prompt is executable and
#      picks the documented first-hit convention across fixtures:
#         (a) Makefile with `migrate-test:` target → `make migrate-test`
#         (b) package.json with `.scripts."migrate:test"` non-null →
#              `npm run migrate:test`
#         (c) executable `bin/migrate-test` → `bin/migrate-test`
#         (d) `tests/migrations/` exists → `pytest tests/migrations -x`
#         (e) None of the above → informational pass-through, exit 0.
#   3. When multiple conventions exist, detection order is 1>2>3>4.
#   4. The hook only triggers when the diff touches a migration path
#      (`*migrations/*` or `*migration*`); otherwise it is silent.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
V2_PROMPT="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -f "$V2_PROMPT" ] || fail "v2-flow prompt missing at $V2_PROMPT"

# --- 1. Keyword + ordering presence ---------------------------------------

for kw in "migration-replay" "make migrate-test" "npm run migrate:test" "bin/migrate-test" "pytest tests/migrations"; do
    grep -qF "$kw" "$V2_PROMPT" || fail "missing keyword: $kw"
done

# Detection order: make < npm < bin < pytest in document order.
make_line=$(grep -n "make migrate-test" "$V2_PROMPT"   | head -1 | cut -d: -f1)
npm_line=$(grep -n "npm run migrate:test" "$V2_PROMPT" | head -1 | cut -d: -f1)
bin_line=$(grep -n "bin/migrate-test" "$V2_PROMPT"     | head -1 | cut -d: -f1)
pyt_line=$(grep -n "pytest tests/migrations" "$V2_PROMPT" | head -1 | cut -d: -f1)
[ "$make_line" -lt "$npm_line" ] || fail "Makefile target must be listed before npm convention"
[ "$npm_line"  -lt "$bin_line" ] || fail "npm convention must be listed before bin/migrate-test"
[ "$bin_line"  -lt "$pyt_line" ] || fail "bin/migrate-test must be listed before pytest"

# Hook must appear in the Finalize section, before "Run the project's test command".
finalize_line=$(grep -n "^## Finalize" "$V2_PROMPT" | head -1 | cut -d: -f1)
project_test_line=$(grep -n "Run the project's test command" "$V2_PROMPT" | head -1 | cut -d: -f1)
[ -n "$finalize_line" ] && [ -n "$project_test_line" ] \
    || fail "Finalize / project test markers missing"
[ "$make_line" -gt "$finalize_line" ] && [ "$make_line" -lt "$project_test_line" ] \
    || fail "migration-replay hook must live in Finalize, before the project test step"

# Diff-touches-migrations gate must be documented.
grep -qE 'migrations/|\*migration\*' "$V2_PROMPT" \
    || fail "diff-touches-migrations predicate not documented"

# --- 2/3. Execute the detection shell block under fixtures ----------------

extract_detect_block() {
    # First fenced ```bash block in the v2-flow prompt that mentions
    # `migrate-test` AND `case` — that's the detection block.
    awk '
        /^```bash[[:space:]]*$/ { in_fence=1; buf=""; next }
        /^```[[:space:]]*$/ && in_fence {
            if (buf ~ /migrate-test/ && buf ~ /detect/) { print buf; exit }
            in_fence=0; buf=""; next
        }
        in_fence { buf = buf $0 "\n" }
    ' "$V2_PROMPT"
}

DETECT_BLOCK="$(extract_detect_block)"
[ -n "$DETECT_BLOCK" ] || fail "could not extract migration-replay detection shell block"

TMPDIR_DETECT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_DETECT"' EXIT

# fixture builder: builds one fake target repo per convention. The detection
# block is sourced with cwd set to the fixture; the block prints the chosen
# convention to stdout (we redirect to a marker file) without actually running
# anything (the test shouldn't invoke pytest/npm/make).
run_fixture() {
    name="$1"
    expected="$2"   # expected detected convention string, or "NONE"
    shift 2
    fix="$TMPDIR_DETECT/$name"
    mkdir -p "$fix/bin"
    # Apply per-convention setup commands passed as remaining args.
    for setup in "$@"; do eval "(cd '$fix' && $setup)"; done

    # Create stubs that record invocation rather than executing the real tool.
    # We need `make`, `npm`, `pytest`, `jq` available. `jq` should be real.
    cat > "$fix/bin/make" <<MAKE_SHIM
#!/usr/bin/env bash
echo "STUB_MAKE: \$*" > "$fix/picked"
exit 0
MAKE_SHIM
    cat > "$fix/bin/npm" <<NPM_SHIM
#!/usr/bin/env bash
echo "STUB_NPM: \$*" > "$fix/picked"
exit 0
NPM_SHIM
    cat > "$fix/bin/pytest" <<PYT_SHIM
#!/usr/bin/env bash
echo "STUB_PYTEST: \$*" > "$fix/picked"
exit 0
PYT_SHIM
    # Shim `git` so the diff-touches-migrations predicate returns a
    # migration path (the test exercises detection on the assumption the
    # caller has already determined the diff touches migrations).
    cat > "$fix/bin/git" <<GIT_SHIM
#!/usr/bin/env bash
if [ "\$1" = "diff" ]; then
    printf 'db/migrations/202605180001_add_x.sql\n'
    exit 0
fi
exit 0
GIT_SHIM
    chmod +x "$fix/bin/make" "$fix/bin/npm" "$fix/bin/pytest" "$fix/bin/git"

    # Build a runner that wires PATH and substitutes the diff-trigger so the
    # detection block always runs (the diff-touches-migrations check is the
    # caller's responsibility; this test exercises the detection path only).
    runner="$fix/run.sh"
    {
        printf '#!/usr/bin/env bash\nset +e\n'
        printf 'export FIX=%s\n' "$fix"
        printf 'export PATH=%s/bin:$PATH\n' "$fix"
        # Source the detection block in the fixture's cwd. Strip any leading
        # `if ... then` diff-predicate the prompt wraps the block in.
        # The block as documented assigns a chosen invocation string to a
        # variable (e.g. REPLAY_CMD) and either prints "running ..." or the
        # informational fallback. We override the invocation so it records
        # instead of executing.
        printf 'cd %s\n' "$fix"
        printf '%s\n' "$DETECT_BLOCK"
    } > "$runner"
    chmod +x "$runner"

    bash "$runner" > "$fix/out" 2>&1 || true

    if [ "$expected" = "NONE" ]; then
        grep -qiE "no.*migrate-test|not opted in|continuing" "$fix/out" \
            || { echo "--- out ---" >&2; cat "$fix/out" >&2;
                 fail "fixture $name: expected informational fallback, got otherwise"; }
        if [ -f "$fix/picked" ]; then
            fail "fixture $name: expected no replay invocation, but $(cat "$fix/picked") was recorded"
        fi
    else
        grep -qF "$expected" "$fix/out" \
            || { echo "--- out ---" >&2; cat "$fix/out" >&2;
                 fail "fixture $name: expected detection of '$expected'"; }
    fi
}

# (a) Makefile alone
run_fixture "make_only" "make migrate-test" \
    "printf 'migrate-test:\n\t@echo hi\n' > Makefile"

# (b) package.json alone
run_fixture "npm_only" "npm run migrate:test" \
    "printf '%s' '{\"scripts\":{\"migrate:test\":\"node t.js\"}}' > package.json"

# (c) bin/migrate-test alone
run_fixture "bin_only" "bin/migrate-test" \
    "mkdir -p bin-target && printf '#!/usr/bin/env bash\necho ok\n' > bin-target/migrate-test && chmod +x bin-target/migrate-test && mv bin-target/migrate-test bin/migrate-test"

# (d) tests/migrations alone
run_fixture "pytest_only" "pytest tests/migrations" \
    "mkdir -p tests/migrations && printf '' > tests/migrations/test_x.py"

# (e) none → informational fallback
run_fixture "none_present" "NONE"

# (f) all four present → make wins (first-hit)
run_fixture "all_make_wins" "make migrate-test" \
    "printf 'migrate-test:\n\t@echo hi\n' > Makefile" \
    "printf '%s' '{\"scripts\":{\"migrate:test\":\"node t.js\"}}' > package.json" \
    "mkdir -p bin && printf '#!/usr/bin/env bash\necho ok\n' > bin/migrate-test && chmod +x bin/migrate-test" \
    "mkdir -p tests/migrations && printf '' > tests/migrations/test_x.py"

# (g) npm + bin + pytest (no Makefile) → npm wins
run_fixture "no_make_npm_wins" "npm run migrate:test" \
    "printf '%s' '{\"scripts\":{\"migrate:test\":\"node t.js\"}}' > package.json" \
    "mkdir -p bin && printf '#!/usr/bin/env bash\necho ok\n' > bin/migrate-test && chmod +x bin/migrate-test" \
    "mkdir -p tests/migrations && printf '' > tests/migrations/test_x.py"

# (h) bin + pytest (no Makefile/npm) → bin wins
run_fixture "bin_over_pytest" "bin/migrate-test" \
    "mkdir -p bin && printf '#!/usr/bin/env bash\necho ok\n' > bin/migrate-test && chmod +x bin/migrate-test" \
    "mkdir -p tests/migrations && printf '' > tests/migrations/test_x.py"

# --- 4. Failure-path scenario: replay non-zero → comment + exit ----------
#
# Build a fixture where the chosen replay invocation exits non-zero, then
# verify the gate exits non-zero (the comment uses the `<issue-N>`/`<PR>`
# placeholders, which we substitute in the runner via a tiny gh stub).

fail_fix="$TMPDIR_DETECT/fail_path"
mkdir -p "$fail_fix/bin"
(cd "$fail_fix" && printf 'migrate-test:\n\t@echo hi\n' > Makefile)
cat > "$fail_fix/bin/make" <<'MAKE_FAIL'
#!/usr/bin/env bash
echo "boom: schema check regressed"
exit 7
MAKE_FAIL
cat > "$fail_fix/bin/gh" <<'GH_FAIL'
#!/usr/bin/env bash
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
    shift 2
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--body" ]; then shift; printf '%s' "$1" > "$FAIL_FIX/comment.body"; fi
        shift
    done
    exit 0
fi
exit 0
GH_FAIL
cat > "$fail_fix/bin/git" <<'GIT_FAIL_SHIM'
#!/usr/bin/env bash
[ "$1" = "diff" ] && { printf 'db/migrations/x.sql\n'; exit 0; }
exit 0
GIT_FAIL_SHIM
chmod +x "$fail_fix/bin/make" "$fail_fix/bin/gh" "$fail_fix/bin/git"

fail_runner="$fail_fix/run.sh"
{
    printf '#!/usr/bin/env bash\nset +e\n'
    printf 'export FAIL_FIX=%s\n' "$fail_fix"
    printf 'export PATH=%s/bin:$PATH\n' "$fail_fix"
    printf 'cd %s\n' "$fail_fix"
    # Substitute placeholders so gh issue comment <issue-N> becomes a real call.
    printf '%s\n' "$DETECT_BLOCK" | sed -e 's/<issue-N>/312/g' -e 's/<PR>/999/g'
} > "$fail_runner"
chmod +x "$fail_runner"

set +e
bash "$fail_runner" > "$fail_fix/out" 2>&1
actual_exit=$?
set -e
[ "$actual_exit" != "0" ] || { echo "--- out ---" >&2; cat "$fail_fix/out" >&2; fail "failure scenario: expected non-zero exit"; }
[ -f "$fail_fix/comment.body" ] \
    || fail "failure scenario: expected an issue comment to be posted via gh"
grep -qF "boom" "$fail_fix/comment.body" \
    || { echo "--- comment.body ---" >&2; cat "$fail_fix/comment.body" >&2;
         fail "failure scenario: expected replay output in posted comment"; }

# --- 5. No-diff scenario: diff touches no migrations → silent skip --------

skip_fix="$TMPDIR_DETECT/no_migrations_diff"
mkdir -p "$skip_fix/bin"
(cd "$skip_fix" && printf 'migrate-test:\n\t@echo hi\n' > Makefile)
cat > "$skip_fix/bin/make" <<MAKE_SHIM2
#!/usr/bin/env bash
echo "STUB_MAKE: \$*" > "$skip_fix/picked"
exit 0
MAKE_SHIM2
cat > "$skip_fix/bin/git" <<'GIT_NO_MIG'
#!/usr/bin/env bash
# No migration files in the diff.
[ "$1" = "diff" ] && { printf 'src/foo.go\n'; exit 0; }
exit 0
GIT_NO_MIG
chmod +x "$skip_fix/bin/make" "$skip_fix/bin/git"

skip_runner="$skip_fix/run.sh"
{
    printf '#!/usr/bin/env bash\nset +e\n'
    printf 'export PATH=%s/bin:$PATH\n' "$skip_fix"
    printf 'cd %s\n' "$skip_fix"
    printf '%s\n' "$DETECT_BLOCK"
} > "$skip_runner"
chmod +x "$skip_runner"

set +e
bash "$skip_runner" > "$skip_fix/out" 2>&1
set -e
[ -f "$skip_fix/picked" ] \
    && { echo "--- picked ---" >&2; cat "$skip_fix/picked" >&2;
         fail "no-diff scenario: detection should not have run when diff is non-migration"; }

echo "PASS"
