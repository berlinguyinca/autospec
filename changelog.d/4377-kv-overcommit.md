### Added

- `autospec-core::kv_overcommit` — over-commitment of a shared KV pool, for
  the invariant that a per-slot ceiling above pool/slots is a deadlock and
  there is no safe middle setting between a fair share and a single slot:
  with N slots each entitled to the whole pool, concurrent large requests
  wedged `llama-server` (000 x 6, GPU 0%, 50 GB held) while `/health` kept
  answering 200. `KvConfig::classify` calls a configuration `FairShare`,
  `SingleSlot`, or `OverCommitted` by its total entitlement against the
  pool; `admit` models concurrent admission and returns the wedge
  (`Deadlocked { stuck, queued, held_mib }`) instead of a silent hang, a
  clean rejection for what does not fit its own ceiling, or a queue for the
  single-slot shape where waiting is a wait, not a hang; `serial_check` is
  the one-request smoke test every over-committed configuration passes, so
  `latent` names the deadlocks that are indistinguishable from healthy
  configurations until load arrives; the fix is `fundable_slots` /
  `KvConfig::remediated` — `parallel = 1` whenever the ceiling is the whole
  pool, with safe configurations untouched; and `fleet_audit` /
  `same_change` enforce that when one worker fails, every worker that took
  the same change is checked, not just the one that failed first — the
  fleet looked 5/6 healthy while every worker carried the same fault
  (#4377, 2026-09-11).
