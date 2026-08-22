#!/usr/bin/env python3
"""RFC 6455 framing, server side. Pure: no sockets, no policy, no I/O of its own.

WHY THIS IS NOT A LIBRARY

The other end of every one of these connections is the agent -- a single native
binary with no dependencies, which rules out a WebSocket module there. So the
wire format is implemented twice, in Go and here, and the risk that the two
copies disagree is real. Neither is allowed to validate the other: both are
tested against the RFC's own published vectors and against every length-encoding
boundary, so they can only agree by both being right.

WHY WEBSOCKET AT ALL, given the payload is opaque bytes

Two reasons, and both are about what sits in between. An HTTP proxy will carry
arbitrary bytes only after an Upgrade, so the framing is what gets the stream
through nginx on the one public port. And ping/pong are CONTROL frames, outside
the data stream -- which is what lets an idle pipe be kept alive without
injecting a byte into the conversation it is carrying.

WHAT THIS DELIBERATELY DOES NOT DO

It does not reassemble fragmented messages. The layer above is a byte stream, so
frames are handed up as they arrive; accumulating a message would reintroduce the
buffering this whole transport exists to avoid.
"""
from __future__ import annotations

import base64
import hashlib
import os
import struct
from dataclasses import dataclass

# RFC 6455 section 1.3. Not a secret and not a nonce: a fixed string both ends
# hash, which is how a client proves the server understood the upgrade.
# Taken from a conformant implementation, not from memory: the first attempt
# here transposed the final group's leading C to the end, which every real client
# would have rejected. The test pins it to the RFC's published vector.
ACCEPT_MAGIC = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

OP_CONT, OP_TEXT, OP_BIN, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA

# A frame larger than this is refused BEFORE anything is allocated for it. The
# gateway relays in 64 KB chunks, so a megabyte is already generous; the number
# exists so a hostile or corrupt length cannot turn into a memory request.
MAX_PAYLOAD = 1 << 20

# Close codes used by this project. 1000/1002 are the RFC's; 4000-4999 is the
# range reserved for an application, and naming them here keeps the meanings in
# one place rather than scattered as literals.
CLOSE_NORMAL = 1000
CLOSE_PROTOCOL = 1002
CLOSE_TOO_BIG = 1009
CLOSE_ALREADY_CONNECTED = 4409      # this server id already has a live control


class ProtocolError(Exception):
    """A frame the connection must be CLOSED over, never interpreted.

    Separate from an EOF on purpose: a truncated stream is a peer going away,
    which is ordinary, while a malformed frame means the two ends disagree about
    the format and continuing would corrupt whatever comes next.
    """


def accept_key(client_key: str) -> str:
    digest = hashlib.sha1((client_key + ACCEPT_MAGIC).encode()).digest()
    return base64.b64encode(digest).decode()


def handshake_response(headers) -> bytes | None:
    """The 101 response for a valid upgrade, or None if this is not one.

    None rather than an exception because "not an upgrade" is a routing answer,
    not a failure: the caller replies with an ordinary HTTP error.

    Header handling is deliberately lenient about form and strict about content.
    `Upgrade` is matched case-insensitively (browsers send `websocket`, some
    clients `WebSocket`), and `Connection` is matched as a token within the
    value, because it legitimately arrives as `keep-alive, Upgrade` and nginx
    forwards it lowercased. Being strict there would mean an endpoint that works
    in tests and fails through the proxy.
    """
    get = headers.get
    if "websocket" not in (get("Upgrade") or "").lower():
        return None
    if "upgrade" not in (get("Connection") or "").lower():
        return None
    # Version 13 is the only one that exists. An unknown version must be refused
    # rather than guessed at.
    if (get("Sec-WebSocket-Version") or "").strip() != "13":
        return None
    key = (get("Sec-WebSocket-Key") or "").strip()
    if not key:
        return None
    return ("HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept_key(key)}\r\n"
            "\r\n").encode()


def encode(op: int, payload: bytes = b"", fin: bool = True,
           mask: bool = False) -> bytes:
    """One frame.

    `mask` is for tests and for anything acting as a CLIENT. A server must never
    mask what it sends -- a masked server frame is a protocol violation the other
    end is required to close on.
    """
    n = len(payload)
    head = bytes([(0x80 if fin else 0x00) | op])
    flag = 0x80 if mask else 0x00
    if n < 126:
        head += bytes([flag | n])
    elif n < (1 << 16):
        head += bytes([flag | 126]) + struct.pack("!H", n)
    else:
        head += bytes([flag | 127]) + struct.pack("!Q", n)
    if not mask:
        return head + payload
    key = os.urandom(4)
    return head + key + bytes(b ^ key[i % 4] for i, b in enumerate(payload))


def close_payload(code: int, reason: str = "") -> bytes:
    return struct.pack("!H", code) + reason.encode()[:123]


def close_code(payload: bytes) -> int | None:
    """The code in a close frame, or None when the peer sent none -- which is
    allowed, and means nothing more than "closing"."""
    if len(payload) < 2:
        return None
    return struct.unpack("!H", payload[:2])[0]


@dataclass
class Frame:
    op: int
    payload: bytes
    fin: bool


class FrameReader:
    """Frames off a readable file object.

    `require_mask` is True for anything reading from a client, which is every
    real use here. It is False only where this project reads back frames it
    encoded itself -- in tests, and in the pipe's own outbound accounting.
    """

    def __init__(self, rfile, *, require_mask: bool = True) -> None:
        self.rfile = rfile
        self.require_mask = require_mask

    def _exactly(self, n: int) -> bytes | None:
        """n bytes, or None at EOF.

        A socket read returns what has ARRIVED, not what was asked for. Treating
        a short read as the whole header is how a decoder silently desynchronises
        and then reports garbage as data, so this loops.
        """
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
        message, because the pipe above it is a byte stream and buffering a whole
        message would reintroduce exactly what this transport avoids.
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
        # Checked BEFORE reading, so a hostile length is a refusal rather than a
        # memory request.
        if n > MAX_PAYLOAD:
            raise ProtocolError(f"frame of {n} bytes exceeds {MAX_PAYLOAD}")
        if self.require_mask and not masked:
            raise ProtocolError("client frame is not masked")
        key = b""
        if masked:
            key = self._exactly(4)
            if key is None:
                return None
        payload = b""
        if n:
            payload = self._exactly(n)
            if payload is None:
                return None
            if masked:
                payload = bytes(b ^ key[i % 4] for i, b in enumerate(payload))
        return Frame(op=op, payload=payload, fin=fin)
