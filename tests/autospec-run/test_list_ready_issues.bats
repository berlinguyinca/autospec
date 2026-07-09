#!/usr/bin/env bats
# tests/autospec-run/test_list_ready_issues.bats — regression contract for
# issue #1663: list-ready-issues.sh must scope dependency extraction to the
# "## Dependencies" section so that prose of the form "#B depends on #A"
# anywhere else in the body (e.g. a "## Shared contracts" sequencing note)
# is NEVER misread as a real dependency edge.
#
# The discovery-engine decomposition put "#1658 depends on #1654" in a
# "## Shared contracts" block, which injected a phantom dependency on #1654
# into all 13 children (including #1654 onto itself), deadlocking the queue.
#
# These tests exercise the real script end-to-end with a mocked `gh` binary
# (PATH-prepend, mirroring tests/autospec-run/test_parallel_dispatch.bats):
#   - `gh issue list --label auto-implement`     -> the candidate fixture
#   - `gh issue list --label in-progress-by-bot` -> [] (no active workers)
#   - `gh issue view N ...`                       -> OPEN (dep targets unmet)

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/list-ready-issues.sh"
    FIXTURE_DIR="$(mktemp -d)"
    MOCK_BIN="$FIXTURE_DIR/bin"
    mkdir -p "$MOCK_BIN"
    # Default active list: no in-progress workers.
    printf '[]\n' > "$FIXTURE_DIR/active.json"
    # Closed dep targets (space/newline separated numbers); default empty => OPEN.
    : > "$FIXTURE_DIR/closed"

    # Mock `gh`. FIXTURE_DIR is expanded at write time so the mock is
    # self-contained regardless of the caller's environment.
    cat > "$MOCK_BIN/gh" <<MOCKEOF
#!/usr/bin/env bash
set -eu
FIXTURE_DIR="$FIXTURE_DIR"
sub="\${1:-} \${2:-}"
case "\$sub" in
  "issue list")
    label=""
    while [ "\$#" -gt 0 ]; do
      if [ "\$1" = "--label" ]; then label="\${2:-}"; fi
      shift
    done
    case "\$label" in
      auto-implement) cat "\$FIXTURE_DIR/auto.json" ;;
      in-progress-by-bot) cat "\$FIXTURE_DIR/active.json" ;;
      *) printf '[]\n' ;;
    esac
    ;;
  "issue view")
    num="\${3:-}"
    state="OPEN"
    if grep -qw "\$num" "\$FIXTURE_DIR/closed" 2>/dev/null; then
      state="CLOSED"
    fi
    printf '{"state":"%s","body":"","labels":[]}\n' "\$state"
    ;;
  "repo view")
    printf 'test/repo\n' ;;
  *) printf '[]\n' ;;
esac
MOCKEOF
    chmod +x "$MOCK_BIN/gh"
}

teardown() {
    rm -rf "$FIXTURE_DIR"
}

# Run the script against the current auto.json fixture; echoes JSON stdout only.
run_list_ready() {
    PATH="$MOCK_BIN:$PATH" bash "$SCRIPT" --repo "test/repo" --batch-size 10 2>/dev/null
}

@test "prose 'depends on #N' outside ## Dependencies yields NO edge (issue is ready)" {
    body="$(cat <<'EOF'
## Summary

A standalone feature.

## Shared contracts

Sequencing note: #101 depends on #102. Handle #101 before #102.

## Implementation outline

- edit `foo/bar.sh`
EOF
)"
    jq -n --arg body "$body" \
      '[{number:100,title:"decoy",body:$body,labels:[]}]' > "$FIXTURE_DIR/auto.json"

    output="$(run_list_ready)"
    # #100 is ready (no real deps), not blocked on the phantom #102 edge.
    [ "$(printf '%s' "$output" | jq -r '.ready | map(.number) | index(100) != null')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.blocked | map(.number) | index(100) != null')" = "false" ]
    # And #102 was never treated as a dependency.
    [ "$(printf '%s' "$output" | jq -r '[.blocked[].unmet_dependencies // [] | .[]] | index(102) != null')" = "false" ]
}

@test "real 'Depends on issue #N' in ## Dependencies still parses (blocks); decoy prose ignored" {
    body="$(cat <<'EOF'
## Summary

Depends on a real upstream issue.

## Shared contracts

Sequencing: #999 depends on #200 for ordering only.

## Dependencies

Depends on issue #201

## Implementation outline

- edit `baz/qux.sh`
EOF
)"
    jq -n --arg body "$body" \
      '[{number:200,title:"real-dep",body:$body,labels:[]}]' > "$FIXTURE_DIR/auto.json"

    output="$(run_list_ready)"
    # Real dep #201 must block #200.
    [ "$(printf '%s' "$output" | jq -r '.blocked | map(.number) | index(200) != null')" = "true" ]
    unmet="$(printf '%s' "$output" | jq -c '.blocked[] | select(.number==200) | .unmet_dependencies | sort')"
    [ "$unmet" = "[201]" ]
    # Neither the self-referential prose #200 nor the decoy #999 leaked in.
    [ "$(printf '%s' "$output" | jq -r '.blocked[] | select(.number==200) | .unmet_dependencies | index(999) != null')" = "false" ]
    [ "$(printf '%s' "$output" | jq -r '.blocked[] | select(.number==200) | .unmet_dependencies | index(200) != null')" = "false" ]
}

@test "multiple real deps in ## Dependencies all parse" {
    body="$(cat <<'EOF'
## Dependencies

Depends on issue #301
Depends on issue #302

## Implementation outline

- edit `a/b.sh`
EOF
)"
    jq -n --arg body "$body" \
      '[{number:300,title:"multi-dep",body:$body,labels:[]}]' > "$FIXTURE_DIR/auto.json"

    output="$(run_list_ready)"
    unmet="$(printf '%s' "$output" | jq -c '.blocked[] | select(.number==300) | .unmet_dependencies | sort')"
    [ "$unmet" = "[301,302]" ]
}

@test "no ## Dependencies section yields no edges (issue is ready)" {
    body="$(cat <<'EOF'
## Summary

Fully standalone.

## Implementation outline

- edit `c/d.sh`
EOF
)"
    jq -n --arg body "$body" \
      '[{number:400,title:"standalone",body:$body,labels:[]}]' > "$FIXTURE_DIR/auto.json"

    output="$(run_list_ready)"
    [ "$(printf '%s' "$output" | jq -r '.ready | map(.number) | index(400) != null')" = "true" ]
    [ "$(printf '%s' "$output" | jq -r '.blocked | map(.number) | index(400) != null')" = "false" ]
}

@test "autospec-run prompt refuses quarantined or unreviewed auto-implement issues" {
    prompt="$REPO_ROOT/skills/autospec-run/SKILL.md"
    grep -q "safety:reviewed" "$prompt"
    grep -q "security:quarantined" "$prompt"
    grep -q "autospec-safety:begin" "$prompt"
    grep -q "refuse" "$prompt"
}
