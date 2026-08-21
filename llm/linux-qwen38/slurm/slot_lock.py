#!/usr/bin/env python3
"""Secure, revisioned shared-filesystem leases for logical Slurm slots."""

from __future__ import annotations

import argparse
import copy
import json
import os
import re
import socket
import stat
import sys
import uuid
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Iterator


SCHEMA = "qwen-slurm-slot/v1"
TXN_SCHEMA = "qwen-slurm-slot-txn/v1"
MAX_STATE_BYTES = 256 * 1024
MAX_ATTEMPTS = 32
MAX_CLEANUP_JOBS = 32

_SLOT_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")
_HEX_ID_RE = re.compile(r"^[0-9a-f]{32}$")
_JOB_ID_RE = re.compile(r"^[1-9][0-9]*$")
_BOOT_ID_RE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
_PHASES = frozenset(
    {"claimed", "submitting", "active", "replacing", "stopping", "blocked"}
)
_ATTEMPT_STATES = frozenset(
    {"intent", "submit_unknown", "bound", "cancel_requested", "terminal"}
)
_CLEANUP_STATES = frozenset(
    {"needed", "submit_unknown", "bound", "observed", "not_needed"}
)
_TOP_FIELDS = frozenset(
    {
        "schema",
        "slot",
        "claim_id",
        "owner",
        "revision",
        "phase",
        "active_attempt_id",
        "retiring_attempt_id",
        "attempts",
        "created_at",
        "updated_at",
    }
)
_OWNER_FIELDS = frozenset({"uid", "user"})
_ATTEMPT_FIELDS = frozenset(
    {
        "attempt_id",
        "ordinal",
        "gpu",
        "state",
        "serve_tag",
        "serve_job_id",
        "cleanup",
    }
)
_CLEANUP_FIELDS = frozenset({"tag", "state", "job_ids"})
_TXN_FIELDS = frozenset(
    {"schema", "host", "boot_id", "pid", "start_ticks", "uid", "nonce"}
)


class LeaseError(RuntimeError):
    """The requested lease operation could not be proven safe."""


@dataclass(frozen=True)
class ExpectedState:
    revision: int
    claim_id: str
    attempt_id: str | None
    serve_job_id: str | None


@dataclass(frozen=True)
class Lease:
    path: Path
    state: dict

    @property
    def revision(self) -> int:
        return self.state["revision"]

    @property
    def claim_id(self) -> str:
        return self.state["claim_id"]

    @property
    def active_attempt_id(self) -> str | None:
        return self.state["active_attempt_id"]

    @property
    def serve_job_id(self) -> str | None:
        return _active_job_id(self.state)


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _lexists(path: Path) -> bool:
    try:
        os.lstat(path)
    except FileNotFoundError:
        return False
    return True


def _absolute_without_resolving(path: Path) -> Path:
    return path if path.is_absolute() else Path.cwd() / path


def _reject_symlink_components(path: Path) -> None:
    absolute = _absolute_without_resolving(path)
    current = Path(absolute.anchor)
    for component in absolute.parts[1:]:
        current /= component
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode):
            raise LeaseError("unsafe symlink in shared state path")


def _validate_slot(slot: str) -> None:
    if not isinstance(slot, str) or not _SLOT_RE.fullmatch(slot):
        raise LeaseError("invalid slot identifier")


def _validate_hex_id(value: object, label: str, *, nullable: bool = False) -> None:
    if value is None and nullable:
        return
    if not isinstance(value, str) or not _HEX_ID_RE.fullmatch(value):
        raise LeaseError(f"invalid {label}")


def _validate_job_id(value: object, label: str, *, nullable: bool = False) -> None:
    if value is None and nullable:
        return
    if not isinstance(value, str) or not _JOB_ID_RE.fullmatch(value):
        raise LeaseError(f"invalid {label}")


def _validate_text(value: object, label: str, maximum: int = 128) -> None:
    if (
        not isinstance(value, str)
        or not value
        or len(value) > maximum
        or any(ord(character) < 0x20 for character in value)
    ):
        raise LeaseError(f"invalid {label}")


def _validate_directory(path: Path, label: str) -> os.stat_result:
    try:
        before = os.lstat(path)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory is unavailable") from exc
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISDIR(before.st_mode):
        raise LeaseError(f"unsafe {label}: expected a directory")
    if stat.S_IMODE(before.st_mode) != 0o700:
        raise LeaseError(f"unsafe {label}: directory mode must be 0700")
    if before.st_uid != os.geteuid():
        raise LeaseError(f"unsafe {label}: directory owner mismatch")

    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory cannot be opened") from exc
    try:
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
        raise LeaseError(f"unsafe {label}: directory changed during validation")
    return before


def _validate_root(root: Path) -> None:
    _reject_symlink_components(root)
    _validate_directory(root, "lease root")


def _fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
        os.fsync(descriptor)
    except OSError as exc:
        raise LeaseError("shared state directory fsync failed") from exc
    finally:
        if "descriptor" in locals():
            os.close(descriptor)


def _strict_object(pairs: list[tuple[str, object]]) -> dict:
    result: dict = {}
    for key, value in pairs:
        if key in result:
            raise LeaseError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _decode_json(payload: bytes, label: str) -> dict:
    try:
        value = json.loads(payload.decode("utf-8"), object_pairs_hook=_strict_object)
    except LeaseError:
        raise
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise LeaseError(f"{label} must be valid UTF-8 JSON") from exc
    if not isinstance(value, dict):
        raise LeaseError(f"{label} must be a JSON object")
    return value


def _read_secure_file(path: Path, label: str, maximum: int) -> bytes:
    try:
        before = os.lstat(path)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: file is unavailable") from exc
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        raise LeaseError(f"unsafe {label}: expected a regular file")
    if stat.S_IMODE(before.st_mode) != 0o600:
        raise LeaseError(f"unsafe {label}: file mode must be 0600")
    if before.st_uid != os.geteuid():
        raise LeaseError(f"unsafe {label}: file owner mismatch")
    if before.st_nlink != 1:
        raise LeaseError(f"unsafe {label}: file link count must be one")
    if before.st_size > maximum:
        raise LeaseError(f"unsafe {label}: file is too large")
    if before.st_size == 0:
        raise LeaseError(f"unsafe {label}: file is empty")

    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: file cannot be opened") from exc
    try:
        after = os.fstat(descriptor)
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise LeaseError(f"unsafe {label}: file changed during validation")
        chunks: list[bytes] = []
        remaining = maximum + 1
        while remaining:
            chunk = os.read(descriptor, min(64 * 1024, remaining))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        payload = b"".join(chunks)
        if len(payload) > maximum:
            raise LeaseError(f"unsafe {label}: file is too large")
        if len(payload) != after.st_size:
            raise LeaseError(f"unsafe {label}: file changed while being read")
        return payload
    finally:
        os.close(descriptor)


def _require_exact_fields(value: dict, expected: frozenset[str], label: str) -> None:
    keys = set(value)
    unknown = sorted(keys - expected)
    missing = sorted(expected - keys)
    if unknown:
        raise LeaseError(f"{label} contains unknown field: {unknown[0]}")
    if missing:
        raise LeaseError(f"{label} is missing field: {missing[0]}")


def _validate_timestamp(value: object, label: str) -> None:
    if not isinstance(value, str) or len(value) > 40:
        raise LeaseError(f"invalid {label}")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise LeaseError(f"invalid {label}") from exc
    if parsed.tzinfo is None:
        raise LeaseError(f"invalid {label}")


def _serve_tag(slot: str, claim_id: str, attempt_id: str) -> str:
    return (
        f"qwen-slurm-v1:slot={slot},claim={claim_id},"
        f"attempt={attempt_id},role=serve"
    )


def _cleanup_tag(slot: str, claim_id: str, attempt_id: str) -> str:
    return (
        f"qwen-slurm-v1:slot={slot},claim={claim_id},"
        f"attempt={attempt_id},role=cleanup"
    )


def _validate_state(state: dict, expected_slot: str) -> None:
    _require_exact_fields(state, _TOP_FIELDS, "slot state")
    if state["schema"] != SCHEMA:
        raise LeaseError("unknown slot state schema")
    if state["slot"] != expected_slot:
        raise LeaseError("slot state names another slot")
    _validate_hex_id(state["claim_id"], "claim id")

    owner = state["owner"]
    if not isinstance(owner, dict):
        raise LeaseError("slot state owner must be an object")
    _require_exact_fields(owner, _OWNER_FIELDS, "slot owner")
    if (
        not isinstance(owner["uid"], int)
        or isinstance(owner["uid"], bool)
        or owner["uid"] != os.geteuid()
    ):
        raise LeaseError("slot state owner does not match effective owner")
    _validate_text(owner["user"], "owner user", 64)

    revision = state["revision"]
    if not isinstance(revision, int) or isinstance(revision, bool) or revision < 1:
        raise LeaseError("invalid slot revision")
    if state["phase"] not in _PHASES:
        raise LeaseError("invalid slot phase")
    _validate_hex_id(state["active_attempt_id"], "active attempt id", nullable=True)
    _validate_hex_id(state["retiring_attempt_id"], "retiring attempt id", nullable=True)
    if (
        state["active_attempt_id"] is not None
        and state["active_attempt_id"] == state["retiring_attempt_id"]
    ):
        raise LeaseError("active and retiring attempts must differ")
    _validate_timestamp(state["created_at"], "created_at")
    _validate_timestamp(state["updated_at"], "updated_at")

    attempts = state["attempts"]
    if not isinstance(attempts, list) or len(attempts) > MAX_ATTEMPTS:
        raise LeaseError("slot attempts must be a bounded list")
    if not all(isinstance(attempt, dict) for attempt in attempts):
        raise LeaseError("each slot attempt must be an object")

    attempt_ids = [attempt.get("attempt_id") for attempt in attempts]
    serve_tags = [attempt.get("serve_tag") for attempt in attempts]
    cleanup_tags = [
        attempt.get("cleanup", {}).get("tag")
        if isinstance(attempt.get("cleanup"), dict)
        else None
        for attempt in attempts
    ]
    ordinals = [attempt.get("ordinal") for attempt in attempts]
    cleanup_job_ids = [
        job_id
        for attempt in attempts
        if isinstance(attempt.get("cleanup"), dict)
        and isinstance(attempt["cleanup"].get("job_ids"), list)
        for job_id in attempt["cleanup"]["job_ids"]
    ]
    for values, label in (
        (attempt_ids, "attempt id"),
        (serve_tags, "serve tag"),
        (cleanup_tags, "cleanup tag"),
        (ordinals, "attempt ordinal"),
        (cleanup_job_ids, "cleanup job id"),
    ):
        if len(values) != len(set(values)):
            raise LeaseError(f"duplicate {label} in slot state")

    known_attempts: set[str] = set()
    for attempt in attempts:
        _require_exact_fields(attempt, _ATTEMPT_FIELDS, "slot attempt")
        attempt_id = attempt["attempt_id"]
        _validate_hex_id(attempt_id, "attempt id")
        known_attempts.add(attempt_id)
        ordinal = attempt["ordinal"]
        if not isinstance(ordinal, int) or isinstance(ordinal, bool) or ordinal < 0:
            raise LeaseError("invalid attempt ordinal")
        _validate_text(attempt["gpu"], "attempt gpu")
        if attempt["state"] not in _ATTEMPT_STATES:
            raise LeaseError("invalid attempt state")
        if attempt["serve_tag"] != _serve_tag(expected_slot, state["claim_id"], attempt_id):
            raise LeaseError("invalid serve tag")
        _validate_job_id(attempt["serve_job_id"], "serve job id", nullable=True)

        cleanup = attempt["cleanup"]
        if not isinstance(cleanup, dict):
            raise LeaseError("attempt cleanup must be an object")
        _require_exact_fields(cleanup, _CLEANUP_FIELDS, "attempt cleanup")
        if cleanup["tag"] != _cleanup_tag(expected_slot, state["claim_id"], attempt_id):
            raise LeaseError("invalid cleanup tag")
        if cleanup["state"] not in _CLEANUP_STATES:
            raise LeaseError("invalid cleanup state")
        jobs = cleanup["job_ids"]
        if not isinstance(jobs, list) or len(jobs) > MAX_CLEANUP_JOBS:
            raise LeaseError("cleanup job ids must be a bounded list")
        for job_id in jobs:
            _validate_job_id(job_id, "cleanup job id")

    for field in ("active_attempt_id", "retiring_attempt_id"):
        attempt_id = state[field]
        if attempt_id is not None and attempt_id not in known_attempts:
            raise LeaseError(f"{field} does not name an attempt record")


def _state_bytes(state: dict, slot: str) -> bytes:
    _validate_state(state, slot)
    payload = (json.dumps(state, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(payload) > MAX_STATE_BYTES:
        raise LeaseError("slot state is too large")
    return payload


def _active_job_id(state: dict) -> str | None:
    active = state["active_attempt_id"]
    if active is None:
        return None
    for attempt in state["attempts"]:
        if attempt["attempt_id"] == active:
            return attempt["serve_job_id"]
    raise LeaseError("active attempt record is missing")


def _load_state(slot_path: Path, slot: str) -> dict:
    _validate_directory(slot_path, "slot directory")
    state_path = slot_path / "state.json"
    payload = _read_secure_file(state_path, "slot state", MAX_STATE_BYTES)
    state = _decode_json(payload, "slot state")
    _validate_state(state, slot)
    return state


def load(root: Path, slot: str) -> Lease:
    root = Path(root)
    _validate_slot(slot)
    _validate_root(root)
    slot_path = root / slot
    state = _load_state(slot_path, slot)
    return Lease(slot_path / "state.json", copy.deepcopy(state))


def _write_exclusive(path: Path, payload: bytes, label: str) -> None:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    descriptor: int | None = None
    try:
        descriptor = os.open(path, flags, 0o600)
        os.fchmod(descriptor, 0o600)
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("short write")
            view = view[written:]
        os.fsync(descriptor)
    except OSError as exc:
        raise LeaseError(f"failed to write {label}") from exc
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _safe_unlink_regular(path: Path, label: str) -> None:
    _read_secure_file(path, label, MAX_STATE_BYTES)
    try:
        path.unlink()
    except OSError as exc:
        raise LeaseError(f"failed to remove {label}") from exc


def _atomic_write_state(slot_path: Path, state: dict, slot: str) -> None:
    payload = _state_bytes(state, slot)
    temp = slot_path / f".state.{state['revision']}.{uuid.uuid4().hex}.tmp"
    _write_exclusive(temp, payload, "temporary slot state")
    try:
        os.replace(temp, slot_path / "state.json")
        _fsync_directory(slot_path)
    except BaseException:
        if _lexists(temp):
            try:
                _safe_unlink_regular(temp, "temporary slot state")
            except LeaseError:
                pass
        raise


def _process_start_ticks(pid: int) -> int | None:
    try:
        content = Path(f"/proc/{pid}/stat").read_text()
    except (FileNotFoundError, ProcessLookupError):
        return None
    except OSError as exc:
        raise LeaseError("cannot prove stale transaction mutex") from exc
    closing = content.rfind(")")
    if closing < 0:
        raise LeaseError("cannot prove stale transaction mutex")
    fields = content[closing + 2 :].split()
    if len(fields) < 20:
        raise LeaseError("cannot prove stale transaction mutex")
    try:
        return int(fields[19])
    except ValueError as exc:
        raise LeaseError("cannot prove stale transaction mutex") from exc


def _boot_id() -> str:
    try:
        value = Path("/proc/sys/kernel/random/boot_id").read_text().strip().lower()
    except OSError as exc:
        raise LeaseError("cannot establish process generation") from exc
    if not _BOOT_ID_RE.fullmatch(value):
        raise LeaseError("cannot establish process generation")
    return value


def _txn_document(nonce: str) -> dict:
    pid = os.getpid()
    start_ticks = _process_start_ticks(pid)
    if start_ticks is None:
        raise LeaseError("cannot establish process generation")
    return {
        "schema": TXN_SCHEMA,
        "host": socket.gethostname(),
        "boot_id": _boot_id(),
        "pid": pid,
        "start_ticks": start_ticks,
        "uid": os.geteuid(),
        "nonce": nonce,
    }


def _validate_txn_document(document: dict) -> None:
    _require_exact_fields(document, _TXN_FIELDS, "transaction owner")
    if document["schema"] != TXN_SCHEMA:
        raise LeaseError("cannot prove stale transaction mutex")
    _validate_text(document["host"], "transaction host", 255)
    if not isinstance(document["boot_id"], str) or not _BOOT_ID_RE.fullmatch(
        document["boot_id"]
    ):
        raise LeaseError("cannot prove stale transaction mutex")
    for field in ("pid", "start_ticks", "uid"):
        if (
            not isinstance(document[field], int)
            or isinstance(document[field], bool)
            or document[field] < 0
        ):
            raise LeaseError("cannot prove stale transaction mutex")
    _validate_hex_id(document["nonce"], "transaction nonce")


def _read_txn_document(mutex: Path) -> dict:
    try:
        _validate_directory(mutex, "transaction mutex")
        entries = os.listdir(mutex)
        if entries != ["owner.json"]:
            raise LeaseError("cannot prove stale transaction mutex")
        document = _decode_json(
            _read_secure_file(mutex / "owner.json", "transaction owner", 4096),
            "transaction owner",
        )
        _validate_txn_document(document)
        return document
    except LeaseError as exc:
        if "cannot prove stale" in str(exc):
            raise
        raise LeaseError("cannot prove stale transaction mutex") from exc


def _remove_txn(mutex: Path, expected_nonce: str | None) -> None:
    document = _read_txn_document(mutex)
    if expected_nonce is not None and document["nonce"] != expected_nonce:
        raise LeaseError("transaction mutex ownership changed")
    _safe_unlink_regular(mutex / "owner.json", "transaction owner")
    try:
        mutex.rmdir()
    except OSError as exc:
        raise LeaseError("transaction mutex could not be removed safely") from exc


def _recover_existing_txn(mutex: Path) -> None:
    document = _read_txn_document(mutex)
    if document["host"] != socket.gethostname():
        raise LeaseError("cannot prove stale transaction mutex on another host")
    current_boot = _boot_id()
    if document["boot_id"] == current_boot:
        observed_start = _process_start_ticks(document["pid"])
        if observed_start == document["start_ticks"]:
            raise LeaseError("slot mutation already in progress")
    _remove_txn(mutex, document["nonce"])


def _acquire_txn(root: Path, slot: str) -> tuple[Path, str]:
    mutex = root / f".{slot}.txn"
    nonce = uuid.uuid4().hex
    for attempt in range(2):
        try:
            os.mkdir(mutex, 0o700)
        except FileExistsError:
            if attempt:
                raise LeaseError("slot transaction mutex remained occupied")
            _recover_existing_txn(mutex)
            continue
        except OSError as exc:
            raise LeaseError("unsafe transaction mutex creation failed") from exc

        try:
            _validate_directory(mutex, "transaction mutex")
            payload = (
                json.dumps(_txn_document(nonce), sort_keys=True, separators=(",", ":"))
                + "\n"
            ).encode()
            _write_exclusive(mutex / "owner.json", payload, "transaction owner")
            _fsync_directory(mutex)
            _fsync_directory(root)
            return mutex, nonce
        except BaseException:
            try:
                if _lexists(mutex / "owner.json"):
                    _safe_unlink_regular(mutex / "owner.json", "transaction owner")
                mutex.rmdir()
            except (LeaseError, OSError):
                pass
            raise
    raise LeaseError("slot transaction mutex could not be acquired")


@contextmanager
def _mutation(root: Path, slot: str) -> Iterator[None]:
    _validate_root(root)
    mutex, nonce = _acquire_txn(root, slot)
    try:
        _recover_tombstones(root, slot)
        _recover_temporary_state(root / slot, slot)
        yield
    finally:
        _remove_txn(mutex, nonce)
        _fsync_directory(root)


def _recover_temporary_state(slot_path: Path, slot: str) -> None:
    if not _lexists(slot_path):
        return
    _validate_directory(slot_path, "slot directory")
    try:
        entries = os.listdir(slot_path)
    except OSError as exc:
        raise LeaseError("unsafe slot directory cannot be inspected") from exc
    temps = [name for name in entries if name.startswith(".state.") and name.endswith(".tmp")]
    unknown = set(entries) - {"state.json", *temps}
    if unknown:
        raise LeaseError("unsafe unknown entry in slot directory")
    if "state.json" not in entries:
        if temps:
            raise LeaseError("unsafe incomplete initial slot transaction")
        return
    _load_state(slot_path, slot)
    for name in temps:
        _safe_unlink_regular(slot_path / name, "temporary slot state")
    if temps:
        _fsync_directory(slot_path)


def _tombstone_pattern(slot: str) -> re.Pattern[str]:
    return re.compile(
        rf"^\.{re.escape(slot)}\.tombstone\.([1-9][0-9]*)\.([0-9a-f]{{32}})$"
    )


def _recover_tombstones(root: Path, slot: str) -> None:
    prefix = f".{slot}.tombstone."
    names = [name for name in os.listdir(root) if name.startswith(prefix)]
    if not names:
        return
    if len(names) != 1 or _lexists(root / slot):
        raise LeaseError("unsafe ambiguous slot tombstone state")
    name = names[0]
    match = _tombstone_pattern(slot).fullmatch(name)
    if match is None:
        raise LeaseError("unsafe malformed slot tombstone")
    tombstone = root / name
    try:
        state = _load_state(tombstone, slot)
    except LeaseError as exc:
        raise LeaseError("unsafe slot tombstone") from exc
    if state["revision"] != int(match.group(1)) or state["claim_id"] != match.group(2):
        raise LeaseError("unsafe slot tombstone generation mismatch")
    entries = os.listdir(tombstone)
    if entries != ["state.json"]:
        raise LeaseError("unsafe slot tombstone contents")
    _safe_unlink_regular(tombstone / "state.json", "slot tombstone state")
    try:
        tombstone.rmdir()
    except OSError as exc:
        raise LeaseError("slot tombstone could not be removed safely") from exc
    _fsync_directory(root)


def _validate_expected(expected: ExpectedState) -> None:
    if (
        not isinstance(expected, ExpectedState)
        or not isinstance(expected.revision, int)
        or isinstance(expected.revision, bool)
        or expected.revision < 1
    ):
        raise LeaseError("invalid expected state")
    try:
        _validate_hex_id(expected.claim_id, "expected claim id")
        _validate_hex_id(expected.attempt_id, "expected attempt id", nullable=True)
        _validate_job_id(expected.serve_job_id, "expected serve job id", nullable=True)
    except LeaseError as exc:
        raise LeaseError("invalid expected state") from exc


def _matches(state: dict, expected: ExpectedState) -> bool:
    return (
        state["revision"] == expected.revision
        and state["claim_id"] == expected.claim_id
        and state["active_attempt_id"] == expected.attempt_id
        and _active_job_id(state) == expected.serve_job_id
    )


def claim(
    root: Path, slot: str, claim_id: str, owner_uid: int, owner_name: str
) -> Lease:
    root = Path(root)
    _validate_slot(slot)
    _validate_hex_id(claim_id, "claim id")
    if (
        not isinstance(owner_uid, int)
        or isinstance(owner_uid, bool)
        or owner_uid != os.geteuid()
    ):
        raise LeaseError("claim owner does not match effective owner")
    _validate_text(owner_name, "owner name", 64)
    created_at = _now()

    def initial(_: dict) -> dict:
        return {
            "schema": SCHEMA,
            "slot": slot,
            "claim_id": claim_id,
            "owner": {"uid": owner_uid, "user": owner_name},
            "revision": 1,
            "phase": "claimed",
            "active_attempt_id": None,
            "retiring_attempt_id": None,
            "attempts": [],
            "created_at": created_at,
            "updated_at": created_at,
        }

    return mutate(root, slot, None, initial)


def mutate(
    root: Path,
    slot: str,
    expected: ExpectedState | None,
    transition: Callable[[dict], dict],
) -> Lease:
    root = Path(root)
    _validate_slot(slot)
    if expected is not None:
        _validate_expected(expected)
    if not callable(transition):
        raise LeaseError("transition must be callable")
    slot_path = root / slot

    with _mutation(root, slot):
        if expected is None:
            if _lexists(slot_path):
                raise LeaseError("unsafe slot already exists or is partial")
            try:
                os.mkdir(slot_path, 0o700)
                _validate_directory(slot_path, "slot directory")
                candidate = transition({})
                if not isinstance(candidate, dict):
                    raise LeaseError("slot transition must return an object")
                _validate_state(candidate, slot)
                if candidate["revision"] != 1 or candidate["phase"] != "claimed":
                    raise LeaseError("initial slot transition must create revision 1 claimed")
                _atomic_write_state(slot_path, candidate, slot)
            except BaseException:
                if _lexists(slot_path) and not _lexists(slot_path / "state.json"):
                    try:
                        _recover_temporary_state(slot_path, slot)
                        slot_path.rmdir()
                    except (LeaseError, OSError):
                        pass
                raise
            return Lease(slot_path / "state.json", copy.deepcopy(candidate))

        if not _lexists(slot_path):
            raise LeaseError("compare-and-swap failed: slot is absent")
        current = _load_state(slot_path, slot)
        if not _matches(current, expected):
            raise LeaseError("compare-and-swap failed: slot generation changed")
        candidate = transition(copy.deepcopy(current))
        if not isinstance(candidate, dict):
            raise LeaseError("slot transition must return an object")
        candidate["revision"] = current["revision"] + 1
        candidate["updated_at"] = _now()
        _validate_state(candidate, slot)
        for immutable in ("schema", "slot", "claim_id", "owner", "created_at"):
            if candidate[immutable] != current[immutable]:
                raise LeaseError(f"slot transition changed immutable field: {immutable}")
        _atomic_write_state(slot_path, candidate, slot)
        return Lease(slot_path / "state.json", copy.deepcopy(candidate))


def release_if_current(
    root: Path, slot: str, claim_id: str, attempt_id: str, serve_job_id: str
) -> str:
    root = Path(root)
    _validate_slot(slot)
    _validate_hex_id(claim_id, "claim id")
    _validate_hex_id(attempt_id, "attempt id")
    _validate_job_id(serve_job_id, "serve job id")
    slot_path = root / slot

    with _mutation(root, slot):
        if not _lexists(slot_path):
            return "stale_cleanup"
        current = _load_state(slot_path, slot)
        expected = ExpectedState(
            revision=current["revision"],
            claim_id=claim_id,
            attempt_id=attempt_id,
            serve_job_id=serve_job_id,
        )
        if not _matches(current, expected):
            return "stale_cleanup"

        tombstone = root / f".{slot}.tombstone.{current['revision']}.{claim_id}"
        if _lexists(tombstone):
            raise LeaseError("unsafe slot tombstone already exists")
        try:
            os.replace(slot_path, tombstone)
            _fsync_directory(root)
        except OSError as exc:
            raise LeaseError("slot release rename failed") from exc
        _safe_unlink_regular(tombstone / "state.json", "slot tombstone state")
        try:
            tombstone.rmdir()
        except OSError as exc:
            raise LeaseError("slot tombstone could not be removed safely") from exc
        _fsync_directory(root)
        return "released"


class _UsageError(Exception):
    pass


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise _UsageError(message)


def _parser() -> _Parser:
    parser = _Parser(prog="slot_lock.py", add_help=True)
    commands = parser.add_subparsers(dest="command", required=True)
    cleanup = commands.add_parser("cleanup", add_help=True)
    cleanup.add_argument("--root", required=True)
    cleanup.add_argument("--slot", required=True)
    cleanup.add_argument("--claim", required=True)
    cleanup.add_argument("--attempt", required=True)
    cleanup.add_argument("--job-id", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = _parser()
    try:
        arguments = parser.parse_args(argv)
        if arguments.command != "cleanup":
            raise _UsageError("unsupported command")
        _validate_slot(arguments.slot)
        _validate_hex_id(arguments.claim, "claim id")
        _validate_hex_id(arguments.attempt, "attempt id")
        _validate_job_id(arguments.job_id, "serve job id")
    except (_UsageError, LeaseError):
        parser.print_usage(sys.stderr)
        return 2

    try:
        result = release_if_current(
            Path(arguments.root),
            arguments.slot,
            arguments.claim,
            arguments.attempt,
            arguments.job_id,
        )
    except LeaseError:
        print("slot cleanup refused: unsafe lease state", file=sys.stderr)
        return 1
    print(json.dumps({"result": result}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
