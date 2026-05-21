#!/usr/bin/env bash
# jacoco.sh — convert JaCoCo XML report to canonical lcov on stdout.
#
# Usage: collect <jacoco_xml>
#   <jacoco_xml>: path to JaCoCo XML report (e.g. target/site/jacoco/jacoco.xml)
#
# Uses a minimal awk/python converter since lcov tools don't natively read JaCoCo.
#
# Exit 0 = ok, 1 = fatal.

set -eu

collect() {
    local jacoco_xml="${1:-target/site/jacoco/jacoco.xml}"

    if [ ! -f "$jacoco_xml" ]; then
        printf 'jacoco: fatal: JaCoCo XML not found: %s\n' "$jacoco_xml" >&2
        exit 1
    fi

    # Use python3 for XML parsing if available, else awk fallback
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$jacoco_xml" <<'PYEOF'
import sys
import xml.etree.ElementTree as ET

xml_file = sys.argv[1]
try:
    tree = ET.parse(xml_file)
    root = tree.getroot()
except Exception as e:
    print(f"jacoco: fatal: XML parse error: {e}", file=sys.stderr)
    sys.exit(1)

# Emit minimal lcov from JaCoCo XML
# JaCoCo format: <report> -> <package> -> <sourcefile> -> <line nr="N" mi="M" ci="C" mb="M" cb="C"/>
for pkg in root.findall('.//package'):
    pkg_name = pkg.get('name', '').replace('/', '.')
    for srcfile in pkg.findall('sourcefile'):
        fname = srcfile.get('name', '')
        full_path = f"{pkg.get('name', '')}/{fname}" if pkg.get('name') else fname
        print(f"SF:{full_path}")
        print("FN:0,<unknown>")
        for line in srcfile.findall('line'):
            nr = line.get('nr', '0')
            ci = line.get('ci', '0')
            print(f"DA:{nr},{ci}")
        # Coverage counters
        for counter in srcfile.findall('counter'):
            ctype = counter.get('type', '')
            covered = int(counter.get('covered', 0))
            missed = int(counter.get('missed', 0))
            total = covered + missed
            if ctype == 'LINE':
                print(f"LH:{covered}")
                print(f"LF:{total}")
            elif ctype == 'BRANCH':
                print(f"BRH:{covered}")
                print(f"BRF:{total}")
        print("end_of_record")
PYEOF
    else
        printf 'jacoco: fatal: python3 not found; cannot convert JaCoCo XML\n' >&2
        exit 1
    fi
}

collect "${1:-target/site/jacoco/jacoco.xml}"
