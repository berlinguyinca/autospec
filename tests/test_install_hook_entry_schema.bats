#!/usr/bin/env bats
# tests/test_install_hook_entry_schema.bats — regression for the Claude Code
# hooks-schema contract.
#
# Claude Code's settings.json hooks schema requires each entry in a hook-event
# array to be an OBJECT of the form {"hooks": [{"type": "command", "command": ...}]}.
# An earlier install_hook_mode_claude appended bare command STRINGS, which
# `claude doctor` flags as:
#   "hooks.SessionStart.N: Expected object, but received string".
# These tests pin the corrected behavior so it can't silently regress.

INSTALL_SH="${BATS_TEST_DIRNAME}/../install.sh"

# Extract the install_hook_mode_claude function body and run it against an
# isolated $HOME, so we exercise the real read-modify-write logic.
run_install_hook_mode() {
    local fn
    fn=$(awk '/^install_hook_mode_claude\(\)/{flag=1} flag{print} /^}$/{if(flag){flag=0; exit}}' "$INSTALL_SH")
    DRY_RUN=0 eval "info(){ :; }; $fn; install_hook_mode_claude"
}

@test "writes SessionStart/PreCompact entries as objects, not bare strings" {
    export HOME="$BATS_TEST_TMPDIR/fresh"
    mkdir -p "$HOME"
    run_install_hook_mode

    run python3 - "$HOME/.claude/settings.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for ev in ("PreCompact", "SessionStart"):
    entries = d["hooks"][ev]
    assert entries, ev
    e = entries[-1]
    assert isinstance(e, dict), f"{ev} entry must be an object, got {type(e)}: {e!r}"
    step = e["hooks"][0]
    assert step["type"] == "command"
    assert step["command"].endswith(ev), step
print("ok")
PY
    [ "$status" -eq 0 ]
}

@test "self-heals legacy bare-string entries while preserving other hooks" {
    export HOME="$BATS_TEST_TMPDIR/legacy"
    mkdir -p "$HOME/.claude"
    cat > "$HOME/.claude/settings.json" <<'JSON'
{
  "hooks": {
    "PreCompact": ["python3 -m autospec_context_monitor --hook-event PreCompact"],
    "SessionStart": [
      {"hooks": [{"type": "command", "command": "budi hook session-start"}]},
      "python3 -m autospec_context_monitor --hook-event SessionStart"
    ]
  }
}
JSON
    run_install_hook_mode

    run python3 - "$HOME/.claude/settings.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
# No bare strings survive anywhere in the hook arrays.
for ev, entries in d["hooks"].items():
    for e in entries:
        assert isinstance(e, dict), f"{ev} still has bare string: {e!r}"
# Pre-existing budi hook is preserved.
cmds = [s["command"] for e in d["hooks"]["SessionStart"] for s in e["hooks"]]
assert "budi hook session-start" in cmds, cmds
assert any(c.endswith("SessionStart") for c in cmds), cmds
print("ok")
PY
    [ "$status" -eq 0 ]
}

@test "re-running is idempotent (no duplicate monitor entries)" {
    export HOME="$BATS_TEST_TMPDIR/idem"
    mkdir -p "$HOME"
    run_install_hook_mode
    run_install_hook_mode

    run python3 - "$HOME/.claude/settings.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for ev in ("PreCompact", "SessionStart"):
    monitor = [e for e in d["hooks"][ev]
               if any("autospec_context_monitor" in s.get("command", "")
                      for s in e.get("hooks", []))]
    assert len(monitor) == 1, f"{ev}: expected 1 monitor entry, got {len(monitor)}"
print("ok")
PY
    [ "$status" -eq 0 ]
}
