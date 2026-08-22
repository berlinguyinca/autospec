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

import pathlib

_HERE = pathlib.Path(__file__).parent

# Ignore the script-style suites by NAME rather than by a blanket glob, so a
# genuine pytest module can live here too. Convention: ``test_unit_*.py`` is a
# real pytest module and IS collected; anything else matching ``test_*.py`` is a
# standalone script and is not.
#
# The blanket glob that used to be here silently swallowed a new pytest module
# added beside the scripts -- it passed when named explicitly and never ran
# under directory collection, which is the worst of both.
collect_ignore = [
    p.name for p in sorted(_HERE.glob("test_*.py"))
    if not p.name.startswith("test_unit_")
]
