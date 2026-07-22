#!/usr/bin/env bats
# tests/unit/test_listener_keywords.bats — exercise scripts/listener-match.sh
# against ≥30 phrase fixtures covering positive issue triggers, positive
# spec triggers, and negative phrases (bare nouns, embedded-but-not-trigger
# substrings, mixed case, leading/trailing punctuation).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    MATCH="$REPO_ROOT/scripts/listener-match.sh"
}

@test "listener matcher avoids ambiguous any token in source" {
    ! grep -Eq '\bany\b' "$MATCH"
}

# ---- positive issue triggers (≥15) ---------------------------------------

@test "issue: 'file an issue'" {
    run "$MATCH" "file an issue"
    [ "$status" -eq 0 ]
    [ "$output" = "issue" ]
}

@test "issue: 'File an issue' (mixed case)" {
    run "$MATCH" "File an issue"
    [ "$output" = "issue" ]
}

@test "issue: 'FILE AN ISSUE' (upper)" {
    run "$MATCH" "FILE AN ISSUE"
    [ "$output" = "issue" ]
}

@test "issue: 'file this as an issue'" {
    run "$MATCH" "file this as an issue"
    [ "$output" = "issue" ]
}

@test "issue: 'new issue'" {
    run "$MATCH" "new issue"
    [ "$output" = "issue" ]
}

@test "issue: 'open an issue'" {
    run "$MATCH" "open an issue"
    [ "$output" = "issue" ]
}

@test "issue: 'create a ticket'" {
    run "$MATCH" "create a ticket"
    [ "$output" = "issue" ]
}

@test "issue: 'make an issue'" {
    run "$MATCH" "make an issue"
    [ "$output" = "issue" ]
}

@test "issue: leading punctuation — 'Could you file an issue?'" {
    run "$MATCH" "Could you file an issue?"
    [ "$output" = "issue" ]
}

@test "issue: embedded — 'we should file an issue here.'" {
    run "$MATCH" "we should file an issue here."
    [ "$output" = "issue" ]
}

@test "issue: 'please open an issue for that'" {
    run "$MATCH" "please open an issue for that"
    [ "$output" = "issue" ]
}

@test "issue: 'create a ticket for the auth bug'" {
    run "$MATCH" "create a ticket for the auth bug"
    [ "$output" = "issue" ]
}

@test "issue: 'let me file this as an issue real quick'" {
    run "$MATCH" "let me file this as an issue real quick"
    [ "$output" = "issue" ]
}

@test "issue: trailing punctuation — 'new issue!'" {
    run "$MATCH" "new issue!"
    [ "$output" = "issue" ]
}

@test "issue: leading whitespace — '   file an issue'" {
    run "$MATCH" "   file an issue"
    [ "$output" = "issue" ]
}

@test "issue: 'make an issue, would you?'" {
    run "$MATCH" "make an issue, would you?"
    [ "$output" = "issue" ]
}

# ---- positive spec triggers (≥10) ---------------------------------------

@test "spec: 'write a spec'" {
    run "$MATCH" "write a spec"
    [ "$output" = "spec" ]
}

@test "spec: 'Write a spec' (mixed case)" {
    run "$MATCH" "Write a spec"
    [ "$output" = "spec" ]
}

@test "spec: 'design spec'" {
    run "$MATCH" "design spec"
    [ "$output" = "spec" ]
}

@test "spec: 'new spec'" {
    run "$MATCH" "new spec"
    [ "$output" = "spec" ]
}

@test "spec: 'start a spec'" {
    run "$MATCH" "start a spec"
    [ "$output" = "spec" ]
}

@test "spec: 'write a design spec'" {
    run "$MATCH" "write a design spec"
    [ "$output" = "spec" ]
}

@test "spec: 'WRITE A SPEC' (upper)" {
    run "$MATCH" "WRITE A SPEC"
    [ "$output" = "spec" ]
}

@test "spec: embedded — 'could you write a spec for this please?'" {
    run "$MATCH" "could you write a spec for this please?"
    [ "$output" = "spec" ]
}

@test "spec: trailing punctuation — 'new spec.'" {
    run "$MATCH" "new spec."
    [ "$output" = "spec" ]
}

@test "spec: 'we need a design spec on this'" {
    run "$MATCH" "we need a design spec on this"
    [ "$output" = "spec" ]
}

@test "spec: 'start a spec when you have a moment'" {
    run "$MATCH" "start a spec when you have a moment"
    [ "$output" = "spec" ]
}

# ---- negative cases (≥10) -----------------------------------------------

@test "none: 'the issue here is real'" {
    run "$MATCH" "the issue here is real"
    [ "$output" = "none" ]
}

@test "none: 'the spec says so'" {
    run "$MATCH" "the spec says so"
    [ "$output" = "none" ]
}

@test "none: bare noun 'issue'" {
    run "$MATCH" "issue"
    [ "$output" = "none" ]
}

@test "none: bare noun 'spec'" {
    run "$MATCH" "spec"
    [ "$output" = "none" ]
}

@test "none: bare noun 'ticket'" {
    run "$MATCH" "ticket"
    [ "$output" = "none" ]
}

@test "none: 'we should issue a refund'" {
    run "$MATCH" "we should issue a refund"
    [ "$output" = "none" ]
}

@test "none: 'this ticket needs review'" {
    run "$MATCH" "this ticket needs review"
    [ "$output" = "none" ]
}

@test "none: 'the spec writer is on PTO'" {
    run "$MATCH" "the spec writer is on PTO"
    [ "$output" = "none" ]
}

@test "none: 'newer issues are appearing'" {
    run "$MATCH" "newer issues are appearing"
    [ "$output" = "none" ]
}

@test "none: 'specifications differ between teams'" {
    run "$MATCH" "specifications differ between teams"
    [ "$output" = "none" ]
}

@test "none: 'I don't want to file anything yet'" {
    run "$MATCH" "I don't want to file anything yet"
    [ "$output" = "none" ]
}

@test "none: 'creates a tickle in throat'" {
    run "$MATCH" "creates a tickle in throat"
    [ "$output" = "none" ]
}
