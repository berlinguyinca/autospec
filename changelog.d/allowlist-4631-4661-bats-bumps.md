# Shell-ratchet allowlist: record the #4631/#4661 test changes' line growth

The same class of drift as #4659: #4631 ("a failing check must say why")
and #4661 ("honour CARGO_TARGET_DIR so the suite runs the binary it built")
both grew bats suites without their allowlist bump, and the drift check has
been red on main since. This records the current state of the fourteen
affected files — the ratchet's deliberate-raise rule, with the reason — so
the check is green again and its next drift is a real signal.

Systemic note: the fleet has now landed three consecutive batches of bats
growth without bumps (#4654/#4655, then #4631/#4661). The drift check runs
in `cargo test`, so the gap is in the merge habit (merging over a red
build-test), not in the ratchet itself.
