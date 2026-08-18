#!/usr/bin/env python3
"""A local port that is always open, whatever the cluster is doing.

    hive-proxy.py --listen 11111 --upstream 11112 [--wait 1800]

OpenCode talks to this. It relays to the SSH forward, which comes and goes: the
job gets preempted, the walltime expires, the network blinks. Without something
holding the near side, every one of those is `Connection refused` in the
client's face, and a refused connection is not a slow request -- it is an error
the user has to notice and act on.

So the listening socket is owned by this process and never closes. When a
connection arrives and the upstream is not there, the client is simply *held*
until it is. An outage becomes latency, which is a thing an HTTP client already
knows how to wait through.

Deliberately a byte relay and not an HTTP proxy: it has to carry streamed
server-sent events and long keep-alive connections without understanding or
buffering them, and the moment it parses HTTP it acquires opinions about
framing that will be wrong somewhere.

The honest limit: a connection that dies MID-RESPONSE cannot be retried here,
because bytes have already reached the client and this layer has no idea which
ones. Requests that had not started yet -- the common case during a reconnect
or a node change -- are fully covered.
"""
from __future__ import annotations

import argparse
import socket
import sys
import threading
import time

SHUTDOWN = threading.Event()


def log(msg: str) -> None:
    print(f"{time.strftime('%Y-%m-%dT%H:%M:%S%z')} {msg}", flush=True)


def connect_upstream(port: int, deadline: float, host: str = "127.0.0.1"):
    """Wait for the upstream to exist, rather than failing the client."""
    waited = 0.0
    announced = False
    while time.time() < deadline and not SHUTDOWN.is_set():
        try:
            s = socket.create_connection((host, port), timeout=10)
            # create_connection's timeout is not just for the handshake: it is
            # left on the socket, so every later recv inherits it. A 10s cap
            # then kills any request with a longer quiet stretch -- a model
            # loading for two minutes, or a long prefill before the first
            # token. Blocking mode is what a relay wants once connected.
            s.settimeout(None)
            if announced:
                log(f"upstream back after {waited:.0f}s")
            return s
        except OSError:
            if not announced:
                log(f"upstream {host}:{port} down; holding client connections")
                announced = True
            time.sleep(0.5)
            waited += 0.5
    return None


def pump(src: socket.socket, dst: socket.socket) -> None:
    try:
        while True:
            data = src.recv(65536)
            if not data:
                break
            dst.sendall(data)
    except OSError:
        pass
    finally:
        # Half-close so the peer sees EOF instead of hanging on a stream that
        # will never produce another byte.
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def handle(client: socket.socket, upstream_port: int, wait_s: int) -> None:
    up = connect_upstream(upstream_port, time.time() + wait_s)
    if up is None:
        log("gave up waiting for upstream; dropping a client connection")
        client.close()
        return
    with client, up:
        a = threading.Thread(target=pump, args=(client, up), daemon=True)
        b = threading.Thread(target=pump, args=(up, client), daemon=True)
        a.start(); b.start()
        a.join(); b.join()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", type=int, default=11111)
    ap.add_argument("--upstream", type=int, default=11112)
    ap.add_argument("--wait", type=int, default=1800,
                    help="seconds to hold a client while the upstream is "
                         "missing; long enough to cover re-acquiring a GPU")
    args = ap.parse_args()

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind(("127.0.0.1", args.listen))
    except OSError as exc:
        log(f"cannot bind 127.0.0.1:{args.listen}: {exc}")
        return 1
    srv.listen(128)
    log(f"listening on 127.0.0.1:{args.listen} -> 127.0.0.1:{args.upstream} "
        f"(holding up to {args.wait}s through an outage)")

    try:
        while True:
            client, _ = srv.accept()
            threading.Thread(target=handle,
                             args=(client, args.upstream, args.wait),
                             daemon=True).start()
    except KeyboardInterrupt:
        SHUTDOWN.set()
    finally:
        srv.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
