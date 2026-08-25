#!/usr/bin/env python3
"""Ban an IP that keeps presenting credentials that do not work.

Policy (deliberately boring): FAILS failures inside WINDOW seconds earns a ban
of BAN seconds. Defaults 5 / 600 / 3600 -- enough to stop someone walking a key
space, forgiving enough that a person pasting a stale key loses an hour at worst.

WHAT COUNTS AS A FAILURE, and what must never:

  * a presented credential that resolved to nothing -- counted.
  * NO credential at all -- not counted. "I did not log in" is not an attack,
    and counting it bans every browser that loads the page before signing in.
  * anything from loopback -- not counted, and never banned. The node's own
    components talk to the gateway over 127.0.0.1; a stale internal credential
    is a configuration bug, and one of those was live on this host for months
    (the dashboard polled /metrics ~2x/sec with the wrong key). A lockout that
    counted those would have banned the node from itself.

IDENTITY. Behind nginx every request arrives from 127.0.0.1, so the real client
comes from X-Real-IP -- and that header is trusted ONLY when the socket peer is
loopback, i.e. only when nginx is the one that set it. Trusting it from an
arbitrary peer would let a caller pick their own identity: evade a ban by
rotating it, or frame somebody else's address into one.

PERSISTENCE. Bans live in SQLite next to the key mirror, so restarting the
gateway does not hand an attacker a clean slate -- restarts are cheap to cause.
Counters are in memory: losing a partial count on restart is harmless.
"""
from __future__ import annotations

import ipaddress
import os
import sqlite3
import threading
import time

FAILS = int(os.environ.get("QT_LOCKOUT_FAILS", "5"))
WINDOW = float(os.environ.get("QT_LOCKOUT_WINDOW", "600"))
BAN = float(os.environ.get("QT_LOCKOUT_BAN", "3600"))

_LOCK = threading.Lock()
_FAILS: dict[str, list[float]] = {}     # ip -> failure timestamps in-window
_BANS: dict[str, float] = {}            # ip -> expiry epoch
_DB: str | None = None


def configure(db_path: str | None) -> None:
    """Point persistence at a database, and reload any bans still standing."""
    global _DB
    _DB = db_path
    if not _DB:
        return
    try:
        with sqlite3.connect(_DB, timeout=5) as c:
            c.execute("CREATE TABLE IF NOT EXISTS lockout ("
                      "ip TEXT PRIMARY KEY, until REAL NOT NULL, "
                      "reason TEXT, at REAL)")
            now = time.time()
            rows = c.execute("SELECT ip, until FROM lockout WHERE until > ?",
                             (now,)).fetchall()
        with _LOCK:
            for ip, until in rows:
                _BANS[ip] = until
    except sqlite3.Error:
        # Persistence is a hardening measure, not a precondition. A gateway that
        # refused to start because a ban table was unavailable would be trading
        # a small security property for an outage.
        pass


def is_loopback(ip: str) -> bool:
    try:
        return ipaddress.ip_address(ip).is_loopback
    except ValueError:
        return False


def client_ip(get_header, peer: str) -> str:
    """The address to hold responsible.

    X-Real-IP only from a loopback peer -- see the module docstring. `peer` is
    the socket address, which is what nginx is.
    """
    if is_loopback(peer):
        fwd = (get_header("X-Real-IP") or "").strip()
        if fwd:
            return fwd
    return peer


def banned_until(ip: str, now: float | None = None) -> float | None:
    now = time.time() if now is None else now
    if is_loopback(ip):
        return None
    with _LOCK:
        until = _BANS.get(ip)
        if until is None:
            return None
        if until <= now:
            _BANS.pop(ip, None)
            return None
        return until


def record_failure(ip: str, now: float | None = None) -> float | None:
    """Count one bad credential. Returns the ban expiry if this caused a ban."""
    now = time.time() if now is None else now
    if is_loopback(ip):
        return None
    with _LOCK:
        hits = [t for t in _FAILS.get(ip, []) if t > now - WINDOW]
        hits.append(now)
        _FAILS[ip] = hits
        if len(hits) < FAILS:
            return None
        until = now + BAN
        _BANS[ip] = until
        _FAILS.pop(ip, None)        # the window is spent; the ban replaces it
    _persist(ip, until, f"{FAILS} bad credentials in {int(WINDOW)}s", now)
    return until


def _persist(ip: str, until: float, reason: str, at: float) -> None:
    if not _DB:
        return
    try:
        with sqlite3.connect(_DB, timeout=5) as c:
            c.execute("INSERT INTO lockout(ip, until, reason, at) "
                      "VALUES(?,?,?,?) ON CONFLICT(ip) DO UPDATE SET "
                      "until=excluded.until, reason=excluded.reason, at=excluded.at",
                      (ip, until, reason, at))
    except sqlite3.Error:
        pass


def clear(ip: str) -> None:
    """Lift a ban. For an operator who locked themselves out, and for tests."""
    with _LOCK:
        _BANS.pop(ip, None)
        _FAILS.pop(ip, None)
    if _DB:
        try:
            with sqlite3.connect(_DB, timeout=5) as c:
                c.execute("DELETE FROM lockout WHERE ip = ?", (ip,))
        except sqlite3.Error:
            pass


def reset_for_tests() -> None:
    global _DB
    with _LOCK:
        _FAILS.clear()
        _BANS.clear()
    _DB = None
