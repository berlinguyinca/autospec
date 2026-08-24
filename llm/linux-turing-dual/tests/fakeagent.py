"""The agent's side of the protocol, over real sockets.

Nothing here is simulated: it performs the real HTTP upgrade, masks its frames as
a client must, and pumps bytes to a real local target over a real connection. A
framing or handshake mistake on EITHER side therefore fails these tests instead of
passing them, which is the whole reason this exists rather than a substitute for
the transport. Shared by the endpoint tests and the routing tests.
"""
import json
import socket
import threading

from nodescripts import load_script

ws = load_script("wsframe")


class FakeAgent:
    def __init__(self, port, credential):
        self.port, self.credential = port, credential
        self.pipes, self.control_sock = [], None
        self.closed_pipes = 0

    # --- the upgrade -------------------------------------------------------
    def _upgrade(self, path, credential=None):
        s = socket.create_connection(("127.0.0.1", self.port), timeout=10)
        cred = self.credential if credential is None else credential
        head = (f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
                # nginx terminates TLS in production and sets this; the endpoint
                # refuses a credential presented in cleartext.
                "X-Forwarded-Proto: https\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                "Sec-WebSocket-Version: 13\r\n"
                "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n")
        if cred:
            head += f"Authorization: Bearer {cred}\r\n"
        s.sendall((head + "\r\n").encode())
        rfile = s.makefile("rb")
        status = rfile.readline()
        while True:
            line = rfile.readline()
            if line in (b"\r\n", b"", b"\n"):
                break
        code = int(status.split()[1]) if len(status.split()) > 1 else 0
        return s, rfile, code

    # --- control -----------------------------------------------------------
    def control(self, server_id, *, gpus="one card", slots=2, credential=None):
        """Returns the HTTP status; on 101 the connection stays live."""
        s, rfile, code = self._upgrade("/api/agent/control", credential)
        if code != 101:
            s.close()
            return code
        s.sendall(ws.encode(ws.OP_TEXT, json.dumps({
            "type": "hello", "agent_version": "test", "server_id": server_id,
            "gpus": gpus, "slots": slots, "note": "fake"}).encode(), mask=True))
        self.control_sock = s
        self.control_closed = threading.Event()
        self.close_code = None
        threading.Thread(target=self._control_loop, args=(s, rfile),
                         daemon=True).start()
        return 101

    def _control_loop(self, s, rfile):
        try:
            reader = ws.FrameReader(rfile, require_mask=False)
            while (f := reader.read()) is not None:
                if f.op == ws.OP_PING:
                    if not getattr(self, "silent", False):
                        s.sendall(ws.encode(ws.OP_PONG, f.payload, mask=True))
                elif f.op == ws.OP_CLOSE:
                    self.close_code = ws.close_code(f.payload)
                    break
        except OSError:
            pass
        finally:
            self.control_closed.set()

    def stop_control(self):
        """Drop the connection.

        shutdown() before close(), because close() alone does NOT send FIN while
        a makefile object still holds the descriptor -- the fd is only released
        when the last reference goes. Without the shutdown the node sees nothing
        and has to wait out the heartbeat, which is a different test.
        """
        if self.control_sock:
            try:
                self.control_sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.control_sock.close()
            self.control_sock = None

    def go_silent(self):
        """Stop answering pings while keeping the socket open -- what a box that
        lost power looks like from here. Only the heartbeat catches this."""
        self.silent = True

    # --- pipes -------------------------------------------------------------
    def add_pipe(self, target, credential=None):
        """Offer one pipe, pumping to `target` == (host, port)."""
        s, rfile, code = self._upgrade("/api/agent/pipe", credential)
        if code != 101:
            s.close()
            return code
        self.pipes.append(s)
        threading.Thread(target=self._pump, args=(s, rfile, target),
                         daemon=True).start()
        return 101

    def _pump(self, s, rfile, target):
        local = None
        try:
            reader = ws.FrameReader(rfile, require_mask=False)
            while (f := reader.read()) is not None:
                if f.op == ws.OP_PING:
                    s.sendall(ws.encode(ws.OP_PONG, f.payload, mask=True))
                    continue
                if f.op in (ws.OP_PONG,):
                    continue
                if f.op == ws.OP_CLOSE:
                    break
                if local is None:
                    # Lazily, exactly as the real agent does: an idle pipe must
                    # not pin an idle connection on the inference server.
                    local = socket.create_connection(target, timeout=10)
                    threading.Thread(target=self._back, args=(local, s),
                                     daemon=True).start()
                local.sendall(f.payload)
        except OSError:
            pass
        finally:
            if local:
                try:
                    local.close()
                except OSError:
                    pass
            try:
                s.close()
            except OSError:
                pass
            self.closed_pipes += 1

    def _back(self, local, s):
        try:
            while (buf := local.recv(65536)):
                s.sendall(ws.encode(ws.OP_BIN, buf, mask=True))
        except OSError:
            pass

    def stop(self):
        for p in [self.control_sock, *self.pipes]:
            if p:
                try:
                    p.close()
                except OSError:
                    pass
