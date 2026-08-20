"""Which OpenCode agents can be spawned as children, and what they declare.

Shared by context-budget-check.py, which prices a fan-out, and
configure-opencode.py, which pins the children so the fan-out stays affordable.
They must agree on what "spawnable" means: if the pricer and the pinner disagree
about which agents inherit, the budget is computed for one set and enforced on
another, and the mismatch is invisible until a fan-out kills every live session.

Import from a sibling script with:

    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from agentmodes import spawnable
"""
from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


def frontmatter(path: Path) -> dict:
    """The frontmatter block only -- deliberately not a YAML dependency."""
    text = path.read_text(errors="replace")
    m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    if not m:
        return {"_body": text}
    out: dict[str, str] = {}
    for line in m.group(1).splitlines():
        km = re.match(r"^([a-z_]+):\s*(.*)$", line)
        if km:
            out[km.group(1)] = km.group(2).strip()
    out["_body"] = text[m.end():]
    return out


def cli_modes() -> dict[str, str] | None:
    """{agent: mode} straight from the client.

    Worth the ~60 s it costs: this is the only source that sees built-in agents
    (`general`, `explore`) and that applies the config's `agent` overrides. File
    frontmatter alone reports agents the config has already retired, and misses
    the two that are actually being spawned.
    """
    try:
        res = subprocess.run(["opencode", "agent", "list"], capture_output=True,
                             text=True, timeout=180)
    except (OSError, subprocess.SubprocessError):
        return None
    if res.returncode != 0:
        return None
    found = dict(re.findall(r"(?m)^(\S+) \((primary|subagent|all)\)$", res.stdout))
    return found or None


def resolved(name: str) -> dict | None:
    """`opencode debug agent NAME` -- the merged view, including built-ins."""
    try:
        res = subprocess.run(["opencode", "debug", "agent", name],
                             capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.SubprocessError):
        return None
    if res.returncode != 0:
        return None
    try:
        return json.loads(res.stdout)
    except ValueError:
        return None


def spawnable(agents: Path, overrides: dict, modes: dict[str, str] | None) -> list[dict]:
    """Agents the task tool may spawn: anything whose effective mode is not primary.

    An absent mode means "all", which IS spawnable -- the quiet default that
    makes a 20k-token skill body reachable as a child.
    """
    files = {}
    for path in sorted(agents.glob("*.md")):
        fm = frontmatter(path)
        files[fm.get("name") or path.stem] = (fm, path)

    names = set(modes) if modes else set(files) | set(overrides)
    found = []
    for name in sorted(names):
        fm, path = files.get(name, ({}, None))
        override = overrides.get(name) or {}
        mode = (modes or {}).get(name) or override.get("mode") or fm.get("mode", "all")
        if mode == "primary":
            continue
        found.append({
            "name": name,
            "mode": mode,
            "model": override.get("model") or fm.get("model"),
            "path": str(path) if path else None,
        })
    return found
