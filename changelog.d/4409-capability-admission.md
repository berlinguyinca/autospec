### Added

- `autospec-core::capability_admission` — admit workers on measured
  throughput, not liveness, for the invariant that health must be the
  capability a pool exists to provide, not the reachability of its members:
  `qwen3.8-flash-next` ran 100x slow (6 tok/s prefill where `qwen3.8-27b`
  managed 918 on the same card, image and node class) with every liveness
  check green — process up, `/health` 200, `/props` answered, the admission
  probe's one-token completion succeeded, registration succeeded, token
  counters advancing — while both GPUs sat at 0% / 74 W with the weights
  resident in VRAM (llama.cpp executing on CPU). `admit` rejects a worker
  whose measured prefill or decode rate falls below the per-(model, card)
  floor, and fails closed on the two states liveness alone cannot judge: no
  recorded floor (`NoFloor`) and no measurement taken (`Unmeasured`);
  `gpu_during_generation` makes near-zero GPU utilisation during generation
  a first-class health signal — one call away, unambiguous, invisible to
  every liveness check; `BaselineTable` records the per-(model, card)
  performance baseline and `deviation` alerts on the measured rate being
  N-times slower than that baseline rather than on an absolute threshold —
  the same check that catches a regression from an image or model upgrade;
  and `diverges` names the general form: liveness and capability diverge
  exactly when something interesting is wrong, which is the only time it
  matters (#4409, 2026-09-16).
