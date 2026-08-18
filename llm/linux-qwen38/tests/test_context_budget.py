#!/usr/bin/env python3
"""context-budget-check.py must catch the ways fan-out overruns the pool.

Each case is the 2026-08-18 failure in miniature: the tier a session actually
picked is not the configured default, and children inherit it. A guard that read
only the config would have called that afternoon healthy.
"""
from __future__ import annotations

import json
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE.parent / "scripts" / "context-budget-check.py"

TIERS = {"qwen3.8-27b": 196608, "qwen3.8-27b-40k": 40960}
POOL, SLOTS = "196608", "4"

passed = failed = 0


def report(ok: bool, what: str, detail: str = "") -> None:
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {what}")
    else:
        failed += 1
        print(f"  FAIL  {what}{(' — ' + detail) if detail else ''}")


def fixture(tmp: Path, live_model: str, pin_child: bool) -> list[str]:
    """A config, an agent directory and a session DB; returns CLI args for them."""
    tmp.mkdir(parents=True)
    config = tmp / "opencode.json"
    config.write_text(json.dumps({
        "model": "qwen-local/qwen3.8-27b-40k",
        "provider": {"qwen-local": {"models": {
            mid: {"limit": {"context": ctx}} for mid, ctx in TIERS.items()}}},
    }))

    agents = tmp / "agent"
    agents.mkdir()
    # No mode: means "all", which is spawnable. No model: means it inherits.
    pin = "model: qwen-local/qwen3.8-27b-40k\n" if pin_child else ""
    (agents / "kid.md").write_text(f"---\nname: kid\n{pin}---\nbody\n")

    db = tmp / "opencode.db"
    con = sqlite3.connect(db)
    con.execute("CREATE TABLE session (id TEXT, parent_id TEXT, model TEXT, "
                "time_created INTEGER)")
    con.execute("INSERT INTO session VALUES (?,?,?,?)",
                ("ses_1", None,
                 json.dumps({"id": live_model, "providerID": "qwen-local"}), 1))
    con.commit()
    con.close()
    return ["--config", str(config), "--agents", str(agents), "--sessions", str(db)]


def run(args: list[str], width: int) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECK), "--width", str(width),
         "--pool", POOL, "--slots", SLOTS, "--no-cli", *args],
        capture_output=True, text=True, timeout=120)


print("== context budget ==")

with tempfile.TemporaryDirectory() as d:
    root = Path(d)

    # 1 — the whole-pool tier picked by hand, which every child inherits. The
    # config default is a 40k tier, so only the session DB reveals this.
    res = run(fixture(root / "solo", "qwen3.8-27b", pin_child=True), width=1)
    report(res.returncode == 1, "a whole-pool live selection is caught",
           f"exit {res.returncode}: {res.stdout.strip()[-160:]}")
    report("live selection" in res.stdout,
           "the report names the live selection, not the config default",
           res.stdout.strip()[-160:])

    # 2 — the configuration this node is meant to run: 40k parent, pinned
    # children, three of them, inside a 196,608 pool on four slots.
    res = run(fixture(root / "ok", "qwen3.8-27b-40k", pin_child=True), width=3)
    report(res.returncode == 0, "a 40k parent with three pinned 40k children fits",
           f"exit {res.returncode}: {res.stdout.strip()[-160:]}")

    # 3 — an unpinned spawnable agent is the silent half of the bug: it declares
    # whatever the parent declared, so it must be named even when the sum fits.
    res = run(fixture(root / "unpinned", "qwen3.8-27b-40k", pin_child=False), width=3)
    report(res.returncode == 1 and "without a pinned tier" in res.stdout,
           "an unpinned spawnable agent is reported as inheriting",
           f"exit {res.returncode}: {res.stdout.strip()[-160:]}")

print(f"== context budget: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
