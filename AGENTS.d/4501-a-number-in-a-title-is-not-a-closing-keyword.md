# A number in a title is not a closing keyword

A merge must close the issue it delivers, by a mechanism the tracker
enforces, not by a convention in a title. The conversion pass's PRs named
the issue in the title (`auto-implement: issue #3246`); GitHub closes on a
closing keyword in the body (`Closes #3246.`). Eighteen merged PRs left
their issues open, and every pass since then re-fetched the patch, applied
it, ran a full gate, and reached "no change" — the gate's entire cost,
spent per pass, on work already on the base (#4501).

The invariant: **a merge carries its closing keyword in the body, and a
patch that yields an empty diff against the base is residue, not a
candidate.**

Consequences, enforced in `convert` + `conversion_pass`:

- Every PR the pass opens ends its body with `Closes #<issue>.` — the
  tracker closes on the body, never on a title.
- The pass detects the residue with a read-only `git apply --reverse
  --check` in one shared worktree at the base, and reports it as its own
  category: `delivered=N` on the selection line, `  DELIVERED #N` per
  patch, a `delivered` array in `--json` — never offered, never gated.
- `--apply` archives a delivered patch (never discards it) and releases
  its queue entry, so the backlog number stops counting work that has
  already landed.

A PR body without the closing keyword is a regression: the close is the
mechanism, and a convention in a title is documentation.
