#!/usr/bin/env bats
# test_continue_extract.bats — extraction matcher tests for
# scripts/extract-conversational-recommendation.sh

setup() {
    SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/extract-conversational-recommendation.sh"
    TMPDIR_T="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMPDIR_T"
}

@test "section heading: ## Next steps is extracted" {
    cat >"$TMPDIR_T/msg" <<'EOF'
Some preamble here.

## Next steps

- fix the login button
- review auth flow

## Other heading

ignore me
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"fix the login button"* ]]
    [[ "$output" == *"review auth flow"* ]]
    [[ "$output" != *"ignore me"* ]]
}

@test "section heading: ## What to do next case-insensitive" {
    cat >"$TMPDIR_T/msg" <<'EOF'
## what to do NEXT

implement the parser
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"implement the parser"* ]]
}

@test "fenced block: triple-backtick autospec-next is extracted verbatim" {
    cat >"$TMPDIR_T/msg" <<'EOF'
Here is the plan.

```autospec-next
add retry logic to the worker
wire it into the queue
```

Trailing chatter.
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"add retry logic to the worker"* ]]
    [[ "$output" == *"wire it into the queue"* ]]
    [[ "$output" != *"Trailing chatter"* ]]
}

@test "fenced block: triple-backtick next-prompt is extracted verbatim" {
    cat >"$TMPDIR_T/msg" <<'EOF'
```next-prompt
refactor the storage layer
```
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"refactor the storage layer"* ]]
}

@test "numbered list with action verbs is extracted" {
    cat >"$TMPDIR_T/msg" <<'EOF'
Background prose.

1. fix the parser bug
2. add a regression test
3. update the changelog
4. ship the release
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"fix the parser bug"* ]]
    [[ "$output" == *"add a regression test"* ]]
}

@test "you should / I suggest patterns are extracted" {
    cat >"$TMPDIR_T/msg" <<'EOF'
Background talk.

You should fix the broken test.
It has been flaky for weeks.

Some other paragraph.
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"You should fix the broken test"* ]]
}

@test "multi-matcher message combines all matches" {
    cat >"$TMPDIR_T/msg" <<'EOF'
## Next steps

- fix the heading bug

```autospec-next
add the new fence
```

I recommend reviewing the diff.
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 0 ]
    [[ "$output" == *"fix the heading bug"* ]]
    [[ "$output" == *"add the new fence"* ]]
    [[ "$output" == *"I recommend reviewing the diff"* ]]
}

@test "empty message exits 3 with continue_no_recommendation" {
    : >"$TMPDIR_T/msg"
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 3 ]
    [[ "$stderr$output" == *"continue_no_recommendation"* ]]
}

@test "chitchat-only message exits 3 with continue_no_recommendation" {
    cat >"$TMPDIR_T/msg" <<'EOF'
Hey, thanks for the question. The weather is nice today.
I had coffee this morning. Conversation has been pleasant.
EOF
    run bash "$SCRIPT" --message "$TMPDIR_T/msg"
    [ "$status" -eq 3 ]
    [[ "$stderr$output" == *"continue_no_recommendation"* ]]
}
