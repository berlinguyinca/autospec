#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

extract_direct_session_contract() {
  awk '
    /^## Direct interactive session launch$/ {
      capture = 1
    }
    capture && /^## / && $0 != "## Direct interactive session launch" {
      exit
    }
    capture {
      print
    }
  ' "$REPO_ROOT/skills/autospec-autonomous/SKILL.md"
}

@test "direct interactive session contract preserves routing, arguments, and attached output" {
  local actual expected
  actual="$(extract_direct_session_contract)"
  expected="$(cat <<'EOF'
## Direct interactive session launch

For direct invocations from Codex, Claude, or OpenCode, treat `"$@"` in the
commands below as every operator-supplied argument, preserving each token and its
original order unchanged.

When invoked without an operator subcommand or explicit launch mode, execute:

```bash
autospec-autonomous start --follow --repo-dir "$PWD" "$@"
```

Forward every supplied non-launch argument through `"$@"`. If the operator
supplied `--repo-dir`, omit the injected `--repo-dir "$PWD"` and preserve the
operator's option and value unchanged; never drop, duplicate, or reorder an
operator argument.

When no subcommand is supplied and the operator supplies exactly one of
`--follow`, `--detach`, or `--foreground`, execute:

```bash
autospec-autonomous start "$@"
```

Preserve the supplied launch mode and every remaining argument unchanged; do not
inject `--follow` or any second launch mode.

When the operator supplies a subcommand such as `start`, `status`, `stop`,
`timeline`, or `watch`, execute:

```bash
autospec-autonomous "$@"
```

Preserve the subcommand and every argument unchanged. Keep every launch using
`--follow` attached and forward its complete output to the initiating session.
Never replace attached session output with a desktop notification. Do not change
the raw CLI default; `autospec autonomous start` without a launch mode remains
detached.
EOF
)"

  run diff <(printf '%s\n' "$expected") <(printf '%s\n' "$actual")
  [ "$status" -eq 0 ]
}

@test "autonomous skill adapters remain derived from the canonical body" {
  run "$REPO_ROOT/scripts/derive-trio.sh" \
    "$REPO_ROOT/skills/autospec-autonomous" --check
  [ "$status" -eq 0 ]
}
