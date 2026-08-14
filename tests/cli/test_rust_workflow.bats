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

heartbeat = "commands::claim::heartbeat_portable::tests::publication_is_idempotent_but_rejects_another_generation"
freebsd_collision = "commands::claim::heartbeat_portable::tests::freebsd_atomic_publication_rejects_destination_collision"
freebsd_crash_recovery = "commands::claim::heartbeat_portable::tests::freebsd_publication_resumes_after_crash_between_link_and_stage_cleanup"
portable_admission = "commands::autonomous::executor_bridge::portability::supported_host_tests::supported_host_retires_predecessor_runs_noop_and_publishes_terminal_receipt"
for name in ("build-test", "macos-test", "windows-test", "freebsd-test"):
    job_commands = commands(jobs[name])
    assert job_commands.count(heartbeat) == 1
    assert job_commands.count(portable_admission) == 1
    assert "--exact --nocapture" in job_commands
    if name == "windows-test":
        assert "$ErrorActionPreference = 'Stop'" in job_commands
        assert job_commands.count("$LASTEXITCODE") >= 3
        assert "portable behavior test did not pass exactly once" in job_commands
        assert "process_owner::tests::windows_creation_filetime_is_part_of_durable_identity" in job_commands
    else:
        assert 'grep -F -c "test $behavior_test ... ok"' in job_commands

assert freebsd_commands.count(freebsd_collision) == 1
assert freebsd_commands.count(freebsd_crash_recovery) == 1

assert "autonomous_executor_bridge_pidfd_adoption_requires_full_exec_identity" in linux_commands
for job_commands in (macos_commands, freebsd_commands):
    assert "process_owner::tests::wait_preserves_zero_exit" in job_commands

for job_commands in (macos_commands, windows_commands, freebsd_commands):
    assert "cargo build --release -p autospec-cli" in job_commands
PY
}
