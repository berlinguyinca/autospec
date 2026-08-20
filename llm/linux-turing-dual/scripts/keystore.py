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

# Duplicated from upstreams.RESERVED rather than imported, to keep this module
# free of the registry parser (and of pyyaml). A test asserts the two agree --
# a server called `auto` would shadow the balancer itself.
RESERVED_SERVER_IDS = ("local", "auto")
_SERVER_ID_RE = __import__("re").compile(r"^[a-z0-9][a-z0-9-]{0,30}$")
SERVER_KINDS = ("tunnel", "static", "file")
ENROL_TTL_SECONDS = 1800

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

-- Servers, and the tokens that enrol them.
--
-- NODE-LOCAL ON PURPOSE, unlike keys and usage. A tunnelled server's capacity
-- exists only where its pipes are held, so a row replicated to another node
-- would describe capacity that node cannot reach. When a second node exists and
-- static servers want sharing, this becomes a registry table like the others --
-- until then, replicating it would be inventing a distributed problem.
CREATE TABLE IF NOT EXISTS servers (
  server_id    TEXT PRIMARY KEY,
  sub          TEXT,                          -- NULL for a file-configured entry
  kind         TEXT NOT NULL,                 -- tunnel | static | file
  base_url     TEXT,                          -- static/file only
  note         TEXT,
  gpus         TEXT,
  priority     INTEGER NOT NULL DEFAULT 0,
  pool_member  INTEGER NOT NULL DEFAULT 0,
  secret_hash  TEXT,                          -- NULL for a file entry
  created_at   TEXT NOT NULL,
  revoked_at   TEXT,
  last_seen    TEXT
);
CREATE TABLE IF NOT EXISTS enrol_tokens (
  token_id     TEXT PRIMARY KEY,
  server_id    TEXT NOT NULL,
  sub          TEXT NOT NULL,
  kind         TEXT NOT NULL,
  base_url     TEXT,
  note         TEXT,
  gpus         TEXT,
  secret_hash  TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  expires_at   TEXT NOT NULL,
  used_at      TEXT
);
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


@dataclass
class ServerRow:
    server_id: str
    sub: str | None
    kind: str
    base_url: str | None = None
    note: str | None = None
    gpus: str | None = None
    priority: int = 0
    pool_member: bool = False
    created_at: str = ""
    revoked_at: str | None = None
    last_seen: str | None = None

    def as_public(self) -> dict:
        """What the dashboard may see. No credential hash, and for a tunnelled
        server no address either -- there is none, and inventing a field for it
        would invite someone to fill it in."""
        return {"server_id": self.server_id, "sub": self.sub, "kind": self.kind,
                "base_url": self.base_url if self.kind != "tunnel" else None,
                "note": self.note, "gpus": self.gpus, "priority": self.priority,
                "pool_member": self.pool_member, "created_at": self.created_at,
                "revoked_at": self.revoked_at, "last_seen": self.last_seen}


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

    def last_upstream(self, key_id: str,
                      endpoints: "tuple[str, ...] | None" = None) -> str | None:
        """The server this key used most recently, for routing affinity.

        Read from recorded usage rather than kept only in memory, so a gateway
        restart does not scatter every caller onto a cold prefix cache.

        `endpoints` restricts the answer to the endpoints that were actually
        ROUTED. Every request is recorded, including the /health and /v1/models
        polls that always say "local" -- so without this filter an agent's health
        poll would, after a restart, look like its last routing decision and pull
        it off a warm remote slot. The in-memory path already ignores those; this
        is the same rule for the durable one.
        """
        q = "SELECT upstream FROM usage_events WHERE key_id=?"
        args: list = [key_id]
        if endpoints:
            q += " AND endpoint IN (%s)" % ",".join("?" * len(endpoints))
            args.extend(endpoints)
        with self._lock, self._conn() as c:
            r = c.execute(q + " ORDER BY ts DESC LIMIT 1", args).fetchone()
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

    # --- servers -----------------------------------------------------------
    def _validate_server_id(self, server_id: str) -> None:
        """Refuse an id that cannot be a path segment, is reserved, or is taken.

        Raised rather than returned because every caller is a person attaching a
        machine and needs to be told which rule they hit.
        """
        if not _SERVER_ID_RE.match(server_id or ""):
            raise ValueError("a server id is lowercase letters, digits and "
                             "dashes, starting with a letter or digit")
        if server_id in RESERVED_SERVER_IDS:
            raise ValueError(f"'{server_id}' is reserved")
        with self._lock, self._conn() as c:
            taken = c.execute("SELECT 1 FROM servers WHERE server_id=? "
                              "AND revoked_at IS NULL", (server_id,)).fetchone()
            pending = c.execute("SELECT 1 FROM enrol_tokens WHERE server_id=? "
                                "AND used_at IS NULL", (server_id,)).fetchone()
        if taken or pending:
            raise ValueError(f"'{server_id}' is already registered or being "
                             f"enrolled")

    def enrol_token(self, sub: str, server_id: str, *, kind: str = "tunnel",
                    base_url: str | None = None, note: str | None = None,
                    gpus: str | None = None, now: float | None = None) -> str:
        """A single-use token for attaching one server. Shown once.

        A static server needs an address up front, because the node dials it; a
        tunnelled one must not carry one, because the node never dials it and a
        stored address would be a lie that outlives the pipe.
        """
        import time
        at = time.time() if now is None else now
        if kind not in ("tunnel", "static"):
            raise ValueError("a server is attached as 'tunnel' or 'static'")
        if kind == "static" and not (base_url or "").startswith(("http://", "https://")):
            raise ValueError("a static server needs a base_url starting with "
                             "http:// or https://")
        if kind == "tunnel" and base_url:
            raise ValueError("a tunnelled server has no address of its own")
        self._validate_server_id(server_id)
        full, token_id, secret_hash = _keys.generate(_keys.PREFIX_ENROL)
        with self._lock, self._conn() as c:
            c.execute("INSERT INTO enrol_tokens (token_id, server_id, sub, kind, "
                      "base_url, note, gpus, secret_hash, created_at, expires_at) "
                      "VALUES (?,?,?,?,?,?,?,?,?,?)",
                      (token_id, server_id, sub, kind, base_url, note, gpus,
                       secret_hash, _iso(at), _iso(at + ENROL_TTL_SECONDS)))
        return full

    def redeem_token(self, token: str, *,
                     now: float | None = None) -> tuple[str, str] | None:
        """(server_id, server credential), or None.

        None covers every failure -- unknown, expired, already used, wrong secret
        -- because the caller is unauthenticated and distinguishing them is an
        oracle. The row is marked used in the SAME transaction that creates the
        server, so a race cannot mint two credentials from one token.
        """
        import time
        at = time.time() if now is None else now
        parsed = _keys.parse(token, _keys.PREFIX_ENROL)
        if not parsed:
            return None
        token_id, secret = parsed
        full, _, secret_hash = _keys.generate(_keys.PREFIX_SERVER)
        with self._lock, self._conn() as c:
            row = c.execute("SELECT * FROM enrol_tokens WHERE token_id=?",
                            (token_id,)).fetchone()
            if not row or row["used_at"]:
                return None
            if not _keys.verify(secret, row["secret_hash"]):
                return None
            if _iso(at) > row["expires_at"]:
                return None
            c.execute("UPDATE enrol_tokens SET used_at=? WHERE token_id=? "
                      "AND used_at IS NULL", (_iso(at), token_id))
            if c.total_changes == 0:
                return None                      # lost the race; mint nothing
            # A retired record with this id is REPLACED, because re-attaching a
            # box under its old name is the ordinary case and forcing people to
            # invent box2 would be worse. Attribution history is not lost by
            # this: usage_events records the upstream as a string, so it survives
            # independently -- but it also means a re-used id inherits the old
            # one's usage rows, which is why the panel shows the owner and the
            # attach date rather than treating the name as an identity.
            c.execute("DELETE FROM servers WHERE server_id=? "
                      "AND revoked_at IS NOT NULL", (row["server_id"],))
            c.execute("INSERT INTO servers (server_id, sub, kind, base_url, note, "
                      "gpus, priority, pool_member, secret_hash, created_at) "
                      "VALUES (?,?,?,?,?,?,0,0,?,?)",
                      (row["server_id"], row["sub"], row["kind"], row["base_url"],
                       row["note"], row["gpus"], secret_hash, _iso(at)))
        return row["server_id"], full

    def authenticate_server(self, presented: str,
                            now: float | None = None) -> ServerRow | None:
        """Resolve a server credential. Never accepts a user key: the namespace
        is part of the pattern, so a qtk_ fails to parse here."""
        parsed = _keys.parse(presented or "", _keys.PREFIX_SERVER)
        if not parsed:
            return None
        server_id, secret = parsed
        with self._lock, self._conn() as c:
            row = c.execute("SELECT * FROM servers WHERE secret_hash IS NOT NULL "
                            "AND revoked_at IS NULL").fetchall()
        for r in row:
            if _keys.verify(secret, r["secret_hash"]):
                return self._server_row(r)
        return None

    @staticmethod
    def _server_row(r) -> ServerRow:
        return ServerRow(server_id=r["server_id"], sub=r["sub"], kind=r["kind"],
                         base_url=r["base_url"], note=r["note"], gpus=r["gpus"],
                         priority=r["priority"],
                         pool_member=bool(r["pool_member"]),
                         created_at=r["created_at"], revoked_at=r["revoked_at"],
                         last_seen=r["last_seen"])

    def server(self, server_id: str) -> ServerRow | None:
        with self._lock, self._conn() as c:
            r = c.execute("SELECT * FROM servers WHERE server_id=?",
                          (server_id,)).fetchone()
        return self._server_row(r) if r else None

    def servers(self, *, sub: str | None = None,
                include_revoked: bool = False) -> list[dict]:
        q = "SELECT * FROM servers"
        where, args = [], []
        if sub:
            where.append("sub=?")
            args.append(sub)
        if not include_revoked:
            where.append("revoked_at IS NULL")
        if where:
            q += " WHERE " + " AND ".join(where)
        with self._lock, self._conn() as c:
            rows = c.execute(q + " ORDER BY server_id", args).fetchall()
        return [self._server_row(r).as_public() for r in rows]

    def set_pool_member(self, server_id: str, value: bool) -> bool:
        return self._update_server(server_id, "pool_member", 1 if value else 0)

    def set_priority(self, server_id: str, value: int) -> bool:
        """The operator's tier. Bounded so a typo cannot create a tier nothing
        else can ever reach."""
        if not -10 <= int(value) <= 10:
            raise ValueError("priority is between -10 and 10")
        return self._update_server(server_id, "priority", int(value))

    def _update_server(self, server_id: str, column: str, value) -> bool:
        # The column name is never caller-supplied -- only these two methods
        # reach here -- but it is checked anyway, because a query built from a
        # string is one refactor away from being built from a request.
        if column not in ("pool_member", "priority", "last_seen"):
            raise ValueError(f"not an updatable column: {column}")
        with self._lock, self._conn() as c:
            cur = c.execute(f"UPDATE servers SET {column}=? WHERE server_id=? "
                            f"AND revoked_at IS NULL", (value, server_id))
            return cur.rowcount > 0

    def revoke_server(self, server_id: str, *, sub: str | None = None,
                      is_admin: bool = False, now: float | None = None) -> bool:
        """Revoke a server. The owner or an admin; a file entry never.

        A file entry is configuration: revoking it here would be undone by the
        next registry reload, and a button that silently does nothing is worse
        than no button.
        """
        import time
        at = time.time() if now is None else now
        with self._lock, self._conn() as c:
            row = c.execute("SELECT * FROM servers WHERE server_id=? "
                            "AND revoked_at IS NULL", (server_id,)).fetchone()
            if not row:
                return False
            if row["kind"] == "file":
                return False
            if not is_admin and (sub is None or row["sub"] != sub):
                return False
            c.execute("UPDATE servers SET revoked_at=? WHERE server_id=?",
                      (_iso(at), server_id))
        return True

    def touch_server(self, server_id: str, now: float | None = None) -> None:
        import time
        self._update_server(server_id, "last_seen",
                            _iso(time.time() if now is None else now))

    # --- what the scheduler measures --------------------------------------
    def throughput(self, *, window_seconds: float = 7 * 86400,
                   now: float | None = None) -> dict:
        """Measured rates per (upstream, model), plus a per-upstream fallback.

        MEASURED, not declared. A box that claims a 4090 and delivers 40 tok/s is
        ranked by the 40, and this node already holds the evidence: prompt_ms and
        predicted_ms come from the model's own response, exactly.

        Keyed by (upstream, model) because rates differ by an order of magnitude
        between a 9B and a 27B on the same card; (upstream, None) aggregates for
        a model that server has not served yet.

        Rows with no timings, or marked truncated, are excluded -- a request whose
        counts were lost would otherwise look like an infinitely fast one.
        """
        import time
        at = time.time() if now is None else now
        since = _iso(at - window_seconds)
        with self._lock, self._conn() as c:
            rows = c.execute(
                "SELECT upstream, model, prompt_tokens, completion_tokens, "
                "prompt_ms, predicted_ms FROM usage_events "
                "WHERE ts >= ? AND truncated = 0 AND prompt_ms IS NOT NULL "
                "AND prompt_ms > 0", (since,)).fetchall()
        acc: dict = {}
        for r in rows:
            for key in ((r["upstream"], r["model"]), (r["upstream"], None)):
                a = acc.setdefault(key, {"ptok": 0, "pms": 0.0, "ctok": 0,
                                         "cms": 0.0, "samples": 0})
                a["ptok"] += r["prompt_tokens"] or 0
                a["pms"] += r["prompt_ms"] or 0.0
                a["ctok"] += r["completion_tokens"] or 0
                a["cms"] += r["predicted_ms"] or 0.0
                a["samples"] += 1
        out = {}
        for key, a in acc.items():
            secs = (a["pms"] + a["cms"]) / 1000.0
            out[key] = {
                "prefill_rate": (a["ptok"] / (a["pms"] / 1000.0)) if a["pms"] else None,
                "predict_rate": (a["ctok"] / (a["cms"] / 1000.0)) if a["cms"] else None,
                "mean_service": (secs / a["samples"]) if a["samples"] else None,
                "samples": a["samples"]}
        return out
