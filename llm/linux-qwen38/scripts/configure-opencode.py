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
import shutil
import sys
import time
from pathlib import Path

DEFAULT_CONFIG = Path.home() / ".config" / "opencode" / "opencode.json"
DEFAULT_PRESETS = Path("/opt/qwen-vllm/etc/router-presets.ini")


def read_presets(path: Path) -> dict[str, dict]:
    """Return {model_id: {context, vision}} from the router presets INI."""
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
        out[name] = {
            "context": int(ctx) if ctx and str(ctx).isdigit() else None,
            # A preset is vision-capable exactly when it loads a projector.
            "vision": bool(sec.get("mmproj")),
        }
    return out


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

    entries = {}
    for mid, info in sorted(models.items()):
        ctx = info["context"]
        entries[mid] = {
            "name": f"{mid} — {ctx:,} ctx ({'vision' if info['vision'] else 'text'})"
                    if ctx else mid,
            "tool_call": True,
            "reasoning": True,
            # Declared from the presets, never optimistically: a client told a
            # server accepts images when no projector is loaded fails in a way
            # that looks nothing like the cause.
            "attachment": info["vision"],
            "limit": {"context": ctx or 32768, "output": 32768},
        }

    cfg["provider"][args.provider] = {
        "name": "Qwen local (llama.cpp router, swaps on model select)",
        "npm": "@ai-sdk/openai-compatible",
        "options": {"baseURL": f"http://{args.host}:{args.port}/v1"},
        "models": entries,
    }

    # Prefer a text preset as the default: vision presets trade context away.
    if args.set_default or "model" not in cfg:
        text_first = sorted(models, key=lambda m: (models[m]["vision"],
                                                   -(models[m]["context"] or 0)))
        cfg["model"] = f"{args.provider}/{text_first[0]}"

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
