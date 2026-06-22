#!/usr/bin/env bats
# test_fab_vision_cli.bats — thin bats wrapper around the Python unittest suite.
# Invoked by validate.sh fab gate (check_autospec_fab_contract) which runs
# every skills/autospec-fab/tests/*.bats file.

# Resolve the repo root from this file's location (tests/ → skill/ → repo root)
REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"
SCRIPTS_DIR="$REPO_ROOT/skills/autospec-fab/scripts"

@test "fab_vision_cli python unittest suite passes" {
    command -v python3 || skip "python3 not found"
    run env PYTHONPATH="$SCRIPTS_DIR" \
        python3 -m unittest discover \
            -s "$REPO_ROOT/skills/autospec-fab/tests" \
            -p "test_fab_vision_cli.py" \
            -v
    [ "$status" -eq 0 ]
}

# --- End-to-end: drive the UNMODIFIED stage_vision.py through fab-vision-cli.py
# installed as `fab-vision` in $TMP/bin, with the stub backend so there is NO
# real Anthropic call. Proves the stub's confirmed observation lands in
# vision_findings and the stage never returns "fail".

# Build a real contact sheet (via stage_render.build_contact_sheet) with one
# present PNG, install the CLI as `fab-vision`, install a stub backend, then run
# stage_vision.py. Echoes back: <bin_dir> <sheet> <stub>.
setup_e2e() {
    local root="$1"
    python3 - "$root" "$SCRIPTS_DIR" <<'PY'
import os, sys, shutil
root, scripts = sys.argv[1], sys.argv[2]
sys.path.insert(0, scripts)
import stage_render

bin_dir = os.path.join(root, "bin")
os.makedirs(bin_dir, exist_ok=True)
# Install the CLI as the PATH-resolvable `fab-vision`.
shutil.copy(os.path.join(scripts, "fab-vision-cli.py"),
            os.path.join(bin_dir, "fab-vision"))
os.chmod(os.path.join(bin_dir, "fab-vision"), 0o755)

# Build a real contact sheet with one present PNG (front view).
render_dir = os.path.join(root, "renders")
view_dir = os.path.join(render_dir, "widget")
os.makedirs(view_dir, exist_ok=True)
results = []
for v in stage_render.REQUIRED_VIEWS:
    png = os.path.join(view_dir, v["name"] + ".png")
    present = v["name"] == "front"
    if present:
        with open(png, "wb") as f:
            f.write(b"\x89PNG\r\n\x1a\n")
    results.append({"name": v["name"], "kind": v["kind"], "png": png,
                    "rendered": present})
sheet = stage_render.build_contact_sheet(render_dir, "widget", results)

# Stub backend: stands in for the real Anthropic call. Emits a confirmed,
# warn-severity observation on judge and confirms on verify.
stub = os.path.join(root, "vision-stub.py")
with open(stub, "w") as f:
    f.write(
        "#!/usr/bin/env python3\n"
        "import json,sys\n"
        "mode=sys.argv[1]\n"
        "sys.stdin.read()\n"
        "if mode=='judge':\n"
        "    print(json.dumps({'observations':[{'observation':"
        "'stub exposed gasket groove','severity':'warn','view':'front',"
        "'rule':'exposed_gasket'}]}))\n"
        "elif mode=='verify':\n"
        "    print(json.dumps({'confirmed':True}))\n"
        "else:\n"
        "    sys.exit(2)\n"
    )
os.chmod(stub, 0o755)
print(f"{bin_dir} {sheet} {stub}")
PY
}

@test "e2e: stub backend confirmed observation flows through unmodified stage_vision.py" {
    command -v python3 || skip "python3 not found"
    local tmp; tmp="$(mktemp -d)"
    local info; info="$(setup_e2e "$tmp")"
    local bin_dir sheet stub
    bin_dir="$(echo "$info" | awk '{print $1}')"
    sheet="$(echo "$info" | awk '{print $2}')"
    stub="$(echo "$info" | awk '{print $3}')"
    local out="$tmp/fragment.json"

    run env PATH="$bin_dir:$PATH" \
        AUTOSPEC_FAB_VISION_CMD=fab-vision \
        AUTOSPEC_FAB_VISION_STUB="$stub" \
        python3 "$SCRIPTS_DIR/stage_vision.py" --in "$sheet" --out "$out"
    [ "$status" -eq 0 ]

    # bats 3.2: the fragment is a real temp file (--out), so [ -f ] is reliable.
    [ -f "$out" ]

    # The stub's confirmed observation is carried in vision_findings, and the
    # stage is warn (advisory), never fail.
    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(d['vision_findings'][0]['rule']); print(d['status'])" "$out"
    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "exposed_gasket" ]
    [ "${lines[1]}" = "warn" ]

    # Blocking findings[] stays empty; status is never "fail".
    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); assert d['findings']==[]; assert d['status']!='fail'; print('ok')" "$out"
    [ "$status" -eq 0 ]
    [ "$output" = "ok" ]

    rm -rf "$tmp"
}

@test "e2e: BACKEND=none degrades to empty vision_findings, never fail, exit 0" {
    command -v python3 || skip "python3 not found"
    local tmp; tmp="$(mktemp -d)"
    local info; info="$(setup_e2e "$tmp")"
    local bin_dir sheet
    bin_dir="$(echo "$info" | awk '{print $1}')"
    sheet="$(echo "$info" | awk '{print $2}')"
    local out="$tmp/fragment.json"

    run env PATH="$bin_dir:$PATH" \
        AUTOSPEC_FAB_VISION_CMD=fab-vision \
        AUTOSPEC_FAB_VISION_BACKEND=none \
        python3 "$SCRIPTS_DIR/stage_vision.py" --in "$sheet" --out "$out"
    [ "$status" -eq 0 ]
    [ -f "$out" ]

    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); assert d['vision_findings']==[]; assert d['status']!='fail'; print('ok')" "$out"
    [ "$status" -eq 0 ]
    [ "$output" = "ok" ]

    rm -rf "$tmp"
}

@test "e2e: stub error degrades, stage stays advisory, exit 0" {
    command -v python3 || skip "python3 not found"
    local tmp; tmp="$(mktemp -d)"
    local info; info="$(setup_e2e "$tmp")"
    local bin_dir sheet
    bin_dir="$(echo "$info" | awk '{print $1}')"
    sheet="$(echo "$info" | awk '{print $2}')"
    # A stub that always fails (write to a real file, then mark executable).
    local errstub="$tmp/err-stub.py"
    printf '#!/usr/bin/env python3\nimport sys\nsys.exit(1)\n' > "$errstub"
    chmod +x "$errstub"
    local out="$tmp/fragment.json"

    run env PATH="$bin_dir:$PATH" \
        AUTOSPEC_FAB_VISION_CMD=fab-vision \
        AUTOSPEC_FAB_VISION_STUB="$errstub" \
        python3 "$SCRIPTS_DIR/stage_vision.py" --in "$sheet" --out "$out"
    [ "$status" -eq 0 ]
    [ -f "$out" ]

    run python3 -c "import json,sys; d=json.load(open(sys.argv[1])); assert d['vision_findings']==[]; assert d['status']!='fail'; print('ok')" "$out"
    [ "$status" -eq 0 ]
    [ "$output" = "ok" ]

    rm -rf "$tmp"
}
