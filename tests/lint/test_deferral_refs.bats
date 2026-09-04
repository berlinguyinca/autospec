#!/usr/bin/env bats
# tests/lint/test_deferral_refs.bats — guards #3497.
#
# #3379 deferred ~383 lines of Rust and promised they were safe:
#
#   "The remaining Rust stays on `feat/pi-agent-handoffs`, with
#    `backup/pi-agent-handoffs-prerebase` and tag `pi-handoffs-prerebase-20260825`
#    as safety refs."
#
# All three refs were later deleted. Nothing noticed, because the promise lives in
# prose that no gate reads, and the PR still reads as safely parked. The work is
# unrecoverable: absent from 6,276 remote branches, 1,758 pull refs, every clone on
# the host, and the dangling-object store.
#
# The rule this pins: a body may name a ref as the resting place of deferred work
# only while that ref exists. Mentioning a branch is not a promise; promising that
# work survives on one is.
#
# Every case runs `git ls-remote` against a REAL local bare repository with REAL
# branches and tags, so ref existence is resolved by git itself rather than a stub.

ROOT="${BATS_TEST_DIRNAME}/../.."
LINT="$ROOT/scripts/lint-deferral-refs.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"

    # A real remote: a bare repo carrying one branch and one tag that exist, so
    # "present" and "absent" are both answered by git ls-remote for real.
    REMOTE="$TEST_TMP/remote.git"
    SEED="$TEST_TMP/seed"
    git init -q --bare "$REMOTE"
    git init -q "$SEED"
    git -C "$SEED" checkout -q -b main
    echo seed > "$SEED/seed.txt"
    git -C "$SEED" add seed.txt
    git -C "$SEED" commit -q -m seed
    git -C "$SEED" push -q "$REMOTE" main
    git -C "$SEED" checkout -q -b feat/still-here
    git -C "$SEED" push -q "$REMOTE" feat/still-here
    git -C "$SEED" tag kept-tag-20260825
    git -C "$SEED" push -q "$REMOTE" kept-tag-20260825

    BODY="$TEST_TMP/body.md"
}

teardown() { rm -rf "$TEST_TMP"; }

lint() { bash "$LINT" --body-file "$BODY" --remote "$REMOTE"; }

@test "a body promising work survives on an absent branch fails and names it" {
    cat > "$BODY" <<'MD'
## What is deliberately not here

The remaining Rust stays on `feat/pi-agent-handoffs`, with
`backup/pi-agent-handoffs-prerebase` as safety refs.
MD
    run lint
    [ "$status" -ne 0 ]
    [[ "$output" == *"DEFERRAL_REF_ABSENT"* ]]
    [[ "$output" == *"feat/pi-agent-handoffs"* ]]
    [[ "$output" == *"backup/pi-agent-handoffs-prerebase"* ]]
}

@test "an absent tag named as a safety ref fails" {
    cat > "$BODY" <<'MD'
Parked with tag `pi-handoffs-prerebase-20260825` as a safety ref.
MD
    run lint
    [ "$status" -ne 0 ]
    [[ "$output" == *"pi-handoffs-prerebase-20260825"* ]]
}

@test "the exact #3379 wording fails, naming all three refs" {
    # Verbatim regression. If the detector stops matching this sentence, the
    # defect it was written for walks straight back in.
    cat > "$BODY" <<'MD'
Reconciling them is an architecture call about the autonomous executor, not a
merge conflict. The remaining Rust stays on `feat/pi-agent-handoffs`, with
`backup/pi-agent-handoffs-prerebase` and tag `pi-handoffs-prerebase-20260825`
as safety refs.
MD
    run lint
    [ "$status" -eq 3 ]
    [[ "$output" == *"feat/pi-agent-handoffs"* ]]
    [[ "$output" == *"backup/pi-agent-handoffs-prerebase"* ]]
    [[ "$output" == *"pi-handoffs-prerebase-20260825"* ]]
}

@test "a ref that still exists on the remote passes" {
    cat > "$BODY" <<'MD'
The remaining work stays on `feat/still-here` as a safety ref.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "an existing tag named as a safety ref passes" {
    cat > "$BODY" <<'MD'
Preserved on tag `kept-tag-20260825` as a safety ref.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "merely mentioning an absent branch is not a promise and passes" {
    # The rule is about a claim that deferred work SURVIVES somewhere. Naming a
    # branch that has since been merged and deleted is ordinary and must not fail
    # — otherwise the check becomes noise and gets disabled.
    cat > "$BODY" <<'MD'
This supersedes the approach taken in `feat/long-since-merged`, and closes #12.
Rebased onto `origin/main` after `feat/also-gone` landed.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "a body with no refs at all passes" {
    printf 'Ordinary change. Closes #1.\n' > "$BODY"
    run lint
    [ "$status" -eq 0 ]
}

@test "a promise sentence naming no ref does not crash or fire" {
    printf 'The rest is parked as a safety ref somewhere sensible.\n' > "$BODY"
    run lint
    [ "$status" -eq 0 ]
}

@test "--help states the rule and exits 0" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"DEFERRAL_REF_ABSENT"* ]]
}

@test "a missing body file is a usage error, not a silent pass" {
    run bash "$LINT" --body-file "$TEST_TMP/absent.md" --remote "$REMOTE"
    [ "$status" -eq 1 ]
    [[ "$output" == *"body-file"* ]]
}

@test "file paths inside a promise sentence are not refs" {
    # Real false positives, from scanning the 20 most recently merged PRs:
    # #3483 named `backend/agent_lsp.rs` and #3481 named `.autospec/` in prose
    # that matched the promise wording. A path is not a ref, and a check that
    # cannot tell them apart is noise that gets switched off.
    cat > "$BODY" <<'MD'
The gateway keeps its state in `backend/agent_lsp.rs`, and receipts remain on
disk under `.autospec/` as a safety net. Docs stay in `docs/API_REFERENCE.md`.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "a dotted tag is still a ref" {
    # The path filter must not swallow dotted refs: a trailing NUMERIC segment is
    # a version, a trailing ALPHABETIC one is a file extension. `release-1.2.3`
    # reaches the filter (it has a dash) and must survive it.
    cat > "$BODY" <<'MD'
Preserved on tag `release-1.2.3` as a safety ref.
MD
    run lint
    [ "$status" -ne 0 ]
    [[ "$output" == *"release-1.2.3"* ]]
}

@test "a token with neither slash nor dash is not treated as a ref" {
    # Documented limit, not an oversight: `v1.2.3` and `main` are shapeless
    # enough that treating them as refs would flag ordinary prose. A ref named
    # in a promise is expected to carry a '/' or '-', which every branch and
    # dated tag in this repository does.
    cat > "$BODY" <<'MD'
Preserved on tag `v1.2.3` as a safety ref, alongside `main`.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "a markdown table does not fuse unrelated cells into one promise" {
    # Real false positive from #3481: table rows carry no sentence-ending
    # punctuation, so the whole table collapsed into a single "sentence" and a
    # promise phrase in one cell paired with `autospec-inferweave` — a repository
    # name — rows away. Blank lines and table rows must break the scan.
    cat > "$BODY" <<'MD'
| Spec | Module | Responsibility |
| --- | --- | --- |
| 3 | `classify` | a larger budget is kept only with evidence |
| 4 | `profile` | versioned profiles, see `autospec-inferweave` |

Unrelated paragraph mentioning `feat/some-branch` in passing.
MD
    run lint
    [ "$status" -eq 0 ]
}

@test "a real promise still fires when a table is present in the same body" {
    # The table filter must not become a blanket amnesty.
    cat > "$BODY" <<'MD'
| a | b |
| --- | --- |
| 1 | 2 |

The remaining Rust stays on `feat/vanished-branch` as a safety ref.
MD
    run lint
    [ "$status" -ne 0 ]
    [[ "$output" == *"feat/vanished-branch"* ]]
}

@test "saying where code lives in another repository is not a deferral promise" {
    # Regression: the first draft matched `lives|live + on/at/in`, which is how
    # prose says where code resides rather than a claim that deferred work is
    # preserved. It fired on this exact sentence from #3481, where
    # `autospec-inferweave` is a sibling repository, not a ref. A linter that
    # flags ordinary cross-repo references gets switched off.
    cat > "$BODY" <<'MD'
Discovery, admission control and placement live in `autospec-inferweave`.
MD
    run lint
    [ "$status" -eq 0 ]
    [[ "$output" != *"DEFERRAL_REF_ABSENT"* ]]
}
