"""RFC 6455 framing, pinned to the RFC's own vectors.

This codec exists twice -- Python here, Go in the agent -- because the agent is a
dependency-free binary and a WebSocket library would end that. Two
implementations of one wire format is a real risk, so neither is allowed to
validate the other: both are tested against the published vectors and against
the length-encoding boundaries, so they can only agree by both being right.

The vector below already earned its place: the first implementation transposed a
character in the RFC's GUID, which every conformant client would have rejected.

Interop was also checked out of band, against Chromium, which cannot live here
because a browser is not a dependency of this project: a page opened a real
WebSocket to a server built on this module, the handshake was accepted, a
browser-masked text frame decoded correctly, and a 200,000-byte server frame --
the 64-bit length form -- arrived intact at an independent decoder.
"""
import io

import pytest

from conftest import load_script

ws = load_script("wsframe")


def test_the_accept_key_matches_the_rfc_example():
    # RFC 6455 section 1.3. A wrong accept key means every conformant client
    # refuses the connection, so this is pinned to the published vector rather
    # than to our own implementation.
    assert ws.accept_key("dGhlIHNhbXBsZSBub25jZQ==") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="


def test_a_client_frame_must_be_masked():
    # RFC 6455 section 5.1: a server MUST close the connection on an unmasked
    # client frame. Accepting it would also mean our decoder disagrees with every
    # conformant client about where the payload starts.
    unmasked = bytes([0x81, 0x03]) + b"abc"
    with pytest.raises(ws.ProtocolError) as exc:
        ws.FrameReader(io.BytesIO(unmasked)).read()
    assert "mask" in str(exc.value)
    # The same frame, masked, must read fine -- otherwise this test would pass
    # against a reader that refuses everything.
    masked = ws.FrameReader(io.BytesIO(ws.encode(ws.OP_TEXT, b"abc", mask=True))).read()
    assert masked.payload == b"abc"
    # And a reader told it is NOT facing a client accepts the unmasked frame,
    # which is how the pipe reads back frames it encoded itself.
    lenient = ws.FrameReader(io.BytesIO(unmasked), require_mask=False).read()
    assert lenient.payload == b"abc"


def test_a_masked_client_frame_round_trips():
    raw = ws.encode(ws.OP_TEXT, b"hello", mask=True)
    f = ws.FrameReader(io.BytesIO(raw)).read()
    assert (f.op, f.payload, f.fin) == (ws.OP_TEXT, b"hello", True)


def test_the_server_never_masks_what_it_sends():
    # A masked server frame is a protocol violation the other end closes on.
    assert ws.encode(ws.OP_BIN, b"x")[1] & 0x80 == 0


@pytest.mark.parametrize("size", [0, 1, 125, 126, 127, 65535, 65536])
def test_every_length_encoding_boundary_round_trips(size):
    # 125/126 and 65535/65536 are where the 7-bit, 16-bit and 64-bit length
    # forms change. An off-by-one here corrupts the stream rather than failing.
    payload = b"z" * size
    f = ws.FrameReader(io.BytesIO(ws.encode(ws.OP_BIN, payload, mask=True))).read()
    assert f.payload == payload


def test_a_payload_split_across_reads_is_reassembled():
    class Trickle(io.RawIOBase):
        """A stream that returns one byte per read, which a socket may do.

        Handed to the reader UNBUFFERED on purpose: wrapping it in a
        BufferedReader would hide short reads behind the buffer and the test
        would pass against a decoder that ignores them.
        """

        def __init__(self, data):
            self.data, self.i = data, 0

        def readable(self):
            return True

        def read(self, n=-1):
            if self.i >= len(self.data):
                return b""
            self.i += 1
            return self.data[self.i - 1:self.i]

    raw = ws.encode(ws.OP_BIN, b"abcdefghij", mask=True)
    f = ws.FrameReader(Trickle(raw)).read()
    assert f.payload == b"abcdefghij"


def test_fragments_are_reported_separately_not_merged():
    # The pipe above this streams bytes; merging fragments would mean buffering a
    # whole message, which is the one thing this transport must never do.
    a = ws.encode(ws.OP_BIN, b"one", fin=False, mask=True)
    b = ws.encode(ws.OP_CONT, b"two", fin=True, mask=True)
    r = ws.FrameReader(io.BytesIO(a + b))
    first, second = r.read(), r.read()
    assert (first.payload, first.fin) == (b"one", False)
    assert (second.payload, second.fin) == (b"two", True)


def test_a_control_frame_between_fragments_is_delivered():
    # RFC 6455 section 5.4 allows this, and the idle-pipe keepalive depends on it.
    frames = (ws.encode(ws.OP_BIN, b"one", fin=False, mask=True)
              + ws.encode(ws.OP_PING, b"", mask=True)
              + ws.encode(ws.OP_CONT, b"two", fin=True, mask=True))
    ops = [f.op for f in iter(ws.FrameReader(io.BytesIO(frames)).read, None)]
    assert ops == [ws.OP_BIN, ws.OP_PING, ws.OP_CONT]


def test_an_oversized_frame_is_refused_rather_than_allocated():
    # Only the HEADER is supplied. A reader that allocates before checking the
    # length would hang waiting for a megabyte that is never coming; one that
    # checks first raises immediately, which is what this asserts.
    header = bytes([0x82, 0xFF]) + (ws.MAX_PAYLOAD + 1).to_bytes(8, "big") + b"\0\0\0\0"
    with pytest.raises(ws.ProtocolError) as exc:
        ws.FrameReader(io.BytesIO(header)).read()
    assert str(ws.MAX_PAYLOAD) in str(exc.value)
    # The boundary itself is legal: refusing at exactly the limit would be an
    # off-by-one nobody would notice until a large body failed.
    at_limit = b"y" * ws.MAX_PAYLOAD
    f = ws.FrameReader(io.BytesIO(ws.encode(ws.OP_BIN, at_limit, mask=True))).read()
    assert len(f.payload) == ws.MAX_PAYLOAD


def test_a_truncated_frame_reads_as_eof_not_as_data():
    truncated = ws.encode(ws.OP_BIN, b"abcdefghij", mask=True)[:6]
    assert ws.FrameReader(io.BytesIO(truncated)).read() is None


def test_a_clean_eof_is_none():
    assert ws.FrameReader(io.BytesIO(b"")).read() is None


def test_a_close_frame_carries_its_code():
    raw = ws.encode(ws.OP_CLOSE, ws.close_payload(4409, "already connected"), mask=True)
    f = ws.FrameReader(io.BytesIO(raw)).read()
    assert f.op == ws.OP_CLOSE
    assert ws.close_code(f.payload) == 4409


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


def test_the_handshake_accepts_the_header_casing_a_proxy_produces():
    # nginx forwards `Connection: upgrade` lowercased, and browsers send
    # `Connection: keep-alive, Upgrade`. Both are valid; refusing either would
    # mean the endpoint works in tests and not through the proxy.
    for conn in ("upgrade", "Upgrade", "keep-alive, Upgrade"):
        assert ws.handshake_response({
            "Upgrade": "WebSocket", "Connection": conn,
            "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ==",
            "Sec-WebSocket-Version": "13"}) is not None, conn
