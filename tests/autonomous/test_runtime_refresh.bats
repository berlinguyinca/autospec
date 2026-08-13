#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  TEST_ROOT="$(mktemp -d "${BATS_TMPDIR:-/tmp}/runtime-refresh.XXXXXX")"
  TEST_HOME="$TEST_ROOT/home"
  FIXTURE_REPO="$TEST_ROOT/repo with spaces"
  mkdir -p "$TEST_HOME" "$FIXTURE_REPO/crates/demo/src" "$FIXTURE_REPO/.cargo"
  printf '[workspace]\nmembers=["crates/demo"]\n' >"$FIXTURE_REPO/Cargo.toml"
  printf '# lock\n' >"$FIXTURE_REPO/Cargo.lock"
  printf '[build]\ntarget-dir="target"\n' >"$FIXTURE_REPO/.cargo/config.toml"
  printf '[package]\nname="demo"\nversion="0.1.0"\n' >"$FIXTURE_REPO/crates/demo/Cargo.toml"
  printf 'pub const VALUE: u8 = 1;\n' >"$FIXTURE_REPO/crates/demo/src/lib.rs"
  printf 'fn main() { println!("cargo:rerun-if-changed=asset.bin"); }\n' >"$FIXTURE_REPO/crates/demo/build.rs"
  printf 'asset-one\n' >"$FIXTURE_REPO/crates/demo/asset.bin"
  printf '# docs\n' >"$FIXTURE_REPO/README.md"
  git -C "$FIXTURE_REPO" init -q
  git -C "$FIXTURE_REPO" config user.email test@example.com
  git -C "$FIXTURE_REPO" config user.name 'Runtime Test'
  git -C "$FIXTURE_REPO" add .
  git -C "$FIXTURE_REPO" commit -qm fixture
}

teardown() {
  rm -rf "$TEST_ROOT"
}

identity() {
  run env HOME="$TEST_HOME" bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" identity --repo-dir "$FIXTURE_REPO"
}

@test "identity is deterministic and covers manifests Rust and crate assets in one source snapshot" {
  identity
  [ "$status" -eq 0 ]
  first="$output"
  [[ "$first" =~ ^[0-9a-f]{64}$ ]]

  identity
  [ "$status" -eq 0 ]
  [ "$output" = "$first" ]

  printf 'asset-two\n' >"$FIXTURE_REPO/crates/demo/asset.bin"
  identity
  [ "$status" -eq 0 ]
  [ "$output" != "$first" ]
}

@test "identity includes staged unstaged untracked and deleted build inputs but excludes documentation" {
  identity
  baseline="$output"

  printf 'docs only\n' >>"$FIXTURE_REPO/README.md"
  printf 'notes\n' >"$FIXTURE_REPO/NOTES.md"
  identity
  [ "$output" = "$baseline" ]

  printf 'pub const STAGED: u8 = 2;\n' >>"$FIXTURE_REPO/crates/demo/src/lib.rs"
  git -C "$FIXTURE_REPO" add crates/demo/src/lib.rs
  identity
  staged="$output"
  [ "$staged" != "$baseline" ]

  printf 'pub const UNTRACKED: u8 = 3;\n' >"$FIXTURE_REPO/crates/demo/src/untracked.rs"
  identity
  untracked="$output"
  [ "$untracked" != "$staged" ]

  rm "$FIXTURE_REPO/crates/demo/Cargo.toml"
  identity
  [ "$output" != "$untracked" ]
}

@test "check fails closed for missing generation and ensure returns an exact verified generation" {
  run env HOME="$TEST_HOME" bash "$REPO_ROOT/scripts/autonomous-runtime-refresh.sh" check --repo-dir "$FIXTURE_REPO"
  [ "$status" -eq 10 ]
  [[ "$output" == *"stale:"* ]]
}
