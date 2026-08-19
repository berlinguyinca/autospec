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

# --- the child tier must come from the server, not from a guess ---------------
# The same client runs against a 24 GiB 4090, a 96 GiB RTX 6000, an H100 and a
# MacBook. It never learns which: gen-preset.py sized `c` and `parallel` to the
# card, so solving parent + width*child <= pool against those two is correct
# everywhere. A fixed "smallest tier" would be safe and wasteful on a big card.
SMALL = """version = 1

[testmodel]
model = /nonexistent/weights.gguf
c = 131072
parallel = 4
alias = testmodel-128k,testmodel-64k,testmodel-40k
"""

BIG = """version = 1

[testmodel]
model = /nonexistent/weights.gguf
c = 262144
parallel = 8
alias = testmodel-256k,testmodel-128k,testmodel-64k,testmodel-40k
"""

with tempfile.TemporaryDirectory() as d:
    tmp = Path(d)
    agents = tmp / "agents"
    agents.mkdir()
    (agents / "kid.md").write_text("---\nmode: subagent\n---\nbody\n")
    (agents / "boss.md").write_text("---\nmode: primary\n---\nbody\n")

    def pins(presets_text: str, *extra: str) -> dict:
        pf = tmp / "p.ini"
        pf.write_text(presets_text)
        cfg = tmp / f"cfg-{abs(hash(presets_text + ''.join(extra)))}.json"
        return generate(pf, cfg, "--agents", str(agents), *extra).get("agent", {})

    small = pins(SMALL)
    big = pins(BIG)

    report("kid" in small and "kid" in big,
           "a spawnable agent is pinned on every card", str(small))
    report("boss" not in small,
           "a primary agent is left alone — it is never spawned as a child",
           str(small))

    small_ctx = small.get("kid", {}).get("model", "")
    big_ctx = big.get("kid", {}).get("model", "")
    # At the default width both land on the ladder's bottom rung: the bigger
    # pool also buys a bigger PARENT, which eats the gain. What the extra card
    # actually funds is more children at that rung, not fatter ones -- the
    # ladder's granularity is the limit here, not the arithmetic.
    report(small_ctx.endswith("-40k") and big_ctx.endswith("-40k"),
           "at full width both cards land on the ladder's bottom rung",
           f"{small_ctx} vs {big_ctx}")

    # Hold the width still and the pool difference becomes visible, which is
    # the property that matters: the same client sizes differently per host
    # without being told which host it is on.
    small_solo = pins(SMALL, "--fanout", "1").get("kid", {}).get("model", "")
    big_solo = pins(BIG, "--fanout", "1").get("kid", {}).get("model", "")
    report(small_solo.endswith("-64k"),
           "a 131k pool funds one 64k child", small_solo)
    report(big_solo.endswith("-128k"),
           "a 262k pool funds one 128k child", big_solo)
    report(small_solo != big_solo,
           "the tier tracks the card, not a compiled-in default",
           f"{small_solo} vs {big_solo}")

    off = pins(BIG, "--no-pin-subagents")
    report("kid" not in off,
           "--no-pin-subagents leaves children inheriting", str(off))

# --- a variant is never the default -------------------------------------------
# An abliterated preset loads no projector, so gen-preset.py gives it MORE pool
# than the plain model. The seat filter then drops the plain model for funding
# fewer sessions and keeps the variant -- and the "prefer the plainest section"
# ranking never runs, because there is no tie left to break. Observed on a
# 46 GiB card: the default came out abliterated.
VARIANTS = """version = 1

[testmodel]
model = /nonexistent/weights.gguf
c = 131072
parallel = 4
alias = testmodel-128k,testmodel-64k,testmodel-40k

[testmodel-uncensored]
model = /nonexistent/abliterated.gguf
c = 163840
parallel = 4
alias = testmodel-uncensored-128k,testmodel-uncensored-64k,testmodel-uncensored-40k
"""

with tempfile.TemporaryDirectory() as d:
    tmp = Path(d)
    pf = tmp / "variants.ini"
    pf.write_text(VARIANTS)

    cfg = generate(pf, tmp / "v.json")
    chosen = cfg.get("model", "")
    report("uncensored" not in chosen,
           "the default is the plain model even when a variant has more pool",
           chosen)
    report(chosen.split("/")[-1].startswith("testmodel-"),
           "and it is still a tier of that model, not the whole pool", chosen)

    # Children must not cross families either. Only tiers of one LOADED model
    # switch for free; crossing costs a full reload. The variant has the bigger
    # pool here, so a preference would not be enough -- a width can exist where
    # only its tier fits.
    agents = tmp / "kids"
    agents.mkdir()
    (agents / "kid.md").write_text("---\nmode: subagent\n---\nbody\n")
    pinned = generate(pf, tmp / "p2.json", "--agents", str(agents))
    kid = pinned.get("agent", {}).get("kid", {}).get("model", "")
    report("uncensored" not in kid,
           "children stay in the parent's family, not the roomier variant", kid)

    named = generate(pf, tmp / "n.json",
                     "--default-model", "testmodel-uncensored-64k")
    report(named.get("model", "").endswith("testmodel-uncensored-64k"),
           "a variant is still reachable by naming it explicitly",
           named.get("model", ""))

print(f"== configure-opencode: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
