#!/usr/bin/env bash
# The validation catalog's size is asserted as a LITERAL in three test files
# across two crates, so they cannot share a constant. Adding one check means
# bumping every copy, and missing one breaks main in a way that then fails
# every subsequent check-adding patch -- a cascade, not a single failure.
#
# That happened twice in one day. The first miss held seven patches falsely;
# the fix measured the cost and filed it (#3892); an hour later a NINTH site
# (crates/autospec-cli/tests/validation_parity.rs) was found the same way, and
# held three more. A description of the hazard did not prevent the second
# occurrence, so this is the mechanical control: divergence fails here, with
# the file list, instead of surfacing as an unrelated-looking test failure.
#
# It asserts CONSISTENCY, not any particular value -- bumping the catalog is
# expected and fine; bumping it in some copies and not others is the defect.
set -uo pipefail
cd "$(dirname "$0")/.."

python3 - <<'PYCHK'
import re, sys, collections

# (file, regex capturing the literal, label) -- each label must agree everywhere.
SITES = [
    ("crates/autospec-core/tests/validation_runner.rs",  r'assert_eq!\(full\.ids\(\)\.len\(\),\s*(\d+)\)',            "full_plan"),
    ("crates/autospec-core/tests/validation_runner.rs",  r'assert_eq!\(full\.unique_ids\(\)\.len\(\),\s*(\d+)\)',     "unique"),
    ("crates/autospec-core/tests/validation_catalog.rs", r'assert_eq!\(calls\.len\(\),\s*(\d+)\)',                    "full_plan"),
    ("crates/autospec-core/tests/validation_catalog.rs", r'BTreeSet<_>>\(\)\.len\(\),\s*(\d+)\)',                     "unique"),
    ("crates/autospec-cli/tests/validation_parity.rs",   r'assert_eq!\(full\.ids\(\)\.len\(\),\s*(\d+)\)',            "full_plan"),
    ("crates/autospec-cli/tests/validation_parity.rs",   r'assert_eq!\(full\.unique_ids\(\)\.len\(\),\s*(\d+)\)',     "unique"),
]

seen = collections.defaultdict(list)
missing = []
for path, pattern, label in SITES:
    try:
        text = open(path).read()
    except FileNotFoundError:
        missing.append(path); continue
    m = re.search(pattern, text)
    if not m:
        missing.append(f"{path} :: {label}"); continue
    seen[label].append((path, int(m.group(1))))

bad = False
for path in missing:
    print(f"FAIL: catalog count site not found: {path}")
    print("      the assertion moved or was renamed; update SITES in this script")
    bad = True

for label, entries in sorted(seen.items()):
    values = {v for _, v in entries}
    if len(values) > 1:
        bad = True
        print(f"FAIL: catalog '{label}' count disagrees across files: {sorted(values)}")
        for path, v in entries:
            print(f"        {v:>5}  {path}")
        print("      adding a check means bumping EVERY copy; one missed copy breaks main")
        print("      and then falsely holds every subsequent check-adding patch")

if not bad:
    summary = ", ".join(f"{label}={entries[0][1]}" for label, entries in sorted(seen.items()))
    print(f"ok:   catalog count literals agree across {len(SITES)} sites ({summary})")
sys.exit(1 if bad else 0)
PYCHK
