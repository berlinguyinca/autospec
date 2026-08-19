# Patches

**There are none, and that is the intended state.**

The spec anticipated patching upstream vLLM to serve Qwen3.8-27B. That turned
out to be unnecessary: released vLLM **0.27.1** already registers the hybrid
architecture and its MTP draft head.

Verified against `vllm/model_executor/models/registry.py` at three release tags:

| tag | `Qwen3_5ForConditionalGeneration` | `Qwen3_5MTP` |
|---|---|---|
| `v0.26.0` | present | present |
| `v0.27.0` | present | present |
| `v0.27.1` | present | present |

`setup-linux-qwen38.sh` asserts registration at install time rather than
trusting this table, so a future version bump that silently drops the
architecture fails the install instead of failing at first inference.

## Workarounds that are not patches

Two upstream problems are handled by configuration rather than by editing
source. They are recorded here because they carry the same obligation a patch
does — a reason and a removal condition.

### 1. FlashInfer sampler disabled (`VLLM_USE_FLASHINFER_SAMPLER=0`)

FlashInfer JIT-compiles its sampling kernels on first use, and that compile
fails on this host:

```
flashinfer/data/include/flashinfer/sampling.cuh(623): error:
class "cub::_V_300302_SM_890::BlockAdjacentDifference<__nv_bool,512,1,1>"
has no member "FlagHeads"
```

`BlockAdjacentDifference::FlagHeads` was removed from CUB in CUDA 13, and
torch 2.13 is `+cu130`. The failure is nasty to diagnose because it surfaces as
`Engine core initialization failed` minutes into startup rather than as a build
error. Set in `config/common.conf`; correctness is unaffected, sampling
throughput takes the cost.

**Remove when** FlashInfer ships a CUDA 13-compatible `sampling.cuh`, or vLLM
pins one that is. Re-test by flipping the value to `1` and issuing one request.

### 2. The venv's `bin` is forced onto `PATH`

Running `/opt/qwen-vllm/venv/bin/vllm` by absolute path does **not** put the
venv's `bin` on `PATH`. FlashInfer's JIT shells out to `ninja` **by name**, so
without the export the engine dies with `FileNotFoundError: 'ninja'` — while
`ninja` sits in the same directory as the `vllm` binary being executed.

**Remove when** nothing in the stack shells out to a venv-installed tool by
bare name. Both `serve-profile.sh` and `measure-ceiling.sh` set it, and
`tests/test_structural.sh` asserts both still do.

## If a patch ever becomes necessary

Add it here as `NNNN-short-description.patch` and give it a header block:

```
# why:        one sentence on the failure it fixes
# upstream:   issue or PR URL, or "none filed" and why
# applies-to: exact vllm version(s)
# remove-when: the condition that makes this patch redundant
```

Then teach `setup-linux-qwen38.sh` to apply it and to **fail loudly if it no
longer applies cleanly** — a patch that silently stops applying after an upgrade
is worse than no patch, because the install still reports success.
