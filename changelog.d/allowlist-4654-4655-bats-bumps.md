# Shell-ratchet allowlist: record the #4654/#4655 test fixes' line growth

Two fleet test fixes landed on main without their allowlist bump, and the
ratchet's own drift check has been red since: #4654 ("build the binary
every run, not only when it is missing") replaced the one-line conditional
build gate in eight bats suites with an unconditional build plus a fresh-
binary assertion (+4 lines each), and #4655 ("keep the script's JSON before
`run` overwrites `$output`") grew
`tests/autospec/autonomous-promote-open-issues.bats` by 24 lines.

The ratchet's rule is that an entry may only fall, and a deliberate raise
needs a documented reason — this is that documentation: the growth is the
two fixes themselves, already reviewed and merged. This commit records the
current state so the ratchet is green again and its next drift is a real
signal. No shell logic was added or removed by either fix; the lines are
test setup and assertions.
