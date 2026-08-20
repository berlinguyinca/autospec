package main

// Configuration and the credential file.
//
// The target address lives HERE, on the machine being offered, and never arrives
// from the node. That is deliberate and load-bearing: if the node could name a
// destination, whoever controlled the node would have a port scanner inside the
// private network of every attached machine. Nothing in the protocol carries one.

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

type Config struct {
	// The node, as a host or host:port. Scheme is added by the agent: wss unless
	// --insecure was passed.
	Node string `json:"node"`
	// The OpenAI-compatible server this agent fronts. A scheme, host and port --
	// no path. The pipe carries bytes verbatim, so the request line the node
	// sends must already be what this target expects; a base path here could not
	// be applied without rewriting the stream.
	Target string `json:"target"`
	// Credential from enrolment. Never printed after it is written.
	Credential string `json:"credential"`
	ServerID   string `json:"server_id"`
	// How many idle pipes to keep open, so a request never pays for a TLS
	// handshake it could have prepaid.
	Pipes int `json:"pipes"`
	// What to tell the node about this box, for the panel.
	GPUs  string `json:"gpus"`
	Slots int    `json:"slots"`
	// Plain ws:// instead of wss://. For a node on a trusted local network only;
	// the node itself refuses a credential that did not arrive over TLS.
	Insecure bool `json:"insecure"`
}

func defaultConfigPath() string {
	switch runtime.GOOS {
	case "windows":
		if dir := os.Getenv("LOCALAPPDATA"); dir != "" {
			return filepath.Join(dir, "qwen-turing-agent", "agent.json")
		}
	case "darwin":
		if home, err := os.UserHomeDir(); err == nil {
			return filepath.Join(home, "Library", "Application Support",
				"qwen-turing-agent", "agent.json")
		}
	}
	if os.Geteuid() == 0 {
		return "/etc/qwen-turing-agent/agent.json"
	}
	if home, err := os.UserHomeDir(); err == nil {
		return filepath.Join(home, ".config", "qwen-turing-agent", "agent.json")
	}
	return "agent.json"
}

func loadConfig(path string) (*Config, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	cfg := &Config{Pipes: 4, Slots: 1}
	if err := json.Unmarshal(raw, cfg); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if cfg.Node == "" || cfg.Credential == "" {
		return nil, fmt.Errorf("%s: node and credential are required", path)
	}
	if cfg.Target == "" {
		cfg.Target = "http://127.0.0.1:8080"
	}
	if strings.Count(strings.TrimSuffix(cfg.Target, "/"), "/") > 2 {
		// A base path cannot be honoured: the pipe carries the node's request
		// line verbatim. Refused loudly rather than silently ignored, because the
		// symptom would be 404s from the target and nothing pointing here.
		return nil, fmt.Errorf("target must be scheme://host:port with no path, got %q",
			cfg.Target)
	}
	if cfg.Pipes < 1 {
		cfg.Pipes = 1
	}
	return cfg, nil
}

func saveConfig(path string, cfg *Config) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	raw, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	// 0600 on Unix. On Windows this is advisory -- the file inherits the
	// directory's ACL -- which `install` reports honestly along with the icacls
	// line that tightens it. DPAPI would be a dependency, and dependencies are
	// the thing this agent does not have.
	return os.WriteFile(path, append(raw, '\n'), 0o600)
}

func (c *Config) wsURL(path string) string {
	scheme := "wss"
	if c.Insecure {
		scheme = "ws"
	}
	return scheme + "://" + c.Node + path
}

func (c *Config) httpsURL(path string) string {
	scheme := "https"
	if c.Insecure {
		scheme = "http"
	}
	return scheme + "://" + c.Node + path
}
