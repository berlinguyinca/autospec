### Added

- `autospec-core::env_preconditions` — environmental-precondition primitives for fixes
  that rest on properties of the fleet rather than the code: a per-model context-window
  homogeneity audit that warns when workers for one model report differing windows, a
  load-aware worker picker that filters on eligibility against the request requirement
  before ranking on free slots (total over answering workers, holding a zero-free-slot
  worker as a separate admission decision), and `Valid while:` precondition lines a
  closeout records for the conditions under which the fix holds — a precondition with no
  named assertion is rejected at construction (#4224, 2026-09-10).
