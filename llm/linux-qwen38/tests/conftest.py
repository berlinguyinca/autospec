"""Keep pytest out of suites that are deliberately not pytest modules.

Every ``test_*.py`` here is a standalone script: run it as
``python3 tests/test_<name>.py [base-url] [model]``, and it exits non-zero on
failure, the same contract as the ``.sh`` suites beside it. Most need a live
inference endpoint named on the command line, which is why they are scripts —
there is nothing for a collector to parameterise.

They call ``sys.exit()`` at import time. Collected, that surfaces as an
INTERNALERROR rather than a failure, which reads like a broken test rather
than a misuse. Ignoring them keeps ``pytest`` honest: it reports no tests
here, because there are none of its kind. ``test_structural.sh`` is the entry
point that knows how to invoke each one.
"""

collect_ignore_glob = ["test_*.py"]
