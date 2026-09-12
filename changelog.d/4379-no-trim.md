### Added

- `autospec-core::declaration_ownership` — the capacity reconciler's two
  halves can now run separately: `CapacityReconciler::with_no_trim`
  (mirrored by the fleet reconciler's `--no-trim` flag) submits the
  deficit and never cancels a worker, while the default mode keeps
  trimming surplus newest-first with a `RevertNotice`. The split exists
  because the reconciler was the only component that could submit a
  missing worker and was pinned to dry-run: its other half trimmed
  workers an operator held warm on purpose, so the wedged-worker
  rotation could remove capacity and nothing could replace it — the
  fleet sat at 5 against a desired 12 while autoscale, keyed on
  percentage-busy, logged a lower utilisation and never fired, because
  fewer workers serving the same load report lower busy. The declaration
  review is identical in both modes; `no_trim()` reports which half is
  split off (#4379, 2026-09-11).
