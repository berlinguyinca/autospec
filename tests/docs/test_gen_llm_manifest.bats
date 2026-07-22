#!/usr/bin/env bats

@test "gen-llm-manifest writes CLI JSON without debug console logging" {
  script="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/gen-llm-manifest.mjs"

  run grep -nE '(^|[^[:alnum:]_])console\.(log|debug|warn|error)\s*\(' "$script"
  [ "$status" -ne 0 ]

  run node "$script" --repo-root "$BATS_TEST_DIRNAME/../.." --output "$BATS_TEST_TMPDIR/manifest.json"
  if ! printf '%s' "$output" | grep -qF '"schema_version": "1.0"'; then
    printf 'manifest CLI output missing schema_version\n' >&2
    return 1
  fi
  node -e 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));' "$BATS_TEST_TMPDIR/manifest.json"
}
