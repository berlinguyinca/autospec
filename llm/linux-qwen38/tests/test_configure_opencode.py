#!/usr/bin/env python3
"""configure-opencode.py must be able to withhold the whole-pool tier.

A subagent inherits the parent's model id. The bare pool-sized id is therefore
the one entry whose mere availability can oversubscribe the server: pick it as a
parent and every child declares a window the size of the pool. --no-solo-tier is
how a node that fans out stops offering it.
"""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
GEN = HERE.parent / "scripts" / "configure-opencode.py"

PRESETS = """version = 1

[*]
n-gpu-layers = 999
kv-unified = true

[testmodel]
model = /nonexistent/weights.gguf
c = 196608
parallel = 4
alias = testmodel-160k,testmodel-40k
"""

passed = failed = 0


def report(ok: bool, what: str, detail: str = "") -> None:
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {what}")
    else:
        failed += 1
        print(f"  FAIL  {what}{(' — ' + detail) if detail else ''}")


def generate(presets: Path, config: Path, *extra: str) -> dict:
    res = subprocess.run(
        [sys.executable, str(GEN), "--presets", str(presets), "--config",
         str(config), "--dry-run", *extra],
        capture_output=True, text=True, timeout=120)
    if res.returncode != 0:
        raise AssertionError(f"generator failed: {res.stderr.strip()}")
    return json.loads(res.stdout)


print("== configure-opencode ==")

with tempfile.TemporaryDirectory() as d:
    tmp = Path(d)
    presets = tmp / "presets.ini"
    presets.write_text(PRESETS)
    config = tmp / "opencode.json"          # absent: a first-run generation

    models = generate(presets, config)["provider"]["qwen-local"]["models"]
    report("testmodel" in models,
           "the whole-pool tier is offered by default", str(sorted(models)))
    report(max(m["limit"]["context"] for m in models.values()) == 196608,
           "and it declares the whole pool")

    models = generate(presets, config, "--no-solo-tier")["provider"]["qwen-local"]["models"]
    report("testmodel" not in models,
           "--no-solo-tier withholds it", str(sorted(models)))
    report(max(m["limit"]["context"] for m in models.values()) == 163840,
           "leaving the -160k tier as the largest selectable window")
    report({"testmodel-160k", "testmodel-40k"} <= set(models),
           "while every -NNk tier survives", str(sorted(models)))

print(f"== configure-opencode: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
