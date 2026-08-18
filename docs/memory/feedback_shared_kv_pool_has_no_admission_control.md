---
name: llama.cpp shared KV pool over-commits and kills every live session
description: kv-unified lets sessions differ in size but has no admission control; over-subscribing the pool fails all in-flight requests, so rationing must happen client-side
type: feedback
wing: synthesis
drawer_class: lesson
---
`llama-server` has three settings that look related and are not:

- `--ctx-size` is the **total** KV pool, not a per-session limit.
  `-c 196608 --parallel 4` logs `n_slots = 4, n_ctx_slot = 49152`.
- `--parallel` costs VRAM in **compute buffers**, not KV. So seats are
  bought with context: on a 24 GiB card, 4 slots fit a 196,608 pool,
  6 fit 180,224, 8 fit 163,840, 12 fit 131,072. Asking for 8 slots at
  196,608 dies on `cudaMalloc 872.28 MiB: out of memory`.
- `--kv-unified` decides whether slots get equal fixed shares
  (`false`, each capped at `c / parallel`) or all draw on one pool
  (`true`). Only the shared pool allows sessions of different sizes.

**The trap:** the shared pool has **no admission control**. Three 80k
sessions against a 196k pool were all accepted, prefilled for 58
seconds, and then died *together*:

```
decode: failed to find free space in the KV cache ... n_batch = 1
srv  decode: Context size has been exceeded.
srv  send_error: task id = 97 / 98 / 99
```

A session that stayed inside its budget is killed by a neighbour that
did not. There is no queueing, eviction, or back-pressure.

**Why:** it means the server cannot be made safe by configuration
alone. The pool has to be rationed by the client, which is the only
component that knows how much context a session intends to use.

**How to apply:** publish one loaded model under several ids that differ
only in declared context (`alias = id-160k,id-80k,id-40k`), and let the
client's own compaction hold each session inside its share. Aliases,
not separate presets — separate presets are separate processes each
holding a full copy of the weights. Enforce the invariants in CI
(`tests/check_presets.py`): a tier must not exceed its pool, must not be
offered without `kv-unified`, and must not advertise more concurrent
sessions than there are slots.

Related: [[feedback_self_consistent_test_fixtures_mask_bugs]] — the
first "4 × 40k" run that appeared to disprove this configuration was
sending 4 × 51,800, because the benchmark assumed 17 tokens per filler
line and the real figure was 21. Size prompts with the server's own
`/tokenize`, and force a fixed decode length with `ignore_eos` so a
short reply cannot report a meaningless aggregate rate.
