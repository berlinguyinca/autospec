## A test may not reserve a machine-global name

Two tests failed whenever another `cargo test` ran concurrently on the same
host, because they reserved machine-global resources under fixed or
predictable names.

- **`publish_overwrite` leaked its temp file when the second holder check
  refused** (and `publish_versioned` leaked on a failed write): a refusal
  between the write and the rename left a `.safe-publish-*` file in the
  target directory, which the test's "no temp residue" assertion then
  reported as a failure. A `TempGuard` now removes the temp file on every
  path except a successful rename — a refused publish leaves no trace.
- **The port reservation test asserted the released port was bindable
  again**, which is a machine-global race: on a busy host a peer process
  takes the port between the release and the bind. The test now retries the
  whole cycle with a fresh kernel-assigned port, tolerating a peer that
  snipes one (ten in a row panics).
- **The holder test now waits for the holder to hold** — polling the same
  `/proc` fact the producer checks — before asserting the refusal, so the
  precondition is deterministic instead of load-dependent.

Proven: two full `cargo test -p autospec-core` runs concurrent on one host,
zero failed suites. An audit of the workspace found no other test binding a
fixed port or writing a fixed path under a shared temp root (the remaining
fixed-name paths are read-only test data).
