#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  WORKFLOW="$REPO_ROOT/.github/workflows/rust.yml"
}

@test "rust workflow keeps Linux coverage and builds autospec-cli on macOS" {
  python3 - "$WORKFLOW" <<'PY'
import pathlib
import sys
import yaml

workflow = yaml.safe_load(pathlib.Path(sys.argv[1]).read_text())
jobs = workflow["jobs"]

linux = jobs["build-test"]
assert linux["runs-on"] == "ubuntu-latest"
linux_commands = "\n".join(
    step.get("run", "") for step in linux["steps"] if isinstance(step, dict)
)
assert "cargo clippy --workspace --all-targets" in linux_commands
assert "cargo build --workspace" in linux_commands
assert "cargo test -p autospec-core --lib" in linux_commands

macos = jobs["macos-build"]
assert macos["runs-on"] == "macos-latest"
macos_commands = "\n".join(
    step.get("run", "") for step in macos["steps"] if isinstance(step, dict)
)
assert "cargo check -p autospec-cli" in macos_commands
assert "cargo build --release -p autospec-cli" in macos_commands
PY
}
