#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  AUTOSPEC_BIN="${AUTOSPEC_TEST_BIN:-$REPO_ROOT/target/debug/autospec}"
  TMP="$(mktemp -d)"
  git -C "$TMP" init -q
  git -C "$TMP" remote add origin git@github.com:acme/widgets.git
  mkdir -p "$TMP/.autospec"
  cat > "$TMP/.autospec/autonomous.yml" <<'YAML'
project_board:
  mode: managed
  product_key: widgets
  owner: acme
  repo_allowlist: ["acme/*"]
  repository_seeds: ["acme/widgets"]
  discovery_max_repos: 10
YAML
}

teardown() {
  rm -rf "$TMP"
}

@test "project onboard dry-run reports admitted repositories without executing project files" {
  cat > "$TMP/package.json" <<'JSON'
{"scripts":{"postinstall":"exit 99"},"repository":"https://github.com/acme/widgets-ui"}
JSON

  run "$AUTOSPEC_BIN" project onboard --repo-dir "$TMP" --dry-run

  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r '.adopted')" -eq 1 ]
  [ "$(printf '%s' "$output" | jq -r '.repositories[].repository' | paste -sd ',' -)" = "acme/widgets,acme/widgets-ui" ]
}

@test "project onboard rejects candidates outside owner and allowlist boundaries" {
  cat > "$TMP/.gitmodules" <<'EOF'
[submodule "outside"]
url = https://github.com/other/private
EOF

  run "$AUTOSPEC_BIN" project onboard --repo-dir "$TMP" --dry-run

  [ "$status" -eq 0 ]
  [ "$(printf '%s' "$output" | jq -r '.out_of_bound')" -eq 1 ]
  ! printf '%s' "$output" | jq -e '.repositories[] | select(.repository == "other/private")' >/dev/null
}
