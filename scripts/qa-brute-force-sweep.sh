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

mkdir -p "$(dirname "$VERDICT_FILE")"

SWEEP_TMP="$(mktemp -d)"
OPEN_ISSUES="$SWEEP_TMP/open-issues.json"
CLOSED_ISSUES="$SWEEP_TMP/closed-issues.json"
CATALOG_STATUS="not-loaded"
trap 'rm -rf "$SWEEP_TMP"' EXIT HUP INT TERM

# Directive map — must stay byte-identical to AGENTS.md ### Corrective
# directive map entries for these two RULE_IDs. The implementer retry loop
# reads the body of the filed issue and feeds the directive into the next
# implementer prompt; divergence here silently breaks the rewrite loop.
DIRECTIVE_STRING_MATCH='Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case.'
DIRECTIVE_REPEATED_STRUCTURE='Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a Map/sealed-trait registry; in Rust use a &[(predicate, value)] slice; in Go use a []struct{...} table. Each new entry should be one row, not a ~10-line block.'

# origin:self provenance (issue #1744): idempotent, best-effort label
ensure_origin_self_label() {
    gh label create origin:self --color 8250df --force >/dev/null 2>&1 || true
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
    gh issue list --state open --limit 100000 --json number,state,title,body,url > "$OPEN_ISSUES" 2>/dev/null || open_ok=0
    gh issue list --state closed --limit 100000 --json number,state,title,body,url > "$CLOSED_ISSUES" 2>/dev/null || closed_ok=0
    if [ "$open_ok" -ne 1 ] || [ "$closed_ok" -ne 1 ] || \
       ! jq -e 'type == "array" and all(.[]; (.number | type == "number") and (.body | type == "string"))' "$OPEN_ISSUES" >/dev/null 2>&1 || \
       ! jq -e 'type == "array" and all(.[]; (.number | type == "number") and (.body | type == "string"))' "$CLOSED_ISSUES" >/dev/null 2>&1; then
        CATALOG_STATUS="failed"
        printf 'WARN: brute-force issue catalog unavailable or malformed; findings will not mutate GitHub\n' >&2
        return 0
    fi
    CATALOG_STATUS="ready"
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

file_issue() {
    local file="$1" lang="$2" rule_id="$3" line="$4" scope="$5" blob="$6" directive="$7" marker="$8"
    local title body exact_number semantic_prefix semantic_match issue_number old_marker old_blob recurrence_file
    if [ "$CATALOG_STATUS" != "ready" ]; then
        printf '%s\n' "not-filed-catalog"
        return 0
    fi

    exact_number="$(exact_issue_number "$OPEN_ISSUES" "$marker")"
    if [ -n "$exact_number" ]; then
        printf '%s\n' "existing-open"
        return 0
    fi
    exact_number="$(exact_issue_number "$CLOSED_ISSUES" "$marker")"
    if [ -n "$exact_number" ]; then
        printf '%s\n' "existing-closed"
        return 0
    fi

    semantic_prefix="<!-- autospec-qa-brute-force:v1 rule=$rule_id path=$file scope=$scope blob="
    semantic_match="$(semantic_issue_match "$CLOSED_ISSUES" "$semantic_prefix")"
    if [ -n "$semantic_match" ]; then
        issue_number="${semantic_match%%$'\t'*}"
        old_marker="${semantic_match#*$'\t'}"
        old_blob="${old_marker##* blob=}"
        old_blob="${old_blob% -->}"
        recurrence_file="$(mktemp "$SWEEP_TMP/recurrence.XXXXXX")"
        printf 'The same brute-force heuristic recurred at a new Git blob.\n\nPrevious blob: `%s`\nCurrent blob: `%s`\n\n%s\n' \
            "$old_blob" "$blob" "$marker" > "$recurrence_file"
        if ! gh issue comment "$issue_number" --body-file "$recurrence_file" >/dev/null 2>&1; then
            printf '%s\n' "not-filed-comment-failed"
            return 0
        fi
        if ! gh issue reopen "$issue_number" >/dev/null 2>&1; then
            printf '%s\n' "not-filed-reopen-failed"
            return 0
        fi
        printf '%s\n' "reopened"
        return 0
    fi

    title="code_health: rewrite brute-force string heuristics in $file ($rule_id)"
    body=$(printf '%s\n\nDetected %s in `%s` (%s)\n\nFunction/method: `%s`\nLine: %s\nGit blob: `%s`\n\nDirective (verbatim from AGENTS.md):\n\n> %s\n\nLanguage: %s\n' \
        "$marker" "$rule_id" "$file" "$lang" "$scope" "$line" "$blob" "$directive" "$lang")
    ensure_origin_self_label
    if gh issue create \
        --title "$title" \
        --body "$body" \
        --label "auto-implement,autospec:v2-flow" \
        --label origin:self >/dev/null 2>&1; then
        printf '%s\n' "created"
        return 0
    fi
    if gh issue create \
        --title "$title" \
        --body "$body" \
        --label "auto-implement,autospec:v2-flow" \
        --label origin:self >/dev/null 2>&1; then
        printf '%s\n' "created"
    else
        printf '%s\n' "not-filed-create-failed"
    fi
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

# Heuristic: count substring-style checks in the file (used only for
# STRING_MATCH_DOMAIN_LOGIC which remains a file-scope heuristic — the
# proper-rep-library import already scopes it).
count_substring_checks() {
    local file="$1" lang="$2"
    case "$lang" in
        python)     grep -cE '\bin (name|s|x|input|text|target)\b' "$file" || true ;;
        javascript) grep -cE '\.(includes|indexOf|startsWith|endsWith)\(' "$file" || true ;;
        go)         grep -cE '\bcontains\(.*"[^"]+"\)|strings\.Contains' "$file" || true ;;
        java)       grep -cE '\.contains\("[^"]+"\)' "$file" || true ;;
        scala)      grep -cE '\.contains\("[^"]+"\)' "$file" || true ;;
        rust)       grep -cE '\.contains\("[^"]+"\)' "$file" || true ;;
    esac
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

    awk -v lang="$lang" '
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
            line = $0
            # strip line comments to avoid counting braces in `// {`
            gsub(/\/\/.*/, "", line)
            if (!in_fn) {
                if (is_sig($0)) {
                    fname = extract_name($0)
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
    local subs line
    subs=$(count_substring_checks "$file" "$lang")
    subs="${subs:-0}"
    if [ "$subs" -ge 3 ] && has_proper_rep_library "$file" "$lang"; then
        line=$(grep -nE '\b(contains|includes|in name|in s)\b' "$file" 2>/dev/null | head -1 | cut -d: -f1)
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
load_issue_catalogs
scan_lang python     "-name *.py"
scan_lang javascript "-name *.js -o -name *.ts -o -name *.jsx -o -name *.tsx"
scan_lang go         "-name *.go"
scan_lang java       "-name *.java"
scan_lang scala      "-name *.scala"
scan_lang rust       "-name *.rs"

exit 0
