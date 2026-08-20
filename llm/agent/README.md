# qwen-turing-agent

One native executable that offers a machine's inference capacity to a node.

It makes **outbound connections only**. The machine it runs on needs no inbound
port, no firewall rule and no routable address — which is the entire point: the
alternative is an unauthenticated inference port on the network with a firewall
rule as its only protection.

```
qwen-turing-agent enrol --node <node-host> --token qte_… [--target URL]
qwen-turing-agent run
qwen-turing-agent install
```

## What it does

* Holds one **control connection** open. Its existence is what tells the node this
  server is available; losing it marks the server offline within about half a
  minute, and it comes back by itself with jittered backoff.
* Keeps a few **idle pipes** open. Each pipe carries exactly one HTTP
  conversation, so the node reaches this machine's inference server by taking a
  pipe and speaking HTTP over it. Holding them open in advance is what keeps the
  TLS handshake off the request path.
* Forwards bytes. It does not read them, does not log them, and does not parse
  them.

## The target comes from here, never from the node

`--target` is an OpenAI-compatible base on this machine — `scheme://host:port`,
**no path**. It is stored in this machine's own config file and no message in
either protocol carries a destination.

That is deliberate and load-bearing. If the node could name a destination, then
whoever controlled the node would have a port scanner inside the private network
of every attached machine. A base *path* is refused rather than ignored, because a
pipe carries the node's request line verbatim and a path could not be applied
without rewriting the stream — the symptom would be 404s from the target with
nothing pointing back here.

Any OpenAI-compatible server works: llama.cpp, vLLM, Ollama, LM Studio, TGI, MLX.
llama.cpp is the one that is tested. The node asks the target itself what models
it serves, so nothing depends on a naming convention.

**The target must accept unauthenticated connections from this machine.** A pipe
carries bytes verbatim, so the agent cannot add an `Authorization` header without
parsing and rewriting the request head — which is the one thing it refuses to do.
That is not a gap to work around: bind the target to loopback instead of putting a
key on it. Nothing else can reach it, which is stronger than a key anyway, and it
is why `vllm --api-key …` should simply be left off.

## Supervision

`install` writes the right file for the platform it is running on and prints the
one command that activates it:

| platform | file |
|---|---|
| Linux | a systemd unit, `Restart=always` |
| macOS | a launchd plist, `KeepAlive` |
| Windows | a Task Scheduler XML, at boot |

Deliberately **not** a native Windows service: that needs a third-party module,
and a scheduled task restarts a crashed process just as well. Nothing here has
dependencies — `build.sh` fails the build if that changes.

## The credential

Enrolment trades a one-time token for a durable credential, written to the config
file and never printed again. It grants exactly one thing: the ability to offer
this server's capacity to that node. It cannot run inference, and it can be
revoked from the dashboard by its owner or an administrator.

| platform | location | protection |
|---|---|---|
| Linux | `/etc/qwen-turing-agent/agent.json` or `~/.config/…` | mode `0600` |
| macOS | `~/Library/Application Support/qwen-turing-agent/` | mode `0600` |
| Windows | `%LOCALAPPDATA%\qwen-turing-agent\` | the directory's ACL |

**On Windows the credential is weaker at rest**, said plainly rather than implied:
there is no DPAPI here, because that would be a dependency. `install` prints the
`icacls` command that tightens the file.

## Building

```
./build.sh          # five targets from one host, ~6 MB each
```

`CGO_ENABLED=0`, so no C toolchain and no per-platform build machine. The script
refuses to build if `go list -m all` reports any dependency beyond this module —
the zero-dependency property is asserted, not hoped for.
