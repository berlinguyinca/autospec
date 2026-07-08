#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
BL="$REPO_ROOT/skills/autospec-shared/scripts/growth-ethics-blocklist.sh"

setup() { TMP="$(mktemp -d)"; }
teardown() { rm -rf "$TMP"; }

BUILTINS="fake_reviews undisclosed_incentivized_reviews review_gating rating_vote_manipulation sockpuppets bot_or_fake_signups scraped_email_spam cloaking_doorway link_schemes_pbn platform_tos_violation"

@test "script exists and is bash -n clean" {
  [ -f "$BL" ]; run bash -n "$BL"; [ "$status" -eq 0 ]
}

@test "--list contains every built-in block" {
  run bash "$BL" --list
  [ "$status" -eq 0 ]
  for b in $BUILTINS; do [[ "$output" == *"$b"* ]]; done
}

@test "--effective adds extra_blocks and keeps builtins" {
  echo '{"guardrails":{"extra_blocks":["my_custom_block"]}}' > "$TMP/c.json"
  run bash "$BL" --effective "$TMP/c.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *"my_custom_block"* ]]
  [[ "$output" == *"fake_reviews"* ]]
}

@test "--assert-not-weakened passes for an additive config" {
  echo '{"guardrails":{"extra_blocks":["x"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -eq 0 ]
}

@test "--assert-not-weakened fails if config tries to remove a builtin" {
  # a config that declares an explicit allowlist overriding a builtin
  echo '{"guardrails":{"allow":["fake_reviews"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -ne 0 ]
  [[ "$output" == *"fake_reviews"* ]]
}

@test "--assert-not-weakened fails on malformed JSON" {
  echo 'not json {{{' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -ne 0 ]
}

@test "--assert-not-weakened fails on a string-valued disable key" {
  echo '{"guardrails":{"disable":"fake_reviews"}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -ne 0 ]
}

@test "--assert-not-weakened fails on an unlisted key naming a builtin" {
  echo '{"guardrails":{"disabled_blocks":["fake_reviews"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -ne 0 ]
}

@test "--assert-not-weakened passes when extra_blocks redundantly names a builtin" {
  echo '{"guardrails":{"extra_blocks":["fake_reviews"]}}' > "$TMP/c.json"
  run bash "$BL" --assert-not-weakened "$TMP/c.json"
  [ "$status" -eq 0 ]
}

@test "--effective fails on malformed JSON" {
  echo 'not json {{{' > "$TMP/c.json"
  run bash "$BL" --effective "$TMP/c.json"
  [ "$status" -ne 0 ]
}
