# Local LLM inference nodes

Everything needed to rebuild these deployments from nothing, and the measurements
that justify each choice. Start with whichever matches your situation.

| you have | read | run |
|---|---|---|
| a single GPU workstation | [`linux-qwen38/README.md`](linux-qwen38/README.md) | `linux-qwen38/scripts/install-node.sh --with-opencode` |
| time on an HPC cluster | [`linux-qwen38/slurm/README.md`](linux-qwen38/slurm/README.md) | `linux-qwen38/slurm/opencode_slurm` |
| different hardware entirely | [`QWEN-NODE-SPEC.md`](QWEN-NODE-SPEC.md) | `linux-qwen38/scripts/select-quant.py --platform ...` |

[`QWEN-NODE-SPEC.md`](QWEN-NODE-SPEC.md) is the portable spec: the method, not
the answer. It covers auditing hardware, reading a model's architecture before
sizing anything, choosing quantisation from memory rather than from the name,
predicting the speed ceiling, measuring the real ceiling (three different
numbers look like it and only one is), serving concurrent sessions, and wiring
clients. Appendix B has playbooks per platform — consumer NVIDIA, Blackwell,
Apple Silicon, Intel, AMD, CPU, datacentre A100/H100, and shared Slurm clusters.

## The reproducible artefacts

```
QWEN-NODE-SPEC.md                    the portable method
linux-qwen38/
  README.md                          the RTX 4090 build, as measured
  config/router-presets.ini          what the local node serves
  config/profiles.d/*.conf           one file per verified configuration
  systemd/                           boot service
  scripts/install-node.sh            provisions the whole stack, then VERIFIES it
  scripts/*                          the operator toolkit (see below)
  tests/                             structural checks, smoke, vision, presets
  slurm/                             the Slurm cluster deployment
```

## The toolkit, and what each tool answers

| tool | question it answers |
|---|---|
| `select-quant.py` | which quantisation fits *this* hardware, at what context and speed |
| `measure-ceiling.sh` | what context this node can actually serve — verified at length |
| `measure-slot-frontier.sh` | how much context a concurrent seat costs |
| `bench-concurrency.py` | what happens when several clients arrive at once |
| `bench-context-sweep.sh` | how speed degrades with prompt depth |
| `analyze-session-contexts.py` | how much context your real work needs |
| `add-gguf-model.sh` | add any GGUF to the router, verified end to end |
| `configure-opencode.py` | derive the client config *from the server*, never by hand |
| `tests/check_presets.py` | catch a preset that over-commits its pool before production does |
| `gpu-registry.py` | what every GPU we have run on can do, and what it did |

Every one of them is installed by `install-node.sh`; a structural test fails the
build if a script is added and not shipped.

## Ground rules these deployments follow

- **"Allocates", "starts", and "works at length" are three different claims.**
  Three numbers looked like the context ceiling on the reference build and only
  the smallest survived a prompt that actually filled it.
- **Measure the client you will use.** The context present before any work —
  system prompt, memory, tool schemas — was 14,492 tokens median on OpenCode and
  39,655 on Claude Code. It decides which tiers are even startable.
- **A shared KV pool has no admission control.** Over-subscribe it and every
  live session dies, not just the greedy one. Rationing happens client-side.
- **Predictions are labelled as predictions.** Anything not measured on the
  hardware in question says so. `config/gpu-registry.json` keeps observed
  facts (VRAM, compute capability), measured facts (tok/s per quantisation) and
  the one assumption (vendor bandwidth) in separate fields, and every serving
  job records the card it landed on — so a GPU nobody has seen before enters
  the collection the first time it is used rather than being reconstructed from
  memory afterwards.
