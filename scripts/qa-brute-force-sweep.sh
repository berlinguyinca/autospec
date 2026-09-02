#!/usr/bin/env bash
# scripts/qa-brute-force-sweep.sh
#
# autospec-qa brute-force string-heuristics sweep (issue #637, refined #640).
#
# Scans REPO_DIR for already-merged code that matches either of the two
# LLM-tier RULE_IDs:
#   - STRING_MATCH_DOMAIN_LOGIC      (substring-on-name encoding domain
#                                     meaning while a proper-rep library
#                                     is imported in the same file)
#   - REPEATED_STRUCTURE_AS_CODE     (>=5 branches in ONE function/method
#                                     sharing identical structural shape)
#
# Supported languages: Python, JavaScript/TypeScript, Go, Java, Scala, Rust.
#
# REPEATED_STRUCTURE_AS_CODE is scoped per-function (issue #640): we parse
# function boundaries with a cheap per-language regex, count same-shape
# branches inside each range, and emit one finding per offending function.
#
# For each offender we:
#   1. Append a finding to $VERDICT_FILE (qa-verdict.json) under the
#      category `code_health:brute_force_string_heuristics`.
#   2. File one GitHub issue via `gh issue create` carrying the verbatim
#      RULE_ID directive plus file/function/line so the implementer
#      retry-loop has the corrective instruction.
#
# This is a coarse heuristic scan — intentional, because the contract is
# LLM-tier semantic detection at PR time; this sweep is the "rust on already
# merged code" catcher. Errors here MUST NOT block QA; we emit findings and
# continue.

set -eu

REPO_DIR="${REPO_DIR:-$(pwd)}"
REPO_DIR="$(cd "$REPO_DIR" && pwd -P)"
VERDICT_FILE="${VERDICT_FILE:-$REPO_DIR/.autospec/qa-verdict.json}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
project_sync_issue() {
    local helper="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR/../skills/autospec-shared/scripts}/project-sync-issue.sh"
    bash "$helper" "$1" "$REPO_DIR"
}

project_sync_catalog_issue() {
    local catalog="$1" number="$2" issue_url
    issue_url="$(jq -r --argjson number "$number" '.[] | select(.number == $number) | .url // empty' "$catalog" | head -n 1)"
    [ -n "$issue_url" ] && project_sync_issue "$issue_url"
}

mkdir -p "$(dirname "$VERDICT_FILE")"

SWEEP_TMP="$(mktemp -d)"
OPEN_ISSUES="$SWEEP_TMP/open-issues.json"
CLOSED_ISSUES="$SWEEP_TMP/closed-issues.json"
MARKER_LEDGER="$SWEEP_TMP/markers"
CATALOG_STATUS="not-loaded"
LOCK_DIR=""
LOCK_HELD=0
: > "$MARKER_LEDGER"

cleanup() {
    if [ "$LOCK_HELD" -eq 1 ]; then
        rmdir "$LOCK_DIR" >/dev/null 2>&1 || true
        LOCK_HELD=0
    fi
    rm -rf "$SWEEP_TMP"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

# Directive map — must stay byte-identical to AGENTS.md ### Corrective
# directive map entries for these two RULE_IDs. The implementer retry loop
# reads the body of the filed issue and feeds the directive into the next
# implementer prompt; divergence here silently breaks the rewrite loop.
DIRECTIVE_STRING_MATCH='Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case.'
DIRECTIVE_REPEATED_STRUCTURE='Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a Map/sealed-trait registry; in Rust use a &[(predicate, value)] slice; in Go use a []struct{...} table. Each new entry should be one row, not a ~10-line block.'

# origin:self provenance (issue #1744): idempotent, best-effort label
ensure_origin_self_label() {
    repo_gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
}

repo_gh() {
    (cd "$REPO_DIR" && gh "$@")
}

acquire_sweep_lock() {
    local common_dir
    if ! common_dir="$(git -C "$REPO_DIR" rev-parse --git-common-dir 2>/dev/null)"; then
        return 1
    fi
    case "$common_dir" in
        /*) ;;
        *) common_dir="$REPO_DIR/$common_dir" ;;
    esac
    if ! common_dir="$(cd "$common_dir" 2>/dev/null && pwd -P)"; then
        return 1
    fi
    LOCK_DIR="$common_dir/autospec-qa-brute-force.lock"
    if ! mkdir "$LOCK_DIR" 2>/dev/null; then
        return 1
    fi
    LOCK_HELD=1
}

emit_finding() {
    local file="$1" lang="$2" rule_id="$3" line="$4" scope="$5" blob="$6" filing_status="$7" marker="$8"
    jq -cn \
        --arg category "code_health:brute_force_string_heuristics" \
        --arg rule_id "$rule_id" \
        --arg language "$lang" \
        --arg file "$file" \
        --arg scope "$scope" \
        --arg blob "$blob" \
        --arg filing_status "$filing_status" \
        --arg marker "$marker" \
        --argjson line "$line" \
        '{category:$category,rule_id:$rule_id,language:$language,file:$file,function:$scope,scope:$scope,line:$line,blob:$blob,filing_status:$filing_status,marker:$marker}' \
        >> "$VERDICT_FILE"
}

relative_repo_path() {
    local file="$1"
    local physical
    physical="$(cd "$(dirname "$file")" && pwd -P)/$(basename "$file")"
    case "$physical" in
        "$REPO_DIR"/*) printf '%s\n' "${physical#"$REPO_DIR"/}" ;;
        *) return 1 ;;
    esac
}

load_issue_catalogs() {
    local open_ok=1 closed_ok=1
    repo_gh issue list --state open --limit 100000 --json number,state,title,body,url > "$OPEN_ISSUES" 2>/dev/null || open_ok=0
    repo_gh issue list --state closed --limit 100000 --json number,state,title,body,url > "$CLOSED_ISSUES" 2>/dev/null || closed_ok=0
    if [ "$open_ok" -ne 1 ] || [ "$closed_ok" -ne 1 ] || \
       ! jq -e 'type == "array" and all(.[]; (.number | type == "number") and (.body | type == "string"))' "$OPEN_ISSUES" >/dev/null 2>&1 || \
       ! jq -e 'type == "array" and all(.[]; (.number | type == "number") and (.body | type == "string"))' "$CLOSED_ISSUES" >/dev/null 2>&1; then
        CATALOG_STATUS="failed"
        printf 'WARN: brute-force issue catalog unavailable or malformed; findings will not mutate GitHub\n' >&2
        return 0
    fi
    CATALOG_STATUS="ready"
}

refresh_open_catalog() {
    local refreshed="$SWEEP_TMP/open-refreshed.json"
    if ! repo_gh issue list --state open --limit 100000 --json number,state,title,body,url > "$refreshed" 2>/dev/null || \
       ! jq -e 'type == "array" and all(.[]; (.number | type == "number") and (.body | type == "string"))' "$refreshed" >/dev/null 2>&1; then
        return 1
    fi
    mv "$refreshed" "$OPEN_ISSUES"
}

exact_issue_number() {
    local catalog="$1" marker="$2"
    jq -r --arg marker "$marker" \
        '[.[] | select(.body | split("\n") | index($marker)) | .number][0] // empty' "$catalog"
}

semantic_issue_match() {
    local catalog="$1" prefix="$2"
    jq -r --arg prefix "$prefix" '
        [.[] as $issue
         | ($issue.body | split("\n")[] | select(startswith($prefix) and endswith(" -->"))) as $marker
         | [$issue.number, $marker] | @tsv][0] // empty
    ' "$catalog"
}

existing_issue_status() {
    local marker="$1" exact_number
    exact_number="$(exact_issue_number "$OPEN_ISSUES" "$marker")"
    if [ -n "$exact_number" ]; then printf '%s\n' "existing-open"; return 0; fi
    exact_number="$(exact_issue_number "$CLOSED_ISSUES" "$marker")"
    if [ -n "$exact_number" ]; then printf '%s\n' "existing-closed"; return 0; fi
    return 1
}

pending_issue_match() {
    local pending_prefix="$1" match
    match="$(jq -r --arg prefix "$pending_prefix" --arg state "open" '
        [.[] as $issue
         | select(any($issue.body | split("\n")[]; startswith($prefix) and endswith(" -->")))
         | [$state, ($issue.number | tostring)] | @tsv][0] // empty' "$OPEN_ISSUES")"
    if [ -n "$match" ]; then printf '%s\n' "$match"; return 0; fi
    match="$(jq -r --arg prefix "$pending_prefix" --arg state "closed" '
        [.[] as $issue
         | select(any($issue.body | split("\n")[]; startswith($prefix) and endswith(" -->")))
         | [$state, ($issue.number | tostring)] | @tsv][0] // empty' "$CLOSED_ISSUES")"
    if [ -n "$match" ]; then printf '%s\n' "$match"; return 0; fi
    return 1
}

write_recurrence_body() {
    local catalog="$1" number="$2" semantic_prefix="$3" pending_prefix="$4"
    local marker="$5" pending_marker="$6" output="$7"
    jq -r --argjson number "$number" --arg marker_prefix "$semantic_prefix" \
        --arg pending_prefix "$pending_prefix" --arg marker "$marker" --arg pending "$pending_marker" '
        .[] | select(.number == $number) | .body | split("\n")
        | map(select((startswith($marker_prefix) or startswith($pending_prefix)) | not))
        | . + ["", $marker, $pending] | join("\n")
    ' "$catalog" > "$output"
}

remove_pending_line() {
    local source="$1" pending_prefix="$2" output="$3"
    jq -Rrs --arg pending_prefix "$pending_prefix" \
        'split("\n") | map(select(startswith($pending_prefix) | not)) | join("\n")' "$source" > "$output"
}

resume_pending() {
    local pending_match="$1" pending_prefix="$2"
    local state number catalog source cleaned
    state="${pending_match%%$'\t'*}"
    number="${pending_match#*$'\t'}"
    if [ "$state" = "open" ]; then catalog="$OPEN_ISSUES"; else catalog="$CLOSED_ISSUES"; fi
    if [ "$state" = "closed" ] && ! repo_gh issue reopen "$number" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-reopen-failed"; return 0
    fi
    source="$(mktemp "$SWEEP_TMP/pending-body.XXXXXX")"
    cleaned="$(mktemp "$SWEEP_TMP/clean-body.XXXXXX")"
    jq -r --argjson number "$number" '.[] | select(.number == $number) | .body' "$catalog" > "$source"
    remove_pending_line "$source" "$pending_prefix" "$cleaned"
    if ! repo_gh issue edit "$number" --body-file "$cleaned" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-cleanup-failed"; return 0
    fi
    project_sync_catalog_issue "$catalog" "$number"
    printf '%s\n' "pending-recovered"
}

comment_recurrence() {
    local issue_number="$1" old_marker="$2" blob="$3" marker="$4"
    local old_blob recurrence_file
    old_blob="${old_marker##* blob=}"
    old_blob="${old_blob% -->}"
    recurrence_file="$(mktemp "$SWEEP_TMP/recurrence.XXXXXX")"
    printf 'The same brute-force heuristic recurred at a new Git blob.\n\nPrevious blob: `%s`\nCurrent blob: `%s`\n\n%s\n' \
        "$old_blob" "$blob" "$marker" > "$recurrence_file"
    repo_gh issue comment "$issue_number" --body-file "$recurrence_file" >/dev/null 2>&1
}

handle_recurrence() {
    local semantic_match="$1" marker="$2" blob="$3" semantic_prefix="$4"
    local pending_prefix="$5" pending_marker="$6"
    local issue_number old_marker state_body clean_body
    issue_number="${semantic_match%%$'\t'*}"
    old_marker="${semantic_match#*$'\t'}"
    if ! comment_recurrence "$issue_number" "$old_marker" "$blob" "$marker"; then
        printf '%s\n' "not-filed-comment-failed"; return 0
    fi
    state_body="$(mktemp "$SWEEP_TMP/issue-body.XXXXXX")"
    clean_body="$(mktemp "$SWEEP_TMP/clean-body.XXXXXX")"
    write_recurrence_body "$CLOSED_ISSUES" "$issue_number" "$semantic_prefix" "$pending_prefix" \
        "$marker" "$pending_marker" "$state_body"
    if ! repo_gh issue edit "$issue_number" --body-file "$state_body" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-edit-failed"; return 0
    fi
    if ! repo_gh issue reopen "$issue_number" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-reopen-failed"; return 0
    fi
    remove_pending_line "$state_body" "$pending_prefix" "$clean_body"
    if ! repo_gh issue edit "$issue_number" --body-file "$clean_body" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-cleanup-failed"; return 0
    fi
    project_sync_catalog_issue "$CLOSED_ISSUES" "$issue_number"
    printf '%s\n' "reopened"
}

handle_open_recurrence() {
    local semantic_match="$1" marker="$2" blob="$3" semantic_prefix="$4"
    local pending_prefix="$5" pending_marker="$6"
    local issue_number old_marker state_body clean_body
    issue_number="${semantic_match%%$'\t'*}"
    old_marker="${semantic_match#*$'\t'}"
    if ! comment_recurrence "$issue_number" "$old_marker" "$blob" "$marker"; then
        printf '%s\n' "not-filed-comment-failed"; return 0
    fi
    state_body="$(mktemp "$SWEEP_TMP/open-body.XXXXXX")"
    clean_body="$(mktemp "$SWEEP_TMP/clean-body.XXXXXX")"
    write_recurrence_body "$OPEN_ISSUES" "$issue_number" "$semantic_prefix" "$pending_prefix" \
        "$marker" "$pending_marker" "$state_body"
    if ! repo_gh issue edit "$issue_number" --body-file "$state_body" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-edit-failed"; return 0
    fi
    remove_pending_line "$state_body" "$pending_prefix" "$clean_body"
    if ! repo_gh issue edit "$issue_number" --body-file "$clean_body" >/dev/null 2>&1; then
        printf '%s\n' "not-filed-cleanup-failed"; return 0
    fi
    project_sync_catalog_issue "$OPEN_ISSUES" "$issue_number"
    printf '%s\n' "updated-open"
}

create_or_recheck_issue() {
    local title="$1" body="$2" marker="$3" exact_number issue_url
    ensure_origin_self_label
    if issue_url="$(repo_gh issue create --title "$title" --body "$body" \
        --label "auto-implement,autospec:v2-flow" --label origin:self 2>/dev/null)"; then
        project_sync_issue "$issue_url"
        printf '%s\n' "created"; return 0
    fi
    if ! refresh_open_catalog; then
        printf '%s\n' "not-filed-create-refresh-failed"; return 0
    fi
    exact_number="$(exact_issue_number "$OPEN_ISSUES" "$marker")"
    if [ -n "$exact_number" ]; then
        project_sync_catalog_issue "$OPEN_ISSUES" "$exact_number"
        printf '%s\n' "existing-open-after-create"; return 0
    fi
    if issue_url="$(repo_gh issue create --title "$title" --body "$body" \
        --label "auto-implement,autospec:v2-flow" --label origin:self 2>/dev/null)"; then
        project_sync_issue "$issue_url"
        printf '%s\n' "created"
    else
        printf '%s\n' "not-filed-create-failed"
    fi
}

file_issue() {
    local file="$1" lang="$2" rule_id="$3" line="$4" scope="$5" blob="$6" directive="$7" marker="$8"
    local title body existing_status semantic_prefix semantic_match open_match
    local pending_prefix pending_marker pending_match pending_status
    if [ "$CATALOG_STATUS" = "lock-failed" ]; then
        printf '%s\n' "not-filed-lock"; return 0
    fi
    if [ "$CATALOG_STATUS" != "ready" ]; then
        printf '%s\n' "not-filed-catalog"; return 0
    fi
    if grep -Fqx -- "$marker" "$MARKER_LEDGER"; then
        printf '%s\n' "existing-run"; return 0
    fi
    printf '%s\n' "$marker" >> "$MARKER_LEDGER"
    pending_prefix="<!-- autospec-qa-brute-force:pending-reopen:v1 rule=$rule_id path=$file scope=$scope blob="
    pending_marker="$pending_prefix$blob -->"
    if pending_match="$(pending_issue_match "$pending_prefix")"; then
        pending_status="$(resume_pending "$pending_match" "$pending_prefix")"
        if [ "$pending_status" != "pending-recovered" ]; then printf '%s\n' "$pending_status"; return 0; fi
        load_issue_catalogs
        if [ "$CATALOG_STATUS" != "ready" ]; then printf '%s\n' "not-filed-catalog"; return 0; fi
    fi
    if existing_status="$(existing_issue_status "$marker")"; then
        printf '%s\n' "$existing_status"; return 0
    fi
    semantic_prefix="<!-- autospec-qa-brute-force:v1 rule=$rule_id path=$file scope=$scope blob="
    open_match="$(semantic_issue_match "$OPEN_ISSUES" "$semantic_prefix")"
    if [ -n "$open_match" ]; then
        handle_open_recurrence "$open_match" "$marker" "$blob" "$semantic_prefix" \
            "$pending_prefix" "$pending_marker"; return 0
    fi
    semantic_match="$(semantic_issue_match "$CLOSED_ISSUES" "$semantic_prefix")"
    if [ -n "$semantic_match" ]; then
        handle_recurrence "$semantic_match" "$marker" "$blob" "$semantic_prefix" \
            "$pending_prefix" "$pending_marker"; return 0
    fi
    title="code_health: rewrite brute-force string heuristics in $file ($rule_id)"
    body=$(printf '%s\n\nDetected %s in `%s` (%s)\n\nFunction/method: `%s`\nLine: %s\nGit blob: `%s`\n\nDirective (verbatim from AGENTS.md):\n\n> %s\n\nLanguage: %s\n' \
        "$marker" "$rule_id" "$file" "$lang" "$scope" "$line" "$blob" "$directive" "$lang")
    create_or_recheck_issue "$title" "$body" "$marker"
}

process_finding() {
    local file="$1" lang="$2" rule_id="$3" line="$4" scope="$5" directive="$6"
    local repo_file blob marker filing_status
    if ! repo_file="$(relative_repo_path "$file")"; then
        return 0
    fi
    if ! blob="$(git hash-object -- "$file" 2>/dev/null)"; then
        emit_finding "$repo_file" "$lang" "$rule_id" "$line" "$scope" "" "not-filed-blob" ""
        return 0
    fi
    marker="<!-- autospec-qa-brute-force:v1 rule=$rule_id path=$repo_file scope=$scope blob=$blob -->"
    filing_status="$(file_issue "$repo_file" "$lang" "$rule_id" "$line" "$scope" "$blob" "$directive" "$marker")"
    emit_finding "$repo_file" "$lang" "$rule_id" "$line" "$scope" "$blob" "$filing_status" "$marker"
}

# Returns 0 if file contains a proper-rep library import for its language.
has_proper_rep_library() {
    local file="$1" lang="$2"
    case "$lang" in
        python)     grep -qE '^(from |import )(rdkit|ast|urllib\.parse|datetime|ipaddress|lxml|jsonschema)' "$file" ;;
        javascript) grep -qE '\b(URL|Date|@babel/parser|acorn|ts-morph|zod|ajv|joi)\b' "$file" ;;
        go)         grep -qE '"(net/url|time|go/ast|encoding/json)"|net\.ParseIP' "$file" ;;
        java)       grep -qE '\b(java\.net\.URI|java\.time|JavaParser|com\.github\.javaparser|javax\.validation)\b' "$file" ;;
        scala)      grep -qE '\b(java\.net\.URI|java\.time|scala\.meta|scalameta|refined|circe)\b' "$file" ;;
        rust)       grep -qE '\b(url::Url|chrono|::time|syn|std::net::IpAddr|serde)\b' "$file" ;;
        *)          return 1 ;;
    esac
}

# Emit numbered substring-style candidate lines after removing evidence that
# only verifies or reports output. STRING_MATCH_DOMAIN_LOGIC remains a
# file-scope heuristic — the proper-rep-library import already scopes it.
substring_candidate_lines() {
    local file="$1" lang="$2"
    local pattern candidate_re
    case "$lang" in
        python)     pattern='\bin (name|s|x|input|text|target)\b'; candidate_re='[[:space:]]in[[:space:]]+(name|s|x|input|text|target)([^[:alnum:]_]|$)' ;;
        javascript) pattern='\.(includes|indexOf|startsWith|endsWith)\('; candidate_re='[.](includes|indexof|startswith|endswith)[[:space:]]*[(]' ;;
        go)         pattern='\bcontains\(.*"[^"]+"\)|strings\.Contains'; candidate_re='(^|[^[:alnum:]_])(strings[.])?contains[[:space:]]*[(]' ;;
        java)       pattern='\.contains\("[^"]+"\)'; candidate_re='[.]contains[[:space:]]*[(]' ;;
        scala)      pattern='\.contains\("[^"]+"\)'; candidate_re='[.]contains[[:space:]]*[(]' ;;
        rust)       pattern='\.contains\("[^"]+"\)'; candidate_re='[.]contains[[:space:]]*[(]' ;;
        *)          return 0 ;;
    esac
    grep -nE "$pattern" "$file" 2>/dev/null | awk -v candidate_re="$candidate_re" '
        {
            text = $0
            sub(/^[0-9]+:/, "", text)
            lower = tolower(text)
            trimmed = lower
            sub(/^[[:space:]]+/, "", trimmed)

            if (trimmed ~ /^(#|\/\/|\/\*|\*)/) next
            candidate_at = match(lower, candidate_re)
            owner = candidate_at ? substr(lower, 1, candidate_at - 1) : lower
            sub(/^.*;/, "", owner)

            if (owner ~ /(^|[^[:alnum:]_])assert[[:space:]]+/) next
            if (owner ~ /(^|[^[:alnum:]_])([[:alnum:]_]*assert[[:alnum:]_]*|expect[[:alnum:]_]*|[[:alnum:]_]*snapshot[[:alnum:]_]*)([[:space:]]*\(|[[:space:]]*!)/) next
            if (owner ~ /(^|[^[:alnum:]_])(print|println|eprint|eprintln|debug|info|warn|error|trace)!?[[:space:]]*\(/) next
            if (owner ~ /(console|log|logger)\.(log|debug|info|warn|error|trace)[[:space:]]*\(/) next
            if (owner ~ /system\.(out|err)\.print(ln)?[[:space:]]*\(/) next

            print $0
        }
    ' || true
}

# ---- per-function range parsing (issue #640) ----
#
# Emit "start_line end_line name" rows on stdout, one per function in $file.
# Cheap regexes — intentional, see header note.

function_ranges_python() {
    # Python: function range = def line through (last line at deeper indent
    # before another def at <= same indent OR EOF). We use awk to track
    # indentation.
    local file="$1"
    awk '
        function flush(   i) {
            if (start_line) {
                print start_line, last_line, fname
            }
            start_line = 0
        }
        /^[[:space:]]*def[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/ {
            # measure indent of this def
            match($0, /^[[:space:]]*/)
            this_indent = RLENGTH
            if (start_line && this_indent <= def_indent) flush()
            # extract name
            line = $0
            sub(/^[[:space:]]*def[[:space:]]+/, "", line)
            sub(/[(:].*$/, "", line)
            fname = line
            start_line = NR
            def_indent = this_indent
            last_line = NR
            next
        }
        {
            if (start_line && NF > 0) {
                match($0, /^[[:space:]]*/)
                if (RLENGTH > def_indent) last_line = NR
                else if (NF > 0) flush()
            }
        }
        END { flush() }
    ' "$file"
}

function_ranges_brace() {
    # Generic brace-language range parser used by JS/TS, Go, Java, Scala, Rust.
    # Finds function/method signature lines per language via `awk` keyword
    # detection (BSD-awk compatible — no ERE capture groups), then walks
    # `{`/`}` to find the matching close brace.
    local file="$1" lang="$2"

    awk -v lang="$lang" -v sq="'" '
        # Strip string and char literals from a line so `{`/`}` inside them
        # are never counted as real braces (issue #3471). Handles escaped
        # quotes (\"), single-char literals (e.g. sq{sq), Rust raw strings
        # (r"...", r#"..."#, r##"..."#... with any hash count), and leaves
        # Rust lifetime apostrophes (e.g. &sq a str) untouched since they
        # have no closing quote. sq holds a literal single-quote character
        # passed in via -v so the awk source never has to embed one.
        # Single-line only, matching the existing `//` comment strip scope —
        # no cross-line string state.
        function strip_strings(s,   out, i, n, c, c1, c2, j, k, hashes, closer, rest, pos, found) {
            out = ""
            n = length(s)
            i = 1
            while (i <= n) {
                c = substr(s, i, 1)
                if (c == "r" && (i == 1 || substr(s, i - 1, 1) !~ /[A-Za-z0-9_]/)) {
                    j = i + 1
                    hashes = 0
                    while (substr(s, j, 1) == "#") { hashes++; j++ }
                    if (substr(s, j, 1) == "\"") {
                        closer = "\""
                        for (k = 0; k < hashes; k++) closer = closer "#"
                        rest = substr(s, j + 1)
                        pos = index(rest, closer)
                        if (pos > 0) i = j + pos + length(closer)
                        else i = n + 1
                        continue
                    }
                }
                if (c == "\"") {
                    j = i + 1
                    found = 0
                    while (j <= n) {
                        c2 = substr(s, j, 1)
                        if (c2 == "\\") { j += 2; continue }
                        if (c2 == "\"") { found = 1; j++; break }
                        j++
                    }
                    i = found ? j : n + 1
                    continue
                }
                if (c == sq) {
                    c1 = substr(s, i + 1, 1)
                    if (c1 == "\\") {
                        found = 0
                        for (k = i + 2; k <= n && k <= i + 8; k++) {
                            if (substr(s, k, 1) == sq) { found = k; break }
                        }
                        if (found) { i = found + 1; continue }
                    } else if (c1 != "" && substr(s, i + 2, 1) == sq) {
                        i = i + 3
                        continue
                    }
                    # Not a recognizable char literal (e.g. a lifetime like
                    # &sq a) — copy the apostrophe through unchanged.
                    out = out c
                    i++
                    continue
                }
                out = out c
                i++
            }
            return out
        }
        function is_sig(s,   r) {
            if (lang == "javascript") {
                # Accept: classic `function name(`, arrow `const name = (...) =>`,
                # class/object methods `name(...) {`, and `name: function(`.
                # Method form requires `{` on the same line to avoid matching
                # bare call sites; arrow form matches `=>` anywhere on the line.
                # Exclude lines whose leading identifier is a JS control-flow
                # keyword so `switch (x) {` / `for (...) {` etc. do not get
                # mistaken for a method signature.
                if (s ~ /^[[:space:]]*(if|else|for|while|do|switch|case|catch|try|return|throw|with|typeof|new|in|of|delete|void|yield|await|async)[[:space:]]*[\({]/) return 0
                return (s ~ /(^|[^A-Za-z0-9_])function[[:space:]]+[A-Za-z_]/) \
                    || (s ~ /(^|[^A-Za-z0-9_])(const|let|var)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[[:space:]]*(\([^)]*\)|[A-Za-z_][A-Za-z0-9_]*)[[:space:]]*=>/) \
                    || (s ~ /^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\([^)]*\)[[:space:]]*\{/) \
                    || (s ~ /[A-Za-z_][A-Za-z0-9_]*[[:space:]]*:[[:space:]]*function[[:space:]]*\(/) \
                    || (s ~ /[A-Za-z_][A-Za-z0-9_]*[[:space:]]*:[[:space:]]*(\([^)]*\)|[A-Za-z_][A-Za-z0-9_]*)[[:space:]]*=>/)
            } else if (lang == "go") {
                return (s ~ /^func[[:space:]]/)
            } else if (lang == "java") {
                # access modifier + ident + ( ... ) + {  (rough)
                return (s ~ /(public|private|protected)[[:space:]].*[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(/) \
                    && (s ~ /\{/ || s !~ /;[[:space:]]*$/)
            } else if (lang == "scala") {
                return (s ~ /(^|[^A-Za-z0-9_])def[[:space:]]+[A-Za-z_]/)
            } else if (lang == "rust") {
                return (s ~ /(^|[^A-Za-z0-9_])fn[[:space:]]+[A-Za-z_]/)
            }
            return 0
        }
        function extract_name(s,   t, kw_re) {
            if (lang == "javascript") {
                # classic function
                if (match(s, /function[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
                    t = substr(s, RSTART, RLENGTH)
                    sub(/^function[[:space:]]+/, "", t)
                    return t
                }
                # arrow: const|let|var NAME =
                if (match(s, /(const|let|var)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=/)) {
                    t = substr(s, RSTART, RLENGTH)
                    sub(/^(const|let|var)[[:space:]]+/, "", t)
                    sub(/[[:space:]]*=.*/, "", t)
                    return t
                }
                # object/class member: NAME:  or  NAME(
                if (match(s, /^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*[[:space:]]*[(:]/)) {
                    t = substr(s, RSTART, RLENGTH)
                    sub(/^[[:space:]]*/, "", t)
                    sub(/[[:space:]]*[(:].*/, "", t)
                    return t
                }
                return "<unknown>"
            }
            if (lang == "go")         kw_re = "func[[:space:]]+"
            else if (lang == "scala") kw_re = "def[[:space:]]+"
            else if (lang == "rust")  kw_re = "fn[[:space:]]+"
            else                       kw_re = ""
            if (kw_re != "") {
                if (match(s, kw_re "[A-Za-z_][A-Za-z0-9_]*")) {
                    t = substr(s, RSTART, RLENGTH)
                    sub("^" kw_re, "", t)
                    return t
                }
            } else {
                # java path — find ident followed by ( after access modifier
                if (match(s, /[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\(/)) {
                    t = substr(s, RSTART, RLENGTH)
                    sub(/[[:space:]]*\(.*/, "", t)
                    return t
                }
            }
            return "<unknown>"
        }
        {
            # Strip string/char literals BEFORE the `//` comment strip, so a
            # `//` sequence inside a string (e.g. a URL literal) is never
            # mistaken for a line comment. Both is_sig() and extract_name()
            # below now run against this stripped `line`, not raw $0, so a
            # brace or keyword-lookalike inside a string literal cannot be
            # mistaken for a signature or miscounted as a real brace.
            line = strip_strings($0)
            gsub(/\/\/.*/, "", line)
            if (!in_fn) {
                if (is_sig(line)) {
                    fname = extract_name(line)
                    start_line = NR
                    last_line = NR
                    n_open = gsub(/\{/, "{", line)
                    n_close = gsub(/\}/, "}", line)
                    depth = n_open - n_close
                    if (depth > 0) {
                        in_fn = 1
                    } else if (n_open > 0 && depth == 0) {
                        # single-line function body — emit
                        print start_line, NR, fname
                    }
                    next
                }
            } else {
                last_line = NR
                n_open = gsub(/\{/, "{", line)
                n_close = gsub(/\}/, "}", line)
                depth += n_open - n_close
                if (depth <= 0) {
                    print start_line, NR, fname
                    in_fn = 0
                }
            }
        }
        END {
            if (in_fn) print start_line, last_line, fname
        }
    ' "$file"
}

function_ranges() {
    local file="$1" lang="$2"
    case "$lang" in
        python) function_ranges_python "$file" ;;
        javascript|go|java|scala|rust) function_ranges_brace "$file" "$lang" ;;
    esac
}

# Within a function range [start..end], collect branch "shape signatures":
# first 8 chars after the `if`/`elif`/`case`/`match` keyword. Returns the
# max count of any single shape repeated within the range, and the line
# number of the first occurrence of the dominant shape.
#
# Output: "<max_count> <first_line>" (or "0 0" if nothing).
dominant_branch_shape() {
    local file="$1" lang="$2" start="$3" end="$4"
    awk -v start="$start" -v end="$end" -v lang="$lang" '
        NR < start { next }
        NR > end   { exit }
        {
            # strip leading whitespace
            s = $0
            sub(/^[[:space:]]+/, "", s)
            kw = ""
            rest = ""
            if (lang == "python") {
                if (match(s, /^(if|elif)[[:space:]]+/)) {
                    kw = "if"
                    rest = substr(s, RLENGTH+1)
                }
            } else if (lang == "scala") {
                if (match(s, /^case[[:space:]]+/)) {
                    kw = "case"
                    rest = substr(s, RLENGTH+1)
                } else if (match(s, /^(if|else if)[[:space:]]*\(/)) {
                    kw = "if"
                    rest = substr(s, RLENGTH+1)
                }
            } else if (lang == "go") {
                if (match(s, /^case[[:space:]]+/)) {
                    kw = "case"
                    rest = substr(s, RLENGTH+1)
                } else if (match(s, /^if[[:space:]]+/)) {
                    kw = "if"
                    rest = substr(s, RLENGTH+1)
                }
            } else {
                # javascript / java / rust
                if (match(s, /^(if|else if)[[:space:]]*\(/)) {
                    kw = "if"
                    rest = substr(s, RLENGTH+1)
                } else if (match(s, /^case[[:space:]]+/)) {
                    kw = "case"
                    rest = substr(s, RLENGTH+1)
                } else if (lang == "rust" && match(s, /^if[[:space:]]+/)) {
                    kw = "if"
                    rest = substr(s, RLENGTH+1)
                }
            }
            if (kw == "") next
            # shape = first 8 chars of rest, with string literals + numbers
            # normalized away so e.g. `"acid" in name` and `"alcohol" in name`
            # collapse to the same shape (`"" in na`). Spec: issue #640
            # "same first 8 characters after the if/case/match keyword,
            # repeated >=5 times" — applied AFTER literal normalization so
            # ladders that only differ in their string/number arguments
            # still count as one shape.
            gsub(/"[^"]*"/, "\"\"", rest)
            gsub(/'\''[^'\'']*'\''/, "''", rest)
            gsub(/[0-9]+/, "N", rest)
            gsub(/[[:space:]]+/, " ", rest)
            shape = substr(rest, 1, 8)
            key = kw "|" shape
            count[key]++
            if (!(key in firstline)) firstline[key] = NR
            if (count[key] > maxc) {
                maxc = count[key]
                maxline = firstline[key]
            }
        }
        END {
            if (maxc == "") maxc = 0
            if (maxline == "") maxline = 0
            print maxc, maxline
        }
    ' "$file"
}

scan_file_string_match() {
    local file="$1" lang="$2"
    local candidates subs line
    candidates="$(substring_candidate_lines "$file" "$lang")"
    subs="$(printf '%s\n' "$candidates" | awk 'NF { count++ } END { print count + 0 }')"
    subs="${subs:-0}"
    if [ "$subs" -ge 3 ] && has_proper_rep_library "$file" "$lang"; then
        line="$(printf '%s\n' "$candidates" | head -1 | cut -d: -f1)"
        line="${line:-1}"
        process_finding "$file" "$lang" "STRING_MATCH_DOMAIN_LOGIC" "$line" "<file>" "$DIRECTIVE_STRING_MATCH"
    fi
}

scan_file_repeated_structure() {
    local file="$1" lang="$2"
    # For each function range, count dominant branch shape; emit a finding
    # per function meeting the >=5 threshold.
    function_ranges "$file" "$lang" | while read -r start end fname; do
        [ -n "${start:-}" ] || continue
        [ -n "${end:-}" ] || continue
        [ -n "${fname:-}" ] || fname="<unknown>"
        # ranges of <3 lines can't fit 5 branches — skip
        if [ "$((end - start))" -lt 4 ]; then continue; fi
        read -r maxc maxline <<<"$(dominant_branch_shape "$file" "$lang" "$start" "$end")"
        maxc="${maxc:-0}"
        maxline="${maxline:-0}"
        if [ "$maxc" -ge 5 ] && [ "$maxline" -gt 0 ]; then
            process_finding "$file" "$lang" "REPEATED_STRUCTURE_AS_CODE" "$maxline" "$fname" "$DIRECTIVE_REPEATED_STRUCTURE"
        fi
    done
}

scan_file() {
    local file="$1" lang="$2"
    [ -r "$file" ] || return 0
    scan_file_string_match "$file" "$lang"
    scan_file_repeated_structure "$file" "$lang"
}

scan_lang() {
    local lang="$1" ext_pattern="$2"
    # shellcheck disable=SC2086
    find "$REPO_DIR" -type f \( $ext_pattern \) \
        ! -path '*/node_modules/*' \
        ! -path '*/.git/*' \
        ! -path '*/dist/*' \
        2>/dev/null \
        | while read -r f; do
            scan_file "$f" "$lang"
          done
}

# Order matters: lang-tag → find ext pattern.
if acquire_sweep_lock; then
    load_issue_catalogs
else
    CATALOG_STATUS="lock-failed"
    printf 'WARN: brute-force sweep lock unavailable; findings will not mutate GitHub\n' >&2
fi
scan_lang python     "-name *.py"
scan_lang javascript "-name *.js -o -name *.ts -o -name *.jsx -o -name *.tsx"
scan_lang go         "-name *.go"
scan_lang java       "-name *.java"
scan_lang scala      "-name *.scala"
scan_lang rust       "-name *.rs"

exit 0
