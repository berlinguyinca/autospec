package main

// Supervision as data, not as a dependency.
//
// A native Windows service would need golang.org/x/sys, which would end the
// zero-dependency property for a problem three small files solve. A scheduled
// task restarts a crashed process just as well.

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

const systemdUnit = `[Unit]
Description=Qwen node agent (offers this machine's inference capacity)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={{BIN}} run --config {{CONF}}
Restart=always
# Jittered, so a fleet restarting after a node outage does not arrive together.
RestartSec=5
# It makes outbound connections and reads one config file. Nothing else.
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths={{CONFDIR}}

[Install]
WantedBy=multi-user.target
`

const launchdPlist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>us.metabolomics.qwen-turing-agent</string>
  <key>ProgramArguments</key>
  <array>
    <string>{{BIN}}</string><string>run</string>
    <string>--config</string><string>{{CONF}}</string>
  </array>
  <key>KeepAlive</key><true/>
  <key>RunAtLoad</key><true/>
  <key>StandardErrorPath</key><string>/tmp/qwen-turing-agent.log</string>
</dict>
</plist>
`

const taskXML = `<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers>
  <Settings>
    <RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
  </Settings>
  <Actions>
    <Exec>
      <Command>{{BIN}}</Command>
      <Arguments>run --config {{CONF}}</Arguments>
    </Exec>
  </Actions>
</Task>
`

func cmdInstall(args []string) error {
	fs := flag.NewFlagSet("install", flag.ExitOnError)
	confPath := fs.String("config", "", "path to agent.json")
	outPath := fs.String("out", "", "where to write the supervision file")
	_ = fs.Parse(args)

	conf := *confPath
	if conf == "" {
		conf = defaultConfigPath()
	}
	bin, err := os.Executable()
	if err != nil {
		return err
	}
	fill := func(t string) string {
		t = strings.ReplaceAll(t, "{{BIN}}", bin)
		t = strings.ReplaceAll(t, "{{CONF}}", conf)
		return strings.ReplaceAll(t, "{{CONFDIR}}", filepath.Dir(conf))
	}

	var body, dest, after string
	switch runtime.GOOS {
	case "linux":
		body, dest = fill(systemdUnit), "/etc/systemd/system/qwen-turing-agent.service"
		after = "sudo systemctl daemon-reload && " +
			"sudo systemctl enable --now qwen-turing-agent"
	case "darwin":
		body = fill(launchdPlist)
		dest = "/Library/LaunchDaemons/us.metabolomics.qwen-turing-agent.plist"
		after = "sudo launchctl load -w " + dest
	case "windows":
		body, dest = fill(taskXML), "qwen-turing-agent-task.xml"
		after = `schtasks /create /tn "qwen-turing-agent" /xml ` + dest + " /ru SYSTEM"
	default:
		return fmt.Errorf("no supervision template for %s; run it under whatever "+
			"this system uses", runtime.GOOS)
	}
	if *outPath != "" {
		dest = *outPath
	}
	if err := os.WriteFile(dest, []byte(body), 0o644); err != nil {
		// Writing to a system directory needs privilege. Say what to do rather
		// than only what failed.
		return fmt.Errorf("%w\n\nwrite it elsewhere with --out and move it, or "+
			"re-run with privilege", err)
	}
	fmt.Printf("wrote %s\nnext: %s\n", dest, after)
	if runtime.GOOS == "windows" {
		// Said plainly rather than implied: this credential is protected by file
		// permissions, and on Windows that is weaker than the 0600 it gets
		// elsewhere. DPAPI would be a dependency.
		fmt.Printf("\nthe credential in %s is protected only by its ACL.\n"+
			"tighten it with:\n  icacls \"%s\" /inheritance:r "+
			"/grant:r \"%%USERNAME%%:R\" /grant:r \"SYSTEM:R\"\n", conf, conf)
	}
	return nil
}
