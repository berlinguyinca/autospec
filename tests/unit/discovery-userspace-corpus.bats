#!/usr/bin/env bats
# tests/unit/discovery-userspace-corpus.bats — userspace-corpus harvester (issue #1657).
# Read-only scan of sibling peer repos in the operator's workspace for capabilities
# (skills/scripts/docs) peer repos have that this repo lacks. Fixture-driven: builds a
# temp workspace with 2 fake peer repos, never assumes anything about the real
# filesystem or the real autospec repo.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
UC="$REPO_ROOT/skills/autospec-shared/scripts/discovery-userspace-corpus.sh"
TL="$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh"
VALIDATOR="$REPO_ROOT/skills/autospec-shared/scripts/validate-trend-signal.sh"

setup() {
  WS="$(mktemp -d -t discovery-userspace-corpus.XXXXXX)"
  export AUTOSPEC_TREND_LEDGER="$WS/ledger.jsonl"

  # this repo: has one skill + one script
  mkdir -p "$WS/this-repo/skills/existing-skill" "$WS/this-repo/scripts" "$WS/this-repo/docs"
  git init -q "$WS/this-repo"
  echo "# existing" > "$WS/this-repo/skills/existing-skill/SKILL.md"
  echo "echo hi" > "$WS/this-repo/scripts/existing.sh"

  # peer-a: has the same existing-skill PLUS a capability this repo lacks
  mkdir -p "$WS/peer-a/skills/existing-skill" "$WS/peer-a/skills/new-peer-skill" "$WS/peer-a/scripts"
  git init -q "$WS/peer-a"
  echo "# existing" > "$WS/peer-a/skills/existing-skill/SKILL.md"
  echo "# new" > "$WS/peer-a/skills/new-peer-skill/SKILL.md"
  echo "echo hi" > "$WS/peer-a/scripts/existing.sh"

  # peer-b: has a script and a doc this repo lacks
  mkdir -p "$WS/peer-b/scripts" "$WS/peer-b/docs"
  git init -q "$WS/peer-b"
  echo "echo cool" > "$WS/peer-b/scripts/cool-tool.sh"
  echo "# guide" > "$WS/peer-b/docs/GUIDE.md"

  CFG="$WS/c.yml"
  cat > "$CFG" <<'YML'
discovery:
  userspace:
    opt_out: false
YML

  OPTOUT_CFG="$WS/opt-out.yml"
  cat > "$OPTOUT_CFG" <<'YML'
discovery:
  userspace:
    opt_out: true
YML
}

teardown() {
  rm -rf "$WS"
  unset AUTOSPEC_TREND_LEDGER
}

@test "script exists and is bash -n clean" {
  [ -f "$UC" ]
  run bash -n "$UC"
  [ "$status" -eq 0 ]
}

@test "derives capability-gap signals from fixture peer repos" {
  run bash "$UC" "$CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  [ "$status" -eq 0 ]
  [ -f "$AUTOSPEC_TREND_LEDGER" ]

  run bash "$TL" --show --json --source userspace-corpus
  [ "$status" -eq 0 ]
  # peer-a's new-peer-skill, peer-b's cool-tool.sh, peer-b's GUIDE.md == 3 gap signals
  [ "$(echo "$output" | jq 'length')" -eq 3 ]
  [ "$(echo "$output" | jq '[.[] | select(.source=="userspace-corpus")] | length')" -eq 3 ]
  # the shared existing-skill/existing.sh must NOT appear as a gap
  [[ "$(echo "$output" | jq -r '.[].norm_key')" != *"existing-skill"* ]]
  [[ "$(echo "$output" | jq -r '.[].norm_key')" != *"existing.sh"* ]]
}

@test "opt_out config emits nothing and exits 0" {
  run bash "$UC" "$OPTOUT_CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
}

@test "peer repos are unmodified after the run (read-only)" {
  before_a="$(find "$WS/peer-a" -type f -exec shasum {} \; | sort)"
  before_b="$(find "$WS/peer-b" -type f -exec shasum {} \; | sort)"

  run bash "$UC" "$CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  [ "$status" -eq 0 ]

  after_a="$(find "$WS/peer-a" -type f -exec shasum {} \; | sort)"
  after_b="$(find "$WS/peer-b" -type f -exec shasum {} \; | sort)"

  [ "$before_a" = "$after_a" ]
  [ "$before_b" = "$after_b" ]
}

@test "every appended record passes validate-trend-signal.sh" {
  run bash "$UC" "$CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  [ "$status" -eq 0 ]

  while IFS= read -r line; do
    [ -n "$line" ] || continue
    tmp="$(mktemp)"
    printf '%s' "$line" > "$tmp"
    run bash "$VALIDATOR" "$tmp"
    [ "$status" -eq 0 ]
    rm -f "$tmp"
  done < "$AUTOSPEC_TREND_LEDGER"
}

@test "re-running bumps recurrence instead of duplicating entries" {
  bash "$UC" "$CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  run bash "$TL" --show --json --source userspace-corpus
  first_count="$(echo "$output" | jq 'length')"

  run bash "$UC" "$CFG" --repo-root "$WS/this-repo" --workspace-root "$WS"
  [ "$status" -eq 0 ]

  run bash "$TL" --show --json --source userspace-corpus
  second_count="$(echo "$output" | jq 'length')"
  [ "$first_count" -eq "$second_count" ]

  max_rec="$(echo "$output" | jq '[.[].recurrence] | max')"
  [ "$max_rec" -ge 2 ]
}

@test "empty workspace (no peer repos) exits 0 with no signals" {
  EMPTY_WS="$(mktemp -d -t discovery-userspace-corpus-empty.XXXXXX)"
  mkdir -p "$EMPTY_WS/this-repo"
  git init -q "$EMPTY_WS/this-repo"
  run bash "$UC" "$CFG" --repo-root "$EMPTY_WS/this-repo" --workspace-root "$EMPTY_WS"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
  rm -rf "$EMPTY_WS"
}
