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
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import re
import secrets

PREFIX = "qtk"
KEY_ID_LEN = 12
SECRET_LEN = 32

# Lowercase base32 without padding. Restricting the alphabet up front means a
# malformed key is rejected by shape before anything touches the database.
_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567"
_KEY_RE = re.compile(
    r"^%s_([%s]{%d})_([%s]{%d})$"
    % (re.escape(PREFIX), _ALPHABET, KEY_ID_LEN, _ALPHABET, SECRET_LEN)
)


def _token(n_chars: int) -> str:
    """n_chars of lowercase base32. 5 bits per character, so ask for enough
    bytes and slice -- never pad, which would leak the boundary."""
    raw = secrets.token_bytes(n_chars)  # generous; sliced below
    return base64.b32encode(raw).decode().lower().rstrip("=")[:n_chars]


def generate() -> tuple[str, str, str]:
    """Return (full_key, key_id, secret_hash). The full key is the ONLY time the
    secret exists in a returnable form; the caller must show it once and drop
    it."""
    key_id = _token(KEY_ID_LEN)
    secret = _token(SECRET_LEN)
    return f"{PREFIX}_{key_id}_{secret}", key_id, hash_secret(secret)


def parse(presented: str) -> tuple[str, str] | None:
    """Split a presented credential into (key_id, secret), or None if it is not
    shaped like one of ours.

    Deliberately strict and deliberately silent: callers get None for an empty
    string, a wrong prefix, wrong lengths, wrong case, an un-stripped
    "Bearer " prefix, or extra fields. Nothing here reports WHY, because the
    caller answers 401 either way and a more specific error is an oracle.
    """
    if not isinstance(presented, str):
        return None
    m = _KEY_RE.match(presented)
    if not m:
        return None
    return m.group(1), m.group(2)


def public_id(presented: str) -> str | None:
    """The non-secret half, for display and attribution. None if malformed."""
    parsed = parse(presented)
    return parsed[0] if parsed else None


def hash_secret(secret: str) -> str:
    return hashlib.sha256(secret.encode("utf-8")).hexdigest()


def verify(secret: str, stored_hash: str) -> bool:
    """Constant-time comparison. An empty or malformed stored hash is a refusal,
    not a crash -- a row damaged in the mirror must not become a bypass."""
    if not stored_hash or not isinstance(stored_hash, str):
        return False
    return hmac.compare_digest(hash_secret(secret), stored_hash)
