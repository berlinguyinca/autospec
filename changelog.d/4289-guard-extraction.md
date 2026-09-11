### Added

- `autospec-core` `guard_extraction` module: a guard is extracted on its
  second implementation, not its fourth — the copy ledger names the deferral
  from copy 2 and its counted cost (deferral = (copies − 2) × copy_cost,
  multiplying the copy cost, never the extraction cost), sibling files are
  named by a shared name stem (longest common substring ≥ 4 on the
  extension-stripped names), a guard comment citing an issue number in one
  sibling is a coverage gap in the others, and a one-file fix while sibling
  candidates sit in the tree is a smell (`#4289, 2026-09-11`).
