# bwrap containment test: probe the environment before asserting isolation

`check_autonomous_phase2_suite` was red on every CI run on the bwrap adapter
test: the workflow installs Codex's static bwrap build, so `command -v bwrap`
passes, but the runner's container denies the unprivileged `unshare` the
adapter needs (pid/ipc/uts without a user namespace). The test asserted
isolation in an environment where isolation cannot exist.

The adapter documents its permission-profile fallback for exactly this case
("bubblewrap ... fails to launch"), so the test now probes the environment
with the minimal namespace creation before asserting: a bwrap that cannot
unshare here is an environment limitation, not an adapter defect, and the
test skips naming the probe's error. A probe that passes but a full run that
fails is still red — that is a real adapter regression.

`tests/autonomous/test_opencode_containment.bats` grew 96 -> 107 non-blank
lines; the shell-ratchet allowlist entry was raised with this deliberate
reason.
