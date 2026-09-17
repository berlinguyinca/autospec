# A file that every change must touch, but no change is about, is a coordination point

Issue #4463: a conversion batch of 13 Rust patches against clean
`origin/main` produced three applies and ten conflicts. Six of the ten
conflicted in the same place — `crates/autospec-core/src/lib.rs`,
`insights/mod.rs`, `coordination/mod.rs` — and none of those six patches
overlapped in the code they added. Each added its own module in its own file;
they collided because Rust requires the module to be declared, every
declaration goes into the same list, and every agent appends to the same
region of the same file.

The module list is a shared mutable index that nothing owns. It is not where
the work is, but it is where all the work has to arrive. The contention scales
with the number of parallel agents, and it worsens with age: the longer a patch
sits unconverted, the more declarations have landed ahead of it, so the oldest
patches are the most likely to conflict and the backlog becomes self-reinforcing.

## The defect

The conflict classifier decided by *file name*: `lib.rs`/`mod.rs` were
certified keep-both, everything else was refused. So a module list in a file
that happened to be named something else — a hand-held index, a command
registry, a variant enumeration — was held even though its conflict was
confined to declaration lines and needed no judgement. And the resolution
took "both sides, first-seen": it kept the index in the order whoever merged
last appended it, so the next patch still landed at the end of the list and
still collided. The class was also invisible: a held patch did not record
whether its conflict was declaration-only, so its size could only be sampled
from an incident, never counted.

## The fix

`declaration_conflict` decides by *content*, not by file name:

- `detect` reads the conflict hunks and answers whether every line inside
  every hunk is a `mod` declaration (with its `#[cfg]` attributes). A hunk
  containing one line of real code is `Mixed`, and this module is not its
  business — merging that would be exactly the judgement the invariant forbids
  automating.
- `sorted_union` takes the union of both sides in canonical (alphabetical)
  order, each attribute kept attached to the declaration it precedes. Canonical
  position means two independent additions land at different offsets, so git
  resolves them with no help at all: the contention stops instead of being
  resolved ever more cleverly. The union is idempotent, so order stops
  depending on the order in which patches happened to merge.
- `orphaned` is the check that makes the union safe: a two-sided conflict
  cannot distinguish "this side added `foo`" from "that side deleted `foo`",
  and a union resurrects the deleted declaration. The filesystem settles it —
  `pub mod foo;` needs `foo.rs` or `foo/mod.rs` in the merged tree, and absence
  is decidable. A union naming no file is refused, not staged as a build
  failure. The gate's compile stage re-checks; refusing here keeps a
  resurrection from being shipped.
- `Conflict::note` records `declaration-only` on a held patch, so the size of
  the class is measured rather than sampled from an incident.

The conversion pass now resolves a declaration-only conflict by canonical
union whatever the file is called, refuses a union that orphans a module, and
no longer counts a resolvable declaration conflict as a structural refusal
(#4637's deadlock).

## The invariant

1. **A coordination point is not a place to hold work.** A file that every
   change must touch but no change is about is an index. A conflict confined
   to the lines that index needs no judgement, and a judgement-free resolution
   is not a compromise: it is the only resolution that scales to agents.

2. **The position of an entry in an index is not data.** Order that depends on
   the order in which patches merged is not a property of the repository, it
   is an accident of scheduling. A canonical position means the next addition
   lands elsewhere, which is the difference between resolving a contention and
   stopping one.

3. **A union is only as good as its check.** The thing a two-sided merge
   cannot see — a deletion on the other side — is exactly what the filesystem
   can see. A resolution that cannot be checked is not a resolution, and a
   union that orphans a module is a build failure in the making.

## The general rule

When a defect recurs at the same location across independent work, the
location is the cause. Six conflicts in the same three files was not six
unlucky patches; it was one shared mutable index that nothing owned. The fix
is to make the index either generated, or resolvable without judgement — and
to record when the second happens, so the first is known to be worth doing.
