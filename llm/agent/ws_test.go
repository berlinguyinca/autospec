package main

// Pinned to the RFC's own vectors, NOT to the Python implementation on the node.
// Two copies of one wire format can only be trusted if each is checked against
// the standard independently; checking them against each other would let a shared
// misreading pass twice.

import (
	"bytes"
	"encoding/base64"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestAcceptKeyMatchesTheRFCExample(t *testing.T) {
	// RFC 6455 section 1.3. A wrong constant here means every server refuses the
	// handshake, and the failure looks like a network problem.
	got := acceptKey("dGhlIHNhbXBsZSBub25jZQ==")
	if got != "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" {
		t.Fatalf("accept key = %q", got)
	}
}

// pair returns a Conn wired to an in-memory peer, plus what the peer receives.
func pair(t *testing.T, serverWrites []byte) (*Conn, *bytes.Buffer) {
	t.Helper()
	toClient := bytes.NewBuffer(serverWrites)
	fromClient := &bytes.Buffer{}
	return &Conn{c: &fakeConn{r: toClient, w: fromClient},
		r: newReader(toClient)}, fromClient
}

func TestEveryLengthBoundaryRoundTrips(t *testing.T) {
	// 125/126 and 65535/65536 are where the length form changes. An off-by-one
	// corrupts the stream rather than failing.
	for _, n := range []int{0, 1, 125, 126, 127, 65535, 65536} {
		payload := bytes.Repeat([]byte("z"), n)
		c, out := pair(t, nil)
		if err := c.WriteFrame(opBin, payload); err != nil {
			t.Fatalf("n=%d: %v", n, err)
		}
		// Read it back as a server would: masked, from the client.
		rc := &Conn{c: &fakeConn{r: out, w: &bytes.Buffer{}}, r: newReader(out)}
		op, got, fin, err := rc.ReadFrame()
		if err != nil || op != opBin || !fin || len(got) != n {
			t.Fatalf("n=%d: op=%x fin=%v len=%d err=%v", n, op, fin, len(got), err)
		}
		if !bytes.Equal(got, payload) {
			t.Fatalf("n=%d: payload differs", n)
		}
	}
}

func TestClientFramesAreAlwaysMasked(t *testing.T) {
	// An unmasked client frame is a protocol violation the server must close on.
	c, out := pair(t, nil)
	if err := c.WriteFrame(opText, []byte("hi")); err != nil {
		t.Fatal(err)
	}
	raw := out.Bytes()
	if raw[1]&0x80 == 0 {
		t.Fatalf("mask bit not set: % x", raw)
	}
	// And the payload really is masked, not merely flagged.
	if bytes.Contains(raw[2:], []byte("hi")) {
		t.Fatalf("payload sent in clear: % x", raw)
	}
}

func TestAnOversizedFrameIsRefusedBeforeAllocating(t *testing.T) {
	head := []byte{0x82, 127}
	n := uint64(maxPayload + 1)
	for i := 7; i >= 0; i-- {
		head = append(head, byte(n>>(8*i)))
	}
	c, _ := pair(t, head)
	if _, _, _, err := c.ReadFrame(); err == nil {
		t.Fatal("expected a refusal")
	} else if !strings.Contains(err.Error(), fmt.Sprint(maxPayload)) {
		t.Fatalf("unhelpful error: %v", err)
	}
}

func TestATruncatedFrameIsAnError(t *testing.T) {
	c, _ := pair(t, []byte{0x82, 10, 'a', 'b'})
	if _, _, _, err := c.ReadFrame(); err == nil {
		t.Fatal("expected an error on a short frame")
	}
}

func TestTheHandshakeIsVerifiedNotAssumed(t *testing.T) {
	// A proxy that answered 101 without understanding the upgrade would other-
	// wise be indistinguishable from a real peer.
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hj, _ := w.(http.Hijacker)
		conn, buf, _ := hj.Hijack()
		defer conn.Close()
		buf.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n" +
			"Connection: Upgrade\r\nSec-WebSocket-Accept: wrong\r\n\r\n")
		buf.Flush()
	}))
	defer srv.Close()
	_, err := Dial("ws://"+strings.TrimPrefix(srv.URL, "http://")+"/x", "qts_x", 5*time.Second)
	if err == nil || !strings.Contains(err.Error(), "accept") {
		t.Fatalf("a wrong accept key must be refused, got %v", err)
	}
}

func TestDialSendsTheCredentialAndAValidKey(t *testing.T) {
	seen := make(chan *http.Request, 1)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		seen <- r
		hj, _ := w.(http.Hijacker)
		conn, buf, _ := hj.Hijack()
		defer conn.Close()
		buf.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n" +
			"Connection: Upgrade\r\nSec-WebSocket-Accept: " +
			acceptKey(r.Header.Get("Sec-WebSocket-Key")) + "\r\n\r\n")
		buf.Flush()
		time.Sleep(50 * time.Millisecond)
	}))
	defer srv.Close()
	c, err := Dial("ws://"+strings.TrimPrefix(srv.URL, "http://")+"/api/agent/pipe",
		"qts_abcdefghijkl_secret", 5*time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	r := <-seen
	if got := r.Header.Get("Authorization"); got != "Bearer qts_abcdefghijkl_secret" {
		t.Fatalf("credential not sent: %q", got)
	}
	key, err := base64.StdEncoding.DecodeString(r.Header.Get("Sec-WebSocket-Key"))
	if err != nil || len(key) != 16 {
		// 16 random bytes is required; a fixed or short key marks a broken client.
		t.Fatalf("key must be 16 random bytes, got %d (%v)", len(key), err)
	}
}

// --- the pump ---------------------------------------------------------------

func TestAPipeCarriesARequestToTheTargetAndTheAnswerBack(t *testing.T) {
	// The whole job of the agent, end to end: a real HTTP server as the target,
	// a real WebSocket as the pipe, and a large body so this is not a toy.
	big := strings.Repeat("x", 469*1024)
	var got []byte
	target := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got, _ = io.ReadAll(r.Body)
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, `{"ok":true}`)
	}))
	defer target.Close()

	node := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hj, _ := w.(http.Hijacker)
		conn, buf, _ := hj.Hijack()
		defer conn.Close()
		buf.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n" +
			"Connection: Upgrade\r\nSec-WebSocket-Accept: " +
			acceptKey(r.Header.Get("Sec-WebSocket-Key")) + "\r\n\r\n")
		buf.Flush()

		// Play the node: send a request down the pipe, read the answer back.
		peer := &Conn{c: conn, r: newReader(buf)}
		body := `{"model":"m","pad":"` + big + `"}`
		req := "POST /v1/chat/completions HTTP/1.1\r\nHost: t\r\n" +
			fmt.Sprintf("Content-Length: %d\r\n\r\n", len(body)) + body
		// Unmasked, as a server must send.
		if err := writeServerFrame(conn, opBin, []byte(req)); err != nil {
			t.Error(err)
			return
		}
		deadline := time.Now().Add(10 * time.Second)
		var answer []byte
		for time.Now().Before(deadline) {
			conn.SetReadDeadline(time.Now().Add(2 * time.Second))
			op, payload, _, err := peer.ReadFrame()
			if err != nil {
				break
			}
			if op == opBin {
				answer = append(answer, payload...)
				if bytes.Contains(answer, []byte(`{"ok":true}`)) {
					break
				}
			}
		}
		if !bytes.Contains(answer, []byte(`{"ok":true}`)) {
			t.Errorf("no answer came back through the pipe: %q", string(answer))
		}
	}))
	defer node.Close()

	cfg := &Config{Node: strings.TrimPrefix(node.URL, "http://"),
		Target: target.URL, Credential: "qts_x", Insecure: true, Pipes: 1}
	stop := make(chan struct{})
	done := make(chan error, 1)
	go func() { done <- onePipe(cfg, stop) }()

	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) && len(got) == 0 {
		time.Sleep(20 * time.Millisecond)
	}
	close(stop)
	if len(got) == 0 {
		t.Fatal("the target never received the request")
	}
	// Byte-identical: the pipe is not a place where a body is assembled or
	// re-chunked.
	if !bytes.Contains(got, []byte(big)) {
		t.Fatalf("body arrived altered (%d bytes)", len(got))
	}
	select {
	case <-done:
	case <-time.After(3 * time.Second):
	}
}

func TestTheTargetIsNeverTakenFromTheNode(t *testing.T) {
	// The invariant that keeps a compromised node from scanning an owner's
	// network: no field in either protocol message names a destination, so the
	// agent's own config is the only source. Asserted structurally.
	for _, name := range []string{"agent.go", "config.go"} {
		src, err := readFile(name)
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(src, `msg.Target`) || strings.Contains(src, `"target"`) &&
			strings.Contains(src, "json.Unmarshal") && name == "agent.go" {
			t.Fatalf("%s appears to read a target from the node", name)
		}
	}
	cfg := &Config{Target: "http://127.0.0.1:8080"}
	if _, err := dialTarget(cfg.Target); err == nil {
		// Nothing listening is fine; the point is only that the address came
		// from the config.
		t.Log("something answered on the default target")
	}
}

// --- reconnect pacing -------------------------------------------------------
//
// The control loop's backoff grew and never shrank, so after a handful of node
// restarts it pinned at the 60 s ceiling and STAYED there for the life of the
// process. A registered server was then unusable for a full minute after every
// redeploy -- measured at 65 s on an agent that had merely been running a while,
// and it would never have got better on its own.
//
// The fix hinges entirely on oneControl distinguishing "never got in" from "was
// in and got dropped", so that is what these pin.

func controlServer(t *testing.T, after func(net.Conn)) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hj, _ := w.(http.Hijacker)
		conn, buf, _ := hj.Hijack()
		defer conn.Close()
		buf.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n" +
			"Connection: Upgrade\r\nSec-WebSocket-Accept: " +
			acceptKey(r.Header.Get("Sec-WebSocket-Key")) + "\r\n\r\n")
		buf.Flush()
		after(conn)
	}))
}

func controlConfig(url string) *Config {
	// Insecure so the agent dials ws:// rather than wss:// -- httptest serves
	// plain HTTP, and the scheme is derived from this flag rather than set.
	return &Config{Node: strings.TrimPrefix(url, "http://"), Insecure: true,
		ServerID: "test-box", Credential: "qts_abcdefghijkl_secret"}
}

func TestADroppedControlConnectionCountsAsAttached(t *testing.T) {
	// The redeploy case: the node restarts under a healthy agent. This must come
	// straight back rather than waiting out a grown backoff.
	srv := controlServer(t, func(conn net.Conn) { /* hang up at once */ })
	defer srv.Close()
	attached, err := oneControl(controlConfig(srv.URL), make(chan struct{}), &pipeKeeper{})
	if !attached {
		t.Fatalf("a connection that was established must report attached (err %v)", err)
	}
}

func TestANodeThatIsDownCountsAsNotAttached(t *testing.T) {
	// Nothing listening: the backoff SHOULD keep growing here.
	cfg := controlConfig("http://127.0.0.1:1")
	attached, err := oneControl(cfg, make(chan struct{}), &pipeKeeper{})
	if attached || err == nil {
		t.Fatalf("a failed dial must report not attached, got attached=%v err=%v",
			attached, err)
	}
}

func TestBeingReplacedCountsAsNotAttached(t *testing.T) {
	// 4409 means another process legitimately holds this name. Resetting the
	// backoff here would put this one in a one-second loop fighting it.
	srv := controlServer(t, func(conn net.Conn) {
		// Derived, not hand-encoded: the first attempt at these two bytes was
		// 4425 and the test passed for the wrong reason until it did not.
		writeServerFrame(conn, opClose, []byte{byte(4409 >> 8), byte(4409 & 0xff)})
		time.Sleep(30 * time.Millisecond)
	})
	defer srv.Close()
	attached, err := oneControl(controlConfig(srv.URL), make(chan struct{}), &pipeKeeper{})
	if attached {
		t.Fatalf("a 4409 must report not attached, so the backoff keeps growing")
	}
	if err == nil || !strings.Contains(err.Error(), "already connected") {
		t.Fatalf("unexpected error: %v", err)
	}
}
