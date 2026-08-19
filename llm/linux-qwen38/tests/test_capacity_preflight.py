#!/usr/bin/env python3
"""capacity-preflight.py must agree with what the hardware actually did.

Both numbers here are measured on an RTX 4090, not derived: ctx 204800 with
Q4_K_M weights and q4_0 KV crash-looped 54 times, each attempt dying on the
same 600 MiB allocation, and ctx 131072 came up healthy in 15s holding
22718 MiB. A gate that passes the first or refuses the second is not a gate.
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE.parent / "scripts" / "capacity-preflight.py"

# 18095 MiB of weights plus the 600 MiB projector that is pulled with them.
WEIGHTS_MIB = 18695
CARD_MIB = 24564

passed = failed = 0


def report(ok: bool, what: str, detail: str = "") -> None:
    global passed, failed
    if ok:
        passed += 1
        print(f"  PASS  {what}")
    else:
        failed += 1
        print(f"  FAIL  {what}{(' — ' + detail) if detail else ''}")


def verdict(ctx: int, *extra: str) -> tuple[int, dict]:
    res = subprocess.run(
        [sys.executable, str(CHECK), "--ctx", str(ctx),
         "--weights-mib", str(WEIGHTS_MIB), "--vram-mib", str(CARD_MIB),
         "--kv", "q4_0", "--parallel", "1", "--json", *extra],
        capture_output=True, text=True, timeout=120)
    try:
        return res.returncode, json.loads(res.stdout)
    except ValueError:
        return res.returncode, {}


print("== capacity preflight ==")

code, crashed = verdict(204800)
report(not crashed.get("fits") and code == 75,
       "the context that crash-looped is refused, with EX_TEMPFAIL",
       f"exit {code}")

code, worked = verdict(131072)
report(worked.get("fits") and code == 0,
       "the context measured healthy is allowed", f"exit {code}")

report(crashed.get("largest_ctx") == 131072,
       "the refusal names the size that was measured to work",
       str(crashed.get("largest_ctx")))

# The sizing model already reserves 2 GiB + 512 MiB/slot and then takes 80% of
# the rest. An extra default margin here double-counted it and cost a whole
# rung -- 131072 was refused as 98304.
code, tight = verdict(131072, "--headroom-mib", "512")
report(not tight.get("fits"),
       "extra headroom is available and does cost a rung, so it is opt-in",
       str(tight.get("largest_ctx")))

# A fleet spanning a 4090, an RTX 6000, H100s and a MacBook has no default card
# worth guessing.
res = subprocess.run(
    [sys.executable, str(CHECK), "--ctx", "4096", "--weights-mib", "1000",
     "--vram-mib", "0"], capture_output=True, text=True, timeout=120)
report(res.returncode == 2,
       "an unreadable accelerator is undetermined, never assumed",
       f"exit {res.returncode}")

print(f"== capacity preflight: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
