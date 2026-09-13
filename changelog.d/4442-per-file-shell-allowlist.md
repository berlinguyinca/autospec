### Changed

- The shell ratchet is now a per-file allowlist instead of a global zero-slack
  counter (#4442): each counted `.sh`/`.bats` file carries a permitted line
  count in `tests/fixtures/shell-ratchet-allowlist.txt`, a counted file with
  no entry (new shell surface) is a blocking finding, a file over its own
  entry is a blocking finding, and an entry may only be lowered — so a repair
  that fits its file's accumulated slack lands, a new script still cannot,
  and the sum of the entries, the effective ceiling, still only falls.
  Removing shell lowers the entries it touches in the same commit, and an
  entry whose file is gone is a finding of its own, so the allowlist cannot
  drift above reality. The shipped allowlist is pinned by a test that scans
  the real repository (the bats ratchet's shape), and reseed is an env-gated
  helper (`AUTOSPEC_SHELL_RATCHET_RESEED=1`) that refuses to raise an entry.
  `autospec_core::shell_ratchet` replaces `verdict`/`RatchetVerdict` and
  `diff_verdict`/`RatchetDiffVerdict` with `allowlist_verdict`,
  `allowlist_diff_verdict`, `raised_entries`, and the `Allowlist` type.
