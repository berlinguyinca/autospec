from __future__ import annotations

import json
import multiprocessing as mp
import os
import stat
import sys
from functools import partial
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
SLURM = HERE.parent / "slurm"
sys.path.insert(0, str(SLURM))

import slot_lock as sl  # noqa: E402


CLAIM_A = "a" * 32
CLAIM_B = "b" * 32
ATTEMPT_A = "1" * 32
ATTEMPT_B = "2" * 32
JOB_A = "12345"
JOB_B = "67890"
SLOT = "cluster-user-01"


def _private_root(tmp_path: Path) -> Path:
    root = tmp_path / "leases"
    root.mkdir(mode=0o700)
    return root


def _attempt(
    claim_id: str,
    attempt_id: str,
    job_id: str | None,
    *,
    ordinal: int = 0,
    state: str | None = None,
) -> dict:
    return {
        "attempt_id": attempt_id,
        "ordinal": ordinal,
        "gpu": "gpu:rtx6000",
        "state": state or ("bound" if job_id is not None else "intent"),
        "serve_tag": (
            f"qwen-slurm-v1:slot={SLOT},claim={claim_id},"
            f"attempt={attempt_id},role=serve"
        ),
        "serve_job_id": job_id,
        "cleanup": {
            "tag": (
                f"qwen-slurm-v1:slot={SLOT},claim={claim_id},"
                f"attempt={attempt_id},role=cleanup"
            ),
            "state": "not_needed",
            "job_ids": [],
        },
    }


def _claim(root: Path, claim_id: str = CLAIM_A) -> sl.Lease:
    return sl.claim(root, SLOT, claim_id, os.geteuid(), "user")


def _activate(
    root: Path,
    claim_id: str = CLAIM_A,
    attempt_id: str = ATTEMPT_A,
    job_id: str = JOB_A,
) -> sl.Lease:
    lease = _claim(root, claim_id)

    def transition(state: dict) -> dict:
        state["phase"] = "active"
        state["active_attempt_id"] = attempt_id
        state["attempts"] = [_attempt(claim_id, attempt_id, job_id)]
        return state

    return sl.mutate(
        root,
        SLOT,
        sl.ExpectedState(lease.revision, claim_id, None, None),
        transition,
    )


def _claim_worker(
    root: str,
    slot: str,
    claim_id: str,
    start: mp.synchronize.Barrier,
    results: mp.queues.Queue,
) -> None:
    start.wait()
    try:
        lease = sl.claim(Path(root), slot, claim_id, os.geteuid(), "user")
    except sl.LeaseError:
        results.put(("refused", claim_id))
    else:
        results.put(("claimed", lease.claim_id))


def _crash_mutation_worker(root: str, when: str) -> None:
    real_replace = sl.os.replace

    def crash_replace(source, destination):
        if when == "before":
            os._exit(81)
        real_replace(source, destination)
        os._exit(82)

    sl.os.replace = crash_replace

    def transition(state: dict) -> dict:
        state["phase"] = "blocked"
        return state

    sl.mutate(
        Path(root),
        SLOT,
        sl.ExpectedState(1, CLAIM_A, None, None),
        transition,
    )


def _crash_release_worker(root: str, when: str) -> None:
    real_replace = sl.os.replace

    def crash_replace(source, destination):
        if when == "before":
            os._exit(83)
        real_replace(source, destination)
        os._exit(84)

    sl.os.replace = crash_replace
    sl.release_if_current(Path(root), SLOT, CLAIM_A, ATTEMPT_A, JOB_A)


def _replace_state(root: Path, content: bytes) -> Path:
    state_path = root / SLOT / "state.json"
    state_path.unlink()
    state_path.write_bytes(content)
    state_path.chmod(0o600)
    return state_path


def _read_raw(root: Path) -> dict:
    return json.loads((root / SLOT / "state.json").read_text())


def test_claim_creates_private_revisioned_claimed_state(tmp_path: Path):
    root = _private_root(tmp_path)

    lease = _claim(root)

    assert lease.revision == 1
    assert lease.claim_id == CLAIM_A
    assert lease.active_attempt_id is None
    assert lease.serve_job_id is None
    assert lease.state == _read_raw(root)
    assert lease.state["schema"] == "qwen-slurm-slot/v1"
    assert lease.state["phase"] == "claimed"
    assert lease.state["attempts"] == []
    assert stat.S_IMODE((root / SLOT).stat().st_mode) == 0o700
    assert stat.S_IMODE((root / SLOT / "state.json").stat().st_mode) == 0o600


def test_two_processes_claiming_one_slot_have_exactly_one_winner(tmp_path: Path):
    root = _private_root(tmp_path)
    ctx = mp.get_context("fork")
    start = ctx.Barrier(2)
    results = ctx.Queue()
    processes = [
        ctx.Process(
            target=_claim_worker,
            args=(str(root), SLOT, claim_id, start, results),
        )
        for claim_id in (CLAIM_A, CLAIM_B)
    ]

    for process in processes:
        process.start()
    for process in processes:
        process.join(10)
        assert process.exitcode == 0

    outcomes = sorted(results.get(timeout=2)[0] for _ in processes)
    assert outcomes == ["claimed", "refused"]
    assert sl.load(root, SLOT).claim_id in {CLAIM_A, CLAIM_B}


def test_two_processes_can_claim_independent_slots(tmp_path: Path):
    root = _private_root(tmp_path)
    ctx = mp.get_context("fork")
    start = ctx.Barrier(2)
    results = ctx.Queue()
    processes = [
        ctx.Process(
            target=_claim_worker,
            args=(str(root), slot, claim_id, start, results),
        )
        for slot, claim_id in (
            ("cluster-user-01", CLAIM_A),
            ("cluster-user-02", CLAIM_B),
        )
    ]

    for process in processes:
        process.start()
    for process in processes:
        process.join(10)
        assert process.exitcode == 0

    assert sorted(results.get(timeout=2)[0] for _ in processes) == [
        "claimed",
        "claimed",
    ]


def test_mutation_accepts_exact_nullable_prebind_state_and_increments_revision(
    tmp_path: Path,
):
    root = _private_root(tmp_path)
    lease = _claim(root)

    def add_intent(state: dict) -> dict:
        state["phase"] = "submitting"
        state["active_attempt_id"] = ATTEMPT_A
        state["attempts"] = [_attempt(CLAIM_A, ATTEMPT_A, None)]
        return state

    intent = sl.mutate(
        root,
        SLOT,
        sl.ExpectedState(lease.revision, CLAIM_A, None, None),
        add_intent,
    )
    bound = sl.mutate(
        root,
        SLOT,
        sl.ExpectedState(intent.revision, CLAIM_A, ATTEMPT_A, None),
        lambda state: {
            **state,
            "phase": "active",
            "attempts": [_attempt(CLAIM_A, ATTEMPT_A, JOB_A)],
        },
    )

    assert intent.revision == 2
    assert intent.serve_job_id is None
    assert bound.revision == 3
    assert bound.serve_job_id == JOB_A


@pytest.mark.parametrize(
    "expected",
    [
        sl.ExpectedState(99, CLAIM_A, ATTEMPT_A, JOB_A),
        sl.ExpectedState(2, CLAIM_B, ATTEMPT_A, JOB_A),
        sl.ExpectedState(2, CLAIM_A, ATTEMPT_B, JOB_A),
        sl.ExpectedState(2, CLAIM_A, ATTEMPT_A, JOB_B),
        sl.ExpectedState(2, CLAIM_A, None, JOB_A),
        sl.ExpectedState(2, CLAIM_A, ATTEMPT_A, None),
    ],
    ids=[
        "revision",
        "claim",
        "attempt",
        "job",
        "nullable-attempt",
        "nullable-job",
    ],
)
def test_each_exact_cas_field_rejects_independently(
    tmp_path: Path, expected: sl.ExpectedState
):
    root = _private_root(tmp_path)
    _activate(root)

    with pytest.raises(sl.LeaseError, match="compare-and-swap"):
        sl.mutate(root, SLOT, expected, lambda state: state)

    assert sl.load(root, SLOT).revision == 2


@pytest.mark.parametrize(
    "expected",
    [
        sl.ExpectedState(1, "", None, None),
        sl.ExpectedState(1, CLAIM_A, "", None),
        sl.ExpectedState(1, CLAIM_A, None, ""),
    ],
)
def test_empty_cas_identifiers_are_invalid(tmp_path: Path, expected: sl.ExpectedState):
    root = _private_root(tmp_path)
    _claim(root)

    with pytest.raises(sl.LeaseError, match="invalid expected state"):
        sl.mutate(root, SLOT, expected, lambda state: state)

    assert sl.load(root, SLOT).revision == 1


def test_expected_none_cannot_replace_an_existing_or_partial_slot(tmp_path: Path):
    root = _private_root(tmp_path)
    _claim(root)

    with pytest.raises(sl.LeaseError):
        sl.mutate(root, SLOT, None, lambda state: state)

    partial = root / "cluster-user-02"
    partial.mkdir(mode=0o700)
    with pytest.raises(sl.LeaseError):
        sl.claim(root, "cluster-user-02", CLAIM_B, os.geteuid(), "user")

    assert sl.load(root, SLOT).claim_id == CLAIM_A
    assert partial.is_dir()


def test_transition_validation_failure_preserves_current_generation(tmp_path: Path):
    root = _private_root(tmp_path)
    lease = _claim(root)

    with pytest.raises(sl.LeaseError, match="unknown field"):
        sl.mutate(
            root,
            SLOT,
            sl.ExpectedState(lease.revision, CLAIM_A, None, None),
            lambda state: {**state, "invented": True},
        )

    assert sl.load(root, SLOT).revision == 1


def test_old_attempt_cannot_release_a_replacement(tmp_path: Path):
    root = _private_root(tmp_path)
    old = _activate(root)

    def replace(state: dict) -> dict:
        state["phase"] = "replacing"
        state["retiring_attempt_id"] = ATTEMPT_A
        state["active_attempt_id"] = ATTEMPT_B
        state["attempts"] = [
            _attempt(CLAIM_A, ATTEMPT_A, JOB_A, state="cancel_requested"),
            _attempt(CLAIM_A, ATTEMPT_B, JOB_B, ordinal=1),
        ]
        return state

    replacement = sl.mutate(
        root,
        SLOT,
        sl.ExpectedState(old.revision, CLAIM_A, ATTEMPT_A, JOB_A),
        replace,
    )

    assert (
        sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A)
        == "stale_cleanup"
    )
    assert sl.load(root, SLOT).revision == replacement.revision


def test_old_claim_cannot_release_a_new_claim(tmp_path: Path):
    root = _private_root(tmp_path)
    _activate(root)
    assert sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A) == "released"
    _activate(root, CLAIM_B, ATTEMPT_B, JOB_B)

    assert (
        sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A)
        == "stale_cleanup"
    )
    assert sl.load(root, SLOT).claim_id == CLAIM_B


def test_duplicate_cleanup_is_idempotent_and_slot_can_be_reclaimed(tmp_path: Path):
    root = _private_root(tmp_path)
    _activate(root)

    assert sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A) == "released"
    assert (
        sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A)
        == "stale_cleanup"
    )
    assert not (root / SLOT).exists()
    assert sl.claim(root, SLOT, CLAIM_B, os.geteuid(), "user").claim_id == CLAIM_B


@pytest.mark.parametrize("target", ["root", "ancestor", "slot", "state", "mutex"])
def test_symlinks_at_every_shared_state_path_fail_closed(tmp_path: Path, target: str):
    if target == "root":
        actual = _private_root(tmp_path)
        root = tmp_path / "root-link"
        root.symlink_to(actual, target_is_directory=True)
        protected_path = root
        operation = partial(_claim, root)
    elif target == "ancestor":
        actual_parent = tmp_path / "actual"
        actual_parent.mkdir(mode=0o700)
        (actual_parent / "leases").mkdir(mode=0o700)
        linked_parent = tmp_path / "linked"
        linked_parent.symlink_to(actual_parent, target_is_directory=True)
        protected_path = linked_parent
        operation = partial(_claim, linked_parent / "leases")
    else:
        root = _private_root(tmp_path)
        if target == "slot":
            outside = tmp_path / "outside-slot"
            outside.mkdir(mode=0o700)
            (root / SLOT).symlink_to(outside, target_is_directory=True)
            protected_path = root / SLOT
            operation = partial(_claim, root)
        elif target == "state":
            _claim(root)
            state_path = root / SLOT / "state.json"
            state_path.unlink()
            outside = tmp_path / "outside-state"
            outside.write_text("{}")
            outside.chmod(0o600)
            state_path.symlink_to(outside)
            protected_path = state_path
            operation = partial(sl.load, root, SLOT)
        else:
            outside = tmp_path / "outside-mutex"
            outside.mkdir(mode=0o700)
            (root / f".{SLOT}.txn").symlink_to(outside, target_is_directory=True)
            protected_path = root / f".{SLOT}.txn"
            operation = partial(_claim, root)

    with pytest.raises(sl.LeaseError, match="unsafe|cannot prove stale"):
        operation()

    assert protected_path.is_symlink()


@pytest.mark.parametrize("which", ["root", "slot", "state"])
def test_wrong_shared_state_modes_fail_closed(tmp_path: Path, which: str):
    root = _private_root(tmp_path)
    if which == "root":
        root.chmod(0o755)
        path = root
        operation = partial(_claim, root)
    else:
        _claim(root)
        path = root / SLOT if which == "slot" else root / SLOT / "state.json"
        path.chmod(0o755 if which == "slot" else 0o644)
        operation = partial(sl.load, root, SLOT)

    with pytest.raises(sl.LeaseError, match="mode"):
        operation()

    assert stat.S_IMODE(path.stat().st_mode) != (0o700 if which != "state" else 0o600)


def test_filesystem_owner_mismatch_fails_closed(tmp_path: Path, monkeypatch):
    root = _private_root(tmp_path)
    actual_uid = os.geteuid()
    monkeypatch.setattr(sl.os, "geteuid", lambda: actual_uid + 1)

    with pytest.raises(sl.LeaseError, match="owner"):
        sl.claim(root, SLOT, CLAIM_A, actual_uid + 1, "user")

    assert root.stat().st_uid == actual_uid


def test_state_owner_metadata_must_match_effective_uid(tmp_path: Path):
    root = _private_root(tmp_path)
    _claim(root)
    doc = _read_raw(root)
    doc["owner"]["uid"] = os.geteuid() + 1
    _replace_state(root, json.dumps(doc).encode())

    with pytest.raises(sl.LeaseError, match="owner"):
        sl.load(root, SLOT)

    assert _read_raw(root)["owner"]["uid"] != os.geteuid()


def test_hard_linked_state_fails_closed(tmp_path: Path):
    root = _private_root(tmp_path)
    _claim(root)
    os.link(root / SLOT / "state.json", tmp_path / "second-link")

    with pytest.raises(sl.LeaseError, match="link"):
        sl.load(root, SLOT)

    assert (root / SLOT / "state.json").stat().st_nlink == 2


def test_non_regular_state_fails_without_opening_it(tmp_path: Path):
    root = _private_root(tmp_path)
    _claim(root)
    state_path = root / SLOT / "state.json"
    state_path.unlink()
    os.mkfifo(state_path, mode=0o600)

    with pytest.raises(sl.LeaseError, match="regular"):
        sl.load(root, SLOT)

    assert stat.S_ISFIFO(state_path.lstat().st_mode)


@pytest.mark.parametrize(
    ("content", "message"),
    [
        (b"{" + b" " * (sl.MAX_STATE_BYTES + 1), "too large"),
        (b'{"schema":"qwen-slurm-slot/v1"', "JSON"),
        (
            b'{"schema":"qwen-slurm-slot/v1","schema":"qwen-slurm-slot/v1"}',
            "duplicate",
        ),
    ],
)
def test_oversized_truncated_and_duplicate_key_json_fail_closed(
    tmp_path: Path, content: bytes, message: str
):
    root = _private_root(tmp_path)
    _claim(root)
    _replace_state(root, content)

    with pytest.raises(sl.LeaseError, match=message):
        sl.load(root, SLOT)

    assert (root / SLOT / "state.json").read_bytes() == content


@pytest.mark.parametrize("corruption", ["schema", "top-level", "nested"])
def test_unknown_schema_or_fields_fail_closed(tmp_path: Path, corruption: str):
    root = _private_root(tmp_path)
    _claim(root)
    doc = _read_raw(root)
    if corruption == "schema":
        doc["schema"] = "qwen-slurm-slot/v2"
    elif corruption == "top-level":
        doc["future"] = True
    else:
        doc["owner"]["future"] = True
    _replace_state(root, json.dumps(doc).encode())

    with pytest.raises(sl.LeaseError):
        sl.load(root, SLOT)

    assert (root / SLOT / "state.json").is_file()


@pytest.mark.parametrize("duplicate", ["attempt_id", "serve_tag", "cleanup_tag", "job_id"])
def test_attempt_records_reject_duplicate_identities(tmp_path: Path, duplicate: str):
    root = _private_root(tmp_path)
    lease = _claim(root)
    first = _attempt(CLAIM_A, ATTEMPT_A, JOB_A)
    second = _attempt(CLAIM_A, ATTEMPT_B, JOB_B, ordinal=1)
    if duplicate == "attempt_id":
        second["attempt_id"] = first["attempt_id"]
    elif duplicate == "serve_tag":
        second["serve_tag"] = first["serve_tag"]
    elif duplicate == "cleanup_tag":
        second["cleanup"]["tag"] = first["cleanup"]["tag"]
    else:
        first["cleanup"] = {**first["cleanup"], "state": "bound", "job_ids": ["44"]}
        second["cleanup"] = {**second["cleanup"], "state": "bound", "job_ids": ["44"]}

    def transition(state: dict) -> dict:
        state["phase"] = "active"
        state["active_attempt_id"] = ATTEMPT_A
        state["attempts"] = [first, second]
        return state

    with pytest.raises(sl.LeaseError, match="duplicate"):
        sl.mutate(
            root,
            SLOT,
            sl.ExpectedState(lease.revision, CLAIM_A, None, None),
            transition,
        )

    assert sl.load(root, SLOT).revision == 1


@pytest.mark.parametrize("when", ["before", "after"])
def test_process_crash_around_state_replace_recovers_without_torn_state(
    tmp_path: Path, when: str
):
    root = _private_root(tmp_path)
    _claim(root)
    ctx = mp.get_context("fork")
    process = ctx.Process(target=_crash_mutation_worker, args=(str(root), when))

    process.start()
    process.join(10)

    assert process.exitcode == (81 if when == "before" else 82)
    observed = sl.load(root, SLOT)
    assert (observed.revision, observed.state["phase"]) == (
        (1, "claimed") if when == "before" else (2, "blocked")
    )
    recovered = sl.mutate(
        root,
        SLOT,
        sl.ExpectedState(
            observed.revision,
            CLAIM_A,
            observed.active_attempt_id,
            observed.serve_job_id,
        ),
        lambda state: {**state, "phase": "stopping"},
    )
    assert recovered.revision == observed.revision + 1
    assert not (root / f".{SLOT}.txn").exists()
    assert not list((root / SLOT).glob(".state.*.tmp"))


@pytest.mark.parametrize("when", ["before", "after"])
def test_process_crash_around_release_rename_is_recoverable(
    tmp_path: Path, when: str
):
    root = _private_root(tmp_path)
    _activate(root)
    ctx = mp.get_context("fork")
    process = ctx.Process(target=_crash_release_worker, args=(str(root), when))

    process.start()
    process.join(10)

    assert process.exitcode == (83 if when == "before" else 84)
    if when == "after":
        assert not (root / SLOT).exists()
        assert list(root.glob(f".{SLOT}.tombstone.*"))
        expected = "stale_cleanup"
    else:
        assert (root / SLOT).exists()
        expected = "released"
    assert sl.release_if_current(root, SLOT, CLAIM_A, ATTEMPT_A, JOB_A) == expected
    assert not list(root.glob(f".{SLOT}.tombstone.*"))


def test_unprovably_stale_mutex_is_preserved_and_fails_closed(tmp_path: Path):
    root = _private_root(tmp_path)
    mutex = root / f".{SLOT}.txn"
    mutex.mkdir(mode=0o700)
    owner = mutex / "owner.json"
    owner.write_text(
        json.dumps(
            {
                "schema": "qwen-slurm-slot-txn/v1",
                "host": "another-host.example.test",
                "boot_id": "0" * 36,
                "pid": 4242,
                "start_ticks": 99,
                "uid": os.geteuid(),
                "nonce": "f" * 32,
            }
        )
    )
    owner.chmod(0o600)

    with pytest.raises(sl.LeaseError, match="cannot prove stale"):
        _claim(root)

    assert mutex.is_dir()


def test_malformed_mutex_is_preserved_and_fails_closed(tmp_path: Path):
    root = _private_root(tmp_path)
    mutex = root / f".{SLOT}.txn"
    mutex.mkdir(mode=0o700)

    with pytest.raises(sl.LeaseError, match="cannot prove stale"):
        _claim(root)

    assert mutex.is_dir()


def test_tombstone_symlink_is_not_followed_or_deleted(tmp_path: Path):
    root = _private_root(tmp_path)
    outside = tmp_path / "outside"
    outside.mkdir(mode=0o700)
    tombstone = root / f".{SLOT}.tombstone.1.{CLAIM_A}"
    tombstone.symlink_to(outside, target_is_directory=True)

    with pytest.raises(sl.LeaseError, match="tombstone"):
        _claim(root)

    assert tombstone.is_symlink()
    assert outside.is_dir()


def test_cli_cleanup_releases_current_generation_with_redacted_output(
    tmp_path: Path, capsys
):
    root = _private_root(tmp_path)
    _activate(root)

    result = sl.main(
        [
            "cleanup",
            "--root",
            str(root),
            "--slot",
            SLOT,
            "--claim",
            CLAIM_A,
            "--attempt",
            ATTEMPT_A,
            "--job-id",
            JOB_A,
        ]
    )

    captured = capsys.readouterr()
    assert result == 0
    assert json.loads(captured.out) == {"result": "released"}
    assert captured.err == ""
    for secret in (CLAIM_A, ATTEMPT_A, JOB_A, str(root)):
        assert secret not in captured.out + captured.err


@pytest.mark.parametrize(
    ("claim_id", "attempt_id", "job_id"),
    [
        (CLAIM_B, ATTEMPT_A, JOB_A),
        (CLAIM_A, ATTEMPT_B, JOB_A),
        (CLAIM_A, ATTEMPT_A, JOB_B),
    ],
)
def test_cli_cleanup_mismatch_is_stale_noop(
    tmp_path: Path, capsys, claim_id: str, attempt_id: str, job_id: str
):
    root = _private_root(tmp_path)
    active = _activate(root)

    result = sl.main(
        [
            "cleanup",
            "--root",
            str(root),
            "--slot",
            SLOT,
            "--claim",
            claim_id,
            "--attempt",
            attempt_id,
            "--job-id",
            job_id,
        ]
    )

    captured = capsys.readouterr()
    assert result == 0
    assert json.loads(captured.out) == {"result": "stale_cleanup"}
    assert sl.load(root, SLOT).revision == active.revision


def test_cli_cleanup_duplicate_invocation_is_stale_noop(tmp_path: Path, capsys):
    root = _private_root(tmp_path)
    _activate(root)
    argv = [
        "cleanup",
        "--root",
        str(root),
        "--slot",
        SLOT,
        "--claim",
        CLAIM_A,
        "--attempt",
        ATTEMPT_A,
        "--job-id",
        JOB_A,
    ]

    assert sl.main(argv) == 0
    capsys.readouterr()
    assert sl.main(argv) == 0

    assert json.loads(capsys.readouterr().out) == {"result": "stale_cleanup"}


@pytest.mark.parametrize(
    "argv",
    [
        [],
        ["cleanup"],
        ["cleanup", "--root", "/tmp", "--slot", "../bad"],
        [
            "cleanup",
            "--root",
            "/tmp",
            "--slot",
            SLOT,
            "--claim",
            "",
            "--attempt",
            ATTEMPT_A,
            "--job-id",
            JOB_A,
        ],
    ],
)
def test_cli_malformed_arguments_return_two_without_echoing_values(argv, capsys):
    result = sl.main(argv)

    captured = capsys.readouterr()
    assert result == 2
    assert captured.out == ""
    assert "usage:" in captured.err
    assert CLAIM_A not in captured.err
    assert ATTEMPT_A not in captured.err
    assert JOB_A not in captured.err
