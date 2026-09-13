## A path that cannot be attributed to a crate widens the scope, never is dropped from it

`derive_prefilter_scope` now returns `--workspace` when a patch touches any
path outside `crates/<name>/` — a root `Cargo.toml`, `Cargo.lock`,
`rust-toolchain.toml`, a shared config, a doc — not only when it touches
*no* resolvable crate. A crate patch beside the root `Cargo.toml` gated
only that crate before; a root-manifest edit (a dependency bump, a feature
default, a `[workspace.dependencies]` change) can break a crate the scope
never named, and nothing gated it.

The widening is deliberately blunt: a `README.md` beside a crate patch
costs a workspace gate rather than a `-p` one. That is the cheap error
direction (one slower pre-filter); a dropped path is the expensive one (a
batch failure bisected back to a file no scope examined). The module doc
states the rule, replacing the "fail-closed" claim this case refuted.

New public API: `path_attributes_to_crate` (the per-path rule
`crates_touched` applies) and `has_unattributable_path`. The truth table
the issue pins is tested, including the decided answers: a crate's own
manifest stays at the crate (it is under the crate), and the #4532 test
that pinned crate+README to `-p foo` now pins the workspace answer.
