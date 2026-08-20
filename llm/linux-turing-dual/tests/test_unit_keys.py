"""Key format, hashing and verification.

These are the tests that decide whether a leaked database row is enough to
authenticate. They assert the negative cases as hard as the positive one.
"""
import importlib.util
import pathlib
import sys

import pytest

SCRIPTS = pathlib.Path(__file__).resolve().parent.parent / "scripts"


def _load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    # Registered BEFORE exec, exactly as a real import does. `from __future__
    # import annotations` makes @dataclass resolve its field types through
    # sys.modules[cls.__module__], so a module loaded by path alone raises
    # "NoneType has no attribute __dict__" at class-creation time.
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


keys = _load("keys")


def test_generate_round_trips_and_never_stores_the_secret():
    full, key_id, stored = keys.generate()
    assert full.startswith(keys.PREFIX + "_")
    assert full.count("_") == 2
    parsed = keys.parse(full)
    assert parsed is not None
    kid, secret = parsed
    assert kid == key_id
    assert keys.verify(secret, stored)
    # The row must not be sufficient to reconstruct the credential.
    assert secret not in stored
    assert stored != secret


def test_key_id_is_public_and_stable():
    full, key_id, _ = keys.generate()
    assert keys.public_id(full) == key_id
    assert len(key_id) == keys.KEY_ID_LEN
    # A key id is printed in dashboards and logs; it must not contain the secret.
    assert keys.parse(full)[1] not in key_id


def test_a_tampered_secret_is_refused():
    full, _, stored = keys.generate()
    _, secret = keys.parse(full)
    flipped = secret[:-1] + ("a" if secret[-1] != "a" else "b")
    assert not keys.verify(flipped, stored)


def test_a_secret_from_another_key_is_refused():
    _, _, stored_a = keys.generate()
    full_b, _, _ = keys.generate()
    assert not keys.verify(keys.parse(full_b)[1], stored_a)


@pytest.mark.parametrize("bad", [
    "", "qtk", "qtk_", "qtk__", "nope_aaaaaaaaaaaa_bbbb",
    "qtk_aaaaaaaaaaaa",                     # no secret
    "qtk_short_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",   # key id too short
    "Bearer qtk_aaaaaaaaaaaa_bbbb",         # header not stripped by the caller
    "qtk_aaaaaaaaaaaa_bbbb_extra",          # too many parts
    "qtk_AAAAAAAAAAAA_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",  # wrong case
])
def test_malformed_keys_parse_to_none(bad):
    assert keys.parse(bad) is None


def test_public_id_of_a_malformed_key_is_none():
    assert keys.public_id("garbage") is None


def test_generation_does_not_collide():
    seen = {keys.generate()[0] for _ in range(500)}
    assert len(seen) == 500


def test_verify_rejects_an_empty_or_bogus_stored_hash():
    full, _, _ = keys.generate()
    _, secret = keys.parse(full)
    assert not keys.verify(secret, "")
    assert not keys.verify(secret, "not-a-hash")
