# Shell-ratchet allowlist: record #4661's one comment line per suite

#4661 ("honour CARGO_TARGET_DIR so the suite runs the binary it built")
landed without its allowlist bump, and the ratchet's drift check has been
red on main since: +57 lines across 14 bats suites.

Almost all of that was duplicated prose, not logic. The fix carried a
five-line explanation of the bug it prevents, copied verbatim into every
suite it touched — 70 lines of identical comment for 14 lines of code.
That is recorded once now, in the issue and in this note, and each suite
carries a single line pointing at it:

    # Honour CARGO_TARGET_DIR or we build one binary and run another (#4661).

The collapse removes 67 lines. What remains is a deliberate raise of
**14 lines — one comment line per suite** — which is what this note
documents, per the ratchet's rule that an entry may only fall and a raise
needs a stated reason.

The comment is kept rather than dropped to zero because the line it guards
looks like a pointless indirection without it: hardcoding
`$REPO_ROOT/target` is the obvious-looking simplification, and it is the
bug — it builds one binary and executes another, observed running a
2.5-hour-old executable while the fresh one sat in the real target dir.
One line per suite is the smallest thing that stops that being "cleaned
up" again.

No shell logic was added or removed.
