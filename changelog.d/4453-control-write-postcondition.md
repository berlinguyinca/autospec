### Added

- `autospec-core::control_file` — primitives for the invariant that a write to a
  control file is not a control action until verified against the reader's
  predicate (#4451's eight holds were appended as
  `3550 repeatedly-dispatched-no-patch 2026-09-11T23:03:45-07:00` to a list the
  dispatcher matches with `grep -qx "$n"`, so none of them ever took effect):
  a `ControlRecord` whose rendering is exactly the line the consumer's
  whole-line predicate matches (`HoldRecord` renders the bare issue number and
  refuses non-canonical forms like `03550`), a strict `ControlFile` whose
  `parse` rejects any off-format line — a free-text hold list cannot be loaded
  as one, so no free-text append path remains — a `write_verified` publish that
  re-reads the file from disk and fails loudly
  (`InvisibleAfterWrite`, naming every invisible record) when a just-written
  record is not visible to the consumer's predicate, and a `HoldSidecar` keyed
  by issue that keeps the reason and timestamp out of the matched record,
  verified the same way (`hold` / `release` are the verified procedure)
  (#4453, 2026-09-12).
