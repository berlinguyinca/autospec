package main

// RFC 6455, client side, standard library only.
//
// This is the second implementation of one wire format -- the node has a Python
// one -- and neither is allowed to validate the other. Both are tested against
// the RFC's published vectors, so they can only agree by both being right. The
// first cut of the Python side transposed a character in the handshake GUID and
// the vector caught it; that is why the constant below is not typed from memory.
//
// A library would be one dependency, and one is all it takes to stop being a
// single file you can copy onto a machine.

import (
	"bufio"
	"crypto/rand"
	"crypto/sha1"
	"crypto/tls"
	"encoding/base64"
	"encoding/binary"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
)

const acceptMagic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

const (
	opCont  byte = 0x0
	opText  byte = 0x1
	opBin   byte = 0x2
	opClose byte = 0x8
	opPing  byte = 0x9
	opPong  byte = 0xA
)

// Matches the node's MAX_PAYLOAD. A frame larger than this is refused before
// anything is allocated for it.
const maxPayload = 1 << 20

// Conn is one WebSocket. Writes are serialised because the pump and the
// keepalive both send.
type Conn struct {
	c   net.Conn
	r   *bufio.Reader
	mu  sync.Mutex
	rmu sync.Mutex
}

func acceptKey(key string) string {
	h := sha1.Sum([]byte(key + acceptMagic))
	return base64.StdEncoding.EncodeToString(h[:])
}

// Dial performs the upgrade and returns the raw connection.
//
// The URL scheme decides TLS. Plain ws:// exists only for a node reached over a
// trusted local network; the node itself refuses a credential that did not
// arrive over TLS, so in practice this is wss://.
func Dial(raw, credential string, timeout time.Duration) (*Conn, error) {
	u, err := url.Parse(raw)
	if err != nil {
		return nil, err
	}
	host := u.Host
	if u.Port() == "" {
		if u.Scheme == "wss" {
			host = net.JoinHostPort(u.Hostname(), "443")
		} else {
			host = net.JoinHostPort(u.Hostname(), "80")
		}
	}

	var conn net.Conn
	dialer := &net.Dialer{Timeout: timeout}
	if u.Scheme == "wss" {
		// The standard library verifies the chain and the hostname against the
		// system root store, on every platform this builds for. That is the
		// whole reason this agent is written in Go.
		conn, err = tls.DialWithDialer(dialer, "tcp", host,
			&tls.Config{ServerName: u.Hostname()})
	} else {
		conn, err = dialer.Dial("tcp", host)
	}
	if err != nil {
		return nil, err
	}

	nonce := make([]byte, 16)
	if _, err := rand.Read(nonce); err != nil {
		conn.Close()
		return nil, err
	}
	key := base64.StdEncoding.EncodeToString(nonce)
	path := u.Path
	if u.RawQuery != "" {
		path += "?" + u.RawQuery
	}
	req := "GET " + path + " HTTP/1.1\r\n" +
		"Host: " + u.Host + "\r\n" +
		"Upgrade: websocket\r\n" +
		"Connection: Upgrade\r\n" +
		"Sec-WebSocket-Version: 13\r\n" +
		"Sec-WebSocket-Key: " + key + "\r\n" +
		"Authorization: Bearer " + credential + "\r\n\r\n"
	if err := conn.SetDeadline(time.Now().Add(timeout)); err != nil {
		conn.Close()
		return nil, err
	}
	if _, err := io.WriteString(conn, req); err != nil {
		conn.Close()
		return nil, err
	}

	br := bufio.NewReader(conn)
	resp, err := http.ReadResponse(br, nil)
	if err != nil {
		conn.Close()
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusSwitchingProtocols {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		conn.Close()
		return nil, fmt.Errorf("upgrade refused: %s: %s", resp.Status,
			strings.TrimSpace(string(body)))
	}
	// Verifying this is what proves the peer understood the upgrade rather than
	// merely answering 101 -- a proxy that echoed the status would be caught here.
	if got := resp.Header.Get("Sec-WebSocket-Accept"); got != acceptKey(key) {
		conn.Close()
		return nil, fmt.Errorf("handshake accept mismatch")
	}
	// Deadlines are per-operation from here on; a control connection lives for
	// days and a pipe for one request.
	if err := conn.SetDeadline(time.Time{}); err != nil {
		conn.Close()
		return nil, err
	}
	return &Conn{c: conn, r: br}, nil
}

// WriteFrame sends one frame. Client frames are ALWAYS masked: an unmasked one
// is a protocol violation the server must close on.
func (w *Conn) WriteFrame(op byte, payload []byte) error {
	w.mu.Lock()
	defer w.mu.Unlock()

	n := len(payload)
	head := []byte{0x80 | op}
	switch {
	case n < 126:
		head = append(head, 0x80|byte(n))
	case n < 1<<16:
		head = append(head, 0x80|126)
		head = binary.BigEndian.AppendUint16(head, uint16(n))
	default:
		head = append(head, 0x80|127)
		head = binary.BigEndian.AppendUint64(head, uint64(n))
	}
	var mask [4]byte
	if _, err := rand.Read(mask[:]); err != nil {
		return err
	}
	head = append(head, mask[:]...)
	masked := make([]byte, n)
	for i := 0; i < n; i++ {
		masked[i] = payload[i] ^ mask[i%4]
	}
	if _, err := w.c.Write(append(head, masked...)); err != nil {
		return err
	}
	return nil
}

// ReadFrame returns the next frame. A server frame is never masked.
func (w *Conn) ReadFrame() (op byte, payload []byte, fin bool, err error) {
	w.rmu.Lock()
	defer w.rmu.Unlock()

	var head [2]byte
	if _, err = io.ReadFull(w.r, head[:]); err != nil {
		return 0, nil, false, err
	}
	fin = head[0]&0x80 != 0
	op = head[0] & 0x0F
	masked := head[1]&0x80 != 0
	n := uint64(head[1] & 0x7F)
	switch n {
	case 126:
		var ext [2]byte
		if _, err = io.ReadFull(w.r, ext[:]); err != nil {
			return 0, nil, false, err
		}
		n = uint64(binary.BigEndian.Uint16(ext[:]))
	case 127:
		var ext [8]byte
		if _, err = io.ReadFull(w.r, ext[:]); err != nil {
			return 0, nil, false, err
		}
		n = binary.BigEndian.Uint64(ext[:])
	}
	// Checked before allocating, so a corrupt length is a refusal rather than a
	// memory request.
	if n > maxPayload {
		return 0, nil, false, fmt.Errorf("frame of %d bytes exceeds %d", n, maxPayload)
	}
	var mask [4]byte
	if masked {
		if _, err = io.ReadFull(w.r, mask[:]); err != nil {
			return 0, nil, false, err
		}
	}
	payload = make([]byte, n)
	if n > 0 {
		if _, err = io.ReadFull(w.r, payload); err != nil {
			return 0, nil, false, err
		}
		if masked {
			for i := range payload {
				payload[i] ^= mask[i%4]
			}
		}
	}
	return op, payload, fin, nil
}

func (w *Conn) Close() error { return w.c.Close() }

func (w *Conn) SetReadDeadline(t time.Time) error { return w.c.SetReadDeadline(t) }
