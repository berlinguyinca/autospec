## convert: declaration-only conflicts resolve by canonical union, whatever the file is called (#4463)

A conversion batch of 13 Rust patches against clean `origin/main` produced
three applies and ten conflicts, and **six of the ten conflicted in the same
three files** (`lib.rs`, `insights/mod.rs`, `coordination/mod.rs`) — none of
the six patches overlapping in the code they added. Each added its own module
in its own file; they collided because every `pub mod` declaration goes into
the same list and every agent appends to the same region of the same file.

The conflict classifier decided by *file name* — `lib.rs`/`mod.rs` certified
keep-both, everything else refused — so a module list in a file named
something else was held even though its conflict was confined to declaration
lines and needed no judgement. The resolution also took "both sides,
first-seen," leaving the index in merge order so the next patch still landed
at the same offset, and a held patch did not say whether its conflict was
declaration-only, so the size of the class could only be sampled, never
counted.

- New `autospec_core::declaration_conflict` module decides by **content**, not
  file name: `detect` answers whether every line in every conflict hunk is a
  `mod` declaration (with its `#[cfg]` attributes); a hunk with one line of
  real code is `Mixed` and is not the module's business.
- `sorted_union` takes the union of both sides in canonical (alphabetical)
  order, attributes attached to their declaration. Canonical position means
  two independent additions land at different offsets, so git resolves them
  with no help at all — the contention stops instead of being resolved ever
  more cleverly. The union is idempotent, so order stops depending on merge
  order.
- `orphaned` is the check that makes the union safe: a two-sided conflict
  cannot distinguish an addition from the other side's deletion, and a union
  resurrects the deleted declaration. The filesystem settles it —
  `pub mod foo;` needs `foo.rs` or `foo/mod.rs` in the merged tree, and a
  union that names no file is refused rather than staged as a build failure.
- The conversion pass now resolves a declaration-only conflict by canonical
  union **whatever the file is called**, refuses an orphaned union, stops
  counting a resolvable declaration conflict as a structural refusal
  (#4637's deadlock), and records `declaration-only` on a held patch so the
  size of the class is measured rather than sampled from an incident.

In-repo, Rust-first, no shell. The module list still has to be generated
(option 1) to remove the contention at the root; this PR removes the cost of
it in the meantime and makes its remaining size visible.
