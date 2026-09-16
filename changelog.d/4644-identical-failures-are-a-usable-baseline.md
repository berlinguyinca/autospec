## convert: identical pre/post failures are a usable baseline (#4644)

A workspace with one consistently-failing crate is an environmental
failure of the base, not a verdict about the patch — yet a regression test
for that shape was missing. The pass already attributes a stage that also
fails at the base to the base (`stage_origin` re-runs it after restoring
the base from HEAD and yields `BaseUnverifiable`), so the patch is
skipped, never held. This change locks that behavior in:

- a new integration test drives the real `--apply` pass against a fixture
  workspace whose single crate carries a test that fails at the base and
  again after the patch (a required tool the host does not provide, like
  the npm-missing `schema-gen` suite);
- the patch adds a passing test function, so the unchanged-count
  contradiction (#4532) is in play and a baseline is measured — and the
  environmental failure is still attributed to the base;
- the test asserts the patch is named `skipped: base unverifiable`, that
  `held=0` and `skipped=1`, and that no HELD line is written — the patch
  is unmeasured, not defective, and re-enters dispatch for a later pass
  on a host that can measure it.
