package main

// qwen-turing-agent: offer this machine's inference capacity to a node.
//
//	qwen-turing-agent enrol --node <host> --token qte_... [--target URL]
//	qwen-turing-agent run   [--config PATH]
//	qwen-turing-agent install
//
// One file, no dependencies, and outbound connections only: this machine needs
// no inbound port and no firewall rule for the node to use it.

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "enrol", "enroll":
		err = cmdEnrol(os.Args[2:])
	case "run":
		err = cmdRun(os.Args[2:])
	case "install":
		err = cmdInstall(os.Args[2:])
	case "version", "--version", "-v":
		fmt.Println("qwen-turing-agent " + version)
	default:
		usage()
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error: "+err.Error())
		os.Exit(1)
	}
}

func usage() {
	fmt.Fprint(os.Stderr, `qwen-turing-agent -- offer this machine's inference capacity to a node

  enrol    --node HOST --token qte_... [--target URL] [--gpus TEXT] [--slots N]
  run      [--config PATH]
  install  [--config PATH]        write the platform's supervision file
  version

The target is an OpenAI-compatible base (scheme://host:port, no path) on THIS
machine. It comes from local configuration and is never accepted from the node.
`)
}

type enrolReply struct {
	ServerID   string `json:"server_id"`
	Credential string `json:"credential"`
	Pipes      int    `json:"pipes_wanted"`
}

func cmdEnrol(args []string) error {
	fs := flag.NewFlagSet("enrol", flag.ExitOnError)
	node := fs.String("node", "", "the node's hostname (or host:port)")
	token := fs.String("token", "", "the one-time enrolment token (qte_...)")
	target := fs.String("target", "http://127.0.0.1:8080",
		"OpenAI-compatible base on this machine; scheme://host:port, no path")
	gpus := fs.String("gpus", "", "what cards this box has, for the panel")
	slots := fs.Int("slots", 1, "how many concurrent requests it serves")
	pipes := fs.Int("pipes", 0, "idle connections to keep open (default: from the node)")
	insecure := fs.Bool("insecure", false, "use ws:// and http:// (trusted networks only)")
	confPath := fs.String("config", "", "where to write the configuration")
	_ = fs.Parse(args)

	if *node == "" || *token == "" {
		return fmt.Errorf("--node and --token are required")
	}
	cfg := &Config{Node: *node, Target: *target, GPUs: *gpus, Slots: *slots,
		Pipes: *pipes, Insecure: *insecure}
	if strings.Count(strings.TrimSuffix(cfg.Target, "/"), "/") > 2 {
		return fmt.Errorf("--target must be scheme://host:port with no path, got %q",
			cfg.Target)
	}

	body, _ := json.Marshal(map[string]string{"token": *token})
	url := cfg.httpsURL("/api/agent/enrol")
	req, err := http.NewRequest("POST", url, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("%s: %w", url, err)
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(io.LimitReader(resp.Body, 8192))
	if resp.StatusCode != http.StatusCreated {
		return fmt.Errorf("the node refused this token (%s): %s", resp.Status,
			strings.TrimSpace(string(raw)))
	}
	var out enrolReply
	if err := json.Unmarshal(raw, &out); err != nil {
		return err
	}
	cfg.ServerID = out.ServerID
	cfg.Credential = out.Credential
	if cfg.Pipes == 0 {
		cfg.Pipes = out.Pipes
	}

	path := *confPath
	if path == "" {
		path = defaultConfigPath()
	}
	if err := saveConfig(path, cfg); err != nil {
		return err
	}
	// The credential is never printed. It is in the file, with the file's
	// permissions, and echoing it here would put it in a shell history.
	fmt.Printf("attached as %q\nconfiguration: %s\nnow run: qwen-turing-agent run\n",
		cfg.ServerID, path)
	return nil
}

func cmdRun(args []string) error {
	fs := flag.NewFlagSet("run", flag.ExitOnError)
	confPath := fs.String("config", "", "path to agent.json")
	_ = fs.Parse(args)
	path := *confPath
	if path == "" {
		path = defaultConfigPath()
	}
	cfg, err := loadConfig(path)
	if err != nil {
		return err
	}

	stop := make(chan struct{})
	sig := make(chan os.Signal, 1)
	signal.Notify(sig, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-sig
		logf("stopping")
		close(stop)
	}()

	logf("offering %s to %s as %q", cfg.Target, cfg.Node, cfg.ServerID)
	keeper := &pipeKeeper{}
	runControl(cfg, stop, keeper)
	return nil
}
