# 4560 — apply certified-resolvable conflict shapes

A `git apply --3way` conflict in the conversion pass is a hold unless the
pass can prove the resolution safe. Now, when every conflicted file's shape
is certified keep-both by the classifier (`additive_declarations`,
`append_only_list`), the pass resolves the conflict itself:

- **New core module `conflict_merge`**: the strict keep-both merge. Every
  conflict hunk is replaced by both sides — in order for append-only lists,
  as their union without duplicates for additive declarations. A stray
  marker, an unbalanced hunk, a diff3-style region, or a file with no
  markers at all is a named `MergeError`: the parser's own failure modes
  are part of the check, and each one fails closed.
- **New CLI module `convert/conflict`**: the conflict surface moves out of
  `convert.rs`. `resolve_conflicts` classifies every unmerged file, merges
  the certified ones (writing them back and staging them), and holds on any
  file the classifier does not certify — with the reason naming every
  conflicted file and its shape.
- **Canonicalization**: a union is not necessarily canonical — two appended
  `pub mod` lists merge out of rustfmt's alphabetical order, and the gate's
  fmt stage would hold a correct merge for it. The pass runs the
  project's own formatter on the merged tree and holds if it reaches
  beyond the merged files: a base or patch that arrives fmt-dirty is held,
  exactly as before.
- **The gate is the proof**: the full gate (fmt, build, clippy, test) runs
  on the merged tree before any PR opens. A resolution that compiles but
  corrupts meaning is a hypothesis until the gate agrees.
- **The PR says so**: the opened PR's body names every auto-resolution —
  the file, the certified shape, and the plan. An auto-resolution a human
  cannot see is an auto-resolution a human cannot review.

A conflict whose resolution compiles but cuts through a function body is
not an `additive_declarations` conflict; a test pins that the classifier
does not label it one, and the integration test drives the whole path
(conflict → resolve → canonicalize → real cargo gate → PR body) against a
real, minimal Rust crate.
