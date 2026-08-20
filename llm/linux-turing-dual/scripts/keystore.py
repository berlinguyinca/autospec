#!/usr/bin/env python3
"""Key and usage storage.

TWO STORES, ONE AUTHORITY FOR EACH QUESTION.

  * The local SQLite mirror is the ENFORCEMENT point. Authentication reads only
    it, so inference never depends on a remote database being reachable. That is
    a hard requirement: the registry is on another host and is not on this
    node's failure budget.
  * PostgreSQL is the shared REGISTRY of record, so a second node (or the same
    node rebuilt) sees the same keys.

Writes go local-first, then to the registry; anything the registry did not take
stays queued and is retried. A registry outage degrades to "usage is stale", not
"the node is down".

REVOCATION IS MONOTONIC, and that is the subtle part. A pull from the registry
must never clear a local `revoked_at`: if a revoke has not yet been pushed, a
naive "registry wins" merge would resurrect the key it had just killed. See
apply_registry_rows().
"""
from __future__ import annotations

import datetime as _dt
import sqlite3
import threading
import uuid
from dataclasses import dataclass, field

import keys as _keys

_SCHEMA = """
CREATE TABLE IF NOT EXISTS users (
  sub           TEXT PRIMARY KEY,
  email         TEXT,
  display_name  TEXT,
  last_login_at TEXT
);
CREATE TABLE IF NOT EXISTS api_keys (
  key_id       TEXT PRIMARY KEY,
  sub          TEXT NOT NULL,
  secret_hash  TEXT NOT NULL,
  label        TEXT,
  created_at   TEXT NOT NULL,
  expires_at   TEXT,
  revoked_at   TEXT,
  last_used_at TEXT,
  synced       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS api_keys_sub_idx ON api_keys (sub);
CREATE TABLE IF NOT EXISTS usage_events (
  event_id          TEXT PRIMARY KEY,
  ts                TEXT NOT NULL,
  key_id            TEXT NOT NULL,
  sub               TEXT,
  model             TEXT,
  upstream          TEXT NOT NULL DEFAULT 'local',
  endpoint          TEXT,
  prompt_tokens     INTEGER,
  completion_tokens INTEGER,
  cached_tokens     INTEGER,
  prompt_ms         REAL,
  predicted_ms      REAL,
  status_code       INTEGER,
  streamed          INTEGER NOT NULL DEFAULT 0,
  truncated         INTEGER NOT NULL DEFAULT 0,
  synced            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS usage_sync_idx ON usage_events (synced);
"""


def _iso(epoch: float) -> str:
    return _dt.datetime.fromtimestamp(epoch, _dt.timezone.utc).isoformat()


def _epoch(iso: str | None) -> float | None:
    if not iso:
        return None
    try:
        return _dt.datetime.fromisoformat(iso).timestamp()
    except ValueError:
        return None


@dataclass
class KeyRow:
    key_id: str
    sub: str
    label: str | None = None
    created_at: str | None = None
    expires_at: str | None = None
    revoked_at: str | None = None
    last_used_at: str | None = None
    secret_hash: str = field(default="", repr=False)

    def as_public(self) -> dict:
        """What the dashboard may see. Deliberately excludes secret_hash: it is
        not a credential, but it is also of no use to a user and every field that
        never leaves the process is a field that cannot leak."""
        return {"key_id": self.key_id, "sub": self.sub, "label": self.label,
                "created_at": self.created_at, "expires_at": self.expires_at,
                "revoked_at": self.revoked_at, "last_used_at": self.last_used_at}


class KeyStore:
    def __init__(self, sqlite_path: str, dsn: str | None = None) -> None:
        self.path = sqlite_path
        self.dsn = dsn or None
        self._lock = threading.Lock()
        self._pg_ok: bool | None = None
        self._pg_error: str | None = None

    # --- local plumbing ----------------------------------------------------
    def _conn(self) -> sqlite3.Connection:
        c = sqlite3.connect(self.path, timeout=10)
        c.row_factory = sqlite3.Row
        c.execute("PRAGMA journal_mode=WAL")
        c.execute("PRAGMA foreign_keys=ON")
        return c

    def migrate_local(self) -> None:
        with self._lock, self._conn() as c:
            c.executescript(_SCHEMA)

    # --- users -------------------------------------------------------------
    def upsert_user(self, sub: str, email: str | None = None,
                    name: str | None = None, login_at: float | None = None) -> None:
        """email and display_name are refreshed from the verified token on every
        login and are PRESENTATION ONLY -- never an identity or authz input."""
        with self._lock, self._conn() as c:
            c.execute(
                "INSERT INTO users (sub, email, display_name, last_login_at) "
                "VALUES (?,?,?,?) ON CONFLICT(sub) DO UPDATE SET "
                "email=excluded.email, display_name=excluded.display_name, "
                "last_login_at=COALESCE(excluded.last_login_at, users.last_login_at)",
                (sub, email, name, _iso(login_at) if login_at else None))
        self._push_user(sub, email, name, login_at)

    # --- keys --------------------------------------------------------------
    def mint(self, sub: str, label: str | None = None,
             ttl_days: int | None = None, now: float | None = None) -> tuple[str, KeyRow]:
        import time
        at = time.time() if now is None else now
        full, key_id, secret_hash = _keys.generate()
        row = KeyRow(key_id=key_id, sub=sub, label=label, created_at=_iso(at),
                     expires_at=_iso(at + ttl_days * 86400) if ttl_days else None,
                     secret_hash=secret_hash)
        with self._lock, self._conn() as c:
            c.execute("INSERT INTO api_keys (key_id, sub, secret_hash, label, "
                      "created_at, expires_at, synced) VALUES (?,?,?,?,?,?,0)",
                      (row.key_id, row.sub, row.secret_hash, row.label,
                       row.created_at, row.expires_at))
        self._push_key(row)
        return full, row

    def authenticate(self, presented: str, now: float) -> KeyRow | None:
        """Local-only, O(1) by key id. Returns None for every failure -- unknown
        key, wrong secret, revoked, expired -- because the caller answers 401
        either way and a distinguishable error is an oracle."""
        parsed = _keys.parse(presented or "")
        if not parsed:
            return None
        key_id, secret = parsed
        with self._lock, self._conn() as c:
            r = c.execute("SELECT * FROM api_keys WHERE key_id=?", (key_id,)).fetchone()
            if r is None:
                return None
            if not _keys.verify(secret, r["secret_hash"]):
                return None
            if r["revoked_at"]:
                return None
            exp = _epoch(r["expires_at"])
            if exp is not None and now >= exp:
                return None
            c.execute("UPDATE api_keys SET last_used_at=? WHERE key_id=?",
                      (_iso(now), key_id))
            return KeyRow(key_id=r["key_id"], sub=r["sub"], label=r["label"],
                          created_at=r["created_at"], expires_at=r["expires_at"],
                          revoked_at=r["revoked_at"], last_used_at=_iso(now),
                          secret_hash=r["secret_hash"])

    def revoke(self, key_id: str, *, sub: str, is_admin: bool,
               now: float | None = None) -> bool:
        """Applies LOCALLY BEFORE RETURNING, so a caller that saw success can
        rely on the key being dead here. The registry write is best-effort; a
        pull can never undo this (see apply_registry_rows)."""
        import time
        at = _iso(time.time() if now is None else now)
        with self._lock, self._conn() as c:
            r = c.execute("SELECT sub, revoked_at FROM api_keys WHERE key_id=?",
                          (key_id,)).fetchone()
            if r is None:
                return False
            if not is_admin and r["sub"] != sub:
                # Not "not found": the caller is authenticated, just not
                # entitled. Nothing is mutated.
                return False
            if r["revoked_at"]:
                return True                      # idempotent
            c.execute("UPDATE api_keys SET revoked_at=?, synced=0 WHERE key_id=?",
                      (at, key_id))
        self._push_revoke(key_id, at)
        return True

    def list_keys(self, sub: str, *, all_users: bool = False) -> list[KeyRow]:
        with self._lock, self._conn() as c:
            if all_users:
                rows = c.execute("SELECT * FROM api_keys ORDER BY created_at").fetchall()
            else:
                rows = c.execute("SELECT * FROM api_keys WHERE sub=? "
                                 "ORDER BY created_at", (sub,)).fetchall()
        return [KeyRow(key_id=r["key_id"], sub=r["sub"], label=r["label"],
                       created_at=r["created_at"], expires_at=r["expires_at"],
                       revoked_at=r["revoked_at"], last_used_at=r["last_used_at"],
                       secret_hash=r["secret_hash"]) for r in rows]

    # --- usage -------------------------------------------------------------
    def record_usage(self, rec: dict) -> str:
        """One row per completed request. event_id is generated HERE so a flush
        that partially succeeded cannot double-count on retry."""
        eid = rec.get("event_id") or str(uuid.uuid4())
        ts = rec.get("ts")
        with self._lock, self._conn() as c:
            c.execute(
                "INSERT OR IGNORE INTO usage_events (event_id, ts, key_id, sub, "
                "model, upstream, endpoint, prompt_tokens, completion_tokens, "
                "cached_tokens, prompt_ms, predicted_ms, status_code, streamed, "
                "truncated, synced) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0)",
                (eid, _iso(ts) if isinstance(ts, (int, float)) else str(ts),
                 rec.get("key_id"), rec.get("sub"), rec.get("model"),
                 rec.get("upstream") or "local", rec.get("endpoint"),
                 rec.get("prompt_tokens"), rec.get("completion_tokens"),
                 rec.get("cached_tokens"), rec.get("prompt_ms"),
                 rec.get("predicted_ms"), rec.get("status_code"),
                 1 if rec.get("streamed") else 0,
                 1 if rec.get("truncated") else 0))
        return eid

    def pending_usage_ids(self) -> list[str]:
        with self._lock, self._conn() as c:
            return [r["event_id"] for r in
                    c.execute("SELECT event_id FROM usage_events WHERE synced=0")]

    def usage(self, sub: str | None = None, days: int = 30) -> list[dict]:
        """Aggregated from the LOCAL table, so the panel works during a registry
        outage. `truncated_requests` is reported separately and never folded into
        the token totals -- those rows have genuinely unknown counts."""
        where, args = "", []
        if sub:
            where = "WHERE e.sub=?"
            args = [sub]
        sql = (f"SELECT e.key_id AS key_id, e.sub AS sub, u.display_name, u.email, "
               f"k.label AS label, e.model AS model, COUNT(*) AS requests, "
               f"SUM(e.truncated) AS truncated_requests, "
               f"SUM(COALESCE(e.prompt_tokens,0)) AS prompt_tokens, "
               f"SUM(COALESCE(e.completion_tokens,0)) AS completion_tokens, "
               f"SUM(COALESCE(e.cached_tokens,0)) AS cached_tokens, "
               f"SUM(COALESCE(e.prompt_tokens,0) + COALESCE(e.completion_tokens,0)) "
               f"  AS total_tokens "
               f"FROM usage_events e "
               f"LEFT JOIN users u ON u.sub = e.sub "
               f"LEFT JOIN api_keys k ON k.key_id = e.key_id "
               f"{where} GROUP BY e.key_id, e.sub, u.display_name, u.email, k.label, e.model "
               f"ORDER BY total_tokens DESC, requests DESC")
        with self._lock, self._conn() as c:
            return [dict(r) for r in c.execute(sql, args)]

    def last_upstream(self, key_id: str) -> str | None:
        """The server this key used most recently, for routing affinity.

        Read from recorded usage rather than kept only in memory, so a gateway
        restart does not scatter every caller onto a cold prefix cache.
        """
        with self._lock, self._conn() as c:
            r = c.execute("SELECT upstream FROM usage_events WHERE key_id=? "
                          "ORDER BY ts DESC LIMIT 1", (key_id,)).fetchone()
        return r["upstream"] if r and r["upstream"] else None

    def leaderboard(self) -> list[dict]:
        """Per-USER totals, ranked by tokens. The scoreboard.

        Joined to `users` so a row can name a person rather than an opaque
        subject id. Falls back to the subject when a user row has not been seen
        yet -- which happens for a key minted through the break-glass before its
        owner has ever signed in.

        Deliberately NOT scoped to one subject: a scoreboard that shows only your
        own score is not a scoreboard. It stays behind authentication, though --
        this is a lab leaderboard, not a public one.
        """
        sql = ("SELECT e.sub AS sub, u.display_name, u.email, "
               "       COUNT(*) AS requests, "
               "       COUNT(DISTINCT e.key_id) AS keys, "
               "       SUM(e.truncated) AS truncated_requests, "
               "       SUM(COALESCE(e.prompt_tokens,0)) AS prompt_tokens, "
               "       SUM(COALESCE(e.completion_tokens,0)) AS completion_tokens, "
               "       SUM(COALESCE(e.cached_tokens,0)) AS cached_tokens, "
               "       SUM(COALESCE(e.prompt_tokens,0) + COALESCE(e.completion_tokens,0)) "
               "         AS total_tokens "
               "  FROM usage_events e LEFT JOIN users u ON u.sub = e.sub "
               " GROUP BY e.sub, u.display_name, u.email "
               " ORDER BY total_tokens DESC, requests DESC")
        with self._lock, self._conn() as c:
            return [dict(r) for r in c.execute(sql)]

    # --- registry merge ----------------------------------------------------
    def apply_registry_rows(self, rows: list[dict]) -> int:
        """Merge registry rows into the mirror.

        THE MERGE RULE: `revoked_at` takes the EARLIEST non-null of local and
        remote. Revocation is monotonic, so a registry row that predates a local
        revoke must not clear it -- otherwise a routine sync silently restores a
        credential someone just killed. Every other column takes the remote
        value, since the registry is the record of truth for them.
        """
        n = 0
        with self._lock, self._conn() as c:
            for r in rows:
                kid = r.get("key_id")
                if not kid:
                    continue
                local = c.execute("SELECT revoked_at FROM api_keys WHERE key_id=?",
                                  (kid,)).fetchone()
                revoked = r.get("revoked_at")
                if local and local["revoked_at"]:
                    revoked = min(x for x in (local["revoked_at"], revoked) if x)
                c.execute(
                    "INSERT INTO api_keys (key_id, sub, secret_hash, label, "
                    "created_at, expires_at, revoked_at, last_used_at, synced) "
                    "VALUES (?,?,?,?,?,?,?,?,1) ON CONFLICT(key_id) DO UPDATE SET "
                    "sub=excluded.sub, secret_hash=excluded.secret_hash, "
                    "label=excluded.label, expires_at=excluded.expires_at, "
                    "revoked_at=excluded.revoked_at",
                    (kid, r.get("sub"), r.get("secret_hash") or "", r.get("label"),
                     r.get("created_at"), r.get("expires_at"), revoked,
                     r.get("last_used_at")))
                n += 1
        return n

    # --- registry I/O (best effort, never on the auth path) ----------------
    def _pg(self):
        if not self.dsn:
            return None
        try:
            import psycopg2
            return psycopg2.connect(self.dsn, connect_timeout=3)
        except Exception as exc:
            self._pg_ok = False
            self._pg_error = str(exc).strip().splitlines()[0] if str(exc) else "unknown"
            return None

    def _push_user(self, sub, email, name, login_at) -> None:
        conn = self._pg()
        if conn is None:
            return
        try:
            with conn, conn.cursor() as cur:
                cur.execute(
                    "INSERT INTO llm.users (sub, email, display_name, last_login_at) "
                    "VALUES (%s,%s,%s, COALESCE(%s, now())) ON CONFLICT (sub) DO UPDATE "
                    "SET email=EXCLUDED.email, display_name=EXCLUDED.display_name, "
                    "last_login_at=EXCLUDED.last_login_at",
                    (sub, email, name, _iso(login_at) if login_at else None))
            self._pg_ok = True
        except Exception as exc:
            self._pg_ok, self._pg_error = False, str(exc).strip().splitlines()[0]
        finally:
            conn.close()

    def _push_key(self, row: KeyRow) -> None:
        conn = self._pg()
        if conn is None:
            return
        try:
            with conn, conn.cursor() as cur:
                cur.execute(
                    "INSERT INTO llm.api_keys (key_id, sub, secret_hash, label, "
                    "created_at, expires_at) VALUES (%s,%s,%s,%s,%s,%s) "
                    "ON CONFLICT (key_id) DO NOTHING",
                    (row.key_id, row.sub, row.secret_hash, row.label,
                     row.created_at, row.expires_at))
            with self._lock, self._conn() as c:
                c.execute("UPDATE api_keys SET synced=1 WHERE key_id=?", (row.key_id,))
            self._pg_ok = True
        except Exception as exc:
            self._pg_ok, self._pg_error = False, str(exc).strip().splitlines()[0]
        finally:
            conn.close()

    def _push_revoke(self, key_id: str, at: str) -> None:
        conn = self._pg()
        if conn is None:
            return
        try:
            with conn, conn.cursor() as cur:
                cur.execute("UPDATE llm.api_keys SET revoked_at=COALESCE(revoked_at,%s) "
                            "WHERE key_id=%s", (at, key_id))
            with self._lock, self._conn() as c:
                c.execute("UPDATE api_keys SET synced=1 WHERE key_id=?", (key_id,))
            self._pg_ok = True
        except Exception as exc:
            self._pg_ok, self._pg_error = False, str(exc).strip().splitlines()[0]
        finally:
            conn.close()

    def flush(self) -> tuple[int, int]:
        """Send queued usage to the registry. Returns (flushed, remaining).

        With no registry configured this is a no-op that KEEPS the rows -- they
        are the local aggregate the dashboard reads, so discarding them would
        delete the very data the flush exists to preserve.
        """
        pending = self.pending_usage_ids()
        if not self.dsn:
            return 0, len(pending)
        if not pending:
            return 0, 0
        conn = self._pg()
        if conn is None:
            return 0, len(pending)
        sent = 0
        try:
            with self._lock, self._conn() as c:
                rows = c.execute("SELECT * FROM usage_events WHERE synced=0").fetchall()
            with conn, conn.cursor() as cur:
                for r in rows:
                    cur.execute(
                        "INSERT INTO llm.usage_events (event_id, ts, key_id, sub, "
                        "model, upstream, endpoint, prompt_tokens, completion_tokens, "
                        "cached_tokens, prompt_ms, predicted_ms, status_code, "
                        "streamed, truncated) VALUES "
                        "(%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s) "
                        "ON CONFLICT (event_id) DO NOTHING",
                        (r["event_id"], r["ts"], r["key_id"], r["sub"], r["model"],
                         r["upstream"], r["endpoint"], r["prompt_tokens"],
                         r["completion_tokens"], r["cached_tokens"], r["prompt_ms"],
                         r["predicted_ms"], r["status_code"],
                         bool(r["streamed"]), bool(r["truncated"])))
                    sent += 1
            with self._lock, self._conn() as c:
                c.executemany("UPDATE usage_events SET synced=1 WHERE event_id=?",
                              [(r["event_id"],) for r in rows])
            self._pg_ok = True
        except Exception as exc:
            self._pg_ok, self._pg_error = False, str(exc).strip().splitlines()[0]
            return 0, len(pending)
        finally:
            conn.close()
        return sent, len(self.pending_usage_ids())

    def refresh(self) -> int:
        """Pull the registry into the mirror. Bounded staleness, stated: a revoke
        issued on ANOTHER node becomes effective here within one refresh."""
        conn = self._pg()
        if conn is None:
            return 0
        try:
            with conn, conn.cursor() as cur:
                cur.execute("SELECT key_id, sub, secret_hash, label, created_at, "
                            "expires_at, revoked_at, last_used_at FROM llm.api_keys")
                cols = [d[0] for d in cur.description]
                rows = [dict(zip(cols, r)) for r in cur.fetchall()]
            for r in rows:
                for k in ("created_at", "expires_at", "revoked_at", "last_used_at"):
                    if r.get(k) is not None:
                        r[k] = r[k].isoformat() if hasattr(r[k], "isoformat") else str(r[k])
            self._pg_ok = True
            return self.apply_registry_rows(rows)
        except Exception as exc:
            self._pg_ok, self._pg_error = False, str(exc).strip().splitlines()[0]
            return 0
        finally:
            conn.close()

    def health(self) -> dict:
        return {"registry_configured": bool(self.dsn),
                "registry_reachable": self._pg_ok,
                "registry_error": self._pg_error,
                "pending_usage": len(self.pending_usage_ids())}
