#!/usr/bin/env bash
# scripts/qa-brute-force-sweep.sh
#
# autospec-qa brute-force string-heuristics sweep (issue #637).
#
# Scans REPO_DIR for already-merged code that matches either of the two
# LLM-tier RULE_IDs:
#   - STRING_MATCH_DOMAIN_LOGIC      (substring-on-name encoding domain
#                                     meaning while a proper-rep library
#                                     is imported in the same file)
#   - REPEATED_STRUCTURE_AS_CODE     (>=5 branches in one function/method
#                                     sharing identical structural shape)
#
# Supported languages: Python, JavaScript/TypeScript, Go, Java, Scala, Rust.
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
VERDICT_FILE="${VERDICT_FILE:-$REPO_DIR/.autospec/qa-verdict.json}"

mkdir -p "$(dirname "$VERDICT_FILE")"

# Directive map — must stay byte-identical to AGENTS.md ### Corrective
# directive map entries for these two RULE_IDs. The implementer retry loop
# reads the body of the filed issue and feeds the directive into the next
# implementer prompt; divergence here silently breaks the rewrite loop.
DIRECTIVE_STRING_MATCH='Replace substring checks with the proper domain primitive (SMARTS/AST/parsed URL/IP/date/schema). Substring-on-name is brittle to synonyms, locants, salt forms, escaping, and case.'
DIRECTIVE_REPEATED_STRUCTURE='Extract the N branches into a table + single dispatcher loop. In Python use a list of tuples or dict; in Java/Scala use a Map/sealed-trait registry; in Rust use a &[(predicate, value)] slice; in Go use a []struct{...} table. Each new entry should be one row, not a ~10-line block.'

# Per-language ext -> (lang-tag, proper-rep-library-pattern).
# The lang-tag is what gets written into qa-verdict.json `language` field.
emit_finding() {
    local file="$1" lang="$2" rule_id="$3" line="$4" func="$5"
    # Append a compact JSON line; the caller is responsible for wrapping
    # these into a valid JSON document downstream. We use a leniently
    # structured append because qa-verdict.json is consumed by `jq` in
    # autospec-qa with a `if file is not valid JSON, rebuild it` guard.
    printf '{"category":"code_health:brute_force_string_heuristics","rule_id":"%s","language":"%s","file":"%s","function":"%s","line":%s}\n' \
        "$rule_id" "$lang" "$file" "$func" "$line" >> "$VERDICT_FILE"
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

# Heuristic: count substring-style checks in the file.
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

# Heuristic: count repeated branch-shaped structures in the file.
count_repeated_branches() {
    local file="$1"
    # if/elif (Python), case (switch arms), match arms — count whichever wins.
    local a b c
    a=$(grep -cE '^\s*(if|elif|else if)\b.*:' "$file" || true)
    b=$(grep -cE '^\s*case\b' "$file" || true)
    c=$(grep -cE '^\s*if\s+.*\{.*return' "$file" || true)
    # union (max-ish): pick the largest of the three
    printf '%s\n' "$a" "$b" "$c" | sort -nr | head -1
}

# Try to detect the function name an offending block lives in. Best-effort.
detect_function() {
    local file="$1" lang="$2"
    case "$lang" in
        python)     grep -m1 -oE '^def [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        javascript) grep -m1 -oE '\bfunction [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        go)         grep -m1 -oE '^func\s+([A-Za-z_][A-Za-z0-9_]*\s*\)\s*)?[A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $NF}' ;;
        java)       grep -m1 -oE '\b(public|private|protected)?\s*[A-Za-z_<>,\s]+\s+[A-Za-z_][A-Za-z0-9_]*\(' "$file" | head -1 ;;
        scala)      grep -m1 -oE '\bdef [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
        rust)       grep -m1 -oE '\bfn [A-Za-z_][A-Za-z0-9_]*' "$file" | head -1 | awk '{print $2}' ;;
    esac
}

scan_file() {
    local file="$1" lang="$2"
    [ -r "$file" ] || return 0

    local subs reps
    subs=$(count_substring_checks "$file" "$lang")
    reps=$(count_repeated_branches "$file")
    subs="${subs:-0}"
    reps="${reps:-0}"

    local func line
    func=$(detect_function "$file" "$lang")
    func="${func:-<unknown>}"
    line=$(grep -nE '\b(contains|includes|in name|in s)\b' "$file" 2>/dev/null | head -1 | cut -d: -f1)
    line="${line:-1}"

    # STRING_MATCH_DOMAIN_LOGIC requires substring checks AND a proper-rep
    # library imported in the same file.
    if [ "$subs" -ge 3 ] && has_proper_rep_library "$file" "$lang"; then
        emit_finding "$file" "$lang" "STRING_MATCH_DOMAIN_LOGIC" "$line" "$func"
        file_issue "$file" "$lang" "STRING_MATCH_DOMAIN_LOGIC" "$line" "$func" "$DIRECTIVE_STRING_MATCH"
    fi

    # REPEATED_STRUCTURE_AS_CODE — >=5 branch-shaped lines in one file
    # (file-level heuristic, refined by LLM at PR time).
    if [ "$reps" -ge 5 ]; then
        emit_finding "$file" "$lang" "REPEATED_STRUCTURE_AS_CODE" "$line" "$func"
        file_issue "$file" "$lang" "REPEATED_STRUCTURE_AS_CODE" "$line" "$func" "$DIRECTIVE_REPEATED_STRUCTURE"
    fi
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
