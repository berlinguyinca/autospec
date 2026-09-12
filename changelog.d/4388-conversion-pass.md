### Added

- `autospec convert` — the patch-to-PR conversion pass, rebuilt as a Rust
  subcommand after its shell form (`convpass.sh` / `convselect.sh`) was lost
  with a session scratch directory (issue #4388). The pass's decisions live
  in `autospec_core::conversion_pass`; the subcommand wires them to the
  git/cargo/gh I/O.
  - **Selection, not counting.** `select_fresh` selects on three
    disqualifiers — a live branch/PR, or a recorded HELD entry whose re-gate
    still holds — never on the number of patch files on disk. Counting
    produced a "121 patches awaiting conversion" report when only 11 were
    actually fresh.
  - **A no-op pass is distinguishable from a broken one.** `PassOutcome`
    reuses `autospec_core::unfed_pass` so a pass handed no enumeration
    source prints the unfed line (exit `2`), never the idle
    `converted=0 held=0 skipped=0` line; a missing llm root is a
    broken-enumeration error, not an idle pass; a present-but-empty root is a
    true `examined=0` idle pass.
  - The command is side-effect-free by default (plan: enumerate + select +
    report); `--apply` performs the real conversion — branch off
    `origin/<base>`, the affected crate's full gate
    (`fmt --check`, `build`, `clippy`, `test --no-fail-fast`), a PR per
    passing patch, and a HELD line (never a discard) for failures. Its
    must-survive behaviours are reused, not re-implemented: the failing-test
    set comes from the authoritative `failures:` block and is cross-checked
    against each suite's declared count (`autospec_core::failure_attribution`);
    a conflict is HELD rather than auto-resolved unless every conflicted
    file's shape is a proven-safe keep-both (`autospec_core::conflict_resolution`);
    and HELD is a queue — a held patch is re-offered when the patch changes
    or a dependent file moves on the base, and a stale one is archived, never
    discarded (`autospec_core::hold_memo`, `autospec_core::stored_output`).
