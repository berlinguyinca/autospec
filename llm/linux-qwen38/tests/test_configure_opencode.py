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
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
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


def run(presets: Path, config: Path, *extra: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(GEN), "--presets", str(presets), "--config",
         str(config), "--dry-run", *extra],
        capture_output=True, text=True, timeout=120)


def generate(presets: Path, config: Path, *extra: str) -> dict:
    # --no-server-check: these cases are about what the presets promise. The
    # cases that are about what is LOADED stand up a server below and leave the
    # check on.
    res = run(presets, config, "--no-server-check", *extra)
    if res.returncode != 0:
        raise AssertionError(f"generator failed: {res.stderr.strip()}")
    return json.loads(res.stdout)


class ServedModels(BaseHTTPRequestHandler):
    """A stand-in for llama.cpp's /v1/models. `served` is set per test.

    The real payload carries the same models twice: "data" in OpenAI's shape
    with meta.n_ctx, and "models" in llama.cpp's own with the capability list.
    Values here are `ctx` or `(ctx, capabilities)`.
    """

    served: dict = {}

    def do_GET(self):  # noqa: N802 - stdlib spelling
        spec = {mid: (v if isinstance(v, tuple) else (v, ["completion"]))
                for mid, v in self.served.items()}
        body = json.dumps({
            "data": [{"id": mid, "meta": {"n_ctx": ctx}}
                     for mid, (ctx, _) in spec.items()],
            "models": [{"name": mid, "capabilities": caps}
                       for mid, (_, caps) in spec.items()],
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):  # keep the suite's output readable
        pass


def serving(models: dict):
    """Run a /v1/models endpoint on a free port for the duration of a with."""
    handler = type("Handler", (ServedModels,), {"served": models})
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


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
    # Both cards used to land on the ladder's bottom rung. The floor is why
    # they no longer do: a 40k tier leaves ~30k of input budget, under the
    # 37,873 a primary carries before any work, so it cannot be the parent any
    # more. A bigger parent leaves less to divide, and the 131k pool drops from
    # three nominal sessions to a parent plus one real child -- the two it
    # "lost" were sessions that could never have started. The bigger card still
    # funds children at the bottom rung, because its parent is a smaller share
    # of its pool.
    report(small_ctx.endswith("-64k"),
           "a 131k pool funds a startable parent plus one child at its rung",
           small_ctx)
    report(big_ctx.endswith("-40k"),
           "a 262k pool still funds children at the ladder's bottom rung",
           big_ctx)

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


# --- the reserve has to scale, or the small tiers cannot start ----------------
# This is the bug that shipped: `output` was 32,768 on every tier, so the 40k
# tier offered 40,960 - 32,768 = 8,192 tokens of input. OpenCode did not error.
# It compacted on every turn and never answered -- two runs of seven minutes.
LADDER = """version = 1

[testmodel]
model = /nonexistent/weights.gguf
c = 196608
parallel = 4
alias = testmodel-160k,testmodel-80k,testmodel-48k,testmodel-40k
"""

with tempfile.TemporaryDirectory() as d:
    tmp = Path(d)
    pf = tmp / "ladder.ini"
    pf.write_text(LADDER)

    cfg = generate(pf, tmp / "l.json")
    models = cfg["provider"]["qwen-local"]["models"]

    budgets = {mid: m["limit"]["context"] - m["limit"]["output"]
               for mid, m in models.items()}
    report(all(b > 0 for b in budgets.values()),
           "every tier keeps a positive input budget", str(budgets))
    report(budgets["testmodel-40k"] >= 30720,
           "the 40k tier is not eaten by its own reply reserve",
           f"{budgets['testmodel-40k']:,} tokens of input")
    report(models["testmodel-40k"]["limit"]["output"]
           < models["testmodel-160k"]["limit"]["output"],
           "a smaller tier holds back less for the reply",
           f"{models['testmodel-40k']['limit']['output']} vs "
           f"{models['testmodel-160k']['limit']['output']}")

    # The floor decides what a fresh client LANDS on, not what exists.
    default_id = cfg.get("model", "").split("/")[-1]
    report(budgets.get(default_id, 0) >= 37873,
           "the default tier can seat a p90 session before any work",
           f"{default_id} -> {budgets.get(default_id, 0):,}")
    report({"testmodel-40k", "testmodel-48k"} <= set(models),
           "while the tiers under the floor stay offered for pinned children",
           str(sorted(models)))

    loose_id = generate(pf, tmp / "l0.json",
                        "--floor", "0").get("model", "").split("/")[-1]
    report(budgets.get(loose_id, 0) < 37873,
           "--floor 0 goes back to counting seats and picks a tier no "
           "session can start in",
           f"{loose_id} -> {budgets.get(loose_id, 0):,} tokens of input")

TWO_FAMILIES_INI = LADDER + """
[othermodel]
model = /nonexistent/other.gguf
c = 98304
parallel = 2
alias = othermodel-48k
"""


# --- what is LOADED overrules what the presets promise ------------------------
# llama.cpp answers for a model it does not have: ask for an alias the server
# was never started with and it substitutes whatever is loaded, 200 and no
# error. A client offered that id gets a fluent answer from the wrong tier.
with tempfile.TemporaryDirectory() as d:
    tmp = Path(d)
    TWO_FAMILIES = tmp / "families.ini"
    TWO_FAMILIES.write_text(TWO_FAMILIES_INI)

    httpd = serving({"testmodel": 131072})
    try:
        port = httpd.server_address[1]
        res = run(TWO_FAMILIES, tmp / "s.json", "--port", str(port))
        report(res.returncode == 0, "the generator runs against a live server",
               res.stderr.strip()[-200:])
        models = json.loads(res.stdout)["provider"]["qwen-local"]["models"]

        # A "-NNk" alias of a LOADED model survives even though the server
        # never declared it: it resolves to those same weights, and the number
        # is a cap the client enforces. Verified against the real node --
        # `-80k` is not in its /v1/models and drives it correctly.
        report({"testmodel", "testmodel-80k", "testmodel-40k"} <= set(models),
               "tiers of a loaded model stay offered as client-side caps",
               str(sorted(models)))
        # A family that is NOT loaded is the substitution trap: ask for it and
        # the plain model answers, from weights nobody chose.
        report(not any(m.startswith("othermodel") for m in models),
               "a model the server has not loaded is dropped entirely",
               str(sorted(models)))
        report("othermodel" in res.stderr,
               "and the dropped ones are named, not silently omitted")
        report(models["testmodel"]["limit"]["context"] == 131072,
               "a tier declaring more than the server serves is capped",
               str(models["testmodel"]["limit"]))
        report(models["testmodel"]["limit"]["output"] <= 32768
               and models["testmodel"]["limit"]["context"]
               - models["testmodel"]["limit"]["output"] > 0,
               "and its reserve is re-derived from the capped window",
               str(models["testmodel"]["limit"]))
    finally:
        httpd.shutdown()

    # The presets can be wrong about modality in the direction that costs you
    # something: this node's text preset loads no projector on paper, while the
    # loaded model reports "multimodal" and reads images correctly. Dropping
    # its vision family (not loaded) must not also drop images.
    httpd = serving({"testmodel": (131072, ["completion", "multimodal"])})
    try:
        res = run(TWO_FAMILIES, tmp / "v.json",
                  "--port", str(httpd.server_address[1]))
        models = json.loads(res.stdout)["provider"]["qwen-local"]["models"]
        report(all(m["attachment"] for m in models.values()),
               "a loaded projector makes the served tiers multimodal",
               str({k: v["attachment"] for k, v in models.items()}))
    finally:
        httpd.shutdown()

    httpd = serving({"testmodel": 131072})
    try:
        res = run(TWO_FAMILIES, tmp / "t.json",
                  "--port", str(httpd.server_address[1]))
        models = json.loads(res.stdout)["provider"]["qwen-local"]["models"]
        report(not any(m["attachment"] for m in models.values()),
               "and a model without one is still declared text-only",
               str({k: v["attachment"] for k, v in models.items()}))
    finally:
        httpd.shutdown()

    # A server that serves nothing from the presets is a wiring mistake, not a
    # config to write: a client pointed at it would ask for ids that only
    # resolve by substitution.
    httpd = serving({"someone-elses-model": 131072})
    try:
        res = run(TWO_FAMILIES, tmp / "n.json",
                  "--port", str(httpd.server_address[1]))
        report(res.returncode == 1,
               "a server sharing no id with the presets is refused",
               f"exit {res.returncode}")
    finally:
        httpd.shutdown()

    # Provisioning happens before the server is up; that must warn, not fail.
    res = run(TWO_FAMILIES, tmp / "d.json", "--port", "1")
    report(res.returncode == 0 and "unverified" in res.stderr,
           "an unreachable server warns and falls back to the presets",
           res.stderr.strip()[-120:])


print(f"== configure-opencode: {passed} passed, {failed} failed ==")
sys.exit(1 if failed else 0)
