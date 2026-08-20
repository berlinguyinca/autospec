#!/usr/bin/env python3
"""API key format, hashing and constant-time verification. No I/O, ever.

A key looks like

    qtk_<12-char key id>_<32-char secret>

The KEY ID IS NOT SECRET. It is stored in clear, printed in the dashboard,
attached to every usage row, and is how a human refers to one key among several.
It also makes authentication an O(1) lookup instead of a scan across every
stored hash.

The SECRET is 160 bits of CSPRNG output, stored as a SHA-256 digest and shown
exactly once at creation.

WHY SHA-256 AND NOT ARGON2/BCRYPT. A password KDF's work factor exists to make
guessing a LOW-ENTROPY secret expensive. There is nothing to guess here: the
secret is uniformly random over 2^160. A work factor would buy no security and
would add its cost to every single inference request. This is a deliberate
choice, not an oversight -- if the key format ever changes to something
user-chosen, this reasoning stops holding and a KDF becomes required.

A Cognito `sub` must NEVER be accepted as a key: it is neither secret nor
revocable. See the design's section 2.2.

THREE NAMESPACES, NEVER INTERCHANGEABLE

    qtk_  a USER key       -- authenticates inference
    qts_  a SERVER key     -- authenticates an agent offering capacity
    qte_  an ENROLMENT key -- single use, traded once for a qts_

They share this format and nothing else. The prefix is part of the pattern, so
`parse()` for one namespace refuses the others by shape before any lookup
happens -- which is the mechanism that stops a server credential from becoming an
inference key. The cheap alternative, one regex with an alternation, would make
every caller responsible for checking which kind it got.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import re
import secrets

PREFIX = "qtk"              # a user key
PREFIX_SERVER = "qts"       # a server credential
PREFIX_ENROL = "qte"        # a single-use enrolment token
KEY_ID_LEN = 12
SECRET_LEN = 32

# Lowercase base32 without padding. Restricting the alphabet up front means a
# malformed key is rejected by shape before anything touches the database.
_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567"


def _pattern(prefix: str):
    return re.compile(
        r"^%s_([%s]{%d})_([%s]{%d})$"
        % (re.escape(prefix), _ALPHABET, KEY_ID_LEN, _ALPHABET, SECRET_LEN))


# One compiled pattern per namespace, built once. A single pattern with an
# alternation would match all three and push the "which kind is this?" question
# out to every caller, which is exactly where it would eventually be forgotten.
_PATTERNS = {p: _pattern(p) for p in (PREFIX, PREFIX_SERVER, PREFIX_ENROL)}


def _token(n_chars: int) -> str:
    """n_chars of lowercase base32. 5 bits per character, so ask for enough
    bytes and slice -- never pad, which would leak the boundary."""
    raw = secrets.token_bytes(n_chars)  # generous; sliced below
    return base64.b32encode(raw).decode().lower().rstrip("=")[:n_chars]


def generate(prefix: str = PREFIX) -> tuple[str, str, str]:
    """Return (full_key, key_id, secret_hash). The full key is the ONLY time the
    secret exists in a returnable form; the caller must show it once and drop
    it."""
    if prefix not in _PATTERNS:
        raise ValueError(f"unknown credential namespace {prefix!r}")
    key_id = _token(KEY_ID_LEN)
    secret = _token(SECRET_LEN)
    return f"{prefix}_{key_id}_{secret}", key_id, hash_secret(secret)


def parse(presented: str, prefix: str = PREFIX) -> tuple[str, str] | None:
    """Split a presented credential into (key_id, secret), or None if it is not
    shaped like one of ours.

    Deliberately strict and deliberately silent: callers get None for an empty
    string, a wrong prefix, wrong lengths, wrong case, an un-stripped
    "Bearer " prefix, or extra fields. Nothing here reports WHY, because the
    caller answers 401 either way and a more specific error is an oracle.
    """
    if not isinstance(presented, str):
        return None
    pattern = _PATTERNS.get(prefix)
    if pattern is None:
        raise ValueError(f"unknown credential namespace {prefix!r}")
    m = pattern.match(presented)
    if not m:
        return None
    return m.group(1), m.group(2)


def public_id(presented: str, prefix: str = PREFIX) -> str | None:
    """The non-secret half, for display and attribution. None if malformed."""
    parsed = parse(presented, prefix)
    return parsed[0] if parsed else None


def hash_secret(secret: str) -> str:
    return hashlib.sha256(secret.encode("utf-8")).hexdigest()


def verify(secret: str, stored_hash: str) -> bool:
    """Constant-time comparison. An empty or malformed stored hash is a refusal,
    not a crash -- a row damaged in the mirror must not become a bypass."""
    if not stored_hash or not isinstance(stored_hash, str):
        return False
    return hmac.compare_digest(hash_secret(secret), stored_hash)
