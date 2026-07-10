setup() {
  FILL_SH="${BATS_TEST_DIRNAME}/../../scripts/groom-fill.sh"   # tests/autospec/ → repo/scripts/
  BIN_DIR="$BATS_TEST_TMPDIR/bin"; mkdir -p "$BIN_DIR"
  # default: codex stub echoes a canned "filled" body
  cat > "$BIN_DIR/codex-ok" <<'SH'
#!/usr/bin/env bash
# ignore "exec" subcommand + args, read prompt from stdin, emit a body
cat >/dev/null
printf '## Summary\nFilled template body.\n## Acceptance\n- done\n'
SH
  chmod +x "$BIN_DIR/codex-ok"
  # validate stub: ok
  cat > "$BIN_DIR/validate-ok" <<'SH'
#!/usr/bin/env bash
printf '{"ok":true}\n'; exit 0
SH
  chmod +x "$BIN_DIR/validate-ok"
}

@test "codex-absent → ok:false reason codex-absent, holds" {
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/does-not-exist" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "false" ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "codex-absent" ]
}

@test "codex-ok + validate-ok → ok:true with body" {
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-ok" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --title T --body B
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "true" ]
  printf '%s' "$output" | jq -e '.body | test("Filled template body")' >/dev/null
}

@test "validate fails then succeeds within attempts → ok:true" {
  # codex emits attempt-numbered body; validate stub fails on attempt 1, ok on 2
  cat > "$BIN_DIR/codex-count" <<'SH'
#!/usr/bin/env bash
cat >/dev/null
n=$(cat "$COUNTER" 2>/dev/null || echo 0); n=$((n+1)); echo "$n" > "$COUNTER"
printf 'BODY attempt %s\n' "$n"
SH
  chmod +x "$BIN_DIR/codex-count"
  cat > "$BIN_DIR/validate-2nd" <<'SH'
#!/usr/bin/env bash
grep -q 'attempt 1' "$1" && { printf '{"ok":false,"findings":["missing Summary"]}\n'; exit 1; }
printf '{"ok":true}\n'; exit 0
SH
  chmod +x "$BIN_DIR/validate-2nd"
  run env COUNTER="$BATS_TEST_TMPDIR/c" \
          AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-count" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-2nd" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 3 --title T --body B
  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r .ok)" = "true" ]
}

@test "attempts exhausted → ok:false attempts-exhausted" {
  cat > "$BIN_DIR/validate-no" <<'SH'
#!/usr/bin/env bash
printf '{"ok":false,"findings":["nope"]}\n'; exit 1
SH
  chmod +x "$BIN_DIR/validate-no"
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-ok" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-no" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 2 --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "attempts-exhausted" ]
}

@test "codex nonzero exit → ok:false codex-error" {
  cat > "$BIN_DIR/codex-fail" <<'SH'
#!/usr/bin/env bash
cat >/dev/null; exit 7
SH
  chmod +x "$BIN_DIR/codex-fail"
  run env AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-fail" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --attempts 2 --title T --body B
  [ "$status" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r .reason)" = "codex-error" ]
}

@test "passes 'exec --skip-git-repo-check' to codex (headless trust flag)" {
  # Real codex refuses in a non-trusted dir without --skip-git-repo-check, which
  # would fail-close every groom. Stubs ignore args, so this records + asserts
  # the exact invocation to lock the flag against regression.
  cat > "$BIN_DIR/codex-argrec" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ARGREC"
cat >/dev/null
printf '## Summary\nok\n'
SH
  chmod +x "$BIN_DIR/codex-argrec"
  run env ARGREC="$BATS_TEST_TMPDIR/args.txt" \
          AUTOSPEC_GROOM_FILL_BIN="$BIN_DIR/codex-argrec" \
          AUTOSPEC_GROOM_VALIDATE_BIN="$BIN_DIR/validate-ok" \
      bash "$FILL_SH" --issue 1 --repo o/r --title T --body B
  [ "$status" -eq 0 ]
  grep -q 'exec' "$BATS_TEST_TMPDIR/args.txt"
  grep -q -- '--skip-git-repo-check' "$BATS_TEST_TMPDIR/args.txt"
}
