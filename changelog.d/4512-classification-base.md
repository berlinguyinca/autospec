# A classification is a fact about a base, and expires when the base moves (#4512)

The backlog's classification (fresh / interrupted / delivered / held) was
computed against a base the report never named, and the pass acted on it even
when trunk had moved: a patch measured `fresh` at the start of a pass can be
already-delivered by the time the pass acts on it, if a merge landed in
between.

- The selection line now ends with the base the classification was derived
  against (`@ origin/<base>#<sha8>`), and the `--json` plan carries the full
  sha as `classified_base` — the counts are a measurement, not a standing
  fact.
- `convert/stale.rs` (new): the classification is `(base_sha, entries)`, not
  `entries`. `--apply` revalidates against the current base immediately
  before mutating; a moved base re-runs the liveness and delivered checks and
  reports each changed candidate on a `STALE #N: <from> -> <to>` line. The
  move decision fails closed: an unresolvable base keeps the recorded states
  (with a warning) instead of authorising action on an uncomputed
  reclassification.
- The candidate classification loop extracted from `build_plan` into
  `stale::classify` (shared by the plan and the revalidation); the HELD
  ledger rides on `ConvertPlan` so the re-gate can be re-run.
- The `base_sha` a HELD record now cites is the tip of `origin/<base>` the
  patch is gated against, not the checkout's `HEAD`.
- `convert.rs` shrank (2336 -> 2319 lines) — the extraction paid for the
  new reporting.
