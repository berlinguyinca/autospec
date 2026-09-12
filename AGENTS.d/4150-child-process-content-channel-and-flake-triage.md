# Child-process content channel and flake triage (issue #4150)

- **Content goes on stdin, never in argv.** An argv entry is a NUL-terminated C
  string: a value containing a NUL byte (every ELF binary does) is rejected with
  `InvalidInput: nul byte found in provided data` before the child is spawned,
  and a value past `ARG_MAX` fails with `E2BIG` on some inputs and not others,
  which presents as a size-dependent flake. The mechanical rule: if a value's
  length is determined by data rather than by configuration, it is not an
  argument. argv is for parameters — paths, flags, modes. Where an
  out-of-process write exists for a reason (e.g. the #3495 ETXTBSY fix requires
  the child, not the parent, to hold the write descriptor on the published
  inode), stdin preserves it: the child opens the target, the parent only
  writes to a pipe.
- **Prove isolation before blaming load.** A suite with many sub-100 ms
  deadlines on loaded hosts makes "probably flaky, probably load" the cheap and
  usually-correct explanation — which is how a deterministic failure sat
  unexamined. Before attributing a failure to load or environment, run it
  alone, single-threaded, three times, and record the outcome in the issue:
  fails 3/3 = deterministic defect, fix it; fails 1–2/3 = genuinely
  nondeterministic race or deadline; passes 3/3 = environmental, reproduce
  under the original conditions before touching code. A failure at 0.00 s
  elapsed cannot be a timing failure — there was no time in which to race.
- **Check that the harness actually ran.** `cargo test --test <name>` against a
  name that is a support *module* rather than a test target emits no
  `test result:` line at all. No result line means "the harness never ran" —
  not a pass, not a failure (same rule as #4105).
