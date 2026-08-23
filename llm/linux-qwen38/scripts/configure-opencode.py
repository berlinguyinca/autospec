#!/usr/bin/env python3
"""Point OpenCode at this node's router endpoint, idempotently.

    configure-opencode.py [--config PATH] [--port 8080] [--provider qwen-local]
                          [--set-default] [--dry-run]

Reads the router presets to discover which models exist and what each one's
context and vision capability are, so the client cannot drift from the server.

Why this is a script and not a documented JSON snippet: moving the endpoint
silently breaks every configured client, and hand-editing is where that mistake
happens. It also writes a timestamped backup, because this file usually holds
other providers and API keys that must survive.
"""
from __future__ import annotations

import argparse
import configparser
import json
import re
import shutil
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

# Shared with context-budget-check.py so the pinner and the pricer cannot
# disagree about which agents inherit the parent tier.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from agentmodes import spawnable  # noqa: E402

# A "-NNk" alias is a context TIER: the same loaded model, offered to the client
# under a smaller declared limit. It exists because llama.cpp shares one KV pool
# across slots with no admission control -- over-subscribe it and every
# in-flight session dies together, not just the greedy one. OpenCode compacts a
# session before it reaches the declared limit, so the client is where the pool
# actually gets rationed. See the header of router-presets.ini.
TIER_RE = re.compile(r"-(\d+)k$")

# The floor a PRIMARY session starts at: system prompt, project instructions,
# memory, skill and MCP tool schemas, before a single line of work. Measured
# with analyze-session-contexts.py across this operator's own OpenCode sessions
# -- 14,492 tokens at the median but 37,873 at p90, and 39,843 sampled off
# /slots for one plain question asked inside the autospec repo. A tier whose
# INPUT budget sits under this cannot start a session; it does not error, it
# compacts on every turn and never answers.
DEFAULT_PRIMARY_FLOOR = 37873

DEFAULT_CONFIG = Path.home() / ".config" / "opencode" / "opencode.json"
DEFAULT_PRESETS = Path("/opt/qwen-vllm/etc/router-presets.ini")
DEFAULT_AGENTS = Path.home() / ".config" / "opencode" / "agent"


def output_reserve(ctx: int) -> int:
    """How much of a tier to hold back for the reply.

    The client spends `context - output` on input, so a reserve copied
    unchanged across every tier quietly eats the small ones: 32,768 held back
    from a 40,960 tier leaves 8,192 tokens to say everything in, which is less
    than any real session's opening context. The failure is silent -- OpenCode
    compacts, retries, exceeds again, and never answers -- so the reserve has
    to scale with the window rather than be a constant.
    """
    return max(4096, min(32768, ctx // 4))


def read_served(base_url: str, timeout: float) -> dict[str, dict] | None:
    """{served id: {n_ctx, vision}} from the server's /v1/models, or None.

    The presets say what a deployment COULD serve; this says what is loaded and
    answering right now, and only the second one bills. They diverge in the way
    that hurts: llama.cpp answers for a model it does not have, substituting
    whatever is loaded, 200 and no error. So a client offered an id the server
    never declared gets a fluent reply from the wrong tier and no way to tell.
    """
    url = base_url.rstrip("/") + "/models"
    try:
        with urllib.request.urlopen(url, timeout=timeout) as fh:
            payload = json.loads(fh.read().decode())
    except (urllib.error.URLError, OSError, ValueError, TimeoutError):
        return None
    # Two shapes in one payload: "data" carries the OpenAI-style entries with
    # meta.n_ctx, "models" carries llama.cpp's capability list. A model that
    # loaded a projector says so here, which is how a client learns it may send
    # images without the presets having to be right about it.
    caps = {m.get("name"): set(m.get("capabilities") or [])
            for m in payload.get("models") or [] if m.get("name")}
    served: dict[str, dict] = {}
    for item in payload.get("data") or []:
        mid = item.get("id")
        if not mid:
            continue
        meta = item.get("meta") or {}
        ctx = meta.get("n_ctx")
        served[mid] = {
            "n_ctx": int(ctx) if isinstance(ctx, int) and ctx > 0 else 0,
            "vision": bool(caps.get(mid, set()) & {"multimodal", "vision"}),
        }
    return served or None


def read_presets(path: Path) -> dict[str, dict]:
    """Return {model_id: {context, vision, parallel, tiers}} from the presets."""
    ini = configparser.ConfigParser(strict=False)
    ini.optionxform = str  # keys are CLI flags; do not lowercase
    # llama.cpp allows top-level keys before the first section ("version = 1").
    # configparser refuses that with MissingSectionHeaderError, so synthesise a
    # header rather than editing a file the server is the authority on.
    ini.read_string("[__top__]\n" + path.read_text())

    globals_ = dict(ini["*"]) if ini.has_section("*") else {}
    out: dict[str, dict] = {}
    for name in ini.sections():
        # "*" is llama.cpp's shared-defaults section; "__top__" is the header we
        # synthesised above for the file's leading keys. Neither is a model.
        if name in ("*", "__top__"):
            continue
        sec = {**globals_, **dict(ini[name])}
        ctx = sec.get("c") or sec.get("ctx-size")
        pool = int(ctx) if ctx and str(ctx).isdigit() else None
        par = sec.get("parallel")
        tiers = {}
        for alias in (a.strip() for a in sec.get("alias", "").split(",")):
            m = TIER_RE.search(alias) if alias else None
            if not m:
                continue
            limit = int(m.group(1)) * 1024
            # A tier larger than the pool cannot be honoured by any client
            # discipline, so it is a configuration error, not a preference.
            if pool and limit > pool:
                print(f"skipping {alias}: {limit:,} exceeds the "
                      f"{pool:,} pool", file=sys.stderr)
                continue
            tiers[alias] = limit
        out[name] = {
            "context": pool,
            # A preset is vision-capable exactly when it loads a projector.
            "vision": bool(sec.get("mmproj")),
            "parallel": int(par) if par and str(par).isdigit() else 1,
            "tiers": tiers,
        }
    return out


def pick_default(models: dict, entries: dict, owner: dict, args) -> str:
    """Choose the model id a fresh client lands on.

    Not the biggest context. A default that claims the whole pool means the
    second window you open has nothing to draw on, and over-subscribing the
    pool kills every live session rather than the newest one. So the default is
    the roomiest tier that still leaves room for --default-seats sessions;
    someone who wants one huge session picks the solo tier deliberately.
    """
    if args.default_model:
        if args.default_model not in entries:
            print(f"--default-model {args.default_model} is not a served id",
                  file=sys.stderr)
            raise SystemExit(1)
        return args.default_model

    # Vision presets spend part of the budget on a projector, so they are never
    # the default; a client that defaults to one silently serves less context.
    text = {mid: e for mid, e in entries.items()
            if not models[owner[mid]]["vision"]}
    if not text:
        text = entries

    # A variant is never the default, and this has to be an exclusion rather
    # than a tie-break. An abliterated preset loads no projector, so it is given
    # MORE pool than the plain model -- 163840 against 131072 on a 46 GiB card.
    # The seat filter below then drops the plain model for funding fewer
    # sessions and keeps the variant, and the "prefer the plainest section"
    # ranking never runs because there is no tie left to break. That silently
    # made an abliterated model the default for every new session.
    #
    # The base is the shortest section name: every variant is that name plus a
    # suffix. Callers who want a variant name it with --default-model.
    base = min((owner[mid] for mid in text), key=lambda n: (len(n), n),
               default=None)
    plain = {mid: e for mid, e in text.items() if owner[mid] == base}
    if plain:
        text = plain

    # A tier a session cannot START in must not be the one a fresh client
    # lands on. This filters the default only: the small tiers stay offered,
    # because a pinned subagent floors far lower than a primary does (a stock
    # child measured 37,113 and the same child with the skill tool denied
    # 18,030 -- the skill roster alone is ~16.3k), and those pins are the whole
    # point of the ladder.
    if args.floor:
        startable = {mid: e for mid, e in text.items()
                     if e["limit"]["context"] - e["limit"]["output"] >= args.floor}
        if startable:
            text = startable
        else:
            print(f"no tier leaves {args.floor:,} tokens of input budget; "
                  f"defaulting to the roomiest anyway", file=sys.stderr)

    def pool_of(model_id: str) -> int:
        return models[owner[model_id]]["context"] or 0

    # Tie-break toward the PLAINEST preset, by shortest owning section name.
    # Sorting ids as strings put "qwen3.8-27b-uncensored-40k" above
    # "qwen3.8-27b-40k" at equal context, which silently made an abliterated
    # model the default for every new session. A default must be the boring
    # choice; a variant is something you opt into.
    def rank(mid: str, ctx: int) -> tuple:
        return (ctx, -len(owner[mid]), -len(mid))

    roomy = [mid for mid, e in text.items()
             if pool_of(mid) // max(1, e["limit"]["context"])
             >= args.default_seats]
    if roomy:
        return max(roomy, key=lambda m: rank(m, text[m]["limit"]["context"]))
    # No tier is small enough to seat that many; fall back to the smallest.
    return min(text, key=lambda m: (text[m]["limit"]["context"], len(owner[m])))


def fit_child_tier(text_tiers: dict[str, int], models: dict, owner: dict,
                   entries: dict, default_model: str, provider: str,
                   fanout: int | None) -> tuple[str | None, int]:
    """Largest child tier a full fan-out still fits inside the pool.

        parent + width * child  <=  pool

    Everything here comes from the server's own numbers -- `c` and `parallel`,
    which gen-preset.py sized to whatever this host has, be it 24 GiB, 96 GiB,
    an H100 or a MacBook's unified memory. The client never learns which.

    Slot count is a ceiling on width, not a target: a card can have more slots
    than the pool can fund at any useful tier. So width walks down from the
    slots until some tier fits, and the width actually priced is returned with
    it. If even width 1 leaves nothing, the smallest tier is still better than
    leaving children unpinned -- unpinned means inheriting the parent, which is
    never smaller.
    """
    if not text_tiers:
        return None, 0
    parent_id = default_model.split("/", 1)[-1] if default_model else ""
    section = owner.get(parent_id)
    pool = models[section]["context"] if section else None
    slots = models[section]["parallel"] if section else 1
    if not pool:
        return min(text_tiers,
                   key=lambda m: (text_tiers[m], len(owner[m]), len(m))), 0
    parent_ctx = entries.get(parent_id, {}).get("limit", {}).get("context", 0)

    # Children stay in the parent's family, and this is a restriction rather
    # than a preference. Only tiers of one LOADED model switch for free;
    # crossing families costs a full reload of the weights. Preferring the
    # parent's section on a tie is not enough, because the families do not have
    # equal pools -- an abliterated preset loads no projector and is given more,
    # so a width can exist where only its tier fits and every child silently
    # moves onto it. That is the same asymmetry that made it the default.
    own = {mid: ctx for mid, ctx in text_tiers.items() if owner[mid] == section}
    if own:
        text_tiers = own

    def rank(mid: str) -> tuple:
        # Within the family, biggest that fits; ties to the plainest id.
        return (text_tiers[mid], -len(owner[mid]), -len(mid))

    for width in range(max(1, slots - 1) if fanout is None else fanout, 0, -1):
        fits = [m for m in text_tiers if parent_ctx + width * text_tiers[m] <= pool]
        if fits:
            return max(fits, key=rank), width
    return min(text_tiers, key=lambda m: (text_tiers[m], len(m))), 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--presets", type=Path, default=DEFAULT_PRESETS)
    ap.add_argument("--port", default="8080")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--provider", default="qwen-local",
                    help="provider key; keep it stable so an existing default "
                         "'model' string keeps resolving")
    ap.add_argument("--set-default", action="store_true",
                    help="also set the top-level default model")
    ap.add_argument("--default-model",
                    help="model id to make the default; overrides the "
                         "automatic choice")
    ap.add_argument("--default-seats", type=int, default=4,
                    help="the automatic default is the roomiest text tier that "
                         "still funds this many concurrent sessions")
    ap.add_argument("--api-key",
                    help="bearer token the server requires; needed for a "
                         "remote node, where binding an open port on a shared "
                         "network without one exposes the model to every other "
                         "user of that network")
    ap.add_argument("--no-solo-tier", action="store_true",
                    help="do not offer the bare pool-sized model id. It is the "
                         "one tier a subagent must never inherit: children take "
                         "the parent's model id, so a parent on the whole pool "
                         "hands every child a window the size of the pool. Drop "
                         "it and the largest selectable window is the -NNk tier "
                         "below it, which leaves the pool room for fan-out")
    ap.add_argument("--provider-name",
                    help="human-readable provider label in the client")
    ap.add_argument("--no-pin-subagents", action="store_true",
                    help="leave spawnable agents without a model. They then "
                         "inherit the parent's tier, so a parent that picked a "
                         "big window by hand hands the same window to every "
                         "child it spawns -- width multiplies the largest "
                         "window instead of a chosen one")
    ap.add_argument("--fanout", type=int,
                    help="how many children may be live at once when choosing "
                         "the child tier (default: the server's slot count "
                         "minus the parent's own slot)")
    ap.add_argument("--child-tier",
                    help="model id to pin spawnable agents to "
                         "(default: the smallest text tier offered)")
    ap.add_argument("--agents", type=Path, default=DEFAULT_AGENTS,
                    help="directory of agent definitions to pin")
    ap.add_argument("--floor", type=int, default=DEFAULT_PRIMARY_FLOOR,
                    help="tokens a primary session carries before any work. A "
                         "tier whose input budget (context minus output) is "
                         "under this is still offered but never made the "
                         "default; 0 disables the check. Measure yours with "
                         "analyze-session-contexts.py rather than guessing")
    ap.add_argument("--no-server-check", action="store_true",
                    help="do not ask the server which models it actually "
                         "serves. The check exists because llama.cpp answers "
                         "for a model it does not have by substituting "
                         "whatever is loaded, so an id the server never "
                         "declared looks like it works. Skip it only when "
                         "generating a config before the server is up")
    ap.add_argument("--server-timeout", type=float, default=5.0,
                    help="seconds to wait for /v1/models")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    if not args.presets.is_file():
        print(f"presets not found: {args.presets}", file=sys.stderr)
        return 1
    models = read_presets(args.presets)
    if not models:
        print(f"no model presets in {args.presets}", file=sys.stderr)
        return 1

    cfg = {}
    if args.config.is_file():
        cfg = json.loads(args.config.read_text())
    cfg.setdefault("$schema", "https://opencode.ai/config.json")
    cfg.setdefault("provider", {})

    entries: dict[str, dict] = {}
    # entry id -> the preset section that serves it. Derived here rather than
    # by prefix matching later: "qwen3.8-27b" is a prefix of
    # "qwen3.8-27b-abliterated", so string matching silently files the
    # abliterated tiers under the wrong (text) preset.
    owner: dict[str, str] = {}
    for mid, info in sorted(models.items()):
        pool, par = info["context"], info["parallel"]

        def add(model_id: str, ctx: int | None) -> None:
            kind = "vision" if info["vision"] else "text"
            if ctx and pool:
                # How many sessions of this size the pool funds, and how many
                # the server can decode at once. The smaller number is the one
                # that holds, so show it.
                seats = min(max(1, pool // ctx), par)
                label = (f"{ctx:,} ctx · up to {seats} concurrent"
                         if seats > 1 else f"{ctx:,} ctx · solo")
            else:
                label = kind
            owner[model_id] = mid
            entries[model_id] = {
                "name": f"{model_id} — {label} ({kind})",
                "tool_call": True,
                "reasoning": True,
                # Declared from the presets, never optimistically: a client
                # told a server accepts images when no projector is loaded
                # fails in a way that looks nothing like the cause.
                "attachment": info["vision"],
                "limit": {"context": ctx or 32768,
                          "output": output_reserve(ctx or 32768)},
            }

        if not args.no_solo_tier:
            add(mid, pool)
        # Tiers are aliases of the SAME loaded model, so switching between them
        # in the client costs nothing -- no unload, no reload.
        for alias, limit in sorted(info["tiers"].items(),
                                   key=lambda kv: -kv[1]):
            add(alias, limit)

    base_url = f"http://{args.host}:{args.port}/v1"
    if not args.no_server_check:
        served = read_served(base_url, args.server_timeout)
        if served is None:
            print(f"warning: {base_url}/models did not answer; the config "
                  f"below is what the PRESETS promise, unverified against "
                  f"what is loaded", file=sys.stderr)
        else:
            # Keep a family iff its own model id is loaded. A "-NNk" alias of a
            # loaded model is not a lie even when the server never declared it:
            # it resolves to those same weights and the number is a cap the
            # CLIENT enforces, which is the whole mechanism. A family the
            # server does not serve is a different matter -- ask for a vision
            # or abliterated id that is not loaded and llama.cpp substitutes
            # the plain model, 200 and no error, and the answer comes from
            # weights nobody chose.
            dropped = sorted(mid for mid in entries if owner[mid] not in served)
            families = sorted({owner[mid] for mid in dropped})
            for mid in dropped:
                del entries[mid]
                owner.pop(mid, None)
            if dropped:
                print(f"dropping {len(dropped)} id(s) from {len(families)} "
                      f"model(s) the server has not loaded "
                      f"({', '.join(families)}): {', '.join(dropped)}",
                      file=sys.stderr)
                print("  asking for one of those does not fail -- the server "
                      "substitutes whatever IS loaded, so the answer comes "
                      "from weights nobody chose. Load them, or leave them out.",
                      file=sys.stderr)
            for mid, entry in entries.items():
                info = served.get(owner[mid]) or {}
                # The server loaded a projector for a model the presets call
                # text-only. Believe the server: dropping its vision family
                # would otherwise take away images this node demonstrably
                # serves, and the tier is a cap, not a modality.
                if info.get("vision") and not entry["attachment"]:
                    print(f"marking {mid} multimodal: the loaded model has a "
                          f"projector", file=sys.stderr)
                    entry["attachment"] = True
                live = info.get("n_ctx") or 0
                if live and entry["limit"]["context"] > live:
                    print(f"capping {mid}: {entry['limit']['context']:,} "
                          f"declared, {live:,} served", file=sys.stderr)
                    entry["limit"]["context"] = live
                    entry["limit"]["output"] = output_reserve(live)
            if not entries:
                print(f"the server at {base_url} serves none of the models in "
                      f"{args.presets}", file=sys.stderr)
                return 1

    options = {"baseURL": base_url}
    if args.api_key:
        options["apiKey"] = args.api_key
    cfg["provider"][args.provider] = {
        "name": args.provider_name
                or "Qwen local (llama.cpp router, swaps on model select)",
        "npm": "@ai-sdk/openai-compatible",
        "options": options,
        "models": entries,
    }

    if args.set_default or "model" not in cfg:
        cfg["model"] = f"{args.provider}/{pick_default(models, entries, owner, args)}"

    # Pin the children. A spawnable agent with no model of its own inherits the
    # parent's, so the worst case is width x whatever the parent happened to
    # pick -- including the whole pool, which the TUI remembers per session and
    # no config file records. Pinning does not shrink today's windows; it makes
    # them independent of a choice made elsewhere, which is what makes the
    # budget in context-budget-check.py enforceable rather than advisory.
    pin_summary = ""
    if not args.no_pin_subagents:
        # Derived from the server, never from a guess about the hardware.
        # gen-preset.py already sized `c` and `parallel` to whatever card this
        # host has -- 24 GiB, 96 GiB, an H100, or a MacBook's unified memory --
        # so solving the budget against those two numbers is correct on all of
        # them without the client knowing which one it is. Taking the SMALLEST
        # tier would be safe everywhere and wasteful on a big card: a 96 GiB
        # pool can fund 64k children where a 24 GiB pool cannot.
        text_tiers = {mid: e["limit"]["context"] for mid, e in entries.items()
                      if not models[owner[mid]]["vision"]
                      and mid in owner and TIER_RE.search(mid)}
        child, priced_width = fit_child_tier(
            text_tiers, models, owner, entries, cfg.get("model", ""),
            args.provider, args.fanout)
        if args.child_tier:
            child, priced_width = args.child_tier, 0
        if child and child not in entries:
            print(f"--child-tier {child} is not a served id", file=sys.stderr)
            return 1
        if child:
            agent_cfg = cfg.setdefault("agent", {})
            pinned = []
            for kid in spawnable(args.agents, agent_cfg, None):
                # Never override a deliberate choice -- only fill the gap that
                # inheritance would otherwise fill for us.
                if kid["model"]:
                    continue
                agent_cfg.setdefault(kid["name"], {})["model"] = \
                    f"{args.provider}/{child}"
                pinned.append(kid["name"])
            # Reported after the dry-run branch below: --dry-run's stdout is
            # parsed as JSON by callers and by the test suite, so nothing may
            # precede it there.
            pin_summary = (f"pinned    : {len(pinned)} spawnable agent(s) to "
                           f"{child} ({', '.join(sorted(pinned)[:4])}"
                           f"{', ...' if len(pinned) > 4 else ''})"
                           f"{f' — priced for {priced_width} concurrent' if priced_width else ''}"
                           ) if pinned else ""

    rendered = json.dumps(cfg, indent=2, sort_keys=True) + "\n"
    if args.dry_run:
        print(rendered)
        return 0

    args.config.parent.mkdir(parents=True, exist_ok=True)
    if args.config.is_file():
        backup = args.config.with_suffix(
            f".json.bak-{time.strftime('%Y%m%d-%H%M%S')}")
        shutil.copy2(args.config, backup)
        print(f"backup    : {backup}")
    args.config.write_text(rendered)

    if pin_summary:
        print(pin_summary)
    print(f"config    : {args.config}")
    print(f"endpoint  : http://{args.host}:{args.port}/v1")
    print(f"default   : {cfg['model']}")
    for mid, e in sorted(entries.items()):
        print(f"  {mid:26} ctx={e['limit']['context']:>7} "
              f"vision={e['attachment']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
