#!/usr/bin/env bats
setup() {
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    mkdir -p "$TMP/bin"
    GH_LOG="$TMP/gh.log"
    : >"$GH_LOG"
    cat >"$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
printf 'gh %s\n' "\$*" >> "$GH_LOG"
case "\$*" in
  *"actions/runs?branch=main"*)
    if [ -n "\${RUNS_FILE:-}" ] && [ -f "\$RUNS_FILE" ]; then
      cat "\$RUNS_FILE"
      exit 0
    fi
    printf '{}'
    ;;
  *"commits/"*)
    sha="\${\${*##*commits/}%% *}"
    printf '%s\n' "\${SUBJ:-subject of \$sha}"
    ;;
  *)
    printf '{}'
    ;;
esac
EOF
    chmod +x "$TMP/bin/gh"
    PATH="$TMP/bin:$PATH"
    export PATH
    SCRIPT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)/scripts/first-failing-commit.sh"
}

@test "minimal: run the script, expect failure status" {
    run bash "$SCRIPT" --repo OWNER/REPO
    echo "DEBUG status=$status"
    [ "$status" -eq 2 ]
}

@test "minimal two: trivially pass" {
    [ 1 -eq 1 ]
}
