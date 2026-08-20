#!/usr/bin/env python3
"""The unified-memory arithmetic, exercised without an Apple machine.

What cannot be checked here is what `sysctl` actually returns on real hardware —
that needs a Mac. What can be checked is everything downstream of it, which is
where the judgement calls live: that a 48 GiB laptop is not promised 48 GiB of
weights, that an operator's explicit wired limit wins, and that a missing key
produces no reading rather than an invented one.

Feeding known values in is the whole point. A test that called the real sysctl
would pass vacuously on Linux by taking the early return, which is exactly the
kind of green that means nothing.
"""
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "gpu_registry", HERE.parent / "scripts" / "gpu-registry.py")
gr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gr)

GIB = 1024 * 1024 * 1024
passed = failed = 0


def report(ok: bool, what: str, detail: str = "") -> None:
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {what}")
    else:
        failed += 1
        print(f"  FAIL  {what}{(' — ' + detail) if detail else ''}")


def fake(**values):
    return lambda key: values.get(key)


print("== apple unified memory ==")

# A 48 GiB MacBook, no explicit limit. Metal hands a process roughly three
# quarters of RAM; reporting the full 48 would promise weights that fail to load.
seen = gr.observe_metal(sysctl=fake(**{"hw.memsize": 48 * GIB}), is_apple=True)
report(len(seen) == 1, "a Mac reports exactly one accelerator", str(seen))
report(seen and seen[0]["vram_mib"] == 36864,
       "48 GiB of RAM budgets 36 GiB, not 48",
       str(seen[0]["vram_mib"]) if seen else "")
report(seen and seen[0].get("unified") is True,
       "and is flagged unified, so no caller reads it as a dedicated pool")
report(seen and seen[0]["system_ram_mib"] == 49152,
       "while the real RAM figure is still reported alongside")

# An operator who has raised iogpu.wired_limit_mb means it.
seen = gr.observe_metal(
    sysctl=fake(**{"hw.memsize": 48 * GIB, "iogpu.wired_limit_mb": 40960}),
    is_apple=True)
report(seen and seen[0]["vram_mib"] == 40960,
       "an explicit wired limit wins over the three-quarters default",
       str(seen[0]["vram_mib"]) if seen else "")

# Fail closed: no reading is better than a guessed one.
report(gr.observe_metal(sysctl=fake(), is_apple=True) == [],
       "a missing hw.memsize yields no reading rather than a default")
report(gr.observe_metal(sysctl=fake(**{"hw.memsize": 48 * GIB}),
                        is_apple=False) == [],
       "a non-Apple host is never described as one")

print(f"== apple unified memory: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
