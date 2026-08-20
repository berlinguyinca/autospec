package main

// The two things this process does: hold one control connection, and keep a few
// pipes open for the node to invoke inference over.

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/rand"
	"net"
	"net/url"
	"os"
	"sync"
	"time"
)

type hello struct {
	Type         string `json:"type"`
	AgentVersion string `json:"agent_version"`
	ServerID     string `json:"server_id"`
	GPUs         string `json:"gpus,omitempty"`
	Slots        int    `json:"slots,omitempty"`
	Note         string `json:"note,omitempty"`
}

const version = "1"

// runControl holds the connection that says this server exists.
//
// Its existence IS the liveness signal, so it is the one thing that must come
// back after any failure. Backoff is exponential with jitter: without the
// jitter, a node restart would bring every agent in the fleet back in the same
// second.
func runControl(cfg *Config, stop <-chan struct{}, pipes *pipeKeeper) {
	backoff := time.Second
	for {
		select {
		case <-stop:
			return
		default:
		}
		attached, err := oneControl(cfg, stop, pipes)
		if err != nil && !stopped(stop) {
			// Not logged during shutdown: closing the connection is HOW a
			// blocked read is ended, so the resulting error is expected and
			// reporting it makes an orderly stop look like a fault.
			logf("control connection lost: %v", err)
		}
		select {
		case <-stop:
			return
		case <-time.After(jitter(backoff)):
		}
		if attached {
			// A connection that WORKED resets the delay. Without this the
			// backoff only ever grew: after a handful of node restarts it pinned
			// at the 60 s ceiling and STAYED there for the life of the process,
			// so every later redeploy cost a full minute of this server being
			// registered but unusable. Measured at 65 s on an agent that had
			// simply been running a while -- and it would never have improved.
			//
			// Growth is still right for a node that is genuinely down: that case
			// never attaches, so it never takes this branch.
			backoff = time.Second
		} else if backoff < 60*time.Second {
			backoff *= 2
		}
	}
}

// oneControl reports whether it ever attached, so the caller can tell a node
// that is down (keep backing off) from a node that restarted under a healthy
// agent (come straight back).
func oneControl(cfg *Config, stop <-chan struct{}, pipes *pipeKeeper) (bool, error) {
	c, err := Dial(cfg.wsURL("/api/agent/control"), cfg.Credential, 20*time.Second)
	if err != nil {
		return false, err
	}
	defer c.Close()
	// A read blocks until a frame or a deadline, so checking `stop` between reads
	// is not enough: on SIGTERM this process would linger for up to the deadline,
	// and the node would keep the server marked ONLINE the whole time -- taking
	// traffic for a machine that is shutting down. Closing the connection is what
	// unblocks the read.
	defer closeOnStop(c, stop)()
	logf("attached as %q", cfg.ServerID)

	msg, _ := json.Marshal(hello{Type: "hello", AgentVersion: version,
		ServerID: cfg.ServerID, GPUs: cfg.GPUs, Slots: cfg.Slots})
	if err := c.WriteFrame(opText, msg); err != nil {
		return true, err
	}

	// Pipes are opened only while the control connection is up, so a node that
	// has forgotten this server does not get a stream of connections it will
	// refuse.
	pipes.start(cfg)
	defer pipes.stop()

	for {
		select {
		case <-stop:
			return true, nil
		default:
		}
		// Comfortably longer than the node's heartbeat: if two of those pass in
		// silence the connection is gone, and reconnecting is cheaper than
		// waiting to find out.
		if err := c.SetReadDeadline(time.Now().Add(90 * time.Second)); err != nil {
			return true, err
		}
		op, payload, _, err := c.ReadFrame()
		if err != nil {
			return true, err
		}
		switch op {
		case opPing:
			if err := c.WriteFrame(opPong, payload); err != nil {
				return true, err
			}
		case opClose:
			code := uint16(0)
			if len(payload) >= 2 {
				code = uint16(payload[0])<<8 | uint16(payload[1])
			}
			if code == 4409 {
				// Another process is already attached under this name.
				// Reported as NOT attached so the backoff keeps growing:
				// resetting it here would put this process in a one-second
				// loop fighting the one that legitimately holds the name.
				return false, fmt.Errorf("this server id is already connected elsewhere")
			}
			return true, fmt.Errorf("node closed the control connection (code %d)", code)
		}
	}
}

// pipeKeeper maintains the idle pipes.
type pipeKeeper struct {
	mu      sync.Mutex
	running bool
	stopCh  chan struct{}
	wg      sync.WaitGroup
}

func (k *pipeKeeper) start(cfg *Config) {
	k.mu.Lock()
	defer k.mu.Unlock()
	if k.running {
		return
	}
	k.running = true
	k.stopCh = make(chan struct{})
	for i := 0; i < cfg.Pipes; i++ {
		k.wg.Add(1)
		go k.hold(cfg, k.stopCh)
	}
}

func (k *pipeKeeper) stop() {
	k.mu.Lock()
	if !k.running {
		k.mu.Unlock()
		return
	}
	k.running = false
	close(k.stopCh)
	k.mu.Unlock()
	k.wg.Wait()
}

// hold keeps one pipe slot filled: open a pipe, serve exactly one request, open
// another. One conversation per pipe is what makes backpressure and cancellation
// the operating system's problem rather than this program's.
func (k *pipeKeeper) hold(cfg *Config, stop <-chan struct{}) {
	defer k.wg.Done()
	backoff := time.Second
	for {
		select {
		case <-stop:
			return
		default:
		}
		if err := onePipe(cfg, stop); err != nil {
			if !errors.Is(err, io.EOF) && !stopped(stop) {
				logf("pipe ended: %v", err)
			}
			select {
			case <-stop:
				return
			case <-time.After(jitter(backoff)):
			}
			if backoff < 30*time.Second {
				backoff *= 2
			}
			continue
		}
		backoff = time.Second
	}
}

func onePipe(cfg *Config, stop <-chan struct{}) error {
	c, err := Dial(cfg.wsURL("/api/agent/pipe"), cfg.Credential, 20*time.Second)
	if err != nil {
		return err
	}
	defer c.Close()
	// An idle pipe has no read deadline at all -- that is the point of holding it
	// open -- so this is the only thing that ends it on shutdown.
	defer closeOnStop(c, stop)()

	var local net.Conn
	defer func() {
		if local != nil {
			local.Close()
		}
	}()

	for {
		select {
		case <-stop:
			return nil
		default:
		}
		// No deadline: an idle pipe waits indefinitely, which is the point of
		// holding it open. The node pings it every few minutes so the path stays
		// alive through any proxy in between.
		op, payload, _, err := c.ReadFrame()
		if err != nil {
			return err
		}
		switch op {
		case opPing:
			if err := c.WriteFrame(opPong, payload); err != nil {
				return err
			}
		case opPong:
			// nothing to do
		case opClose:
			return nil
		case opBin, opText, opCont:
			if local == nil {
				// Opened LAZILY, on the first byte: an idle pipe must not pin an
				// idle connection on the inference server.
				local, err = dialTarget(cfg.Target)
				if err != nil {
					return err
				}
				go pumpBack(c, local)
			}
			if _, err := local.Write(payload); err != nil {
				return err
			}
		}
	}
}

func dialTarget(target string) (net.Conn, error) {
	u, err := url.Parse(target)
	if err != nil {
		return nil, err
	}
	host := u.Host
	if u.Port() == "" {
		if u.Scheme == "https" {
			host = net.JoinHostPort(u.Hostname(), "443")
		} else {
			host = net.JoinHostPort(u.Hostname(), "80")
		}
	}
	return net.DialTimeout("tcp", host, 15*time.Second)
}

// pumpBack copies the target's answer into the pipe, in bounded chunks, flushing
// each one. That is what makes a completion appear token by token rather than all
// at once -- and why nothing here ever holds a whole response.
func pumpBack(c *Conn, local net.Conn) {
	buf := make([]byte, 65536)
	for {
		n, err := local.Read(buf)
		if n > 0 {
			if werr := c.WriteFrame(opBin, buf[:n]); werr != nil {
				return
			}
		}
		if err != nil {
			// The target finished or went away: closing the pipe is how that is
			// reported, exactly as closing a socket would be.
			c.Close()
			return
		}
	}
}

// closeOnStop closes the connection when `stop` fires, so a blocked read returns.
// The watcher retires with the caller, which is why it takes a done channel
// rather than leaking one goroutine per reconnection.
func closeOnStop(c *Conn, stop <-chan struct{}) func() {
	done := make(chan struct{})
	go func() {
		select {
		case <-stop:
			c.Close()
		case <-done:
		}
	}()
	return func() { close(done) }
}

func stopped(stop <-chan struct{}) bool {
	select {
	case <-stop:
		return true
	default:
		return false
	}
}

func jitter(d time.Duration) time.Duration {
	// Up to +/-25%, so a fleet reconnecting after a node restart spreads out
	// instead of arriving together.
	delta := time.Duration(rand.Int63n(int64(d/2) + 1)) - d/4
	return d + delta
}

func logf(format string, args ...any) {
	fmt.Fprintf(os.Stderr, time.Now().Format(time.RFC3339)+" "+format+"\n", args...)
}
