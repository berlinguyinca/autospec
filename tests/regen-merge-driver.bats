#!/usr/bin/env bats
# tests/regen-merge-driver.bats — issue #4058.
#
# Generated artifacts (the 116 committed .sha256 skill goldens plus the
# harness-runtime-alias outputs and the integrity pin) must never be
# text-merged: a single-value file cannot survive a text merge, the losing
# side becomes a second hash, and a build passes over the corruption. The
# fix is a `merge=autospec-regen` gitattributes declaration plus a
# take-theirs placeholder driver; authoritative content comes from running
# the owning generator after the merge (AC2), and the generator itself is
# machine-readable via the `generated-by` attribute (AC4).

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
DRIVER="$REPO_ROOT/scripts/merge-regen-driver.sh"

# --- real-repo coverage (AC1 + AC4) ---------------------------------------

# attr_value <attribute> <path> — print the value from `git check-attr`'s
# `path: attr: value` line (values carry no spaces).
attr_value() {
    git check-attr "$1" -- "$2" | awk -F': ' '{print $NF}'
}

@test "every committed generated artifact declares merge=autospec-regen and a resolvable generated-by" {
    cd "$REPO_ROOT"
    local files
    files="$(git ls-files \
        'tests/fixtures/skill-goldens/*.sha256' \
        'config/generated-artifact-integrity.sha256' \
        'templates/generated/harness-runtime-aliases.sh' \
        'templates/generated/harness-runtime-aliases.fish' \
        'docs/generated/harness-runtime-aliases.md')"

    [ -n "$files" ]
    local n
    n="$(printf '%s\n' "$files" | wc -l | tr -d ' ')"
    # the inventory exists: 116 skill goldens plus four alias artifacts
    [ "$n" -ge 116 ]

    local f merge generated_by
    while IFS= read -r f; do
        merge="$(attr_value merge "$f")"
        generated_by="$(attr_value generated-by "$f")"
        [ "$merge" = "autospec-regen" ]
        [ "$generated_by" != "unspecified" ]
        # the pointer names a script that actually exists
        [ -f "$generated_by" ]
    done <<<"$files"
}

@test "an artifact with no declaration is unspecified (negative control)" {
    cd "$REPO_ROOT"
    [ "$(attr_value merge src/lib.rs 2>/dev/null || true)" = "unspecified" ]
}

# --- temp-repo end-to-end (AC2 + AC3) -------------------------------------

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/regen-merge-driver-XXXXXX")"
    REPO="$TMP/repo"
    mkdir -p "$REPO"
    cd "$REPO" || return 1
    git init -q -b main .
    git config user.email t@example.com
    git config user.name Tester

    printf 'l1\nl2\nl3\nl4\n' > src.txt
    regenerate
    printf 'out.sha256 merge=autospec-regen\n' > .gitattributes
    git add .
    git commit -q -m "chore: base"

    # register the real driver from the repo under test
    git config merge.autospec-regen.driver "bash $DRIVER %O %A %B"
}

teardown() {
    cd /
    rm -rf "$TMP"
}

# regenerate — the conversion pass's post-merge step: run the owning
# generator (here a one-liner standing in for scripts/gen-skill-goldens.sh)
# and stage the result.
regenerate() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum src.txt | cut -d' ' -f1 > out.sha256
    else
        shasum -a 256 src.txt | cut -d' ' -f1 > out.sha256
    fi
    git add out.sha256
}

# branch_change <branch> <line> <replacement> — off main, change one source
# line and regenerate the golden, exactly what an implementer patch does.
branch_change() {
    git checkout -q -b "$1" main
    sed -i.bak "s/^$2$/$3/" src.txt && rm -f src.txt.bak
    git add src.txt
    regenerate
    git commit -q -m "feat: $1 changes $2"
}

# fresh_hash — the hash the merged src.txt must have after regeneration.
fresh_hash() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum src.txt | cut -d' ' -f1
    else
        shasum -a 256 src.txt | cut -d' ' -f1
    fi
}

@test "two patches each regenerate the same golden and merge cleanly; the final hash equals the freshly regenerated output" {
    branch_change a l2 l2A
    branch_change b l4 l4B

    # conversion-path scenario: b's patch (common ancestor main -> b) applied
    # 3-way on top of a. Both sides touched the golden; neither side touched
    # the other's source line, so only the generated file could conflict.
    git checkout -q a
    git diff main b > "$TMP/patch-b"
    local out rc=0
    out="$(git apply --3way "$TMP/patch-b" 2>&1)" || rc=$?

    [ "$rc" -eq 0 ]
    [[ "$out" == *"Applied patch to 'out.sha256' cleanly"* ]]
    [[ "$out" == *"Applied patch to 'src.txt' cleanly"* ]]
    [ -z "$(git ls-files -u)" ]

    # the driver's take-theirs output is b's hash: a placeholder, not the
    # answer — the merged src.txt has both l2A and l4B, which b's hash
    # cannot be.
    [ "$(cat out.sha256)" = "$(git show b:out.sha256)" ]
    [ "$(cat out.sha256)" != "$(fresh_hash)" ]

    # AC2: the conversion pass regenerates; the final hash equals the
    # freshly regenerated output.
    regenerate
    [ "$(cat out.sha256)" = "$(fresh_hash)" ]
}

@test "a source conflict is refused: the apply fails and only the source is unmerged" {
    branch_change a l2 l2A
    branch_change c l2 l2C   # overlaps a on the same source line

    git checkout -q a
    git diff main c > "$TMP/patch-c"
    local out rc=0
    out="$(git apply --3way "$TMP/patch-c" 2>&1)" || rc=$?

    [ "$rc" -ne 0 ]
    [[ "$out" == *"Applied patch to 'src.txt' with conflicts"* ]]
    # the refusal names the source file; the generated file is not the
    # reported conflict
    [ "$(git ls-files -u | awk '{print $4}' | sort -u)" = "src.txt" ]
    git status --porcelain | grep -q '^UU src.txt$'
}

@test "the same merge completes cleanly via plain git merge (human workflow)" {
    branch_change a l2 l2A
    branch_change b l4 l4B

    git checkout -q a
    local out rc=0
    out="$(git merge --no-edit b 2>&1)" || rc=$?

    [ "$rc" -eq 0 ]
    # the source merged both sides
    grep -qx l2A src.txt
    grep -qx l4B src.txt
    # the golden is the placeholder until the generator runs, then correct
    [ "$(cat out.sha256)" = "$(git show b:out.sha256)" ]
    regenerate
    [ "$(cat out.sha256)" = "$(fresh_hash)" ]
}
