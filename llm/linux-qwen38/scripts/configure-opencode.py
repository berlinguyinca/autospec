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
from pathlib import Path

# A "-NNk" alias is a context TIER: the same loaded model, offered to the client
# under a smaller declared limit. It exists because llama.cpp shares one KV pool
# across slots with no admission control -- over-subscribe it and every
# in-flight session dies together, not just the greedy one. OpenCode compacts a
# session before it reaches the declared limit, so the client is where the pool
# actually gets rationed. See the header of router-presets.ini.
TIER_RE = re.compile(r"-(\d+)k$")

DEFAULT_CONFIG = Path.home() / ".config" / "opencode" / "opencode.json"
DEFAULT_PRESETS = Path("/opt/qwen-vllm/etc/router-presets.ini")


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

    def pool_of(model_id: str) -> int:
        return models[owner[model_id]]["context"] or 0

    roomy = [(e["limit"]["context"], mid) for mid, e in text.items()
             if pool_of(mid) // max(1, e["limit"]["context"])
             >= args.default_seats]
    if roomy:
        return max(roomy)[1]
    # No tier is small enough to seat that many; fall back to the smallest.
    return min((e["limit"]["context"], mid) for mid, e in text.items())[1]


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
                "limit": {"context": ctx or 32768, "output": 32768},
            }

        add(mid, pool)
        # Tiers are aliases of the SAME loaded model, so switching between them
        # in the client costs nothing -- no unload, no reload.
        for alias, limit in sorted(info["tiers"].items(),
                                   key=lambda kv: -kv[1]):
            add(alias, limit)

    cfg["provider"][args.provider] = {
        "name": "Qwen local (llama.cpp router, swaps on model select)",
        "npm": "@ai-sdk/openai-compatible",
        "options": {"baseURL": f"http://{args.host}:{args.port}/v1"},
        "models": entries,
    }

    if args.set_default or "model" not in cfg:
        cfg["model"] = f"{args.provider}/{pick_default(models, entries, owner, args)}"

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

    print(f"config    : {args.config}")
    print(f"endpoint  : http://{args.host}:{args.port}/v1")
    print(f"default   : {cfg['model']}")
    for mid, e in sorted(entries.items()):
        print(f"  {mid:26} ctx={e['limit']['context']:>7} "
              f"vision={e['attachment']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
