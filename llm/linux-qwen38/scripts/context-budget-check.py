#!/usr/bin/env python3
"""Refuse a fan-out that the KV pool cannot fund.

    context-budget-check.py [--width N] [--server URL] [--config PATH]
                            [--agents DIR] [--json]

The server accepts more sessions than its pool can pay for and then fails EVERY
live one with "Context size has been exceeded" -- not just the greedy one. The
client's declared context limit is the only admission control that exists, and
nothing in the client sizes it for subagents: a child inherits the parent's model
id, so a parent sitting on the whole-pool tier hands each of its children a
window as large as the pool itself.

This reads the LIVE server rather than the presets, because what is loaded is
what bills. Then it prices the worst case:

    parent_tier + width * child_tier  <=  pool

and names the agents that are spawnable as children without a pinned tier, since
those are the ones that silently inherit.

Exit 0 fits, 1 does not, 2 could not tell (server down, config unreadable).

Known blind spot: the parent tier is read from the client config's default
model, and the TUI's remembered per-session selection overrides that. A parent
that picked the whole-pool tier by hand will not show up here.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_SERVER = "http://127.0.0.1:8080"
DEFAULT_SESSIONS = Path.home() / ".local/share/opencode/opencode.db"
DEFAULT_CONFIG = Path.home() / ".config/opencode/opencode.json"
DEFAULT_AGENTS = Path.home() / ".config/opencode/agent"


def fetch(url: str, timeout: float = 10.0):
    with urllib.request.urlopen(url, timeout=timeout) as fh:
        return json.load(fh)


def server_facts(base: str) -> dict:
    """Pool, slots and KV mode of the loaded model, from the router itself."""
    data = fetch(f"{base}/v1/models")
    for entry in data.get("data", []):
        status = entry.get("status") or {}
        args = status.get("args") or []
        if not args:
            continue
        def opt(name, default=None):
            if name in args:
                i = args.index(name)
                if i + 1 < len(args):
                    return args[i + 1]
            return default
        # The router does not answer /tokenize -- only the model instance it
        # spawned does, on a port it chose at launch. Read it off the args
        # rather than guessing, because it changes on every model swap.
        host = opt("--host", "127.0.0.1")
        port = opt("--port")
        return {
            "id": entry.get("id"),
            "aliases": entry.get("aliases") or [],
            "pool": int(opt("--ctx-size", 0) or 0),
            "slots": int(opt("--parallel", 0) or 0),
            # Present as a bare flag; --no-kv-unified is the hard-partition mode.
            "unified": "--kv-unified" in args and "--no-kv-unified" not in args,
            "instance": f"http://{host}:{port}" if port else None,
        }
    raise RuntimeError("no loaded model reported by the router")


def declared_limits(config: Path) -> tuple[dict[str, int], str | None, dict]:
    """{provider/model: declared context}, the default model, and agent overrides."""
    cfg = json.loads(config.read_text())
    out: dict[str, int] = {}
    for pid, provider in (cfg.get("provider") or {}).items():
        for mid, model in (provider.get("models") or {}).items():
            ctx = ((model.get("limit") or {}).get("context"))
            if ctx:
                out[f"{pid}/{mid}"] = int(ctx)
    return out, cfg.get("model"), (cfg.get("agent") or {})


def live_parent(db: Path, limits: dict[str, int]) -> tuple[str, int] | None:
    """The model the newest top-level session actually picked.

    The config default is not what a session necessarily runs: the TUI remembers
    a per-session selection, and on 2026-08-18 that selection was the whole-pool
    tier -- which every child then inherited. Reading it back is the only way this
    check sees the tier that is really in play.
    """
    if not db.is_file():
        return None
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    except sqlite3.Error:
        return None
    try:
        for (raw,) in con.execute(
                "SELECT model FROM session WHERE parent_id IS NULL "
                "AND model IS NOT NULL ORDER BY time_created DESC LIMIT 8"):
            try:
                m = json.loads(raw)
            except (TypeError, ValueError):
                continue
            mid = f"{m.get('providerID')}/{m.get('id')}"
            if mid in limits:
                return mid, limits[mid]
    except sqlite3.Error:
        return None
    finally:
        con.close()
    return None


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


def tokens(base: str | None, text: str) -> tuple[int, bool]:
    """(count, exact). Falls back to chars/4 when the tokenizer is unreachable."""
    if not base:
        return len(text) // 4, False
    req = urllib.request.Request(
        f"{base}/tokenize",
        data=json.dumps({"content": text}).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as fh:
            return len(json.load(fh).get("tokens", [])), True
    except (urllib.error.URLError, OSError, ValueError):
        return len(text) // 4, False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--width", type=int, default=4,
                    help="how many children may be live at once (worst case)")
    ap.add_argument("--server", default=os.environ.get("QWEN_ROUTER", DEFAULT_SERVER))
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--agents", type=Path, default=DEFAULT_AGENTS)
    ap.add_argument("--sessions", type=Path, default=DEFAULT_SESSIONS,
                    help="OpenCode session database, read to find the tier the "
                         "newest live session actually picked")
    ap.add_argument("--pool", type=int,
                    help="price a hypothetical pool instead of the loaded one, "
                         "to test a preset before restarting on it")
    ap.add_argument("--slots", type=int, help="likewise for the slot count")
    ap.add_argument("--no-cli", action="store_true",
                    help="skip `opencode agent list`; read files and config only, "
                         "which cannot see built-in agents")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    if args.pool and args.slots:
        # Fully specified by hand: price a preset without needing it loaded.
        facts = {"pool": args.pool, "slots": args.slots, "unified": True,
                 "instance": None}
    else:
        try:
            facts = server_facts(args.server)
        except Exception as exc:                   # noqa: BLE001 - report, not raise
            print(f"cannot read the router at {args.server}: {exc}", file=sys.stderr)
            return 2
    try:
        limits, default_model, overrides = declared_limits(args.config)
    except Exception as exc:                       # noqa: BLE001
        print(f"cannot read {args.config}: {exc}", file=sys.stderr)
        return 2

    pool = args.pool or facts["pool"]
    slots = args.slots or facts["slots"]
    parent_tier = limits.get(default_model or "", 0)
    live = live_parent(args.sessions, limits)
    if live and live[1] > parent_tier:
        default_model, parent_tier = f"{live[0]} (live selection)", live[1]
    modes = None if args.no_cli else cli_modes()
    kids = spawnable(args.agents, overrides, modes)

    problems: list[str] = []
    inheriting = [k for k in kids if not k["model"]]
    pinned = [k for k in kids if k["model"]]

    # The worst child is the most expensive window any child can declare.
    child_tier = 0
    worst_child = None
    if inheriting:
        child_tier, worst_child = parent_tier, f"{inheriting[0]['name']} (inherits)"
    for k in pinned:
        t = limits.get(k["model"], 0)
        if t > child_tier:
            child_tier, worst_child = t, k["name"]

    # A child beyond the slot count does not draw KV -- it queues. Pricing width
    # above what can actually be resident reports a failure the server cannot
    # reach, and a guard that cries wolf gets ignored on the day it is right.
    priced_width = min(args.width, max(0, slots - 1)) if slots else args.width
    queued = args.width - priced_width
    demand = parent_tier + priced_width * child_tier
    fits = demand <= pool
    max_width = max(0, (pool - parent_tier) // child_tier) if child_tier else None

    if not fits:
        problems.append(
            f"worst case {demand:,} > pool {pool:,}: parent {parent_tier:,} "
            f"+ {priced_width} x {child_tier:,}")
    # Every slot can be filled, so the bound that matters is a full house:
    # one parent plus slots-1 children, all declaring their maximum.
    full_house = parent_tier + max(0, slots - 1) * child_tier if slots else 0
    if facts["unified"] and full_house > pool:
        problems.append(
            f"a full house of {slots} slots declares {full_house:,} against a "
            f"{pool:,} pool: the server can be oversubscribed by slot count alone, "
            f"whatever the caller's fan-out width")
    if inheriting:
        problems.append(
            "spawnable without a pinned tier, so each inherits the parent's window: "
            + ", ".join(sorted(k["name"] for k in inheriting)))

    # The skill tool injects the whole skill roster. Measured on this host, a
    # child floor went 34,937 -> 18,685 with it denied: 16.3k of a 40,960 tier
    # spent on skills a child should not be loading anyway.
    if not args.no_cli:
        for k in kids:
            info = resolved(k["name"]) or {}
            tools = info.get("tools") or {}
            k["skill_tool"] = tools.get("skill")
            k["task_tool"] = tools.get("task")
            k["pinned"] = (info.get("model") or {}).get("modelID")
        taxed = [k["name"] for k in kids if k.get("skill_tool")]
        if taxed:
            problems.append(
                "skill tool enabled, which costs ~16k of preamble per child: "
                + ", ".join(sorted(taxed)))
        nesting = [k["name"] for k in kids if k.get("task_tool")]
        if nesting:
            problems.append(
                "can spawn children of its own, so the width bound above does not "
                "hold: " + ", ".join(sorted(nesting)))

    # A child's own prompt body is paid before it reads a line of code.
    heavy = []
    estimated = False
    for k in kids:
        if not k["path"]:
            continue                       # built-in: no body on disk to price
        n, exact = tokens(facts.get("instance"), Path(k["path"]).read_text(errors="replace"))
        estimated = estimated or not exact
        k["body_tokens"] = n
        budget = limits.get(k["model"] or default_model or "", 0)
        if n and budget and n > budget // 4:
            heavy.append(f"{k['name']} ({n:,} tokens, {n * 100 // budget}% of its tier)")
    if heavy:
        problems.append("prompt body eats its own window: " + ", ".join(sorted(heavy)))

    report = {
        "pool": pool, "slots": slots, "kv_unified": facts["unified"],
        "default_model": default_model, "parent_tier": parent_tier,
        "width": args.width, "priced_width": priced_width, "queued": queued,
        "worst_child": worst_child, "child_tier": child_tier,
        "demand": demand, "fits": fits, "max_width": max_width,
        "full_house": full_house,
        "token_counts_estimated": estimated,
        "problems": problems,
    }
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if fits and not problems else 1

    print(f"agents      {len(kids)} spawnable as children"
          f"{'' if modes else ' (from files + config; built-ins not visible)'}")
    print(f"pool        {pool:,} tokens, {slots} slots, "
          f"kv={'unified' if facts['unified'] else 'partitioned'}")
    print(f"parent      {default_model} -> {parent_tier:,} "
          + ("" if live else " (config default; no live selection found)"))
    print(f"worst child {worst_child} -> {child_tier:,}")
    print(f"width {args.width}    {demand:,} of {pool:,} "
          f"({'fits' if fits else 'OVER'})"
          + (f"; {queued} beyond the slot count would queue, not draw KV"
             if queued > 0 else ""))
    print(f"full house  {full_house:,} of {pool:,} "
          f"({'fits' if full_house <= pool else 'OVER'}) at {slots} slots")
    if max_width is not None:
        print(f"safe width  {max_width} concurrent children at that tier")
    if estimated:
        print("note        some token counts are chars/4 estimates "
              "(the model instance would not tokenize)")
    for p in problems:
        print(f"  ! {p}")
    return 0 if fits and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
