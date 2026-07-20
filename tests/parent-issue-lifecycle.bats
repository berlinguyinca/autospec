#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
}

@test "every decomposition workflow records the umbrella and child relationship" {
    local skill
    for skill in autospec autospec-define autospec-split; do
        run grep -F 'parent record --repo {repo} --parent "<UMBRELLA>" --children "<CHILDREN_CSV>"' \
            "$REPO_ROOT/skills/$skill/SKILL.md"
        [ "$status" -eq 0 ]
    done
}

@test "implementation workflows reconcile the parent after the child merge" {
    local skill
    for skill in autospec autospec-run; do
        run grep -F 'parent reconcile-child --repo {repo} --child "<ISSUE>"' \
            "$REPO_ROOT/skills/$skill/SKILL.md"
        [ "$status" -eq 0 ]
    done
}

@test "implementation workflows sweep parents closed outside autospec" {
    local skill
    for skill in autospec autospec-run; do
        run grep -F 'parent sweep --repo {repo}' \
            "$REPO_ROOT/skills/$skill/SKILL.md"
        [ "$status" -eq 0 ]
    done
}

@test "run workflow reserves umbrella mutation for the typed parent command" {
    run grep -F 'Only `autospec parent` may update or close an umbrella issue' \
        "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -eq 0 ]
}
