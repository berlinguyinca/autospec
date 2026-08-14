#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  WORKFLOW="$REPO_ROOT/.github/workflows/rust.yml"
}

@test "rust workflow behavior-tests autospec-cli on all supported platforms" {
  python3 - "$WORKFLOW" <<'PY'
import pathlib
import sys
import yaml

workflow = yaml.safe_load(pathlib.Path(sys.argv[1]).read_text())
jobs = workflow["jobs"]

linux = jobs["build-test"]
assert linux["runs-on"] == "ubuntu-latest"
def commands(job):
    return "\n".join(
        "\n".join((step.get("run", ""), step.get("with", {}).get("run", "")))
        for step in job["steps"] if isinstance(step, dict)
    )

linux_commands = commands(linux)
assert "cargo clippy --workspace --all-targets" in linux_commands
assert "cargo build --workspace" in linux_commands
assert "cargo test -p autospec-core --lib" in linux_commands

macos = jobs["macos-test"]
assert macos["runs-on"] == "macos-latest"
macos_commands = commands(macos)
assert "cargo check -p autospec-cli" in macos_commands
assert "cargo build --release -p autospec-cli" in macos_commands

windows = jobs["windows-test"]
assert windows["runs-on"] == "windows-latest"
windows_commands = commands(windows)
assert "cargo check -p autospec-cli --tests --target x86_64-pc-windows-msvc" in windows_commands

freebsd = jobs["freebsd-test"]
freebsd_uses = "\n".join(
    step.get("uses", "") for step in freebsd["steps"] if isinstance(step, dict)
)
freebsd_commands = commands(freebsd)
assert "vmactions/freebsd-vm@v1" in freebsd_uses

for name in ("build-test", "macos-test", "windows-test", "freebsd-test"):
    job_commands = commands(jobs[name])
    for behavior in (
        "heartbeat",
        "process_owner",
        "portability",
        "released_predecessor_advances_through_executor_on_supported_host",
    ):
        assert f"cargo test -p autospec-cli --bin autospec {behavior}" in job_commands

for job_commands in (macos_commands, windows_commands, freebsd_commands):
    assert "cargo build --release -p autospec-cli" in job_commands
PY
}
