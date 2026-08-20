#!/usr/bin/env python3
"""Refuse to start a node whose context does not fit the accelerator.

    capacity-preflight.py --ctx 204800 [--kv q4_0] [--parallel 1]
                          [--weights PATH | --weights-mib N]
                          [--vram-mib N] [--headroom-mib N] [--json]

Sizing a server past its card does not degrade gracefully. llama.cpp allocates
the weights, then fails on a later buffer, aborts, and systemd restarts it into
the same wall: on this workstation that was 54 core dumps in ten minutes, each
one ending at the same 600 MiB allocation. Nothing in the stack asked, before
starting, whether the numbers fit.

The other branch is worse for the same reason it looks better. llama.cpp has an
auto-fitter that would have shrunk the allocation to fit, and it refuses to run
when `n-gpu-layers` is pinned:

    common_fit_params: failed to fit params to free device memory:
      n_gpu_layers already set by user to -2, abort

Unpinning it buys a successful start by moving layers to the CPU, which on a
27B model costs far more than the context it saves. So this gate does not
suggest offloading. It reports the largest context that fits resident, and the
operator lowers the context or the quantisation.

Hardware comes from gpu-registry.py, which answers for CUDA and for Apple
Silicon's unified memory. Where it cannot answer, this exits 2 rather than
assuming a card -- a fleet that spans a 4090, an RTX 6000, H100s and a MacBook
has no default worth guessing.

Exit codes:
    0  fits
   75  does not fit (EX_TEMPFAIL -- the launcher should refuse, not retry)
    2  could not determine (no accelerator reading, no weight size)
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load(path: Path, name: str):
    """Import a sibling whose filename is not a valid module name."""
    spec = importlib.util.spec_from_file_location(name, path)
    if not spec or not spec.loader:
        return None
    mod = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(mod)
    except Exception:                                    # noqa: BLE001
        return None
    return mod


def accelerator_mib() -> tuple[int | None, str]:
    """Free-standing accelerator budget, whichever vendor this host has."""
    reg = _load(HERE / "gpu-registry.py", "gpu_registry")
    if not reg:
        return None, "gpu-registry.py is not importable"
    seen = reg.observe()
    if not seen:
        return None, "no accelerator reported by nvidia-smi or Metal"
    card = max(seen, key=lambda g: g.get("vram_mib") or 0)
    note = card["name"] + (" (unified memory)" if card.get("unified") else "")
    return card.get("vram_mib"), note


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ctx", type=int, required=True,
                    help="context the node is configured to serve, in tokens")
    ap.add_argument("--kv", default="q4_0", choices=["f16", "q8_0", "q4_0"])
    ap.add_argument("--parallel", type=int, default=1)
    ap.add_argument("--weights", type=Path,
                    help="GGUF to size from; its file size is the weight cost")
    ap.add_argument("--weights-mib", type=int)
    ap.add_argument("--vram-mib", type=int,
                    help="override the detected accelerator budget; 0 declares "
                         "this host to have none")
    ap.add_argument("--headroom-mib", type=int, default=0,
                    help="EXTRA margin on top of the sizing model's own. "
                         "Default 0 because gen-preset.plan() already reserves "
                         "2 GiB plus 512 MiB per slot and then takes 80%% of "
                         "what remains; adding more here double-counts and "
                         "costs a whole rung of context. Raise it on a host "
                         "that also drives a desktop, or one whose node sleeps "
                         "and must reclaim its memory later")
    ap.add_argument("--json", action="store_true")
    return ap.parse_args(argv)


def weights_mib(args: argparse.Namespace) -> int | None:
    """Weight cost in MiB: given outright, or the GGUF's own size."""
    if args.weights_mib is not None:
        return args.weights_mib
    if not args.weights:
        return None
    try:
        return args.weights.stat().st_size // (1024 * 1024)
    except OSError as exc:
        print(f"cannot read {args.weights}: {exc}", file=sys.stderr)
        return None


def render(report: dict) -> None:
    print(f"accelerator : {report['accelerator']} — {report['vram_mib']:,} MiB")
    print(f"weights     : {report['weights_mib']:,} MiB")
    print(f"requested   : {report['requested_ctx']:,} tokens at "
          f"kv={report['kv']}, {report['parallel']} slot(s)")
    print(f"largest fit : {report['largest_ctx']:,} tokens"
          + (f" (keeping {report['headroom_mib']} MiB extra headroom)"
             if report["headroom_mib"] else ""))
    if report["fits"]:
        print("verdict     : fits")
        return
    print(f"verdict     : DOES NOT FIT — lower the context to "
          f"{report['largest_ctx']:,}, or use a smaller quantisation.")
    print("              Do NOT unpin n-gpu-layers to make it start: that fits "
          "by moving layers to the CPU, which costs more than the context.")


def main() -> int:
    args = parse_args()
    gen = _load(HERE.parent / "slurm" / "gen-preset.py", "gen_preset")
    if not gen:
        print("gen-preset.py is not importable; cannot price a context",
              file=sys.stderr)
        return 2

    # `is not None`, not truthiness: --vram-mib 0 is a caller saying "this host
    # has no usable accelerator", which must stay undetermined rather than
    # quietly falling back to probing and finding one.
    vram, note = ((args.vram_mib, "given") if args.vram_mib is not None
                  else accelerator_mib())
    if not vram:
        print(f"cannot size this host: {note}", file=sys.stderr)
        return 2

    weights = weights_mib(args)
    if weights is None:
        print("need --weights or --weights-mib", file=sys.stderr)
        return 2

    largest = gen.plan(vram - args.headroom_mib, weights, args.kv, args.parallel)
    report = {"accelerator": note, "vram_mib": vram, "weights_mib": weights,
              "kv": args.kv, "parallel": args.parallel,
              "headroom_mib": args.headroom_mib,
              "requested_ctx": args.ctx, "largest_ctx": largest,
              "fits": largest >= args.ctx}

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        render(report)
    return 0 if report["fits"] else 75


if __name__ == "__main__":
    raise SystemExit(main())
