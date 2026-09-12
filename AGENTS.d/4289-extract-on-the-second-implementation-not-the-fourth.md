# Extract on the second implementation, not the fourth (issue #4289)

The same guard was written four times across sibling files — a timeout
wrapper in two conversion passes, a "the child owns the write" note in
two runners — and each copy was discovered in a separate debugging
session. The copies were siblings in every way that matters: names that
share a domain stem (`convpass`/`iwconv`), guard comments citing issue
numbers that appear in only one sibling, and shared strings that say the
same fact twice. Extraction happened only when the fourth copy forced
the argument; the rule is that the *second* implementation of a guard is
the extraction point, and deferral past it has a cost that is counted,
not assumed.

- **The second implementation is the extraction deadline.** A guard with
  two or more implementations is due for extraction
  (`extraction_due`, `SECOND_COPY`); the ledger names the deferral and
  its cost from the second copy on
  (`CopyLedger::line` → "extraction was due at copy 2 and N copy(s)
  were deferred at C each"). The deferral cost is `(copies − 2) ×
  copy_cost` — it multiplies the copy cost, never the extraction cost
  (`CostModel::deferral_cost`).
- **Sibling files are named, not assumed.** Two files are sibling
  candidates when their extension-stripped names share a common
  substring of at least 4 characters (`name_candidates`,
  `longest_common_substring_len`, `NAME_STEM_MIN_LEN`).
- **A guard comment citing an issue number in one sibling is a
  coverage gap in the others.** Citations are read from comment lines
  (`guard_citations`); an issue cited in at least one but not all files
  of a set is a gap naming every file that misses it (`GuardGap`,
  `guard_coverage`).
- **Grep the tree before writing a one-file fix.** A change that touches
  exactly one file while the tree contains sibling candidates is
  `SingleFile { siblings }` and renders as a `WARN:` naming the
  siblings that need the same fix (`fix_scope`); a change touching two
  or more files is `Broad` and passes.

Checkable in `autospec_core::guard_extraction` (`extraction_due`,
`CostModel`, `CopyLedger`, `name_candidates`, `longest_common_substring_len`,
`distinctive_tokens`, `sibling_pair`, `guard_citations`, `guard_coverage`,
`fix_scope`, `audit`). Tests: `crates/autospec-core/tests/guard_extraction.rs`.
