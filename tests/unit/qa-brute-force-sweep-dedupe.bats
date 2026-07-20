#!/usr/bin/env bats

setup() {
    REPO_FIXTURE="$BATS_TEST_TMPDIR/repo"
    BIN_DIR="$BATS_TEST_TMPDIR/bin"
    GH_LOG="$BATS_TEST_TMPDIR/gh-mutations.log"
    GH_PWD_LOG="$BATS_TEST_TMPDIR/gh-pwd.log"
    OPEN_JSON="$BATS_TEST_TMPDIR/open.json"
    CLOSED_JSON="$BATS_TEST_TMPDIR/closed.json"
    VERDICT="$BATS_TEST_TMPDIR/verdict.json"
    CALLER_DIR="$BATS_TEST_TMPDIR/caller"
    mkdir -p "$REPO_FIXTURE/src" "$BIN_DIR" "$CALLER_DIR"
    printf '[]\n' > "$OPEN_JSON"
    printf '[]\n' > "$CLOSED_JSON"
    : > "$GH_LOG"
    : > "$GH_PWD_LOG"
    : > "$BATS_TEST_TMPDIR/gh-lookups.log"

    cat > "$REPO_FIXTURE/src/classify.py" <<'PY'
import ast

def classify(name):
    if "acid" in name:
        return 1
    if "base" in name:
        return 2
    if "salt" in name:
        return 3
    return ast.parse(name)
PY

    git -C "$REPO_FIXTURE" init -q
    git -C "$REPO_FIXTURE" config user.email test@example.com
    git -C "$REPO_FIXTURE" config user.name Test
    git -C "$REPO_FIXTURE" add src/classify.py
    git -C "$REPO_FIXTURE" commit -qm fixture
    git -C "$CALLER_DIR" init -q

    cat > "$BIN_DIR/gh" <<'SH'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$PWD" >> "$GH_PWD_LOG"
if [ "$1 $2" = "issue list" ]; then
    printf '%s\n' "$*" >> "$BATS_TEST_TMPDIR/gh-lookups.log"
    state=""
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--state" ]; then state="$2"; shift 2; else shift; fi
    done
    if [ "${GH_LIST_FAIL_STATE:-}" = "$state" ]; then exit 1; fi
    if [ "$state" = open ]; then cat "$OPEN_JSON"; else cat "$CLOSED_JSON"; fi
    exit 0
fi
if [ "$1 $2" = "label create" ]; then
    printf 'label\n' >> "$GH_LOG"
    exit 0
fi
if [ "$1 $2" = "issue create" ]; then
    shift 2
    title="" body=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --title) title="$2"; shift 2 ;;
            --body) body="$2"; shift 2 ;;
            *) shift ;;
        esac
    done
    printf 'create\n' >> "$GH_LOG"
    printf '%s\n' "$title" > "$BATS_TEST_TMPDIR/create-title"
    printf '%s\n' "$body" > "$BATS_TEST_TMPDIR/create-body"
    count="$(cat "$BATS_TEST_TMPDIR/create-count" 2>/dev/null || printf '0')"
    count=$((count + 1))
    printf '%s\n' "$count" > "$BATS_TEST_TMPDIR/create-count"
    if [ "${GH_CREATE_REMOTE_SUCCESS_ONCE:-0}" = 1 ] && [ "$count" -eq 1 ]; then
        jq --arg body "$body" '. + [{number:99,state:"OPEN",title:"created remotely",body:$body,url:"https://example.test/issues/99"}]' \
            "$OPEN_JSON" > "$OPEN_JSON.next"
        mv "$OPEN_JSON.next" "$OPEN_JSON"
        exit 1
    fi
    printf 'https://example.test/issues/99\n'
    exit 0
fi
if [ "$1 $2" = "issue comment" ]; then
    number="$3"
    shift 3
    body_file=""
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--body-file" ]; then body_file="$2"; shift 2; else shift; fi
    done
    printf 'comment:%s\n' "$number" >> "$GH_LOG"
    cp "$body_file" "$BATS_TEST_TMPDIR/recurrence-body"
    [ "${GH_COMMENT_FAIL:-0}" != 1 ]
    exit
fi
if [ "$1 $2" = "issue edit" ]; then
    number="$3"
    shift 3
    body_file=""
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--body-file" ]; then body_file="$2"; shift 2; else shift; fi
    done
    printf 'edit:%s\n' "$number" >> "$GH_LOG"
    if [ "${GH_EDIT_FAIL:-0}" = 1 ]; then exit 1; fi
    jq --argjson number "$number" --rawfile body "$body_file" \
        'map(if .number == $number then .body = $body else . end)' "$CLOSED_JSON" > "$CLOSED_JSON.next"
    mv "$CLOSED_JSON.next" "$CLOSED_JSON"
    exit 0
fi
if [ "$1 $2" = "issue reopen" ]; then
    printf 'reopen:%s\n' "$3" >> "$GH_LOG"
    [ "${GH_REOPEN_FAIL:-0}" != 1 ]
    exit
fi
exit 2
SH
    chmod +x "$BIN_DIR/gh"
    REAL_FIND="$(command -v find)"
    cat > "$BIN_DIR/find" <<'SH'
#!/usr/bin/env bash
set -eu
if [ "${FIND_DUPLICATE:-0}" = 1 ]; then
    "$REAL_FIND" "$@" | while IFS= read -r path; do printf '%s\n%s\n' "$path" "$path"; done
else
    exec "$REAL_FIND" "$@"
fi
SH
    chmod +x "$BIN_DIR/find"
    SWEEP_SCRIPT="$BATS_TEST_DIRNAME/../../scripts/qa-brute-force-sweep.sh"
    export BATS_TEST_TMPDIR REPO_FIXTURE BIN_DIR GH_LOG GH_PWD_LOG OPEN_JSON CLOSED_JSON VERDICT
    export CALLER_DIR REAL_FIND SWEEP_SCRIPT
}

marker() {
    local scope="$1" blob="$2" rule="${3:-STRING_MATCH_DOMAIN_LOGIC}"
    printf '<!-- autospec-qa-brute-force:v1 rule=%s path=src/classify.py scope=%s blob=%s -->' "$rule" "$scope" "$blob"
}

catalog() {
    local output="$1" number="$2" state="$3" marker_value="$4"
    jq -n --argjson number "$number" --arg state "$state" --arg body "$marker_value" \
        '[{number:$number,state:$state,title:"existing",body:$body,url:("https://example.test/issues/"+($number|tostring))}]' > "$output"
}

run_sweep() {
    run env PATH="$BIN_DIR:$PATH" REPO_DIR="$REPO_FIXTURE" VERDICT_FILE="$VERDICT" \
        OPEN_JSON="$OPEN_JSON" CLOSED_JSON="$CLOSED_JSON" GH_LOG="$GH_LOG" \
        GH_LIST_FAIL_STATE="${GH_LIST_FAIL_STATE:-}" GH_COMMENT_FAIL="${GH_COMMENT_FAIL:-0}" \
        GH_EDIT_FAIL="${GH_EDIT_FAIL:-0}" GH_REOPEN_FAIL="${GH_REOPEN_FAIL:-0}" \
        GH_CREATE_REMOTE_SUCCESS_ONCE="${GH_CREATE_REMOTE_SUCCESS_ONCE:-0}" \
        FIND_DUPLICATE="${FIND_DUPLICATE:-0}" REAL_FIND="$REAL_FIND" GH_PWD_LOG="$GH_PWD_LOG" \
        BATS_TEST_TMPDIR="$BATS_TEST_TMPDIR" CALLER_DIR="$CALLER_DIR" SWEEP_SCRIPT="$SWEEP_SCRIPT" \
        bash -c 'cd "$CALLER_DIR" && exec bash "$SWEEP_SCRIPT"'
}

@test "open and closed catalogs request a repository-complete practical limit" {
    run_sweep

    [ "$status" -eq 0 ]
    [ "$(grep -c -- '--limit 100000' "$BATS_TEST_TMPDIR/gh-lookups.log")" -eq 2 ]
    grep -q -- '--state open .*--limit 100000' "$BATS_TEST_TMPDIR/gh-lookups.log"
    grep -q -- '--state closed .*--limit 100000' "$BATS_TEST_TMPDIR/gh-lookups.log"
}

@test "exact open marker suppresses every GitHub mutation and records repo-relative identity" {
    blob="$(git -C "$REPO_FIXTURE" hash-object src/classify.py)"
    catalog "$OPEN_JSON" 41 OPEN "$(marker '<file>' "$blob")"

    run_sweep

    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"file":"src/classify.py"' "$VERDICT"
    grep -q '"scope":"<file>"' "$VERDICT"
    grep -q '"filing_status":"existing-open"' "$VERDICT"
    ! grep -q '/tmp/' "$VERDICT"
}

@test "exact unchanged closed marker is not reopened or replaced" {
    blob="$(git -C "$REPO_FIXTURE" hash-object src/classify.py)"
    catalog "$CLOSED_JSON" 42 CLOSED "$(marker '<file>' "$blob")"

    run_sweep

    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"filing_status":"existing-closed"' "$VERDICT"
}

@test "changed blob comments recurrence evidence then reopens the same closed issue" {
    catalog "$CLOSED_JSON" 43 CLOSED "$(marker '<file>' 0000000000000000000000000000000000000000)"

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(cat "$GH_LOG")" = $'comment:43\nedit:43\nreopen:43' ]
    grep -q 'Previous blob: `0000000000000000000000000000000000000000`' "$BATS_TEST_TMPDIR/recurrence-body"
    grep -q 'Current blob:' "$BATS_TEST_TMPDIR/recurrence-body"
    grep -q '"filing_status":"reopened"' "$VERDICT"

    : > "$GH_LOG"
    : > "$VERDICT"
    run_sweep
    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"filing_status":"existing-closed"' "$VERDICT"
}

@test "recurrence mutation failure never creates a replacement issue" {
    catalog "$CLOSED_JSON" 44 CLOSED "$(marker '<file>' 0000000000000000000000000000000000000000)"
    export GH_COMMENT_FAIL=1

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(cat "$GH_LOG")" = 'comment:44' ]
    grep -q '"filing_status":"not-filed-comment-failed"' "$VERDICT"

    : > "$GH_LOG"
    : > "$VERDICT"
    export GH_COMMENT_FAIL=0
    export GH_EDIT_FAIL=1

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(cat "$GH_LOG")" = $'comment:44\nedit:44' ]
    ! grep -q '^create$' "$GH_LOG"
    grep -q '"filing_status":"not-filed-edit-failed"' "$VERDICT"

    : > "$GH_LOG"
    : > "$VERDICT"
    export GH_EDIT_FAIL=0
    export GH_REOPEN_FAIL=1

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(cat "$GH_LOG")" = $'comment:44\nedit:44\nreopen:44' ]
    ! grep -q '^create$' "$GH_LOG"
    grep -q '"filing_status":"not-filed-reopen-failed"' "$VERDICT"

    : > "$GH_LOG"
    : > "$VERDICT"
    export GH_REOPEN_FAIL=0
    run_sweep
    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"filing_status":"existing-closed"' "$VERDICT"
}

@test "every GitHub call executes from REPO_DIR even when caller is another repository" {
    run_sweep

    [ "$status" -eq 0 ]
    [ -s "$GH_PWD_LOG" ]
    [ "$(sort -u "$GH_PWD_LOG")" = "$REPO_FIXTURE" ]
}

@test "same-run exact marker ledger suppresses duplicate scanner output" {
    export FIND_DUPLICATE=1

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(grep -c '^create$' "$GH_LOG")" -eq 1 ]
    grep -q '"filing_status":"existing-run"' "$VERDICT"
}

@test "shared common-dir lock contention fails closed without GitHub mutations" {
    mkdir "$REPO_FIXTURE/.git/autospec-qa-brute-force.lock"

    run_sweep

    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    [ ! -s "$GH_PWD_LOG" ]
    grep -q '"filing_status":"not-filed-lock"' "$VERDICT"
}

@test "ambiguous create refresh finds remote success and never retries" {
    export GH_CREATE_REMOTE_SUCCESS_ONCE=1

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(grep -c '^create$' "$GH_LOG")" -eq 1 ]
    grep -q '"filing_status":"existing-open-after-create"' "$VERDICT"
}

@test "malformed or failed catalog lookup fails closed before all mutations" {
    printf '{bad json\n' > "$OPEN_JSON"

    run_sweep

    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"filing_status":"not-filed-catalog"' "$VERDICT"

    : > "$VERDICT"
    export GH_LIST_FAIL_STATE=closed
    printf '[]\n' > "$OPEN_JSON"
    run_sweep
    [ "$status" -eq 0 ]
    [ ! -s "$GH_LOG" ]
    grep -q '"filing_status":"not-filed-catalog"' "$VERDICT"
}

@test "first-function churn keeps STRING_MATCH_DOMAIN_LOGIC at file scope" {
    old_blob="$(git -C "$REPO_FIXTURE" hash-object src/classify.py)"
    catalog "$CLOSED_JSON" 45 CLOSED "$(marker '<file>' "$old_blob")"
    sed -i '3i def helper():\n    return 0\n' "$REPO_FIXTURE/src/classify.py"

    run_sweep

    [ "$status" -eq 0 ]
    [ "$(cat "$GH_LOG")" = $'comment:45\nedit:45\nreopen:45' ]
    grep -q '"scope":"<file>"' "$VERDICT"
    ! grep -q '"scope":"helper"' "$VERDICT"
}

@test "a distinct signature creates an origin:self issue with marker and no absolute path" {
    run_sweep

    [ "$status" -eq 0 ]
    grep -q '^label$' "$GH_LOG"
    grep -q '^create$' "$GH_LOG"
    grep -q 'autospec-qa-brute-force:v1 rule=STRING_MATCH_DOMAIN_LOGIC path=src/classify.py scope=<file> blob=' "$BATS_TEST_TMPDIR/create-body"
    ! grep -q '/tmp/' "$BATS_TEST_TMPDIR/create-title"
    ! grep -q '/tmp/' "$BATS_TEST_TMPDIR/create-body"
    grep -q '"filing_status":"created"' "$VERDICT"
}

@test "REPEATED_STRUCTURE_AS_CODE identity uses the detected function scope" {
    rm "$REPO_FIXTURE/src/classify.py"
    cat > "$REPO_FIXTURE/src/repeated.py" <<'PY'
def dispatch(name):
    if "one" in name:
        return 1
    elif "two" in name:
        return 2
    elif "three" in name:
        return 3
    elif "four" in name:
        return 4
    elif "five" in name:
        return 5
PY
    git -C "$REPO_FIXTURE" add src/repeated.py

    run_sweep

    [ "$status" -eq 0 ]
    grep -q 'autospec-qa-brute-force:v1 rule=REPEATED_STRUCTURE_AS_CODE path=src/repeated.py scope=dispatch blob=' "$BATS_TEST_TMPDIR/create-body"
    grep -q '"rule_id":"REPEATED_STRUCTURE_AS_CODE".*"scope":"dispatch"' "$VERDICT"
}
