#!/usr/bin/env python3
"""One WebSocket, one HTTP conversation: the pipe, and the pool that holds them.

WHY A POOL OF PIPES RATHER THAN A MULTIPLEXER

The obvious way to invoke services over a held-open connection is to multiplex:
many requests sharing one socket as interleaved streams. That means writing
credit-based flow control, per-stream cancellation, and a scheduler that stops a
469 KB request body from blocking another session's tokens behind it. Every
measured property of this node -- a 469 KB body, 185 s of silent prefill,
token-by-token streaming, 2.5 MB of RSS growth -- lives on that path, and a
multiplexer would put new failure surface directly underneath them.

So one pipe carries exactly one HTTP conversation, and everything hard is TCP's
job again:

  * backpressure is the socket's, end to end;
  * cancellation is closing the pipe, which is already how a client abort becomes
    a 499 here -- there is no CANCEL message to get wrong;
  * there is no head-of-line blocking, because nothing shares a pipe;
  * and the keep-alive body-drain bug cannot be expressed. A pipe serves one
    request and dies, so there is no "next request on this connection" to
    corrupt. That bug has shipped twice in this project.

The cost is sockets instead of frames -- four to six for a two-slot box -- and a
TLS handshake per request, which is why the agent keeps a few pipes OPEN and
idle: the handshake is then paid before the request exists rather than inside it.

WHAT MAKES THIS CHEAP TO ADOPT

`http.client` needs only `sendall` and `makefile("rb")` from a socket, so a Pipe
is a socket as far as it is concerned. The gateway keeps `_relay`, `_pump`, its
timeouts, its accounting and its model peek exactly as measured; a tunnelled
server is the same upstream reached through a different socket.
"""
from __future__ import annotations

import io
import threading
from collections import deque

import wsframe as _ws

# Matches the gateway's relay chunk. Writes are split at this size so one large
# write cannot produce a frame the other end must refuse as oversized.
CHUNK = 65536


class _FrameIO(io.RawIOBase):
    """The readable half of a pipe: frames in, bytes out.

    A RawIOBase rather than a generator because `http.client` wraps whatever
    `makefile` returns in a BufferedReader and expects `readinto`. Leftovers are
    kept here, so a caller asking for fewer bytes than a frame carries does not
    lose the rest -- a frame boundary is not a record boundary.
    """

    def __init__(self, reader: _ws.FrameReader, pipe: "Pipe") -> None:
        self._reader = reader
        self._pipe = pipe
        self._left = b""
        self._eof = False

    def readable(self) -> bool:
        return True

    def readinto(self, buf) -> int:
        if not self._left:
            if self._eof:
                return 0
            self._left = self._next_payload()
            if not self._left:
                self._eof = True
                return 0
        n = min(len(buf), len(self._left))
        buf[:n] = self._left[:n]
        self._left = self._left[n:]
        return n

    def _next_payload(self) -> bytes:
        """The next DATA payload, having dealt with control frames.

        A ping arriving mid-conversation is answered and skipped, never returned:
        letting a control frame's bytes reach the caller would inject frame
        contents into somebody's completion.
        """
        while True:
            frame = self._reader.read()
            if frame is None:
                return b""
            if frame.op == _ws.OP_CLOSE:
                return b""
            if frame.op == _ws.OP_PING:
                self._pipe.pong(frame.payload)
                continue
            if frame.op == _ws.OP_PONG:
                continue
            if frame.payload:
                return frame.payload
            # An empty data frame is legal and means nothing; asking again is
            # correct, whereas returning b"" would look like end of stream.


class Pipe:
    """A socket-like façade over one WebSocket.

    Deliberately implements only what `http.client` uses -- `sendall`,
    `makefile`, `settimeout`, `close` -- so it cannot be mistaken for a general
    socket and grow uses this transport does not support.
    """

    def __init__(self, rfile, wfile, on_close=None, sock=None) -> None:
        self._rfile = rfile
        self._wfile = wfile
        self._sock = sock
        self._hooks = [on_close] if on_close else []
        self._closed = False
        # Set when the pipe closes. The handler thread that accepted the
        # WebSocket must stay alive while the pipe is in use, because returning
        # from it makes the HTTP server close the socket underneath us -- so it
        # parks on this.
        self._done = threading.Event()
        # Writes are serialised because the pool may ping an IDLE pipe from the
        # housekeeper thread. It never pings one in flight, so this guards a case
        # that should not arise rather than a race the design depends on.
        self._wlock = threading.Lock()
        self._reader = _ws.FrameReader(rfile)

    # --- the socket surface ------------------------------------------------
    def sendall(self, data: bytes) -> None:
        for i in range(0, len(data), CHUNK) or [0]:
            self._write(_ws.OP_BIN, data[i:i + CHUNK])

    def makefile(self, mode: str = "rb", *args, **kwargs):
        if "b" not in mode or "w" in mode:
            raise ValueError(f"a pipe is read-only and binary, not {mode!r}")
        return io.BufferedReader(_FrameIO(self._reader, self))

    def settimeout(self, timeout) -> None:
        # Forwarded rather than ignored: the gateway relies on a bounded wait for
        # an upstream that has stopped answering, and that bound has to reach the
        # real socket to mean anything.
        if self._sock is not None:
            self._sock.settimeout(timeout)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._write(_ws.OP_CLOSE, _ws.close_payload(_ws.CLOSE_NORMAL))
        except (OSError, ValueError):
            pass          # the peer is already gone; nothing to tell it
        for hook in self._hooks:
            try:
                hook()
            except Exception:
                pass      # a bookkeeping hook must never break a close
        try:
            self._rfile.close()
            self._wfile.close()
        except (OSError, ValueError):
            pass
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
        self._done.set()

    # --- liveness ----------------------------------------------------------
    def ping(self, payload: bytes = b"") -> bool:
        """Keep an idle pipe alive. False when it is already dead."""
        try:
            self._write(_ws.OP_PING, payload)
            return True
        except (OSError, ValueError):
            return False

    def pong(self, payload: bytes = b"") -> None:
        try:
            self._write(_ws.OP_PONG, payload)
        except (OSError, ValueError):
            pass

    def add_close_hook(self, fn) -> None:
        """Chained rather than assigned: the pool needs its own bookkeeping
        without taking away whatever the creator asked for."""
        self._hooks.append(fn)

    @property
    def closed(self) -> bool:
        return self._closed

    def wait_closed(self, timeout: float | None = None) -> bool:
        """Block until this pipe is done with.

        Used by the thread that accepted it: an HTTP handler returning is what
        makes the server close the connection, so the thread has to wait even
        though it does no work. One parked thread per idle pipe is the price of
        not writing an event loop, and a two-slot box holds about six.
        """
        return self._done.wait(timeout)

    def _write(self, op: int, payload: bytes) -> None:
        with self._wlock:
            self._wfile.write(_ws.encode(op, payload))
            flush = getattr(self._wfile, "flush", None)
            if flush:
                flush()


class PipePool:
    """Idle pipes per server, and the accounting the panel reports.

    There is no queue here, and there must not be one: the gateway's second
    property is that it adds no admission control, because a queue of its own
    would make the dashboard's queue arithmetic a lie. `take` waits briefly for a
    pipe and then fails, which is honest -- this process cannot invent a socket.
    """

    def __init__(self) -> None:
        self._cv = threading.Condition()
        self._idle: dict[str, deque] = {}
        self._inflight: dict[str, int] = {}

    def offer(self, server_id: str, pipe: Pipe) -> None:
        """A new idle pipe arrived from an agent."""
        with self._cv:
            self._idle.setdefault(server_id, deque()).append(pipe)
            # notify_all rather than notify: waiters are per-server and a single
            # notify could wake one waiting on a different server.
            self._cv.notify_all()

    def take(self, server_id: str, timeout: float) -> Pipe | None:
        """An idle pipe, or None if none became free within `timeout`.

        Waiting on a condition rather than polling means a pipe offered while a
        caller waits is handed straight over. Polling would make every request
        under load pay the whole timeout, which from outside looks exactly like
        the node being slow.
        """
        deadline = None
        with self._cv:
            while True:
                q = self._idle.get(server_id)
                while q:
                    pipe = q.popleft()
                    if pipe.closed:
                        continue          # reaped while it sat here
                    self._inflight[server_id] = self._inflight.get(server_id, 0) + 1
                    pipe.add_close_hook(lambda s=server_id: self._release(s))
                    return pipe
                if deadline is None:
                    deadline = timeout
                if not self._cv.wait(timeout=deadline):
                    return None
                deadline = 0.01   # woken: re-check, but do not restart the wait

    def _release(self, server_id: str) -> None:
        with self._cv:
            self._inflight[server_id] = max(0, self._inflight.get(server_id, 0) - 1)

    def idle(self, server_id: str) -> int:
        with self._cv:
            return sum(1 for p in self._idle.get(server_id, ()) if not p.closed)

    def in_flight(self, server_id: str) -> int:
        with self._cv:
            return self._inflight.get(server_id, 0)

    def ready(self, server_id: str) -> bool:
        """Is a pipe available right now?

        Used for RANKING, not for exclusion: a server with no idle pipe is a last
        resort rather than an error, so the balanced route goes around a busy box
        instead of refusing.
        """
        return self.idle(server_id) > 0

    def drop(self, server_id: str) -> None:
        """The agent went away: close everything it had offered."""
        with self._cv:
            pipes = list(self._idle.pop(server_id, ()))
            self._inflight.pop(server_id, None)
        for p in pipes:
            p.close()

    def servers(self) -> list[str]:
        with self._cv:
            return sorted(set(self._idle) | set(self._inflight))

    def keepalive(self) -> None:
        """Ping every IDLE pipe, dropping the ones that cannot be written.

        nginx reaps a proxied connection with no traffic at `proxy_read_timeout`,
        so a pipe that sits unused would die and the first request after a quiet
        hour would fail on a reset. The ping has to originate HERE: nginx resets
        that timer on data read from the upstream, and this process is the
        upstream, so an agent-side keepalive would be the one direction that does
        not help.

        In-flight pipes are left alone. A control frame would be legal mid-stream,
        but the writer is another thread and this is not a race worth having.
        """
        with self._cv:
            snapshot = {sid: list(q) for sid, q in self._idle.items()}
        dead = []
        for sid, pipes in snapshot.items():
            for p in pipes:
                if not p.ping():
                    dead.append((sid, p))
        if not dead:
            return
        with self._cv:
            for sid, p in dead:
                q = self._idle.get(sid)
                if q and p in q:
                    q.remove(p)
        for _, p in dead:
            p.close()
