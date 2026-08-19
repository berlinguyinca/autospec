#!/usr/bin/env bats
# tests/prompt-context-flat-install.bats — static-context resolution under a
# flat install (install.sh copies helpers into ~/.autospec/scripts, so the
# in-repo "three levels up is the repo root" assumption no longer holds).
#
# A wrong repo root used to be silent: bundle-static-context.sh printed a
# diagnostic to stderr and exited 0, and gen-implementer-prompt.sh swallowed it
# with `2>/dev/null || true`. Implementers were then dispatched with no AGENTS.md
# and no RULE_ID contract at all.

REPO="${BATS_TEST_DIRNAME}/.."
BUNDLE_SRC="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/bundle-static-context.sh"
GEN_SRC="${BATS_TEST_DIRNAME}/../scripts/gen-implementer-prompt.sh"

setup() {
  TMP="$(mktemp -d)"
  FLAT="$TMP/flat"
  mkdir -p "$FLAT"
  cp "$BUNDLE_SRC" "$FLAT/bundle-static-context.sh"
  cp "$GEN_SRC" "$FLAT/gen-implementer-prompt.sh"
  printf '## Goal\n\nfixture issue body\n' > "$TMP/body.md"
}

teardown() {
  rm -rf "$TMP"
}

@test "flat-install bundle resolves the repo root from the working repo" {
  cd "$REPO"
  run env -u AUTOSPEC_REPO_ROOT -u AUTOSPEC_SCRIPTS_DIR \
    bash "$FLAT/bundle-static-context.sh" --role implementer
  [ "$status" -eq 0 ]
  [[ "$output" == *"Implementation-quality contract"* ]]
  [[ "$output" == *"CACHE BOUNDARY"* ]]
}

@test "flat-install bundle fails loudly when no repo root can be resolved" {
  # HOME is redirected so the ~/.autospec/repo last-resort candidate is absent.
  cd "$TMP"
  run env -u AUTOSPEC_REPO_ROOT -u AUTOSPEC_SCRIPTS_DIR HOME="$TMP" \
    bash "$FLAT/bundle-static-context.sh" --role implementer
  [ "$status" -ne 0 ]
  [[ "$output" == *"AGENTS.md"* ]]
}

@test "flat-install prompt generator emits the full cached prefix" {
  cd "$REPO"
  run env -u AUTOSPEC_REPO_ROOT -u AUTOSPEC_SCRIPTS_DIR \
    bash "$FLAT/gen-implementer-prompt.sh" \
      --issue-body "$TMP/body.md" --branch feat/x-y --repo o/r
  [ "$status" -eq 0 ]
  [[ "$output" == *"Implementation-quality contract"* ]]
}

@test "prompt generator fails closed instead of shipping a contract-free prompt" {
  # HOME is redirected so the ~/.autospec/repo last-resort candidate is absent.
  cd "$TMP"
  run env -u AUTOSPEC_REPO_ROOT -u AUTOSPEC_SCRIPTS_DIR HOME="$TMP" \
    bash "$FLAT/gen-implementer-prompt.sh" \
      --issue-body "$TMP/body.md" --branch feat/x-y --repo o/r
  [ "$status" -ne 0 ]
}

@test "prompt generator emits a Linux worktree path, never /private/tmp" {
  cd "$REPO"
  run env -u AUTOSPEC_SCRIPTS_DIR \
    env AUTOSPEC_REPO_ROOT="$REPO" bash "$FLAT/gen-implementer-prompt.sh" \
      --issue-body "$TMP/body.md" --branch feat/x-y --repo o/r
  [ "$status" -eq 0 ]
  [[ "$output" == *"/tmp/wt-feat-x-y"* ]]
  [[ "$output" != *"/private/tmp"* ]]
}

@test "flat-install bundle falls back to the canonical ~/.autospec checkout" {
  fake_home="$TMP/home"
  mkdir -p "$fake_home/.autospec"
  ln -s "$(cd "$REPO" && pwd)" "$fake_home/.autospec/repo"
  cd "$TMP"
  run env -u AUTOSPEC_REPO_ROOT -u AUTOSPEC_SCRIPTS_DIR HOME="$fake_home" \
    bash "$FLAT/bundle-static-context.sh" --role implementer
  [ "$status" -eq 0 ]
  [[ "$output" == *"Implementation-quality contract"* ]]
}
