package main

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"io"
	"net"
	"os"
	"time"
)

// fakeConn is a net.Conn over two buffers, for framing tests that need no socket.
type fakeConn struct {
	r io.Reader
	w io.Writer
}

func (f *fakeConn) Read(b []byte) (int, error)  { return f.r.Read(b) }
func (f *fakeConn) Write(b []byte) (int, error) { return f.w.Write(b) }
func (f *fakeConn) Close() error                { return nil }
func (f *fakeConn) LocalAddr() net.Addr         { return nil }
func (f *fakeConn) RemoteAddr() net.Addr        { return nil }
func (f *fakeConn) SetDeadline(time.Time) error      { return nil }
func (f *fakeConn) SetReadDeadline(time.Time) error  { return nil }
func (f *fakeConn) SetWriteDeadline(time.Time) error { return nil }

func newReader(r io.Reader) *bufio.Reader { return bufio.NewReader(r) }

// writeServerFrame writes an UNMASKED frame, as a server must.
func writeServerFrame(w io.Writer, op byte, payload []byte) error {
	n := len(payload)
	head := []byte{0x80 | op}
	switch {
	case n < 126:
		head = append(head, byte(n))
	case n < 1<<16:
		head = append(head, 126)
		head = binary.BigEndian.AppendUint16(head, uint16(n))
	default:
		head = append(head, 127)
		head = binary.BigEndian.AppendUint64(head, uint64(n))
	}
	_, err := w.Write(append(head, payload...))
	return err
}

func readFile(name string) (string, error) {
	b, err := os.ReadFile(name)
	return string(b), err
}

var _ = bytes.Contains
