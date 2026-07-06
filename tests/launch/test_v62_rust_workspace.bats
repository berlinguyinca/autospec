#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "v62 rust workspace exposes autospec doctor json" {
  cd "$REPO_ROOT"

  [ -f Cargo.toml ]
  cargo run --quiet --bin autospec -- doctor --json > "$BATS_TEST_TMPDIR/doctor.json"

  jq -e '
    .status == "ok"
    and .workspace == "autospec"
    and (.checks | index("rust-core-workspace"))
  ' "$BATS_TEST_TMPDIR/doctor.json"
}
