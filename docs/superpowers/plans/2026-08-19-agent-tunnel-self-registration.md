# Self-registering GPU servers over a held-open connection — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A GPU box runs one native binary, dials out to the node over 443, registers itself, and holds connections open that the node invokes inference over — so the box needs no inbound port and no operator edit.

**Architecture:** One WebSocket carries exactly one HTTP conversation ("a pipe"), so backpressure, cancellation and freedom from head-of-line blocking come from TCP rather than from new code. The agent keeps K idle pipes open to prepay the TLS handshake; the gateway takes one per request, speaks HTTP/1.1 over it via `http.client` with a supplied socket, and closes it. A separate long-lived control WebSocket carries identity and liveness. The node's existing data path — `_relay`, `_pump`, the 8 KB model peek, eligibility, accounting — is untouched; a tunnelled server is the same upstream reached through a different socket.

**Tech Stack:** Node side Python 3.12 stdlib (plus the existing pyyaml/PyJWT), SQLite mirror + Postgres, nginx. Agent side Go, standard library only, `CGO_ENABLED=0`.

**Spec:** [`docs/superpowers/specs/2026-08-19-agent-tunnel-self-registration-design.md`](../specs/2026-08-19-agent-tunnel-self-registration-design.md)

## Global Constraints

Every task's requirements implicitly include all of these.

- **The repository is public.** No hostname, IP address, subnet, pool id, client id, database host or interface name in any committed file. Placeholders in angle brackets only. `tests/test_structural.sh` enforces this and must keep passing.
- **Node side is stdlib.** No new Python dependency. **Agent side is stdlib.** `agent/go.mod` must declare no `require` beyond the module itself; a third-party WebSocket library is not acceptable — RFC 6455 is implemented directly.
- **Neither direction is buffered.** No code may accumulate a request or response body. The model peek stays bounded at `modelpeek.PEEK_BYTES` (8192).
- **Three credential namespaces, never cross-accepted:** `qtk_` user key, `qts_` server credential, `qte_` single-use enrolment token. Each checked by its own function against its own table.
- **Every refusal path drains the request body and sets `Connection: close`** before answering. This bug has shipped twice; use `self._err(...)`, which does both.
- **No queue and no admission control** beyond the single 5 s wait for a pipe.
- **The node never sends the agent a destination.** No protocol message carries a target address (spec §2.3).
- **Commit shape:** the pre-commit gate allows ~3 logical units and ~400 changed lines per commit, and **every source commit needs a documentation touch**. Each task below is one commit; if a task exceeds the gate, split it at the sub-heading boundaries and carry a doc line with each half.
- **Gates that must pass before each commit**, run from `llm/linux-turing-dual`:
  `python3 -m pytest tests/ -q` and `bash tests/test_structural.sh`; from `llm/agent`: `go vet ./... && go test ./...`.
- New Python modules imported by `gateway.py` **must** be added to `scripts/install-node.sh`; a structural check derives the list from the imports and will fail otherwise.

## File Structure

| file | responsibility |
|---|---|
| `llm/linux-turing-dual/scripts/wsframe.py` | RFC 6455 server codec: handshake, frame encode/decode, masking rules, close codes. No sockets, no policy. |
| `llm/linux-turing-dual/scripts/tunnel.py` | `Pipe` (socket-like over a frame stream), `PipePool` (per-server idle pipes, readiness, keepalive). No HTTP knowledge. |
| `llm/linux-turing-dual/scripts/keystore.py` | server identities, enrolment tokens, promotion — mirrored to SQLite like `api_keys`. |
| `llm/linux-turing-dual/sql/002-servers.sql` | the `llm.servers` table, owned by the same role as `001`. |
| `llm/linux-turing-dual/scripts/gateway.py` | the three agent endpoints, the `connect` seam, readiness ranking, `no_capacity`. |
| `llm/linux-turing-dual/scripts/upstreams.py` | tunnel-registered servers merged with file-registered ones; readiness in `pick_auto`. |
| `llm/linux-turing-dual/nginx/qwen-turing.conf` | `location ^~ /api/agent/` with the WebSocket upgrade. |
| `llm/linux-turing-dual/web/index.html` | attach flow, `tunnelled` badge, owner, idle-pipe count. |
| `llm/agent/*.go` | the agent: ws client, pipe pump, enrolment, install. |
| `llm/agent/build.sh` | the five-target cross-build. |

Splitting `wsframe` from `tunnel` from `gateway` is deliberate: the codec is pure and heavily tested, the pool is pure-ish and testable with fake pipes, and only the gateway knows about HTTP and policy.

---

### Task 1: The WebSocket frame codec

**Files:**
- Create: `llm/linux-turing-dual/scripts/wsframe.py`
- Create: `llm/linux-turing-dual/tests/test_unit_wsframe.py`
- Modify: `llm/linux-turing-dual/scripts/install-node.sh` (ship the module)
- Modify: `llm/linux-turing-dual/README.md` (one line: how an agent connects)

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```python
  OP_CONT, OP_TEXT, OP_BIN, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA
  MAX_PAYLOAD = 1 << 20          # a frame larger than this is refused, not buffered

  def accept_key(client_key: str) -> str
  def handshake_response(headers) -> bytes | None      # None when not a valid upgrade
  def encode(op: int, payload: bytes = b"", fin: bool = True, mask: bool = False) -> bytes

  @dataclass
  class Frame:
      op: int
      payload: bytes
      fin: bool

  class FrameReader:
      def __init__(self, rfile, *, require_mask: bool = True)
      def read(self) -> Frame | None                   # None on clean EOF
  ```

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_unit_wsframe.py
import io
import pytest
from conftest import load_script

ws = load_script("wsframe")


def test_the_accept_key_matches_the_rfc_example():
    # RFC 6455 §1.3. A wrong accept key means every browser and every Go client
    # refuses the connection, so this is pinned to the published vector rather
    # than to our own implementation.
    assert ws.accept_key("dGhlIHNhbXBsZSBub25jZQ==") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="


def test_a_client_frame_must_be_masked():
    # RFC 6455 §5.1: a server MUST close the connection on an unmasked client
    # frame. Accepting it would also mean our decoder disagrees with every
    # conformant client about where the payload starts.
    unmasked = bytes([0x81, 0x03]) + b"abc"
    with pytest.raises(ws.ProtocolError):
        ws.FrameReader(io.BytesIO(unmasked)).read()


def test_a_masked_client_frame_round_trips():
    raw = ws.encode(ws.OP_TEXT, b"hello", mask=True)
    f = ws.FrameReader(io.BytesIO(raw)).read()
    assert (f.op, f.payload, f.fin) == (ws.OP_TEXT, b"hello", True)


def test_the_server_never_masks_what_it_sends():
    # A masked server frame is a protocol violation the other end will close on.
    assert ws.encode(ws.OP_BIN, b"x")[1] & 0x80 == 0


@pytest.mark.parametrize("size", [0, 1, 125, 126, 127, 65535, 65536])
def test_every_length_encoding_boundary_round_trips(size):
    # 125/126 and 65535/65536 are where the 7-bit, 16-bit and 64-bit length
    # forms change. Off-by-one here corrupts the stream rather than failing.
    payload = b"z" * size
    f = ws.FrameReader(io.BytesIO(ws.encode(ws.OP_BIN, payload, mask=True))).read()
    assert f.payload == payload


def test_a_payload_split_across_reads_is_reassembled():
    class Trickle(io.RawIOBase):
        """A socket that returns one byte at a time, which a real one may do."""
        def __init__(self, data): self.data, self.i = data, 0
        def readable(self): return True
        def read(self, n=-1):
            if self.i >= len(self.data): return b""
            self.i += 1
            return self.data[self.i - 1:self.i]
    raw = ws.encode(ws.OP_BIN, b"abcdefghij", mask=True)
    f = ws.FrameReader(io.BufferedReader(Trickle(raw))).read()
    assert f.payload == b"abcdefghij"


def test_fragments_are_reported_separately_not_merged():
    # The pipe streams bytes; merging fragments would mean buffering a whole
    # message, which is the one thing this transport must never do.
    a = ws.encode(ws.OP_BIN, b"one", fin=False, mask=True)
    b = ws.encode(ws.OP_CONT, b"two", fin=True, mask=True)
    r = ws.FrameReader(io.BytesIO(a + b))
    assert (r.read().payload, r.read().payload) == (b"one", b"two")


def test_a_control_frame_between_fragments_is_delivered():
    # RFC 6455 §5.4 allows this, and our keepalive depends on it.
    frames = (ws.encode(ws.OP_BIN, b"one", fin=False, mask=True)
              + ws.encode(ws.OP_PING, b"", mask=True)
              + ws.encode(ws.OP_CONT, b"two", fin=True, mask=True))
    ops = [f.op for f in iter(ws.FrameReader(io.BytesIO(frames)).read, None)]
    assert ops == [ws.OP_BIN, ws.OP_PING, ws.OP_CONT]


def test_an_oversized_frame_is_refused_rather_than_allocated():
    header = bytes([0x82, 0xFF]) + (ws.MAX_PAYLOAD + 1).to_bytes(8, "big") + b"\0\0\0\0"
    with pytest.raises(ws.ProtocolError):
        ws.FrameReader(io.BytesIO(header)).read()


def test_a_truncated_frame_reads_as_eof_not_as_data():
    truncated = ws.encode(ws.OP_BIN, b"abcdefghij", mask=True)[:6]
    assert ws.FrameReader(io.BytesIO(truncated)).read() is None


def test_a_clean_eof_is_none():
    assert ws.FrameReader(io.BytesIO(b"")).read() is None


def test_the_handshake_refuses_a_request_that_is_not_an_upgrade():
    assert ws.handshake_response({"Connection": "keep-alive"}) is None


def test_the_handshake_answers_a_valid_upgrade():
    out = ws.handshake_response({
        "Upgrade": "websocket", "Connection": "Upgrade",
        "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
        "Sec-WebSocket-Version": "13"})
    assert out.startswith(b"HTTP/1.1 101 ")
    assert b"Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" in out


def test_the_handshake_refuses_a_version_we_do_not_speak():
    assert ws.handshake_response({
        "Upgrade": "websocket", "Connection": "Upgrade",
        "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
        "Sec-WebSocket-Version": "8"}) is None
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cd llm/linux-turing-dual && python3 -m pytest tests/test_unit_wsframe.py -q`
Expected: collection error — `wsframe` does not exist.

- [ ] **Step 3: Implement `scripts/wsframe.py`**

Module docstring must say why this exists rather than a library: the agent side is
Go with zero dependencies, so the codec exists twice and both copies are tested
against the RFC's own vectors instead of against each other.

```python
import base64, hashlib, os, struct
from dataclasses import dataclass

ACCEPT_MAGIC = "258EAFA5-E914-47DA-95CA-5AB0DC85B11C"
OP_CONT, OP_TEXT, OP_BIN, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA
MAX_PAYLOAD = 1 << 20


class ProtocolError(Exception):
    """A frame we must close the connection over, never interpret."""


def accept_key(client_key: str) -> str:
    digest = hashlib.sha1((client_key + ACCEPT_MAGIC).encode()).digest()
    return base64.b64encode(digest).decode()


def handshake_response(headers) -> bytes | None:
    get = headers.get
    if "websocket" not in (get("Upgrade") or "").lower():
        return None
    if "upgrade" not in (get("Connection") or "").lower():
        return None
    if (get("Sec-WebSocket-Version") or "").strip() != "13":
        return None
    key = (get("Sec-WebSocket-Key") or "").strip()
    if not key:
        return None
    return ("HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept_key(key)}\r\n\r\n").encode()


def encode(op, payload=b"", fin=True, mask=False) -> bytes:
    """One frame. `mask` is for tests and for anything acting as a CLIENT; the
    server must never mask, and a masked server frame is a violation the other
    end closes on."""
    n = len(payload)
    head = bytes([(0x80 if fin else 0) | op])
    flag = 0x80 if mask else 0
    if n < 126:
        head += bytes([flag | n])
    elif n < (1 << 16):
        head += bytes([flag | 126]) + struct.pack("!H", n)
    else:
        head += bytes([flag | 127]) + struct.pack("!Q", n)
    if not mask:
        return head + payload
    key = os.urandom(4)
    masked = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
    return head + key + masked


@dataclass
class Frame:
    op: int
    payload: bytes
    fin: bool


class FrameReader:
    def __init__(self, rfile, *, require_mask=True):
        self.rfile, self.require_mask = rfile, require_mask

    def _exactly(self, n: int) -> bytes | None:
        """n bytes, or None. A socket returns short reads, and treating one as
        the whole frame is how a decoder silently desynchronises."""
        buf = b""
        while len(buf) < n:
            chunk = self.rfile.read(n - len(buf))
            if not chunk:
                return None
            buf += chunk
        return buf

    def read(self) -> Frame | None:
        """One frame, or None at a clean or truncated EOF.

        Fragments are returned as they arrive: this reader never accumulates a
        message, because the pipe above it is a byte stream and buffering a
        whole message would reintroduce exactly what this transport avoids.
        """
        head = self._exactly(2)
        if head is None:
            return None
        fin, op = bool(head[0] & 0x80), head[0] & 0x0F
        masked, n = bool(head[1] & 0x80), head[1] & 0x7F
        if n == 126:
            ext = self._exactly(2)
            if ext is None:
                return None
            n = struct.unpack("!H", ext)[0]
        elif n == 127:
            ext = self._exactly(8)
            if ext is None:
                return None
            n = struct.unpack("!Q", ext)[0]
        # Checked BEFORE reading, so a hostile length cannot make us allocate.
        if n > MAX_PAYLOAD:
            raise ProtocolError(f"frame of {n} bytes exceeds {MAX_PAYLOAD}")
        if self.require_mask and not masked:
            # RFC 6455 §5.1. Accepting it would also mean disagreeing with every
            # conformant client about where the payload starts.
            raise ProtocolError("client frame is not masked")
        key = self._exactly(4) if masked else b""
        if masked and key is None:
            return None
        payload = self._exactly(n) if n else b""
        if payload is None:
            return None
        if masked:
            payload = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
        return Frame(op=op, payload=payload, fin=fin)
```

Implementation notes that the tests pin: read the 2-byte header with a helper
that loops until it has n bytes (a socket returns short reads), treat a short
read as EOF rather than raising, refuse `MAX_PAYLOAD` **before** allocating, and
raise `ProtocolError` on an unmasked client frame.

- [ ] **Step 4: Run the tests until they pass**

Run: `python3 -m pytest tests/test_unit_wsframe.py -q` → 14 passed.

- [ ] **Step 5: Ship the module and document the endpoint**

Add to `scripts/install-node.sh` beside `modelpeek.py`:
```bash
sudo install -m 0644 "${HERE}/wsframe.py"      "${QT_PREFIX}/bin/"
```
Add one line to the README's routing section noting that a server may also
attach itself over `wss://<node-host>/api/agent/…` (detail lands in Task 9).

- [ ] **Step 6: Gates, then commit**

```bash
python3 -m pytest tests/ -q && bash tests/test_structural.sh
git add -A && git commit -m "feat(llm): RFC 6455 frame codec for the agent transport"
```

---

### Task 2: The pipe and the pool

**Files:**
- Create: `llm/linux-turing-dual/scripts/tunnel.py`
- Create: `llm/linux-turing-dual/tests/test_unit_tunnel.py`
- Modify: `llm/linux-turing-dual/scripts/install-node.sh`
- Modify: `llm/linux-turing-dual/docs/measured-ceilings.md` (a stub section for Task 10's numbers)

**Interfaces:**
- Consumes: `wsframe.encode`, `wsframe.FrameReader`, `wsframe.OP_*`.
- Produces:
  ```python
  class Pipe:
      """Socket-like over one WebSocket. `sendall` and `makefile("rb")` are all
      http.client needs -- verified before this design was written."""
      def __init__(self, rfile, wfile, on_close=None)
      def sendall(self, data: bytes) -> None
      def makefile(self, mode="rb", *a, **k)
      def settimeout(self, t) -> None
      def close(self) -> None
      def ping(self) -> bool          # False if the pipe is dead

  class PipePool:
      def offer(self, server_id: str, pipe: Pipe) -> None
      def take(self, server_id: str, timeout: float) -> Pipe | None
      def idle(self, server_id: str) -> int
      def in_flight(self, server_id: str) -> int
      def drop(self, server_id: str) -> None
      def keepalive(self) -> None       # ping idle pipes; drop the dead
  ```

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_unit_tunnel.py
import http.client, io, threading, time
import pytest
from conftest import load_script

ws = load_script("wsframe")
tn = load_script("tunnel")


def _client_stream(*payloads, op=None):
    """Bytes as a CLIENT would send them: masked frames."""
    op = ws.OP_BIN if op is None else op
    return io.BytesIO(b"".join(ws.encode(op, p, mask=True) for p in payloads))


class Sink(io.RawIOBase):
    def __init__(self): self.buf = bytearray()
    def writable(self): return True
    def write(self, b): self.buf += b; return len(b)


def test_bytes_written_to_a_pipe_become_unmasked_frames():
    sink = Sink()
    p = tn.Pipe(io.BytesIO(b""), sink)
    p.sendall(b"POST /v1 HTTP/1.1\r\n")
    f = ws.FrameReader(io.BytesIO(bytes(sink.buf)), require_mask=False).read()
    assert (f.op, f.payload) == (ws.OP_BIN, b"POST /v1 HTTP/1.1\r\n")


def test_frames_read_back_look_like_a_byte_stream():
    p = tn.Pipe(_client_stream(b"HTTP/1.1 200 OK\r\n", b"\r\nbody"), Sink())
    r = p.makefile("rb")
    assert r.readline() == b"HTTP/1.1 200 OK\r\n"
    assert r.read() == b"\r\nbody"


def test_http_client_speaks_over_a_pipe():
    """The whole design rests on this: the gateway's data path is unchanged."""
    body = b'{"choices":[]}'
    resp = (b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
            b"Content-Length: %d\r\n\r\n" % len(body)) + body
    sink = Sink()
    p = tn.Pipe(_client_stream(resp), sink)
    c = http.client.HTTPConnection("upstream", 80)
    c.sock = p
    c.putrequest("POST", "/v1/chat/completions", skip_host=True,
                 skip_accept_encoding=True)
    c.putheader("Host", "upstream")
    c.endheaders()
    c.send(b'{"model":"m"}')
    r = c.getresponse()
    assert (r.status, r.read()) == (200, body)
    assert b'{"model":"m"}' in bytes(sink.buf)      # the body really went out


def test_a_large_body_crosses_the_pipe_byte_identical():
    # 469 KB is the measured real request size at the 100k tier.
    payload = bytes(469 * 1024)
    sink = Sink()
    tn.Pipe(io.BytesIO(b""), sink).sendall(payload)
    got, r = b"", ws.FrameReader(io.BytesIO(bytes(sink.buf)), require_mask=False)
    while (f := r.read()) is not None:
        got += f.payload
    assert got == payload


def test_a_chunked_response_is_read_incrementally():
    frames = _client_stream(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"5\r\nhello\r\n", b"5\r\nworld\r\n", b"0\r\n\r\n")
    c = http.client.HTTPConnection("u", 80)
    c.sock = tn.Pipe(frames, Sink())
    c.putrequest("GET", "/x", skip_host=True, skip_accept_encoding=True)
    c.endheaders()
    r = c.getresponse()
    assert r.read1(5) == b"hello"          # available before the stream ends
    assert r.read() == b"world"


def test_a_close_frame_ends_the_stream_cleanly():
    p = tn.Pipe(io.BytesIO(ws.encode(ws.OP_CLOSE, b"", mask=True)), Sink())
    assert p.makefile("rb").read() == b""


def test_the_pool_hands_out_and_reclaims_pipes():
    pool = tn.PipePool()
    a, b = tn.Pipe(io.BytesIO(b""), Sink()), tn.Pipe(io.BytesIO(b""), Sink())
    pool.offer("box", a); pool.offer("box", b)
    assert pool.idle("box") == 2
    taken = pool.take("box", timeout=0.1)
    assert taken in (a, b)
    assert pool.idle("box") == 1 and pool.in_flight("box") == 1


def test_taking_from_an_empty_pool_waits_then_gives_up():
    pool = tn.PipePool()
    started = time.time()
    assert pool.take("box", timeout=0.2) is None
    assert 0.15 < time.time() - started < 1.0


def test_a_pipe_offered_while_a_caller_waits_is_handed_straight_over():
    """Otherwise every request would pay the full timeout under load."""
    pool = tn.PipePool()
    p = tn.Pipe(io.BytesIO(b""), Sink())
    threading.Timer(0.05, lambda: pool.offer("box", p)).start()
    got = pool.take("box", timeout=2.0)
    assert got is p


def test_dropping_a_server_closes_its_pipes():
    pool = tn.PipePool()
    closed = []
    p = tn.Pipe(io.BytesIO(b""), Sink(), on_close=lambda: closed.append(1))
    pool.offer("box", p)
    pool.drop("box")
    assert pool.idle("box") == 0 and closed == [1]


def test_keepalive_pings_idle_pipes_and_reaps_the_dead():
    """nginx reaps a pipe with no traffic at proxy_read_timeout, so an idle pipe
    must be pinged -- and the ping has to come from THIS side, because nginx
    resets that timer on data read from the upstream."""
    pool = tn.PipePool()
    sink = Sink()
    pool.offer("box", tn.Pipe(io.BytesIO(b""), sink))
    pool.keepalive()
    f = ws.FrameReader(io.BytesIO(bytes(sink.buf)), require_mask=False).read()
    assert f.op == ws.OP_PING

    class Broken(Sink):
        def write(self, b): raise BrokenPipeError("gone")
    pool.offer("box2", tn.Pipe(io.BytesIO(b""), Broken()))
    pool.keepalive()
    assert pool.idle("box2") == 0
```

- [ ] **Step 2: Run them and watch them fail**

Run: `python3 -m pytest tests/test_unit_tunnel.py -q` → collection error, no `tunnel`.

- [ ] **Step 3: Implement `scripts/tunnel.py`**

`Pipe.makefile` returns `io.BufferedReader(_FrameIO(...))` where `_FrameIO` is a
`RawIOBase` whose `readinto` pulls the next frame's payload, keeps any remainder,
and returns 0 on `OP_CLOSE` or EOF. `sendall` splits at `CHUNK` (65536) into
`OP_BIN` frames. Control frames arriving on a live pipe (`OP_PING`) are answered
with `OP_PONG` and skipped, never delivered as data.

`PipePool` holds `{server_id: deque}` plus a `threading.Condition`, so `offer`
notifies a waiting `take` instead of leaving it to time out.

- [ ] **Step 4: Run the tests until they pass**

Run: `python3 -m pytest tests/test_unit_tunnel.py -q` → 11 passed.

- [ ] **Step 5: Ship it, and open the docs section Task 10 fills in**

`install-node.sh`: `sudo install -m 0644 "${HERE}/tunnel.py" "${QT_PREFIX}/bin/"`.
In `docs/measured-ceilings.md`, add `### Through the agent tunnel` with one
sentence saying the numbers land in Task 10 and must be recorded whichever way
they fall.

- [ ] **Step 6: Gates, then commit**

---

### Task 3: Server identities and enrolment

**Files:**
- Create: `llm/linux-turing-dual/sql/002-servers.sql`
- Modify: `llm/linux-turing-dual/scripts/keystore.py`
- Create: `llm/linux-turing-dual/tests/test_unit_keystore_servers.py`
- Modify: `llm/linux-turing-dual/README.md` (the attach flow, in prose)

**Interfaces:**
- Consumes: `keys.generate` / `keys.parse` / `keys.verify` with a new prefix argument, or a
  sibling generator — the existing `qtk_` regex must keep refusing `qts_`.
- Produces:
  ```python
  @dataclass
  class ServerRow:
      server_id: str; sub: str; note: str | None; gpus: str | None
      created_at: str; revoked_at: str | None; pool_member: bool; last_seen: str | None

  def enrol_token(self, sub, server_id, *, note=None, gpus=None, now=None) -> str
  def redeem_token(self, token, *, now=None) -> tuple[str, str] | None   # (server_id, qts_)
  def authenticate_server(self, presented, now=None) -> ServerRow | None
  def servers(self, *, sub=None) -> list[dict]
  def set_pool_member(self, server_id, value: bool) -> bool
  def revoke_server(self, server_id, *, sub=None, is_admin=False) -> bool
  def touch_server(self, server_id, now=None) -> None
  ```

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_unit_keystore_servers.py
import time
import pytest
from keystore import KeyStore


@pytest.fixture
def store(tmp_path):
    s = KeyStore(str(tmp_path / "m.sqlite3"), dsn=None)
    s.migrate_local()
    s.upsert_user("sub-a", email="a@example.org", name="A")
    s.upsert_user("sub-b", email="b@example.org", name="B")
    return s


def test_an_enrolment_token_works_exactly_once(store):
    tok = store.enrol_token("sub-a", "box", note="the workstation")
    assert tok.startswith("qte_")
    first = store.redeem_token(tok)
    assert first is not None and first[0] == "box"
    assert store.redeem_token(tok) is None


def test_an_expired_enrolment_token_is_refused(store):
    now = time.time()
    tok = store.enrol_token("sub-a", "box", now=now)
    assert store.redeem_token(tok, now=now + 1801) is None


def test_a_server_credential_is_not_a_user_key_and_vice_versa(store):
    """The most important test in this task. One authenticate() for everything
    is exactly how a server credential silently becomes an inference key."""
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    assert cred.startswith("qts_")
    assert store.authenticate(cred, time.time()) is None
    full, _ = store.mint("sub-a", label="k")
    assert store.authenticate_server(full) is None
    # And each still works as itself, or the test proves nothing.
    assert store.authenticate(full, time.time()) is not None
    assert store.authenticate_server(cred) is not None


def test_a_new_server_is_not_in_the_default_pool(store):
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    assert store.authenticate_server(cred).pool_member is False


def test_promotion_is_what_puts_it_in_the_pool(store):
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    assert store.set_pool_member("box", True) is True
    assert store.authenticate_server(cred).pool_member is True
    assert store.set_pool_member("box", False) is True
    assert store.authenticate_server(cred).pool_member is False


def test_the_owner_may_revoke_and_a_stranger_may_not(store):
    store.redeem_token(store.enrol_token("sub-a", "box"))
    assert store.revoke_server("box", sub="sub-b") is False
    assert store.revoke_server("box", sub="sub-a") is True


def test_an_admin_may_revoke_anyone_s_server(store):
    store.redeem_token(store.enrol_token("sub-a", "box"))
    assert store.revoke_server("box", sub="sub-b", is_admin=True) is True


def test_a_revoked_credential_stops_authenticating(store):
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    store.revoke_server("box", sub="sub-a")
    assert store.authenticate_server(cred) is None


def test_a_duplicate_server_id_is_refused(store):
    store.redeem_token(store.enrol_token("sub-a", "box"))
    with pytest.raises(ValueError):
        store.enrol_token("sub-b", "box")


@pytest.mark.parametrize("bad", ["local", "auto"])
def test_reserved_ids_are_refused(store, bad):
    # These are path segments with meanings already. A server called `auto`
    # would shadow the balancer itself.
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", bad)


@pytest.mark.parametrize("bad", ["Box", "b ox", "-box", "x" * 32, ""])
def test_an_id_that_is_not_a_path_segment_is_refused(store, bad):
    with pytest.raises(ValueError):
        store.enrol_token("sub-a", bad)


def test_the_credential_is_stored_only_as_a_hash(store):
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    with store._conn() as c:
        rows = list(c.execute("SELECT * FROM servers"))
    assert cred.split("_")[-1] not in str([tuple(r) for r in rows])


def test_listing_scopes_to_a_user_or_shows_everyone(store):
    store.redeem_token(store.enrol_token("sub-a", "boxa"))
    store.redeem_token(store.enrol_token("sub-b", "boxb"))
    assert {s["server_id"] for s in store.servers(sub="sub-a")} == {"boxa"}
    assert {s["server_id"] for s in store.servers()} == {"boxa", "boxb"}


def test_last_seen_is_recorded_for_the_panel(store):
    _, cred = store.redeem_token(store.enrol_token("sub-a", "box"))
    assert store.authenticate_server(cred).last_seen is None
    store.touch_server("box", now=1_700_000_000.0)
    assert store.authenticate_server(cred).last_seen is not None
```

- [ ] **Step 2: Run them and watch them fail**
- [ ] **Step 3: Write `sql/002-servers.sql`** — same ownership and default-privileges pattern as `001-schema.sql`; `pool_member boolean NOT NULL DEFAULT false`.
- [ ] **Step 4: Implement the keystore methods**, mirroring to SQLite exactly as `api_keys` does, so the local copy is the enforcement point.
- [ ] **Step 5: Run the tests until they pass**
- [ ] **Step 6: Document the attach flow in the README, then gates, then commit**

---

### Task 4: The agent endpoints

**Files:**
- Modify: `llm/linux-turing-dual/scripts/gateway.py`
- Create: `llm/linux-turing-dual/tests/test_unit_agent_endpoints.py`
- Modify: `llm/linux-turing-dual/README.md`

**Interfaces:**
- Consumes: Task 1 `wsframe`, Task 2 `PipePool`, Task 3 keystore methods.
- Produces:
  ```python
  AGENT_STATE: dict[str, dict]     # server_id -> {state, last_seen, gpus, slots, error}
  POOL = tunnel.PipePool()
  HEARTBEAT_SECONDS = 20
  HEARTBEAT_GRACE = 10
  PIPE_KEEPALIVE_SECONDS = 240
  PIPE_WAIT_SECONDS = 5
  ```
  Endpoints: `POST /api/servers/enrol` (session, any pool member), `POST /api/agent/enrol`
  (token → credential), `GET /api/agent/control` (upgrade), `GET /api/agent/pipe` (upgrade),
  `POST /api/servers/<id>/pool` (admin), `DELETE /api/servers/<id>` (owner or admin).

- [ ] **Step 1: Write the failing integration tests, driven by a fake agent that speaks the real protocol**

```python
# tests/fakeagent.py -- shared by Task 4 and Task 5.
import json, socket, threading
from conftest import load_script

ws = load_script("wsframe")


class FakeAgent:
    """The agent's side of the protocol, over real sockets.

    Not a mock: it performs the HTTP upgrade, masks its frames as a client must,
    and pumps bytes to a real local target. If the handshake or the framing is
    wrong on either side, these tests fail.
    """

    def __init__(self, port, credential):
        self.port, self.credential = port, credential
        self.pipes, self.control_sock = [], None

    def _upgrade(self, path):
        s = socket.create_connection(("127.0.0.1", self.port), timeout=10)
        s.sendall(
            f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            f"Authorization: Bearer {self.credential}\r\n"
            "Upgrade: websocket\r\nConnection: Upgrade\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n".encode())
        rfile = s.makefile("rb")
        status = rfile.readline()
        while rfile.readline() not in (b"\r\n", b""):
            pass
        return s, rfile, int(status.split()[1])

    def control(self, server_id, *, gpus="one card", slots=2):
        """Returns the status code; on 101 leaves the connection live."""
        s, rfile, code = self._upgrade("/api/agent/control")
        if code != 101:
            s.close()
            return code
        s.sendall(ws.encode(ws.OP_TEXT, json.dumps({
            "type": "hello", "agent_version": "test", "server_id": server_id,
            "gpus": gpus, "slots": slots, "note": "fake"}).encode(), mask=True))
        self.control_sock = s
        threading.Thread(target=self._answer_pings, args=(s, rfile),
                         daemon=True).start()
        return 101

    def _answer_pings(self, s, rfile):
        try:
            while (f := ws.FrameReader(rfile, require_mask=False).read()):
                if f.op == ws.OP_PING:
                    s.sendall(ws.encode(ws.OP_PONG, f.payload, mask=True))
        except OSError:
            pass

    def add_pipe(self, target):
        """Offer one pipe, pumping to `target` == (host, port). The local
        connection is opened lazily on the first byte, as the real agent does."""
        s, rfile, code = self._upgrade("/api/agent/pipe")
        assert code == 101, code
        self.pipes.append(s)
        threading.Thread(target=self._pump, args=(s, rfile, target),
                         daemon=True).start()

    def _pump(self, s, rfile, target):
        local = None
        try:
            reader = ws.FrameReader(rfile, require_mask=False)
            while (f := reader.read()) is not None:
                if f.op == ws.OP_PING:
                    s.sendall(ws.encode(ws.OP_PONG, f.payload, mask=True))
                    continue
                if f.op == ws.OP_CLOSE:
                    break
                if local is None:
                    local = socket.create_connection(target, timeout=10)
                    threading.Thread(target=self._back, args=(local, s),
                                     daemon=True).start()
                local.sendall(f.payload)
        except OSError:
            pass
        finally:
            if local:
                local.close()
            s.close()

    def _back(self, local, s):
        try:
            while (buf := local.recv(65536)):
                s.sendall(ws.encode(ws.OP_BIN, buf, mask=True))
        except OSError:
            pass

    def stop(self):
        for p in [self.control_sock, *self.pipes]:
            if p:
                p.close()
```

Cases: control accepted with a `qts_`; refused with a `qtk_`; refused with no
credential; `hello` names the server and the panel state becomes `online`;
a second control connection for the same id is closed `4409`; killing the
control connection marks it `offline` within `HEARTBEAT_SECONDS + HEARTBEAT_GRACE`;
a pipe offered before any request lands in the pool and shows in `/api/servers`;
enrolment token single use end to end.

- [ ] **Step 2: Run them and watch them fail**
- [ ] **Step 3: Implement the endpoints**

The upgrade path needs care in `BaseHTTPRequestHandler`: after writing
`handshake_response`, the handler must **stop using** `send_response`/`end_headers`,
take `self.rfile`/`self.wfile`, set `self.close_connection = True`, and run the
control loop or park the pipe. Read `_body_left` to zero first — an upgrade
request has no body, but the invariant is unconditional.

- [ ] **Step 4: Run the tests until they pass**
- [ ] **Step 5: Document the protocol table in the README**
- [ ] **Step 6: Gates, then commit**

---

### Task 5: Routing through a tunnel

**Files:**
- Modify: `llm/linux-turing-dual/scripts/gateway.py` (the `connect` seam, `no_capacity`)
- Modify: `llm/linux-turing-dual/scripts/upstreams.py` (readiness in `pick_auto`)
- Modify: `llm/linux-turing-dual/tests/test_unit_upstreams.py`
- Create: `llm/linux-turing-dual/tests/test_unit_tunnel_routing.py`
- Modify: `llm/linux-turing-dual/README.md`

**Interfaces:**
- Produces:
  ```python
  # upstreams.py
  def pick_auto(ups, state, last_used, *, model=None, local_online=True,
                local_models=None, ready=None) -> tuple[str | None, str]
      """`ready` is a callable id -> bool. An eligible server that is not ready
      is ranked LAST, never excluded: /v1 routes around a busy box instead of
      refusing, and only an explicit pin can hit no_capacity."""

  # gateway.py
  def _upstream_conn(self, target) -> http.client.HTTPConnection
      """Direct TCP, or a pipe from POOL with self.sock preset."""
  ```

- [ ] **Step 1: Write the failing tests** — reusing Task 4's `FakeAgent` against a real local target:
  - a 469 KB body reaches the fake target byte-identical **through the tunnel**;
  - the first response chunk arrives before the last (timed, not read whole);
  - `/v1` picks the tunnelled server when it alone serves the model;
  - a client abort closes the pipe and records `499`;
  - starvation: with zero idle pipes, a pin returns `503 no_capacity` after
    `PIPE_WAIT_SECONDS`, while the balanced route ranks around it and succeeds
    locally;
  - the model peek still applies (a wrong-model pin is still `404`);
  - `usage_events.upstream` names the tunnelled server.
- [ ] **Step 2: Run them and watch them fail**
- [ ] **Step 3: Implement** — `_relay` changes only where it builds the connection.
- [ ] **Step 4: Run the tests until they pass**
- [ ] **Step 5: README: the `no_capacity` refusal and readiness ranking**
- [ ] **Step 6: Gates, then commit**

---

### Task 6: nginx, and the structural guards

**Files:**
- Modify: `llm/linux-turing-dual/nginx/qwen-turing.conf`
- Modify: `llm/linux-turing-dual/tests/test_structural.sh`
- Modify: `llm/linux-turing-dual/README.md`

- [ ] **Step 1: Add the location**

```nginx
    # The agent's control and pipe connections. WebSocket only, so `Connection`
    # is a literal rather than the usual `map $http_upgrade` at http level --
    # this file is a server block and must stay self-contained.
    location ^~ /api/agent/ {
        proxy_pass http://qwen_gateway;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        proxy_request_buffering off;
        # An idle pipe is pinged from the node every 240 s, well inside this.
        proxy_read_timeout 900s;
        proxy_send_timeout 900s;
    }
```

- [ ] **Step 2: Write the structural checks, each with a positive control**
  - `/api/agent/` is proxied to `qwen_gateway` and never to the dashboard;
  - that location sets `Upgrade`;
  - **neither protocol message carries a destination field** — grep the agent
    endpoints and the Go agent for a `target`/`url`/`host` key in a message
    written by the node (spec §2.3);
  - `PIPE_KEEPALIVE_SECONDS` is strictly less than the nginx `proxy_read_timeout`
    in the same file — parse both numbers rather than asserting a literal.
- [ ] **Step 3: Prove each check fails on a deliberately broken copy of the tree**
- [ ] **Step 4: `nginx -t` on the node against the rendered config** (`sudo nginx -t`, read its own final line — never a piped exit status)
- [ ] **Step 5: Gates, then commit**

---

### Task 7: The agent — connect, register, pump

**Files:**
- Create: `llm/agent/go.mod`, `main.go`, `ws.go`, `enrol.go`, `config.go`
- Create: `llm/agent/ws_test.go`, `llm/agent/pump_test.go`
- Create: `llm/agent/README.md`

**Interfaces:**
- Produces:
  ```go
  // ws.go
  func Dial(ctx context.Context, url, credential string) (*Conn, error)
  func (c *Conn) ReadFrame() (op byte, payload []byte, fin bool, err error)
  func (c *Conn) WriteFrame(op byte, payload []byte, fin bool) error  // always masked
  func (c *Conn) Close(code uint16, reason string) error

  // main.go
  func runControl(ctx context.Context, cfg Config) error   // reconnects with backoff
  func runPipe(ctx context.Context, cfg Config) error      // one pipe, one conversation
  ```

- [ ] **Step 1: Write the failing Go tests**

`ws_test.go` pins the same RFC vectors as Task 1 (accept key, the length
boundaries, masking) — deliberately duplicated across languages so neither
implementation is validated by the other. `pump_test.go` starts an `httptest`
server as the local target, a fake node as a WebSocket server, and asserts a
469 KB request body arrives byte-identical and a chunked response streams back.

- [ ] **Step 2: Run and watch them fail** — `cd llm/agent && go test ./...`
- [ ] **Step 3: Implement**, stdlib only. `Dial` does `tls.Dial`, writes the
  upgrade request with a random 16-byte `Sec-WebSocket-Key`, verifies the
  accept key, and hands back the raw `net.Conn`. The pipe is
  `io.Copy` in both directions with the frame codec in between; the local
  connection is dialled **lazily on the first byte**, so an idle pipe pins
  nothing on the target.
- [ ] **Step 4: Run until green**, then `go vet ./...`
- [ ] **Step 5: `agent/README.md`** — the three commands (`enrol`, `run`, `install`), the config file, and the flat statement that the target address comes from local config and is never accepted from the node.
- [ ] **Step 6: Commit**

---

### Task 8: The agent as an artifact — install and cross-build

**Files:**
- Create: `llm/agent/install.go`, `llm/agent/build.sh`
- Create: `llm/agent/dist/qwen-turing-agent.service`, `.plist`, `task.xml`
- Modify: `llm/agent/README.md`
- Modify: `.github/workflows/*` (whichever job runs the node checks)

- [ ] **Step 1: `build.sh` builds all five targets** and fails if `go list -m all` reports any dependency beyond the main module:

```bash
set -euo pipefail
for t in linux/amd64 linux/arm64 windows/amd64 darwin/arm64 darwin/amd64; do
  CGO_ENABLED=0 GOOS=${t%/*} GOARCH=${t#*/} \
    go build -trimpath -ldflags="-s -w" -o "dist/qwen-turing-agent-${t%/*}-${t#*/}${ext}" .
done
```
Measured baseline for the size assertion: 3.3–3.6 MB per target.

- [ ] **Step 2: `install` writes the right supervision file for the host OS** and prints it, including the `icacls` line on Windows — where the credential gets no DPAPI, which the README states plainly rather than implying otherwise.
- [ ] **Step 3: CI runs `go vet`, `go test`, and the cross-build** (no GPU, seconds).
- [ ] **Step 4: Commit**

---

### Task 9: The dashboard

**Files:**
- Modify: `llm/linux-turing-dual/web/index.html`
- Modify: `llm/linux-turing-dual/tests/test_structural.sh`

- [ ] **Step 1: Attach flow** — a form in Servers; the one-time token and the exact command line shown after it. **Render it from state**, like the key reveal: a token is shown once and the panel refreshes itself (see `MINTED`/`mintedBox`, and the structural check that guards it).
- [ ] **Step 2: Per-server card** — `tunnelled`/`direct`, owner, `in the default pool` or `reachable at /u/<id>/v1 only`, idle-pipe count; and **no** "reachable without a key" warning for a tunnelled server, because nothing can reach it.
- [ ] **Step 3: Verify in a browser, not by reading.** Use the local page lab (real gateway, injected session, playwright) and screenshot Servers with one tunnelled and one direct server. Three page bugs in this project were invisible to reading.
- [ ] **Step 4: Extend the reveal-ownership structural check to the token box**
- [ ] **Step 5: Gates, then commit**

---

### Task 10: Acceptance — and deleting the firewall rule

This is the task the design exists for.

- [ ] **Step 1: Attach the workstation as a tunnelled server** using only the dashboard and one command on the box.
- [ ] **Step 2: Bind its inference port back to loopback and remove its LAN exposure.** Revert `QWEN_HOST` to `127.0.0.1` in `/opt/qwen-local/etc/qwen-local.conf` (backup at `/root/qwen-local.conf.bak`) and remove the `QWEN38-FED` accept rule. Verify from the node that the port is **unreachable directly** and that inference still works **through the tunnel**.
- [ ] **Step 3: Promote it to the default pool** and confirm `/v1` routes to it with `X-Routed-To`.
- [ ] **Step 4: Measure a ~100k request through the tunnel** — prefill, wall clock, gateway RSS before/after — and record it in `docs/measured-ceilings.md` beside the direct figures, including the comparison whichever way it falls. Do not attribute a difference to the tunnel without a second sample.
- [ ] **Step 5: Kill the agent.** Assert `offline` within 30 s and that `/v1` routes around it; restart it and assert recovery with no operator action.
- [ ] **Step 6: Update the spec's status line to implemented, note anything the implementation decided differently, and commit.**

---

## Self-review notes

- **Spec coverage:** §2.1 → T3/T4/T9; §2.2 → T3 (the cross-namespace test); §2.3 → T6 structural check + T7 README; §3.1–3.3 → T1/T2/T4; §3.4 → T2 keepalive + T6 numeric check; §3.5 → T2's `http.client` test; §4 → T5; §5 → T4; §6 → T7/T8; §7 → T1–T6; §8 → T9; §9 → the tests in each task; §10 → T10.
- **Deliberately deferred inside this plan:** nothing. The spec's own out-of-scope list (multiplexing, node-as-client, auto-update, non-llama.cpp targets) stays out.
- **Ordering constraint:** T1 → T2 → T4 → T5 is a hard chain. T3 can run parallel to T1/T2. T7 needs only the protocol as specified, so it can be written against Task 4's endpoints before T5 exists.
