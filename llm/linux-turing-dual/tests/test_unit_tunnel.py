"""One WebSocket, one HTTP conversation: the pipe, and the pool that holds them.

The design claim these tests exist to hold up is that the gateway's measured data
path does not change. So the central test is not about frames at all -- it is
`http.client` issuing a request and reading a response over a pipe, because if
that works then `_relay`, `_pump`, the 900 s timeouts and the accounting all work
unaltered over a tunnel.

Two properties are asserted with sources that CANNOT cheat:

  * a 469 KB body -- the measured real request size at the 100k tier -- crosses
    byte-identical, so the pipe is not a place where a body gets assembled;
  * a response chunk is delivered before the rest of the response EXISTS, using a
    real OS pipe written after the first read returns. Reading a chunk out of a
    fully-formed buffer would prove nothing about streaming.
"""
import http.client
import io
import os
import threading
import time

import pytest

from conftest import load_script

ws = load_script("wsframe")
tn = load_script("tunnel")


class Sink(io.RawIOBase):
    """Stands in for the wfile of a connection, keeping what was written."""

    def __init__(self):
        self.buf = bytearray()

    def writable(self):
        return True

    def write(self, b):
        self.buf += b
        return len(b)

    def flush(self):
        pass


def _client_stream(*payloads, op=None):
    """Bytes as a CLIENT would send them: masked frames."""
    op = ws.OP_BIN if op is None else op
    return io.BytesIO(b"".join(ws.encode(op, p, mask=True) for p in payloads))


def _frames_out(sink):
    """Every frame the pipe wrote, decoded."""
    out, r = [], ws.FrameReader(io.BytesIO(bytes(sink.buf)), require_mask=False)
    while (f := r.read()) is not None:
        out.append(f)
    return out


# --- the pipe as a byte stream ----------------------------------------------

def test_bytes_written_to_a_pipe_become_unmasked_binary_frames():
    sink = Sink()
    p = tn.Pipe(io.BytesIO(b""), sink)
    p.sendall(b"POST /v1/chat/completions HTTP/1.1\r\n")
    frames = _frames_out(sink)
    assert len(frames) == 1
    assert frames[0].op == ws.OP_BIN
    assert frames[0].payload == b"POST /v1/chat/completions HTTP/1.1\r\n"


def test_a_write_larger_than_a_chunk_is_split_rather_than_framed_whole():
    # MAX_PAYLOAD would otherwise be reachable by a single large write, and the
    # other end refuses those -- correctly.
    sink = Sink()
    tn.Pipe(io.BytesIO(b""), sink).sendall(b"a" * (tn.CHUNK * 2 + 17))
    frames = _frames_out(sink)
    assert [len(f.payload) for f in frames] == [tn.CHUNK, tn.CHUNK, 17]
    assert all(len(f.payload) <= ws.MAX_PAYLOAD for f in frames)


def test_frames_read_back_look_like_an_ordinary_byte_stream():
    p = tn.Pipe(_client_stream(b"HTTP/1.1 200 OK\r\n", b"\r\nbody"), Sink())
    r = p.makefile("rb")
    assert r.readline() == b"HTTP/1.1 200 OK\r\n"
    assert r.read() == b"\r\nbody"


def test_a_frame_boundary_is_not_a_record_boundary():
    # A reader must not assume one frame is one line or one read: the agent
    # chooses its own chunking and a header can straddle two frames.
    p = tn.Pipe(_client_stream(b"HTTP/1.1 200 ", b"OK\r\nX: 1\r", b"\n\r\n"), Sink())
    r = p.makefile("rb")
    assert r.readline() == b"HTTP/1.1 200 OK\r\n"
    assert r.readline() == b"X: 1\r\n"


def test_a_frame_larger_than_the_read_buffer_is_not_truncated():
    """The case that actually happens: the agent relays in 64 KB chunks while a
    BufferedReader asks for 8 KB at a time, so most of a frame is leftover and
    has to be kept. A mutation that dropped the remainder passed every other test
    in this file -- none of them used a frame bigger than the read buffer.
    """
    payload = os.urandom(100 * 1024)
    p = tn.Pipe(_client_stream(payload), Sink())
    assert p.makefile("rb").read() == payload


def test_reading_less_than_a_frame_carries_leaves_the_rest_available():
    p = tn.Pipe(_client_stream(b"0123456789"), Sink())
    r = p.makefile("rb")
    assert r.read(4) == b"0123"
    assert r.read() == b"456789"


def test_a_close_frame_ends_the_stream_cleanly():
    p = tn.Pipe(io.BytesIO(ws.encode(ws.OP_CLOSE, b"", mask=True)), Sink())
    assert p.makefile("rb").read() == b""


def test_a_ping_on_a_live_pipe_is_answered_and_not_delivered_as_data():
    """A ping arriving mid-conversation must not appear in the body -- that would
    inject frame bytes into someone's completion."""
    src = io.BytesIO(ws.encode(ws.OP_BIN, b"abc", mask=True)
                     + ws.encode(ws.OP_PING, b"hb", mask=True)
                     + ws.encode(ws.OP_BIN, b"def", mask=True))
    sink = Sink()
    p = tn.Pipe(src, sink)
    assert p.makefile("rb").read() == b"abcdef"
    assert [(f.op, f.payload) for f in _frames_out(sink)] == [(ws.OP_PONG, b"hb")]


# --- the property the whole design rests on ---------------------------------

def test_http_client_speaks_over_a_pipe():
    """If this works, the gateway's data path needs no rewrite at all."""
    body = b'{"choices":[]}'
    resp = (b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
            b"Content-Length: %d\r\n\r\n" % len(body)) + body
    sink = Sink()
    c = http.client.HTTPConnection("upstream", 80)
    c.sock = tn.Pipe(_client_stream(resp), sink)
    c.putrequest("POST", "/v1/chat/completions", skip_host=True,
                 skip_accept_encoding=True)
    c.putheader("Host", "upstream")
    c.putheader("Content-Length", "13")
    c.endheaders()
    c.send(b'{"model":"m"}')
    r = c.getresponse()
    assert (r.status, r.read()) == (200, body)
    # And the request really went out through the frames, not into a void.
    sent = b"".join(f.payload for f in _frames_out(sink))
    assert sent.startswith(b"POST /v1/chat/completions HTTP/1.1\r\n")
    assert sent.endswith(b'{"model":"m"}')


def test_a_469_kb_body_crosses_byte_identical():
    # The measured real request size at the 100k tier. If the pipe assembled
    # bodies, this is where it would show.
    payload = os.urandom(469 * 1024)
    sink = Sink()
    tn.Pipe(io.BytesIO(b""), sink).sendall(payload)
    assert b"".join(f.payload for f in _frames_out(sink)) == payload


def test_a_chunk_arrives_before_the_rest_of_the_response_exists():
    """Streaming, proved against a source that does not yet HAVE the remainder.

    A chunked read out of a complete buffer would pass even if the pipe
    accumulated the whole response first.
    """
    rd, wr = os.pipe()
    rfile = io.FileIO(rd, "rb")
    wfile = io.FileIO(wr, "wb")

    def send(*payloads):
        for pl in payloads:
            wfile.write(ws.encode(ws.OP_BIN, pl, mask=True))

    send(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
         b"5\r\nhello\r\n")

    c = http.client.HTTPConnection("u", 80)
    c.sock = tn.Pipe(rfile, Sink())
    c.putrequest("GET", "/x", skip_host=True, skip_accept_encoding=True)
    c.endheaders()
    r = c.getresponse()
    assert r.read1(5) == b"hello"        # delivered while the rest is unwritten

    send(b"5\r\nworld\r\n", b"0\r\n\r\n")
    wfile.close()
    assert r.read() == b"world"
    rfile.close()


# --- the pool ---------------------------------------------------------------

def _pipe():
    return tn.Pipe(io.BytesIO(b""), Sink())


def test_the_pool_hands_out_and_accounts_for_pipes():
    pool = tn.PipePool()
    a, b = _pipe(), _pipe()
    pool.offer("box", a)
    pool.offer("box", b)
    assert pool.idle("box") == 2
    taken = pool.take("box", timeout=0.1)
    assert taken in (a, b)
    assert (pool.idle("box"), pool.in_flight("box")) == (1, 1)


def test_closing_a_taken_pipe_returns_it_to_the_accounting():
    """One request per pipe, so a closed pipe is capacity that has been used --
    the count has to fall or the pool looks permanently busy."""
    pool = tn.PipePool()
    pool.offer("box", _pipe())
    p = pool.take("box", timeout=0.1)
    assert pool.in_flight("box") == 1
    p.close()
    assert (pool.in_flight("box"), pool.idle("box")) == (0, 0)


def test_taking_from_an_empty_pool_waits_and_then_gives_up():
    pool = tn.PipePool()
    started = time.time()
    assert pool.take("box", timeout=0.2) is None
    assert 0.15 < time.time() - started < 2.0


def test_a_pipe_offered_while_a_caller_waits_is_handed_straight_over():
    """Otherwise every request under load pays the whole timeout before
    proceeding, which would look exactly like the node being slow."""
    pool = tn.PipePool()
    p = _pipe()
    threading.Timer(0.05, lambda: pool.offer("box", p)).start()
    started = time.time()
    assert pool.take("box", timeout=3.0) is p
    assert time.time() - started < 1.0


def test_pools_do_not_leak_across_servers():
    pool = tn.PipePool()
    pool.offer("boxa", _pipe())
    assert pool.take("boxb", timeout=0.05) is None
    assert pool.idle("boxa") == 1


def test_dropping_a_server_closes_its_pipes():
    pool = tn.PipePool()
    closed = []
    pool.offer("box", tn.Pipe(io.BytesIO(b""), Sink(),
                              on_close=lambda: closed.append(1)))
    pool.drop("box")
    assert pool.idle("box") == 0
    assert closed == [1]


def test_keepalive_pings_idle_pipes():
    """nginx reaps a connection with no traffic at proxy_read_timeout, so an idle
    pipe must be pinged -- and from THIS side, because nginx resets that timer on
    data read from the upstream, which is this process."""
    pool = tn.PipePool()
    sink = Sink()
    pool.offer("box", tn.Pipe(io.BytesIO(b""), sink))
    pool.keepalive()
    assert [f.op for f in _frames_out(sink)] == [ws.OP_PING]
    assert pool.idle("box") == 1


def test_keepalive_reaps_a_pipe_that_cannot_be_written():
    class Broken(Sink):
        def write(self, b):
            raise BrokenPipeError("gone")

    pool = tn.PipePool()
    pool.offer("box", tn.Pipe(io.BytesIO(b""), Broken()))
    pool.keepalive()
    assert pool.idle("box") == 0


def test_keepalive_does_not_disturb_a_pipe_in_flight():
    """A ping written into a conversation would be fine on the wire -- control
    frames are outside the data stream -- but the pool must not touch a pipe it
    has handed out, because the writer is another thread."""
    pool = tn.PipePool()
    sink = Sink()
    pool.offer("box", tn.Pipe(io.BytesIO(b""), sink))
    pool.take("box", timeout=0.1)
    pool.keepalive()
    assert _frames_out(sink) == []
