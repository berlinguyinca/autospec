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

# Directive map — must stay byte-identical to AGENTS.md ### Corrective
# directive map entries for these two RULE_IDs. The implementer retry loop
# reads the body of the filed issue and feeds the directive into the next
# implementer prompt; divergence here silently breaks the rewrite loop.
DIRECTIVE_STRING_MATCH='Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case.'
DIRECTIVE_REPEATED_STRUCTURE='Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a Map/sealed-trait registry; in Rust use a &[(predicate, value)] slice; in Go use a []struct{...} table. Each new entry should be one row, not a ~10-line block.'

emit_finding() {
    local file="$1" lang="$2" rule_id="$3" line="$4" func="$5"
    file="$(relative_repo_path "$file")"
    printf '{"category":"code_health:brute_force_string_heuristics","rule_id":"%s","language":"%s","file":"%s","function":"%s","line":%s}\n' \
        "$rule_id" "$lang" "$file" "$func" "$line" >> "$VERDICT_FILE"
}

relative_repo_path() {
    local file="$1"
    local physical
    physical="$(cd "$(dirname "$file")" && pwd -P)/$(basename "$file")"
    case "$physical" in
        "$REPO_DIR"/*) printf '%s\n' "${physical#"$REPO_DIR"/}" ;;
        *) printf '%s\n' "$file" ;;
    esac
}

file_issue() {
    local file="$1" lang="$2" rule_id="$3" line="$4" func="$5" directive="$6"
    local title body
    title="code_health: rewrite brute-force string heuristics in $file ($rule_id)"
    body=$(printf 'Detected %s in `%s` (%s)\n\nFunction/method: `%s`\nLine: %s\n\nDirective (verbatim from AGENTS.md):\n\n> %s\n\nLanguage: %s\n' \
        "$rule_id" "$file" "$lang" "$func" "$line" "$directive" "$lang")
    gh issue create \
        --title "$title" \
        --body "$body" \
        --label "auto-implement,autospec:v2-flow" >/dev/null 2>&1 || \
        gh issue create \
        --title "$title" \
        --body "$body" \
        --label "auto-implement,autospec:v2-flow" || true
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

# Detect a file-level function name for the STRING_MATCH_DOMAIN_LOGIC
# finding (file-scoped rule). Best-effort; falls back to <unknown>.
detect_first_function() {
    local file="$1" lang="$2"
    case "$lang" in
        python)     grep -m1 -oE '^def [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        javascript) grep -m1 -oE '\bfunction [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        go)         grep -m1 -oE '^func\s+([A-Za-z_][A-Za-z0-9_]*\s*\)\s*)?[A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $NF}' ;;
        java)       grep -m1 -oE '\b(public|private|protected)\s+[A-Za-z_<>,\s\[\]]+\s+[A-Za-z_][A-Za-z0-9_]*\(' "$file" | head -1 | grep -oE '[A-Za-z_][A-Za-z0-9_]*\(' | head -1 | tr -d '(' ;;
        scala)      grep -m1 -oE '\bdef [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        rust)       grep -m1 -oE '\bfn [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
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
    local subs func line
    subs=$(count_substring_checks "$file" "$lang")
    subs="${subs:-0}"
    if [ "$subs" -ge 3 ] && has_proper_rep_library "$file" "$lang"; then
        func=$(detect_first_function "$file" "$lang")
        func="${func:-<unknown>}"
        line=$(grep -nE '\b(contains|includes|in name|in s)\b' "$file" 2>/dev/null | head -1 | cut -d: -f1)
        line="${line:-1}"
        emit_finding "$file" "$lang" "STRING_MATCH_DOMAIN_LOGIC" "$line" "$func"
        file_issue "$file" "$lang" "STRING_MATCH_DOMAIN_LOGIC" "$line" "$func" "$DIRECTIVE_STRING_MATCH"
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
            emit_finding "$file" "$lang" "REPEATED_STRUCTURE_AS_CODE" "$maxline" "$fname"
            file_issue "$file" "$lang" "REPEATED_STRUCTURE_AS_CODE" "$maxline" "$fname" "$DIRECTIVE_REPEATED_STRUCTURE"
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
scan_lang python     "-name *.py"
scan_lang javascript "-name *.js -o -name *.ts -o -name *.jsx -o -name *.tsx"
scan_lang go         "-name *.go"
scan_lang java       "-name *.java"
scan_lang scala      "-name *.scala"
scan_lang rust       "-name *.rs"

exit 0
