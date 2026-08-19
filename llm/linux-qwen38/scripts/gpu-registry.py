#!/usr/bin/env python3
"""The collection of GPUs this project has run on, and what they can do.

    gpu-registry.py record [--site NAME] [--out FILE]   observe this machine
    gpu-registry.py merge OBSERVED.jsonl                fold observations in
    gpu-registry.py show [--name NAME]                  what do we know
    gpu-registry.py predict --weights-gib N [--name X]  expected tok/s
    gpu-registry.py benchmark --base URL --model ID ...  measure and record

Every serving job calls `record`, so a card nobody has seen before is captured
the first time a job lands on it instead of being reconstructed from memory
afterwards. That matters here because the GPU is chosen dynamically -- this
project met four different cards in a single session, and their specs were
otherwise being retyped into three separate places.

What is observed and what is assumed are kept apart on purpose:

    vram_mib, compute_cap   read from the device
    measured_tps            benchmarked, per quantisation
    bandwidth_gbs           the vendor number, and the only assumption here

`predict` is bandwidth / resident weight bytes x an efficiency constant. That
constant is not a guess either: two cards a generation and 4x of memory apart
both landed within 0.3 points of 80% of their roofline.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

DEFAULT = Path(__file__).resolve().parent.parent / "config" / "gpu-registry.json"
# Vendor bandwidth for cards we can identify but have not benchmarked. Without
# this a newly-seen GPU has no prediction at all; with it the prediction is
# labelled as resting on a spec sheet.
KNOWN_BANDWIDTH = {
    "NVIDIA GeForce RTX 4090": 1008, "NVIDIA GeForce RTX 5090": 1792,
    "NVIDIA GeForce RTX 3090": 936, "NVIDIA L40S": 864, "NVIDIA L40": 864,
    "NVIDIA RTX A6000": 768, "NVIDIA RTX 5000 Ada Generation": 576,
    "NVIDIA A100-SXM4-80GB": 2039, "NVIDIA A100 80GB PCIe": 1935,
    "NVIDIA A100-PCIE-40GB": 1555, "NVIDIA A100-SXM4-40GB": 1555,
    "NVIDIA H100 80GB HBM3": 3350, "NVIDIA H100 PCIe": 2000,
    "NVIDIA RTX PRO 6000 Blackwell Max-Q Workstation Edition": 1792,
    "NVIDIA RTX PRO 6000 Blackwell Workstation Edition": 1792,
}
# FP8 needs Ada (8.9) or newer; NVFP4 needs Blackwell (10.0+).
def caps(cc: str) -> tuple[bool, bool]:
    try:
        major, minor = (int(x) for x in cc.split(".")[:2])
    except ValueError:
        return (False, False)
    v = major * 10 + minor
    return (v >= 89, major >= 10)


def load(path: Path) -> dict:
    if path.is_file():
        return json.loads(path.read_text())
    return {"roofline_efficiency": 0.80, "gpus": {}}


def observe() -> list[dict]:
    """What nvidia-smi says about the GPUs actually present."""
    if not shutil.which("nvidia-smi"):
        return []
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-gpu=name,memory.total,compute_cap,driver_version",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=60).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    seen = []
    for line in out.strip().splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) < 3:
            continue
        name, mib, cc = parts[0], parts[1], parts[2]
        if not re.fullmatch(r"\d+", mib):
            continue
        seen.append({"name": name, "vram_mib": int(mib), "compute_cap": cc,
                     "driver": parts[3] if len(parts) > 3 else ""})
    return seen


def upsert(reg: dict, obs: dict, site: str) -> bool:
    """Add or refine one GPU. Returns True if anything changed."""
    gpus = reg.setdefault("gpus", {})
    name = obs["name"]
    entry = gpus.get(name)
    changed = False
    if entry is None:
        fp8, nvfp4 = caps(obs["compute_cap"])
        entry = {
            "vram_mib": obs["vram_mib"],
            "compute_cap": obs["compute_cap"],
            # A card we have never met gets the vendor figure if we recognise
            # it, and null if we do not -- never a guess dressed as a fact.
            "bandwidth_gbs": KNOWN_BANDWIDTH.get(name),
            "fp8": fp8, "nvfp4": nvfp4,
            "measured_tps": {},
            "seen_at": [],
            "first_seen": time.strftime("%Y-%m-%d"),
        }
        gpus[name] = entry
        changed = True
    # Observed fields win over stored ones: the device is the authority.
    for k in ("vram_mib", "compute_cap"):
        if entry.get(k) != obs[k]:
            entry[k] = obs[k]
            changed = True
    if site and site not in entry.setdefault("seen_at", []):
        entry["seen_at"].append(site)
        changed = True
    return changed


def cmd_record(args) -> int:
    seen = observe()
    if not seen:
        print("no NVIDIA GPU visible here", file=sys.stderr)
        return 1
    site = args.site or socket.getfqdn()
    if args.out:
        # Append-only observation log, for hosts that cannot write the repo --
        # a compute node has the registry read-only over a shared filesystem.
        with open(args.out, "a") as fh:
            for o in seen:
                fh.write(json.dumps({**o, "site": site}) + "\n")
        print(f"recorded {len(seen)} GPU(s) to {args.out}")
        return 0
    reg = load(args.registry)
    changed = any(upsert(reg, o, site) for o in seen)
    if changed:
        args.registry.write_text(json.dumps(reg, indent=2) + "\n")
    for o in seen:
        print(f"{o['name']}  {o['vram_mib']} MiB  cc {o['compute_cap']}")
    print("registry updated" if changed else "registry already current")
    return 0


def cmd_merge(args) -> int:
    reg = load(args.registry)
    changed = False
    added = []
    for line in Path(args.observed).read_text().splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            o = json.loads(line)
        except json.JSONDecodeError:
            continue
        if o["name"] not in reg.get("gpus", {}):
            added.append(o["name"])
        changed |= upsert(reg, o, o.get("site", ""))
    if changed:
        args.registry.write_text(json.dumps(reg, indent=2) + "\n")
    for n in added:
        print(f"new GPU: {n}")
    print("registry updated" if changed else "registry already current")
    return 0


def cmd_show(args) -> int:
    reg = load(args.registry)
    gpus = reg.get("gpus", {})
    if args.name:
        gpus = {k: v for k, v in gpus.items() if args.name.lower() in k.lower()}
    print(f"{'GPU':<52} {'VRAM':>8} {'cc':>5} {'GB/s':>6}  measured")
    print("-" * 92)
    for name, g in sorted(gpus.items(), key=lambda kv: -(kv[1].get("vram_mib") or 0)):
        m = ", ".join(f"{k.split('-')[-1]}={v}" for k, v in
                      (g.get("measured_tps") or {}).items()) or "-"
        bw = g.get("bandwidth_gbs")
        print(f"{name:<52} {g.get('vram_mib', 0):>7} M {g.get('compute_cap', '?'):>5} "
              f"{bw if bw else '?':>6}  {m}")
    return 0


def cmd_predict(args) -> int:
    reg = load(args.registry)
    eff = reg.get("roofline_efficiency", 0.80)
    gib = args.weights_gib
    gb = gib * 1.073741824
    rows = []
    for name, g in reg.get("gpus", {}).items():
        if args.name and args.name.lower() not in name.lower():
            continue
        bw = g.get("bandwidth_gbs")
        if not bw:
            continue
        fits = (g.get("vram_mib") or 0) >= (gib + 12) * 1024
        rows.append((bw / gb * eff, name, g.get("vram_mib"), fits))
    rows.sort(reverse=True)
    print(f"predicted tok/s for {gib:.2f} GiB of weights "
          f"(roofline x {eff:.0%}); 'fits' allows ~12 GiB for KV and compute\n")
    print(f"{'GPU':<52} {'tok/s':>7}  fits")
    print("-" * 70)
    for tps, name, vram, fits in rows:
        print(f"{name:<52} {tps:>7.1f}  {'yes' if fits else 'NO'}")
    return 0


def cmd_benchmark(args) -> int:
    """Measure single-stream decode against a live endpoint and record it.

    A registry of spec sheets predicts; a registry with measurements calibrates.
    The efficiency constant only means something because two cards were actually
    benchmarked, so every new card should contribute one.
    """
    import urllib.error
    import urllib.request

    tps = args.tps
    if tps is None:
        body = json.dumps({
            "model": args.model, "temperature": 0,
            "max_tokens": args.max_tokens, "stream": True,
            # Fixed-length decode: a model that stops after three tokens
            # reports a fine-looking rate computed over nothing.
            "ignore_eos": True,
            "chat_template_kwargs": {"enable_thinking": False},
            "messages": [{"role": "user", "content": "Count upward from one."}],
        }).encode()
        headers = {"Content-Type": "application/json"}
        if args.api_key:
            headers["Authorization"] = f"Bearer {args.api_key}"
        req = urllib.request.Request(f"{args.base}/v1/chat/completions",
                                     data=body, headers=headers)
        first = None
        n = 0
        try:
            with urllib.request.urlopen(req, timeout=1800) as resp:
                for raw in resp:
                    line = raw.decode("utf-8", "replace").strip()
                    if not line.startswith("data:"):
                        continue
                    payload = line[5:].strip()
                    if payload == "[DONE]":
                        break
                    try:
                        chunk = json.loads(payload)
                    except json.JSONDecodeError:
                        continue
                    ch = (chunk.get("choices") or [{}])[0]
                    d = ch.get("delta") or {}
                    if not (d.get("content") or d.get("reasoning_content")):
                        continue
                    if first is None:
                        first = time.perf_counter()
                    n += 1
        except (urllib.error.HTTPError, urllib.error.URLError, OSError) as exc:
            print(f"benchmark failed: {exc}", file=sys.stderr)
            return 1
        if first is None or n < 2:
            print("no tokens decoded", file=sys.stderr)
            return 1
        # Timed from the FIRST token, so prefill and queueing do not contaminate
        # a decode-rate figure.
        tps = (n - 1) / (time.perf_counter() - first)

    reg = load(args.registry)
    gpus = reg.setdefault("gpus", {})
    name = args.gpu or (observe() or [{}])[0].get("name")
    if not name:
        print("could not determine the GPU; pass --gpu", file=sys.stderr)
        return 1
    entry = gpus.get(name)
    if entry is None:
        print(f"{name} is not in the registry; run `record` there first",
              file=sys.stderr)
        return 1
    entry.setdefault("measured_tps", {})[args.quant] = round(tps, 2)
    args.registry.write_text(json.dumps(reg, indent=2) + "\n")

    bw = entry.get("bandwidth_gbs")
    print(f"{name}\n  {args.quant}: {tps:.2f} tok/s recorded")
    if bw and args.weights_gib:
        roof = bw / (args.weights_gib * 1.073741824)
        print(f"  roofline {roof:.1f} -> {tps / roof:.1%} of it "
              f"(the registry assumes {reg.get('roofline_efficiency', 0.8):.0%})")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--registry", type=Path, default=DEFAULT)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record"); r.add_argument("--site"); r.add_argument("--out")
    r.set_defaults(fn=cmd_record)
    m = sub.add_parser("merge"); m.add_argument("observed")
    m.set_defaults(fn=cmd_merge)
    s = sub.add_parser("show"); s.add_argument("--name")
    s.set_defaults(fn=cmd_show)
    b = sub.add_parser("benchmark")
    b.add_argument("--base", default="http://127.0.0.1:8080")
    b.add_argument("--model", required=True)
    b.add_argument("--quant", required=True,
                   help="weights filename this rate belongs to")
    b.add_argument("--api-key", default="")
    b.add_argument("--max-tokens", type=int, default=128)
    b.add_argument("--tps", type=float, help="record an externally measured rate")
    b.add_argument("--gpu", help="GPU name, if not benchmarking locally")
    b.add_argument("--weights-gib", type=float,
                   help="weight size, to report efficiency against the roofline")
    b.set_defaults(fn=cmd_benchmark)

    p = sub.add_parser("predict")
    p.add_argument("--weights-gib", type=float, required=True)
    p.add_argument("--name")
    p.set_defaults(fn=cmd_predict)
    args = ap.parse_args()
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
