#!/usr/bin/env python3
# fake freecadcmd shim — records argv, writes sentinel output files.
# Used by test_freecad_harness.py; installed into a $TMP/bin at test time.
import os
import re
import sys

argv_record = os.environ.get("FREECAD_ARGV_RECORD", "")
if argv_record:
    with open(argv_record, "a") as _f:
        _f.write(" ".join(sys.argv[1:]) + "\n")

if len(sys.argv) < 2:
    sys.exit(0)

script_file = sys.argv[1]
if not os.path.isfile(script_file):
    sys.exit(0)

script = open(script_file).read()

# export_stl: Mesh.export(objects, 'PATH') or Mesh.export(objects, "PATH")
m = re.search(r"Mesh\.export\([^,]+,\s*['\"]([^'\"]+)['\"]\)", script)
if m:
    path = m.group(1)
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as f:
        f.write("solid shim_cube\n")
        f.write("  facet normal 0 0 1\n")
        f.write("    outer loop\n")
        f.write("      vertex 0 0 0\n")
        f.write("      vertex 1 0 0\n")
        f.write("      vertex 1 1 0\n")
        f.write("    endloop\n")
        f.write("  endfacet\n")
        f.write("endsolid shim_cube\n")

# section: section.exportDXF('PATH') or section.exportDXF("PATH")
m = re.search(r"section\.exportDXF\(['\"]([^'\"]+)['\"]\)", script)
if m:
    path = m.group(1)
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as f:
        f.write("DXF_SENTINEL\n")

# render: view.saveImage('PATH', ...) or view.saveImage("PATH", ...)
m = re.search(r"view\.saveImage\(['\"]([^'\"]+)['\"]", script)
if m:
    path = m.group(1)
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as f:
        f.write("PNG_SENTINEL\n")

sys.exit(0)
