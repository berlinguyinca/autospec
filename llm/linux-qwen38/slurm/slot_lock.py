#!/usr/bin/env python3
"""Revisioned shared-filesystem leases for contributor-owned Slurm slots.

The effective contributor UID is the trust principal. These checks coordinate
cooperative processes and fail closed on pre-existing ambiguity, crashes, and
ordinary I/O faults. They do not isolate the lease from a malicious process
running as the same UID: that process can already read the slot's mode-0600 qts
credential and ptrace or replace the contributor-owned agent.
"""

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
_TEMP_RE = re.compile(r"^\.state\.([1-9][0-9]*)\.([0-9a-f]{32})\.tmp$")
_BACKUP_RE = re.compile(
    r"^\.state\.backup\.([1-9][0-9]*)\.([0-9a-f]{32})$"
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
    {
        "schema",
        "host",
        "boot_id",
        "pid",
        "start_ticks",
        "uid",
        "nonce",
        "status",
        "reason",
    }
)


class LeaseError(RuntimeError):
    """The requested lease operation could not be proven safe."""


class _PublicationBlocked(LeaseError):
    def __init__(self, reason: str):
        super().__init__(f"slot transaction blocked: {reason}")
        self.reason = reason


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


@dataclass
class _PinnedDirectory:
    path: Path
    fd: int
    device: int
    inode: int
    parent_fd: int | None = None
    name: str | None = None

    def close(self) -> None:
        os.close(self.fd)


@dataclass(frozen=True)
class _EntryIdentity:
    device: int
    inode: int
    file_type: int
    owner: int
    mode: int
    links: int


def _entry_identity(info: os.stat_result) -> _EntryIdentity:
    return _EntryIdentity(
        device=info.st_dev,
        inode=info.st_ino,
        file_type=stat.S_IFMT(info.st_mode),
        owner=info.st_uid,
        mode=stat.S_IMODE(info.st_mode),
        links=info.st_nlink,
    )


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


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


def _validate_directory_stat(info: os.stat_result, label: str) -> None:
    if not stat.S_ISDIR(info.st_mode):
        raise LeaseError(f"unsafe {label}: expected a directory")
    if stat.S_IMODE(info.st_mode) != 0o700:
        raise LeaseError(f"unsafe {label}: directory mode must be 0700")
    if info.st_uid != os.geteuid():
        raise LeaseError(f"unsafe {label}: directory owner mismatch")


def _open_pinned_directory(
    path: Path,
    label: str,
    *,
    parent: _PinnedDirectory | None = None,
    name: str | None = None,
) -> _PinnedDirectory:
    try:
        if parent is None:
            before = os.lstat(path)
        else:
            if name is None:
                raise LeaseError(f"unsafe {label}: missing directory name")
            before = os.stat(name, dir_fd=parent.fd, follow_symlinks=False)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory is unavailable") from exc
    _validate_directory_stat(before, label)

    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(name if parent is not None else path, flags, dir_fd=parent.fd if parent else None)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory cannot be opened") from exc
    try:
        after = os.fstat(descriptor)
        _validate_directory_stat(after, label)
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise LeaseError(f"unsafe {label}: directory changed during validation")
        pin = _PinnedDirectory(
            path=path,
            fd=descriptor,
            device=after.st_dev,
            inode=after.st_ino,
            parent_fd=parent.fd if parent else None,
            name=name,
        )
        _revalidate_directory(pin, label)
        return pin
    except BaseException:
        os.close(descriptor)
        raise


def _revalidate_directory(directory: _PinnedDirectory, label: str) -> None:
    try:
        _revalidate_open_directory(directory, label)
        if directory.parent_fd is None:
            bound = os.lstat(directory.path)
        else:
            bound = os.stat(
                directory.name,
                dir_fd=directory.parent_fd,
                follow_symlinks=False,
            )
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory binding changed") from exc
    _validate_directory_stat(bound, label)
    identity = (directory.device, directory.inode)
    if (bound.st_dev, bound.st_ino) != identity:
        raise LeaseError(f"unsafe {label}: directory binding changed")


def _revalidate_open_directory(directory: _PinnedDirectory, label: str) -> None:
    try:
        opened = os.fstat(directory.fd)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: directory descriptor changed") from exc
    _validate_directory_stat(opened, label)
    if (opened.st_dev, opened.st_ino) != (directory.device, directory.inode):
        raise LeaseError(f"unsafe {label}: directory descriptor changed")


def _entry_exists(directory: _PinnedDirectory, name: str, label: str) -> bool:
    try:
        os.stat(name, dir_fd=directory.fd, follow_symlinks=False)
    except FileNotFoundError:
        return False
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: entry cannot be inspected") from exc
    return True


def _fsync_pinned(directory: _PinnedDirectory, label: str) -> None:
    try:
        os.fsync(directory.fd)
        _revalidate_directory(directory, label)
    except OSError as exc:
        raise LeaseError("shared state directory fsync failed") from exc


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


def _read_secure_snapshot_at(
    directory: _PinnedDirectory,
    name: str,
    label: str,
    maximum: int,
    *,
    expected_links: int = 1,
) -> tuple[bytes, _EntryIdentity]:
    try:
        before = os.stat(name, dir_fd=directory.fd, follow_symlinks=False)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: file is unavailable") from exc
    if stat.S_ISLNK(before.st_mode) or not stat.S_ISREG(before.st_mode):
        raise LeaseError(f"unsafe {label}: expected a regular file")
    if stat.S_IMODE(before.st_mode) != 0o600:
        raise LeaseError(f"unsafe {label}: file mode must be 0600")
    if before.st_uid != os.geteuid():
        raise LeaseError(f"unsafe {label}: file owner mismatch")
    if before.st_nlink != expected_links:
        raise LeaseError(f"unsafe {label}: unexpected file link count")
    if before.st_size > maximum:
        raise LeaseError(f"unsafe {label}: file is too large")
    if before.st_size == 0:
        raise LeaseError(f"unsafe {label}: file is empty")

    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(name, flags, dir_fd=directory.fd)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: file cannot be opened") from exc
    try:
        after = os.fstat(descriptor)
        if _entry_identity(before) != _entry_identity(after):
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
        _revalidate_directory(directory, f"{label} parent")
        return payload, _entry_identity(after)
    finally:
        os.close(descriptor)


def _read_secure_at(
    directory: _PinnedDirectory, name: str, label: str, maximum: int
) -> bytes:
    return _read_secure_snapshot_at(directory, name, label, maximum)[0]


def _read_recovery_snapshot_at(
    directory: _PinnedDirectory, name: str, label: str, maximum: int
) -> tuple[bytes, _EntryIdentity]:
    try:
        info = os.stat(name, dir_fd=directory.fd, follow_symlinks=False)
    except OSError as exc:
        raise LeaseError(f"unsafe {label}: file is unavailable") from exc
    if info.st_nlink not in {1, 2}:
        raise LeaseError(f"unsafe {label}: unexpected recovery link count")
    return _read_secure_snapshot_at(
        directory,
        name,
        label,
        maximum,
        expected_links=info.st_nlink,
    )


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


def _validate_state_document(state: dict, expected_slot: str) -> None:
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
    if not isinstance(state["phase"], str) or state["phase"] not in _PHASES:
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

    known_attempts: set[str] = set()
    serve_tags: set[str] = set()
    cleanup_tags: set[str] = set()
    ordinals: set[int] = set()
    cleanup_job_ids: set[str] = set()
    for attempt in attempts:
        _require_exact_fields(attempt, _ATTEMPT_FIELDS, "slot attempt")
        attempt_id = attempt["attempt_id"]
        _validate_hex_id(attempt_id, "attempt id")
        if attempt_id in known_attempts:
            raise LeaseError("duplicate attempt id in slot state")
        known_attempts.add(attempt_id)
        ordinal = attempt["ordinal"]
        if not isinstance(ordinal, int) or isinstance(ordinal, bool) or ordinal < 0:
            raise LeaseError("invalid attempt ordinal")
        if ordinal in ordinals:
            raise LeaseError("duplicate attempt ordinal in slot state")
        ordinals.add(ordinal)
        _validate_text(attempt["gpu"], "attempt gpu")
        if not isinstance(attempt["state"], str) or attempt["state"] not in _ATTEMPT_STATES:
            raise LeaseError("invalid attempt state")
        serve_tag = attempt["serve_tag"]
        if not isinstance(serve_tag, str):
            raise LeaseError("invalid serve tag")
        if serve_tag in serve_tags:
            raise LeaseError("duplicate serve tag in slot state")
        serve_tags.add(serve_tag)
        if serve_tag != _serve_tag(expected_slot, state["claim_id"], attempt_id):
            raise LeaseError("invalid serve tag")
        _validate_job_id(attempt["serve_job_id"], "serve job id", nullable=True)

        cleanup = attempt["cleanup"]
        if not isinstance(cleanup, dict):
            raise LeaseError("attempt cleanup must be an object")
        _require_exact_fields(cleanup, _CLEANUP_FIELDS, "attempt cleanup")
        cleanup_tag = cleanup["tag"]
        if not isinstance(cleanup_tag, str):
            raise LeaseError("invalid cleanup tag")
        if cleanup_tag in cleanup_tags:
            raise LeaseError("duplicate cleanup tag in slot state")
        cleanup_tags.add(cleanup_tag)
        if cleanup_tag != _cleanup_tag(expected_slot, state["claim_id"], attempt_id):
            raise LeaseError("invalid cleanup tag")
        if (
            not isinstance(cleanup["state"], str)
            or cleanup["state"] not in _CLEANUP_STATES
        ):
            raise LeaseError("invalid cleanup state")
        jobs = cleanup["job_ids"]
        if not isinstance(jobs, list) or len(jobs) > MAX_CLEANUP_JOBS:
            raise LeaseError("cleanup job ids must be a bounded list")
        for job_id in jobs:
            _validate_job_id(job_id, "cleanup job id")
            if job_id in cleanup_job_ids:
                raise LeaseError("duplicate cleanup job id in slot state")
            cleanup_job_ids.add(job_id)

    for field in ("active_attempt_id", "retiring_attempt_id"):
        attempt_id = state[field]
        if attempt_id is not None and attempt_id not in known_attempts:
            raise LeaseError(f"{field} does not name an attempt record")


def _validate_state(state: dict, expected_slot: str) -> None:
    try:
        _validate_state_document(state, expected_slot)
    except LeaseError:
        raise
    except (AttributeError, IndexError, KeyError, TypeError, ValueError) as exc:
        raise LeaseError("invalid slot state shape") from exc


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


def _load_state_snapshot_pinned(
    slot_directory: _PinnedDirectory, slot: str, *, expected_links: int = 1
) -> tuple[dict, _EntryIdentity, bytes]:
    payload, identity = _read_secure_snapshot_at(
        slot_directory,
        "state.json",
        "slot state",
        MAX_STATE_BYTES,
        expected_links=expected_links,
    )
    state = _decode_json(payload, "slot state")
    _validate_state(state, slot)
    return state, identity, payload


def _load_state_pinned(slot_directory: _PinnedDirectory, slot: str) -> dict:
    return _load_state_snapshot_pinned(slot_directory, slot)[0]


def _load_lease_from_root(
    root: Path,
    root_directory: _PinnedDirectory,
    slot: str,
    *,
    transaction_expected: bool,
) -> Lease:
    slot_path = root / slot
    slot_directory = _open_pinned_directory(
        slot_path,
        "slot directory",
        parent=root_directory,
        name=slot,
    )
    try:
        if os.listdir(slot_directory.fd) != ["state.json"]:
            raise LeaseError("unsafe unexpected entry in slot directory")
        state = _load_state_pinned(slot_directory, slot)
        _revalidate_directory(root_directory, "lease root")
        transaction_present = _entry_exists(
            root_directory, f".{slot}.txn", "transaction mutex"
        )
        if transaction_present != transaction_expected:
            raise LeaseError("slot mutation boundary changed during read")
        return Lease(slot_path / "state.json", copy.deepcopy(state))
    finally:
        slot_directory.close()


def load(root: Path, slot: str) -> Lease:
    root = Path(root)
    _validate_slot(slot)
    _reject_symlink_components(root)
    root_directory = _open_pinned_directory(root, "lease root")
    try:
        transaction_exists = _entry_exists(
            root_directory, f".{slot}.txn", "transaction mutex"
        )
        if not transaction_exists:
            return _load_lease_from_root(
                root,
                root_directory,
                slot,
                transaction_expected=False,
            )
    finally:
        root_directory.close()

    with _mutation(root, slot) as recovered_root:
        return _load_lease_from_root(
            root,
            recovered_root,
            slot,
            transaction_expected=True,
        )


def _write_exclusive_at(
    directory: _PinnedDirectory, name: str, payload: bytes, label: str
) -> _EntryIdentity:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    descriptor: int | None = None
    try:
        descriptor = os.open(name, flags, 0o600, dir_fd=directory.fd)
        os.fchmod(descriptor, 0o600)
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise OSError("short write")
            view = view[written:]
        os.fsync(descriptor)
        written_info = os.fstat(descriptor)
        if (
            not stat.S_ISREG(written_info.st_mode)
            or written_info.st_uid != os.geteuid()
            or stat.S_IMODE(written_info.st_mode) != 0o600
            or written_info.st_nlink != 1
        ):
            raise OSError("written file identity changed")
        _revalidate_directory(directory, f"{label} parent")
        return _entry_identity(written_info)
    except OSError as exc:
        raise LeaseError(f"failed to write {label}") from exc
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _quarantine_exact_entry(
    directory: _PinnedDirectory,
    name: str,
    expected: _EntryIdentity,
    label: str,
    maximum: int = MAX_STATE_BYTES,
) -> str:
    quarantine = f".quarantine.{uuid.uuid4().hex}.entry"
    try:
        os.replace(
            name,
            quarantine,
            src_dir_fd=directory.fd,
            dst_dir_fd=directory.fd,
        )
    except OSError as exc:
        raise LeaseError(f"failed to quarantine {label}") from exc
    try:
        _, observed = _read_secure_snapshot_at(
            directory,
            quarantine,
            f"quarantined {label}",
            maximum,
            expected_links=expected.links,
        )
    except LeaseError as exc:
        raise LeaseError(f"unsafe quarantined {label} preserved") from exc
    if observed != expected:
        raise LeaseError(f"quarantined {label} inode mismatch; entry preserved")
    _revalidate_directory(directory, f"{label} parent")
    return quarantine


def _discard_exact_entry(
    directory: _PinnedDirectory,
    name: str,
    expected: _EntryIdentity,
    label: str,
    maximum: int = MAX_STATE_BYTES,
) -> None:
    quarantine = _quarantine_exact_entry(
        directory, name, expected, label, maximum
    )
    _, observed = _read_secure_snapshot_at(
        directory,
        quarantine,
        f"quarantined {label}",
        maximum,
        expected_links=expected.links,
    )
    if observed != expected:
        raise LeaseError(f"quarantined {label} changed; entry preserved")
    try:
        os.unlink(quarantine, dir_fd=directory.fd)
    except OSError as exc:
        raise LeaseError(f"failed to remove quarantined {label}") from exc
    _revalidate_directory(directory, f"{label} parent")


def _identity_with_links(identity: _EntryIdentity, links: int) -> _EntryIdentity:
    return _EntryIdentity(
        device=identity.device,
        inode=identity.inode,
        file_type=identity.file_type,
        owner=identity.owner,
        mode=identity.mode,
        links=links,
    )


def _restore_prior_state(
    slot_directory: _PinnedDirectory,
    backup: str,
    expected: _EntryIdentity,
    slot: str,
) -> None:
    try:
        os.replace(
            backup,
            "state.json",
            src_dir_fd=slot_directory.fd,
            dst_dir_fd=slot_directory.fd,
        )
    except OSError as exc:
        raise LeaseError("prior slot state restore failed") from exc
    _, restored, _ = _load_state_snapshot_pinned(slot_directory, slot)
    if restored != expected:
        raise LeaseError("prior slot state restore inode mismatch")


def _rollback_publication(
    slot_directory: _PinnedDirectory,
    slot: str,
    temp_identity: _EntryIdentity,
    backup: str | None,
    prior_identity: _EntryIdentity | None,
) -> None:
    _, current_identity = _read_recovery_snapshot_at(
        slot_directory,
        "state.json",
        "published slot state",
        MAX_STATE_BYTES,
    )
    if (
        backup is not None
        and prior_identity is not None
        and current_identity == _identity_with_links(prior_identity, 2)
    ):
        _, backup_identity = _read_secure_snapshot_at(
            slot_directory,
            backup,
            "slot state backup",
            MAX_STATE_BYTES,
            expected_links=2,
        )
        _discard_exact_entry(
            slot_directory, backup, backup_identity, "slot state backup"
        )
        _fsync_pinned(slot_directory, "slot directory")
        return

    if current_identity == temp_identity:
        _discard_exact_entry(
            slot_directory,
            "state.json",
            current_identity,
            "failed published slot state",
        )
    else:
        _quarantine_exact_entry(
            slot_directory,
            "state.json",
            current_identity,
            "suspicious published slot state",
        )
    if backup is not None and prior_identity is not None:
        _restore_prior_state(
            slot_directory,
            backup,
            prior_identity,
            slot,
        )
    _fsync_pinned(slot_directory, "slot directory")


def _atomic_write_state(
    slot_directory: _PinnedDirectory,
    state: dict,
    slot: str,
    prior_identity: _EntryIdentity | None = None,
) -> None:
    payload = _state_bytes(state, slot)
    temp = f".state.{state['revision']}.{uuid.uuid4().hex}.tmp"
    temp_identity = _write_exclusive_at(
        slot_directory, temp, payload, "temporary slot state"
    )
    backup: str | None = None
    publication_started = False
    try:
        if prior_identity is not None:
            backup = f".state.backup.{state['revision'] - 1}.{uuid.uuid4().hex}"
            try:
                os.link(
                    "state.json",
                    backup,
                    src_dir_fd=slot_directory.fd,
                    dst_dir_fd=slot_directory.fd,
                    follow_symlinks=False,
                )
            except OSError as exc:
                raise LeaseError("prior slot state backup failed") from exc
            linked_identity = _identity_with_links(prior_identity, 2)
            _, observed_state, _ = _load_state_snapshot_pinned(
                slot_directory, slot, expected_links=2
            )
            _, observed_backup = _read_secure_snapshot_at(
                slot_directory,
                backup,
                "slot state backup",
                MAX_STATE_BYTES,
                expected_links=2,
            )
            if observed_state != linked_identity or observed_backup != linked_identity:
                raise LeaseError("prior slot state backup inode mismatch; preserved")

        publication_started = True
        os.replace(
            temp,
            "state.json",
            src_dir_fd=slot_directory.fd,
            dst_dir_fd=slot_directory.fd,
        )
        observed_payload, observed_identity = _read_secure_snapshot_at(
            slot_directory, "state.json", "published slot state", MAX_STATE_BYTES
        )
        if observed_identity != temp_identity or observed_payload != payload:
            raise LeaseError("slot state publication inode mismatch")
        _validate_state(_decode_json(observed_payload, "published slot state"), slot)

        if backup is not None:
            _, backup_identity = _read_secure_snapshot_at(
                slot_directory, backup, "slot state backup", MAX_STATE_BYTES
            )
            if backup_identity != prior_identity:
                raise LeaseError("slot state backup changed; preserved")
            _discard_exact_entry(
                slot_directory,
                backup,
                backup_identity,
                "slot state backup",
            )
            backup = None
        _fsync_pinned(slot_directory, "slot directory")
    except BaseException as publication_error:
        blocked_reason: str | None = None
        if publication_started:
            try:
                _rollback_publication(
                    slot_directory,
                    slot,
                    temp_identity,
                    backup,
                    prior_identity,
                )
            except (LeaseError, OSError):
                blocked_reason = "rollback_failed"
            else:
                blocked_reason = "publication_failed"
        try:
            _, observed_temp = _read_secure_snapshot_at(
                slot_directory, temp, "temporary slot state", MAX_STATE_BYTES
            )
        except OSError:
            pass
        except LeaseError:
            pass
        else:
            if observed_temp == temp_identity:
                try:
                    _discard_exact_entry(
                        slot_directory,
                        temp,
                        temp_identity,
                        "temporary slot state",
                    )
                except LeaseError:
                    pass
        if blocked_reason is None and backup is not None and prior_identity is not None:
            try:
                _, current_identity, _ = _load_state_snapshot_pinned(
                    slot_directory, slot, expected_links=2
                )
            except LeaseError:
                pass
            else:
                linked_identity = _identity_with_links(prior_identity, 2)
                if current_identity == linked_identity:
                    try:
                        _, backup_identity = _read_secure_snapshot_at(
                            slot_directory,
                            backup,
                            "slot state backup",
                            MAX_STATE_BYTES,
                            expected_links=2,
                        )
                        _discard_exact_entry(
                            slot_directory,
                            backup,
                            backup_identity,
                            "slot state backup",
                        )
                    except LeaseError:
                        pass
        if blocked_reason is not None:
            raise _PublicationBlocked(blocked_reason) from publication_error
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
        "status": "active",
        "reason": None,
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
    if document["uid"] != os.geteuid():
        raise LeaseError("cannot prove stale transaction mutex owner")
    _validate_hex_id(document["nonce"], "transaction nonce")
    if document["status"] == "active" and document["reason"] is None:
        return
    if document["status"] == "blocked" and document["reason"] in {
        "publication_failed",
        "rollback_failed",
    }:
        return
    raise LeaseError("cannot prove transaction mutex status")


def _read_txn_pinned(mutex: _PinnedDirectory) -> dict:
    try:
        _revalidate_directory(mutex, "transaction mutex")
        entries = os.listdir(mutex.fd)
        if entries != ["owner.json"]:
            raise LeaseError("cannot prove stale transaction mutex")
        document = _decode_json(
            _read_secure_at(mutex, "owner.json", "transaction owner", 4096),
            "transaction owner",
        )
        _validate_txn_document(document)
        return document
    except LeaseError as exc:
        if "cannot prove stale" in str(exc):
            raise
        raise LeaseError("cannot prove stale transaction mutex") from exc


def _remove_txn(
    root: _PinnedDirectory,
    mutex_name: str,
    mutex_path: Path,
    expected_nonce: str | None,
) -> None:
    mutex = _open_pinned_directory(
        mutex_path,
        "transaction mutex",
        parent=root,
        name=mutex_name,
    )
    try:
        document = _read_txn_pinned(mutex)
        if expected_nonce is not None and document["nonce"] != expected_nonce:
            raise LeaseError("transaction mutex ownership changed")
        if document["status"] != "active":
            raise LeaseError("blocked transaction mutex cannot be removed")
        _, owner_identity = _read_secure_snapshot_at(
            mutex, "owner.json", "transaction owner", 4096
        )
        _discard_exact_entry(
            mutex,
            "owner.json",
            owner_identity,
            "transaction owner",
            4096,
        )
        if os.listdir(mutex.fd):
            raise LeaseError("unexpected entry in transaction mutex before removal")
        _revalidate_directory(mutex, "transaction mutex")
        _revalidate_directory(root, "lease root")
        try:
            os.rmdir(mutex_name, dir_fd=root.fd)
        except OSError as exc:
            raise LeaseError("transaction mutex could not be removed safely") from exc
        _revalidate_open_directory(mutex, "transaction mutex")
    finally:
        mutex.close()
    _revalidate_directory(root, "lease root")


def _recover_existing_txn(
    root: _PinnedDirectory, mutex_name: str, mutex_path: Path
) -> None:
    mutex = _open_pinned_directory(
        mutex_path,
        "transaction mutex",
        parent=root,
        name=mutex_name,
    )
    try:
        document = _read_txn_pinned(mutex)
    finally:
        mutex.close()
    if document["status"] == "blocked":
        raise LeaseError("slot transaction is blocked; explicit repair required")
    if document["host"] != socket.gethostname():
        raise LeaseError("cannot prove stale transaction mutex on another host")
    current_boot = _boot_id()
    if document["boot_id"] == current_boot:
        observed_start = _process_start_ticks(document["pid"])
        if observed_start == document["start_ticks"]:
            raise LeaseError("slot mutation already in progress")
    _remove_txn(root, mutex_name, mutex_path, document["nonce"])


def _mark_txn_blocked(
    root: _PinnedDirectory,
    mutex_name: str,
    mutex_path: Path,
    nonce: str,
    reason: str,
) -> None:
    mutex = _open_pinned_directory(
        mutex_path,
        "transaction mutex",
        parent=root,
        name=mutex_name,
    )
    temp = f".owner.blocked.{uuid.uuid4().hex}.tmp"
    try:
        document = _read_txn_pinned(mutex)
        if document["nonce"] != nonce or document["status"] != "active":
            raise LeaseError("transaction mutex ownership changed before block")
        document["status"] = "blocked"
        document["reason"] = reason
        payload = (
            json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n"
        ).encode()
        _write_exclusive_at(mutex, temp, payload, "blocked transaction owner")
        os.replace(
            temp,
            "owner.json",
            src_dir_fd=mutex.fd,
            dst_dir_fd=mutex.fd,
        )
        _fsync_pinned(mutex, "transaction mutex")
        blocked = _read_txn_pinned(mutex)
        if blocked["nonce"] != nonce or blocked["status"] != "blocked":
            raise LeaseError("blocked transaction marker verification failed")
        _fsync_pinned(root, "lease root")
    finally:
        mutex.close()


def _acquire_txn(root: _PinnedDirectory, slot: str) -> tuple[str, Path, str]:
    mutex_name = f".{slot}.txn"
    mutex_path = root.path / mutex_name
    nonce = uuid.uuid4().hex
    for attempt in range(2):
        try:
            os.mkdir(mutex_name, 0o700, dir_fd=root.fd)
        except FileExistsError:
            if attempt:
                raise LeaseError("slot transaction mutex remained occupied")
            _recover_existing_txn(root, mutex_name, mutex_path)
            continue
        except OSError as exc:
            raise LeaseError("unsafe transaction mutex creation failed") from exc

        try:
            mutex = _open_pinned_directory(
                mutex_path,
                "transaction mutex",
                parent=root,
                name=mutex_name,
            )
            payload = (
                json.dumps(_txn_document(nonce), sort_keys=True, separators=(",", ":"))
                + "\n"
            ).encode()
            try:
                _write_exclusive_at(
                    mutex, "owner.json", payload, "transaction owner"
                )
                _fsync_pinned(mutex, "transaction mutex")
                _fsync_pinned(root, "lease root")
            finally:
                mutex.close()
            return mutex_name, mutex_path, nonce
        except BaseException:
            try:
                _remove_txn(root, mutex_name, mutex_path, nonce)
            except (LeaseError, OSError):
                pass
            raise
    raise LeaseError("slot transaction mutex could not be acquired")


@contextmanager
def _mutation(root: Path, slot: str) -> Iterator[_PinnedDirectory]:
    _reject_symlink_components(root)
    root_directory = _open_pinned_directory(root, "lease root")
    try:
        mutex_name, mutex_path, nonce = _acquire_txn(root_directory, slot)
        remove_transaction = True
        try:
            _recover_tombstones(root_directory, slot)
            _recover_temporary_state(root_directory, slot)
            yield root_directory
        except _PublicationBlocked as exc:
            remove_transaction = False
            try:
                _mark_txn_blocked(
                    root_directory,
                    mutex_name,
                    mutex_path,
                    nonce,
                    exc.reason,
                )
            except LeaseError as marker_error:
                raise LeaseError(
                    "slot transaction blocked; marker update failed closed"
                ) from marker_error
            raise LeaseError(f"slot transaction blocked: {exc.reason}") from exc
        finally:
            if remove_transaction:
                _remove_txn(root_directory, mutex_name, mutex_path, nonce)
                _fsync_pinned(root_directory, "lease root")
    finally:
        root_directory.close()


def _recover_temporary_state(root: _PinnedDirectory, slot: str) -> None:
    try:
        os.stat(slot, dir_fd=root.fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    except OSError as exc:
        raise LeaseError("unsafe slot directory cannot be inspected") from exc
    slot_path = root.path / slot
    slot_directory = _open_pinned_directory(
        slot_path,
        "slot directory",
        parent=root,
        name=slot,
    )
    try:
        try:
            entries = os.listdir(slot_directory.fd)
        except OSError as exc:
            raise LeaseError("unsafe slot directory cannot be inspected") from exc
        extras = [name for name in entries if name != "state.json"]
        parsed_temps = [
            (name, match)
            for name in extras
            if (match := _TEMP_RE.fullmatch(name)) is not None
        ]
        parsed_backups = [
            (name, match)
            for name in extras
            if (match := _BACKUP_RE.fullmatch(name)) is not None
        ]
        known = {name for name, _ in [*parsed_temps, *parsed_backups]}
        if set(extras) != known:
            raise LeaseError("unsafe unknown entry in slot directory")
        if len(parsed_temps) > 1 or len(parsed_backups) > 1:
            raise LeaseError("unsafe ambiguous slot publication state")
        if "state.json" not in entries:
            if parsed_temps or parsed_backups:
                raise LeaseError("unsafe incomplete initial slot transaction")
            return

        if parsed_backups:
            backup_name, backup_match = parsed_backups[0]
            try:
                current_payload, current_identity = _read_recovery_snapshot_at(
                    slot_directory,
                    "state.json",
                    "slot state",
                    MAX_STATE_BYTES,
                )
                current = _decode_json(current_payload, "slot state")
                _validate_state(current, slot)
                backup_payload, backup_identity = _read_recovery_snapshot_at(
                    slot_directory,
                    backup_name,
                    "slot state backup",
                    MAX_STATE_BYTES,
                )
                backup_state = _decode_json(backup_payload, "slot state backup")
                _validate_state(backup_state, slot)
            except LeaseError as exc:
                raise LeaseError("unsafe slot publication recovery state") from exc
            if (
                backup_state["revision"] != int(backup_match.group(1))
                or backup_state["claim_id"] != current["claim_id"]
            ):
                raise LeaseError("unsafe slot state backup generation")
            same_inode = (
                current_identity.device,
                current_identity.inode,
            ) == (backup_identity.device, backup_identity.inode)
            if same_inode:
                if (
                    current_identity.links != 2
                    or backup_identity != current_identity
                    or current["revision"] != backup_state["revision"]
                ):
                    raise LeaseError("unsafe linked slot state backup")
            elif (
                current_identity.links != 1
                or backup_identity.links != 1
                or current["revision"] != backup_state["revision"] + 1
            ):
                raise LeaseError("unsafe replaced slot state backup")
            _discard_exact_entry(
                slot_directory,
                backup_name,
                backup_identity,
                "slot state backup",
            )
            current = _load_state_pinned(slot_directory, slot)
        else:
            current = _load_state_pinned(slot_directory, slot)

        for name, match in parsed_temps:
            try:
                temp_payload, temp_identity = _read_secure_snapshot_at(
                    slot_directory,
                    name,
                    "temporary slot state",
                    MAX_STATE_BYTES,
                )
                candidate = _decode_json(
                    temp_payload,
                    "temporary slot state",
                )
                _validate_state(candidate, slot)
            except LeaseError as exc:
                raise LeaseError("unsafe temporary slot state") from exc
            if (
                match is None
                or candidate["revision"] != int(match.group(1))
                or candidate["revision"] != current["revision"] + 1
                or candidate["claim_id"] != current["claim_id"]
            ):
                raise LeaseError("unsafe temporary slot state generation")
            _discard_exact_entry(
                slot_directory,
                name,
                temp_identity,
                "temporary slot state",
            )
        if parsed_temps or parsed_backups:
            _fsync_pinned(slot_directory, "slot directory")
    finally:
        slot_directory.close()


def _tombstone_pattern(slot: str) -> re.Pattern[str]:
    return re.compile(
        rf"^\.{re.escape(slot)}\.tombstone\.([1-9][0-9]*)\.([0-9a-f]{{32}})$"
    )


def _recover_tombstones(root: _PinnedDirectory, slot: str) -> None:
    prefix = f".{slot}.tombstone."
    names = [name for name in os.listdir(root.fd) if name.startswith(prefix)]
    if not names:
        return
    try:
        os.stat(slot, dir_fd=root.fd, follow_symlinks=False)
        slot_exists = True
    except FileNotFoundError:
        slot_exists = False
    except OSError as exc:
        raise LeaseError("unsafe ambiguous slot tombstone state") from exc
    if len(names) != 1 or slot_exists:
        raise LeaseError("unsafe ambiguous slot tombstone state")
    name = names[0]
    match = _tombstone_pattern(slot).fullmatch(name)
    if match is None:
        raise LeaseError("unsafe malformed slot tombstone")
    tombstone_path = root.path / name
    tombstone = _open_pinned_directory(
        tombstone_path,
        "slot tombstone",
        parent=root,
        name=name,
    )
    try:
        try:
            state, state_identity, _ = _load_state_snapshot_pinned(
                tombstone, slot
            )
        except LeaseError as exc:
            raise LeaseError("unsafe slot tombstone") from exc
        if state["revision"] != int(match.group(1)) or state["claim_id"] != match.group(2):
            raise LeaseError("unsafe slot tombstone generation mismatch")
        entries = os.listdir(tombstone.fd)
        if entries != ["state.json"]:
            raise LeaseError("unsafe slot tombstone contents")
        _discard_exact_entry(
            tombstone,
            "state.json",
            state_identity,
            "slot tombstone state",
        )
        if os.listdir(tombstone.fd):
            raise LeaseError("unexpected slot tombstone entry before removal")
        _revalidate_directory(tombstone, "slot tombstone")
        _revalidate_directory(root, "lease root")
        try:
            os.rmdir(name, dir_fd=root.fd)
        except OSError as exc:
            raise LeaseError("slot tombstone could not be removed safely") from exc
        _revalidate_open_directory(tombstone, "slot tombstone")
    finally:
        tombstone.close()
    _fsync_pinned(root, "lease root")


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


def _validate_initial_state(state: dict) -> None:
    if (
        state["revision"] != 1
        or state["phase"] != "claimed"
        or state["active_attempt_id"] is not None
        or state["retiring_attempt_id"] is not None
        or state["attempts"] != []
        or state["created_at"] != state["updated_at"]
        or not state["created_at"].endswith("Z")
    ):
        raise LeaseError("initial slot state must be the canonical empty claim")


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

    with _mutation(root, slot) as root_directory:
        if expected is None:
            if _entry_exists(root_directory, slot, "slot creation state"):
                raise LeaseError("unsafe slot already exists or is partial")
            slot_directory: _PinnedDirectory | None = None
            try:
                os.mkdir(slot, 0o700, dir_fd=root_directory.fd)
                slot_directory = _open_pinned_directory(
                    slot_path,
                    "slot directory",
                    parent=root_directory,
                    name=slot,
                )
                candidate = transition({})
                if not isinstance(candidate, dict):
                    raise LeaseError("slot transition must return an object")
                _validate_state(candidate, slot)
                _validate_initial_state(candidate)
                _atomic_write_state(slot_directory, candidate, slot)
                _revalidate_directory(root_directory, "lease root")
            except BaseException:
                if slot_directory is not None:
                    try:
                        if os.listdir(slot_directory.fd) == []:
                            _revalidate_directory(
                                slot_directory, "slot directory"
                            )
                            os.rmdir(slot, dir_fd=root_directory.fd)
                            _revalidate_open_directory(
                                slot_directory, "slot directory"
                            )
                            _fsync_pinned(root_directory, "lease root")
                    except (LeaseError, OSError, ValueError):
                        pass
                raise
            finally:
                if slot_directory is not None:
                    slot_directory.close()
            return Lease(slot_path / "state.json", copy.deepcopy(candidate))

        if not _entry_exists(root_directory, slot, "slot directory"):
            raise LeaseError("compare-and-swap failed: slot is absent")
        slot_directory = _open_pinned_directory(
            slot_path,
            "slot directory",
            parent=root_directory,
            name=slot,
        )
        try:
            current, current_identity, _ = _load_state_snapshot_pinned(
                slot_directory, slot
            )
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
            _atomic_write_state(
                slot_directory,
                candidate,
                slot,
                prior_identity=current_identity,
            )
            _revalidate_directory(root_directory, "lease root")
            return Lease(slot_path / "state.json", copy.deepcopy(candidate))
        finally:
            slot_directory.close()


def release_if_current(
    root: Path, slot: str, claim_id: str, attempt_id: str, serve_job_id: str
) -> str:
    root = Path(root)
    _validate_slot(slot)
    _validate_hex_id(claim_id, "claim id")
    _validate_hex_id(attempt_id, "attempt id")
    _validate_job_id(serve_job_id, "serve job id")
    slot_path = root / slot

    with _mutation(root, slot) as root_directory:
        if not _entry_exists(root_directory, slot, "slot directory"):
            return "stale_cleanup"
        slot_directory = _open_pinned_directory(
            slot_path,
            "slot directory",
            parent=root_directory,
            name=slot,
        )
        try:
            current, state_identity, _ = _load_state_snapshot_pinned(
                slot_directory, slot
            )
            expected = ExpectedState(
                revision=current["revision"],
                claim_id=claim_id,
                attempt_id=attempt_id,
                serve_job_id=serve_job_id,
            )
            if not _matches(current, expected):
                return "stale_cleanup"

            tombstone_name = (
                f".{slot}.tombstone.{current['revision']}.{claim_id}"
            )
            try:
                os.stat(
                    tombstone_name,
                    dir_fd=root_directory.fd,
                    follow_symlinks=False,
                )
            except FileNotFoundError:
                pass
            except OSError as exc:
                raise LeaseError("unsafe slot tombstone state") from exc
            else:
                raise LeaseError("unsafe slot tombstone already exists")
            try:
                os.replace(
                    slot,
                    tombstone_name,
                    src_dir_fd=root_directory.fd,
                    dst_dir_fd=root_directory.fd,
                )
                slot_directory.name = tombstone_name
                slot_directory.path = root / tombstone_name
                _revalidate_directory(slot_directory, "slot tombstone")
                _fsync_pinned(root_directory, "lease root")
            except OSError as exc:
                raise LeaseError("slot release rename failed") from exc
            _discard_exact_entry(
                slot_directory,
                "state.json",
                state_identity,
                "slot tombstone state",
            )
            if os.listdir(slot_directory.fd):
                raise LeaseError("unexpected slot tombstone entry before removal")
            _revalidate_directory(slot_directory, "slot tombstone")
            _revalidate_directory(root_directory, "lease root")
            try:
                os.rmdir(tombstone_name, dir_fd=root_directory.fd)
            except OSError as exc:
                raise LeaseError("slot tombstone could not be removed safely") from exc
            _revalidate_open_directory(slot_directory, "slot tombstone")
        finally:
            slot_directory.close()
        _fsync_pinned(root_directory, "lease root")
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
