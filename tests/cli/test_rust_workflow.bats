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
freebsd_vm = next(
    step for step in freebsd["steps"]
    if isinstance(step, dict) and step.get("uses") == "vmactions/freebsd-vm@v1"
)
assert freebsd_vm["with"]["usesh"] is True
assert freebsd_vm["with"]["run"].lstrip().startswith("set -eu\n")

for name in ("build-test", "macos-test", "windows-test", "freebsd-test"):
    job_commands = commands(jobs[name])
    for behavior in (
        "heartbeat",
        "process_owner",
        "portability",
    ):
        assert f"cargo test -p autospec-cli --bin autospec {behavior}" in job_commands

assert "cargo test -p autospec-cli --bin autospec released_predecessor_advances_through_executor_on_supported_host" in linux_commands
portable_admission = "commands::autonomous::executor_bridge::portability::supported_host_tests::supported_host_retires_predecessor_runs_noop_and_publishes_terminal_receipt"
for job_commands in (macos_commands, freebsd_commands):
    assert job_commands.count(f'cargo test -p autospec-cli --bin autospec "$admission_test" -- --exact --nocapture') == 1
    assert job_commands.count('grep -F -c "test $admission_test ... ok"') == 1
assert windows_commands.count('cargo test -p autospec-cli --bin autospec $admissionTest -- --exact --nocapture') == 1
assert windows_commands.count('[regex]::Matches($admissionOutput') == 1
for job_commands in (macos_commands, windows_commands, freebsd_commands):
    assert job_commands.count(portable_admission) == 1

for job_commands in (macos_commands, windows_commands, freebsd_commands):
    assert "cargo build --release -p autospec-cli" in job_commands
PY
}
