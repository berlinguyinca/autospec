#!/usr/bin/env python3
"""Pick the highest-quality quantisation that actually fits this machine.

    select-quant.py [--repo REPO ...] [--variant standard|uncensored|all]
                    [--target-context N] [--kv q4_0|q8_0|f16]
                    [--memory-mib N] [--vision] [--emit-preset]

Enumerates the GGUF files a repository publishes, computes what each one leaves
for KV cache on the detected hardware, and reports the largest that still serves
the target context. Size is the ranking key: within one base model, more bytes
means more bits means better.

The arithmetic is the same as ../../QWEN-NODE-SPEC.md sections 2-3, and its KV
term is validated against measurement on the reference host: the formula
predicts 18.4 KiB/token for q4_0 and the server allocated 3,533 MiB at 196,608
tokens, which is 18.4 KiB/token.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request

HF = "https://huggingface.co/api/models"

# Quant quality tiers. File SIZE alone is the wrong ranking key across families:
# Q4_K_L is larger than Q5_K_M yet lower quality, because the "_L"/"_XL" variants
# keep a 4-bit body and only raise the embedding and output tensors. So rank by
# the major level first, then the sub-variant, and use size only to break ties.
_SUB_RANK = {"XXS": 0, "XS": 1, "S": 2, "NL": 3, "0": 3, "1": 3,
             "M": 4, "K": 4, "L": 5, "XL": 6}


def quant_rank(filename: str) -> tuple[int, int]:
    """(major_bits, sub_rank); (0,0) when the tag is unrecognised."""
    up = filename.upper()
    if "BF16" in up or "F16" in up or "FP16" in up:
        return (16, 0)
    m = re.search(r"\bI?Q(\d+)((?:_[A-Z0-9]+)*)", up)
    if not m:
        return (0, 0)
    major = int(m.group(1))
    # The variant is the LAST recognised suffix, not the highest-ranked one:
    # in Q5_K_S the "K" names the family and the "S" is the variant, so taking a
    # max would score Q5_K_S the same as Q5_K_M.
    parts = [p for p in m.group(2).split("_") if p]
    sub = next((_SUB_RANK[p] for p in reversed(parts) if p in _SUB_RANK), 4)
    return (major, sub)

# Effective bytes per cached element. Block quants carry a scale per 32 values:
# q4_0 is 4 bits + an fp16 scale => 4.5 bits, q8_0 is 8 bits + scale => 8.5.
KV_BYTES = {"f16": 2.0, "bf16": 2.0, "q8_0": 8.5 / 8, "q4_0": 4.5 / 8}

# Curated, download-weighted catalogue. Uncensored/abliterated builds are
# third-party modifications of the base weights: they are ordinary model choices
# for a local node, but they are NOT vendor artefacts -- pin the revision, check
# the size, and re-measure quality rather than assuming it survived the edit.
# Named targets so a ladder can be planned for a machine you are not sitting at.
# "budget" already has the platform's headroom convention applied: discrete GPUs
# can be driven to ~95% of VRAM, while unified memory must leave the OS real room
# (macOS caps GPU-wired memory near 75% of RAM by default; raising it is a
# deliberate `sysctl iogpu.wired_limit_mb` change, not something to assume).
PLATFORMS = {
    #                    budget MiB, bandwidth GB/s, reserve MiB, note
    "nvidia-24":        (23_300,      1008, 1200, "RTX 3090/4090 24 GB"),
    "nvidia-32":        (31_000,      1792, 1200, "RTX 5090 32 GB"),
    "mac-32":           (24_576,       273, 2048, "Apple 32 GB unified (75%)"),
    "mac-48":           (36_864,       273, 2048, "Apple 48 GB unified (75%)"),
    "mac-64-max":       (49_152,       546, 2048, "Apple 64 GB M-Max (75%)"),
    "mac-128-max":      (98_304,       546, 3072, "Apple 128 GB M-Max (75%)"),
    "mac-512-ultra":    (393_216,      819, 4096, "Apple 512 GB M-Ultra (75%)"),
    "spark-128":        (108_000,      273, 3072, "DGX Spark GB10 128 GB (~85%)"),
    # Datacentre Ampere. No FP8 and no NVFP4 -- those need Ada (sm_89) and
    # Blackwell respectively -- so the quant choice here is GGUF or W4A16
    # Marlin, never the fp8 recipes written for H100.
    "a100-40":          (39_000,      1555, 2048, "A100 40 GB SXM4"),
    "a100-80":          (79_000,      2039, 3072, "A100 80 GB SXM4"),
    "a100-80-pcie":     (79_000,      1935, 3072, "A100 80 GB PCIe"),
    "h100-80":          (79_000,      3350, 3072, "H100 80 GB SXM5"),
    # sm_120: FP8 and NVFP4 both available, unlike Ampere. Budget is the real
    # 97,887 MiB the card reports, less a larger reserve than a 24 GiB part
    # needs because compute buffers grow with the slot counts this much memory
    # makes practical.
    "blackwell-96":     (97_887,      1792, 4096, "RTX PRO 6000 Blackwell 96 GB"),
}

CATALOGUE = {
    "standard": [
        "unsloth/Qwen3.8-27B-GGUF",
        "ggml-org/Qwen3.8-27B-GGUF",
        "bartowski/Qwen3.8-27B-GGUF",
    ],
    "uncensored": [
        "Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF",
        "huihui-ai/Huihui-Qwen3.8-27B-abliterated-GGUF",
        "JonathanColetti/Qwen3.8-27B-Uncensored-GGUF",
        "orcarouter/Qwen3.8-27B-Uncensored-GGUF",
    ],
}


def parse_device_totals(raw: str) -> list[int]:
    """Every device nvidia-smi reported, in MiB.

    The first line is not the answer. Reading only `.splitlines()[0]` made a
    two-card host report one card's VRAM and conclude that nothing fits.
    """
    out = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            out.append(int(line))
        except ValueError:
            continue
    return out


def aggregate_budget(devices: list[int]) -> int:
    """Total VRAM across the devices a job may use."""
    return sum(devices)


def effective_reserve(reserve_mib: int, n_devices: int) -> int:
    """Scale the reserve by card count.

    The reserve pays for compute buffers and the CUDA context, and those are
    paid on EACH card rather than shared -- so two cards cost two of them.

    It is applied here, where the single global reserve already lives, and
    deliberately NOT inside the budget detection: adding headroom on top of
    headroom double-counts and silently costs a whole rung of context. One
    device returns the reserve unchanged, so measured single-card numbers
    do not move.
    """
    return reserve_mib * max(1, n_devices)


def per_card_ceiling(devices: list[int], reserve_mib: int) -> int:
    """What a model PINNED to one card may use.

    Bounded by the smallest card, not by the aggregate: a 16 GiB model does not
    fit two 11 GiB cards if it may not be split.
    """
    if not devices:
        return 0
    return min(devices) - reserve_mib


def detect_memory() -> tuple[int, str, int]:
    """Return (budget_mib, how, n_devices). n_devices is 1 for non-CUDA hosts."""
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-gpu=memory.total", "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=10, check=True).stdout
        devices = parse_device_totals(out)
        if devices:
            total = aggregate_budget(devices)
            if len(devices) == 1:
                return total, "nvidia vram", 1
            return total, f"nvidia vram ({len(devices)} devices, {'+'.join(str(d) for d in devices)} MiB)", len(devices)
    except Exception:
        pass
    try:  # Apple Silicon / any unified-memory box
        out = subprocess.run(["sysctl", "-n", "hw.memsize"],
                             capture_output=True, text=True, timeout=10, check=True).stdout
        # Unified memory is shared with the OS; do not offer it all to the model.
        return int(int(out.strip()) / 1024 / 1024 * 0.75), "unified memory (75% of total)", 1
    except Exception:
        pass
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemTotal:"):
                return int(int(line.split()[1]) / 1024 * 0.75), "system RAM (75%)", 1
    except Exception:
        pass
    return 0, "unknown", 1


def detect_memory_mib() -> tuple[int, str]:
    """Back-compatible shim: (budget_mib, how), dropping the device count."""
    budget, how, _ = detect_memory()
    return budget, how


def get_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.load(r)


def kv_bytes_per_token(base_model: str, kv: str) -> tuple[float, str]:
    """Derive KV cost per token from the base model's own config."""
    cfg = get_json(f"https://huggingface.co/{base_model}/raw/main/config.json")
    t = cfg.get("text_config", cfg)
    layers = t.get("layer_types") or []
    n_full = layers.count("full_attention") or t.get("num_hidden_layers", 0)
    heads = t.get("num_key_value_heads") or 0
    dim = t.get("head_dim") or 0
    per_tok = 2 * n_full * heads * dim * KV_BYTES[kv]
    note = (f"{n_full} full-attention layers of {len(layers) or t.get('num_hidden_layers')}, "
            f"{heads} kv heads x {dim} -> {per_tok/1024:.1f} KiB/token at {kv}")
    return per_tok, note, int(t.get("max_position_embeddings") or 0)


def list_quants(repo: str) -> tuple[list[tuple[str, float]], list[str], str]:
    """Return ([(filename, GiB)], [projector files], revision)."""
    d = get_json(f"{HF}/{repo}?blobs=true")
    files, proj = [], []
    for s in d.get("siblings", []):
        n = s["rfilename"]
        if not n.endswith(".gguf"):
            continue
        gib = (s.get("size") or 0) / 2**30
        low = n.lower()
        # Projectors and MTP draft heads are not candidate weight files.
        if "mmproj" in low or "vision" in low:
            proj.append(n)
        elif "draft" in low or low.startswith("mtp") or "-mtp" in low:
            continue
        else:
            files.append((n, gib))

    # Split GGUFs ("...-00001-of-00003.gguf") are ONE model. Left ungrouped, a
    # single shard looks like a small candidate and gets recommended -- loading
    # it alone fails or silently serves a broken model.
    grouped: dict[str, float] = {}
    for n, gib in files:
        m = re.search(r"-(\d{5})-of-(\d{5})\.gguf$", n)
        key = n if not m else re.sub(r"-\d{5}-of-\d{5}\.gguf$", "-00001-of-%s.gguf" % m.group(2), n)
        grouped[key] = grouped.get(key, 0.0) + gib
    return sorted(grouped.items()), proj, d.get("sha", "")[:12]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", action="append", default=[])
    ap.add_argument("--variant", choices=["standard", "uncensored", "all"],
                    default="standard")
    ap.add_argument("--base-model", default="Qwen/Qwen3.8-27B")
    ap.add_argument("--target-context", type=int, default=131072)
    ap.add_argument("--kv", choices=sorted(KV_BYTES), default="q4_0")
    ap.add_argument("--memory-mib", type=int, default=0)
    ap.add_argument("--reserve-mib", type=int, default=1200,
                    help="compute buffers plus safety headroom; ~800 MiB of "
                         "buffers were measured on the reference host, and some "
                         "runtimes allocate outside their own budget")
    ap.add_argument("--vision", action="store_true",
                    help="also budget for a projector (~900 MiB)")
    ap.add_argument("--json", action="store_true",
                    help="machine-readable output; other tools should consume "
                         "this rather than scraping the human table, which "
                         "changes shape")
    ap.add_argument("--emit-preset", action="store_true",
                    help="print a llama.cpp router preset for the winner")
    ap.add_argument("--platform", choices=sorted(PLATFORMS),
                    help="plan for a named machine instead of this one")
    ap.add_argument("--bandwidth-gbs", type=float, default=0.0,
                    help="for the predicted tokens/sec column")
    ap.add_argument("--min-tps", type=float, default=0.0,
                    help="reject quants whose predicted tokens/sec falls below "
                         "this. On a bandwidth-limited machine the highest quant "
                         "that FITS is often not the one you want.")
    ap.add_argument("--model-id-prefix", default="qwen3.8-27b",
                    help="prefix for generated ladder model ids")
    ap.add_argument("--ladder",
                    help="emit a profile per tier, e.g. "
                         "'small=32768,medium=131072,large=262144'. Each tier "
                         "gets the highest-quality quant that serves it.")
    args = ap.parse_args()

    bandwidth = args.bandwidth_gbs
    if args.platform:
        pb, pbw, pres, pnote = PLATFORMS[args.platform]
        budget, how = pb, f"{args.platform} — {pnote}"
        bandwidth = bandwidth or pbw
        if args.reserve_mib == 1200:
            args.reserve_mib = pres
    else:
        if args.memory_mib:
            budget, how, n_devices = args.memory_mib, "specified", 1
        else:
            budget, how, n_devices = detect_memory()
            # Compute buffers and the CUDA context are paid per card, so the
            # reserve is multiplied rather than the budget being reduced --
            # see effective_reserve().
            args.reserve_mib = effective_reserve(args.reserve_mib, n_devices)
    if args.memory_mib:
        budget, how = args.memory_mib, "specified"
    if not budget:
        print("could not determine a memory budget; pass --memory-mib", file=sys.stderr)
        return 1

    try:
        per_tok, kv_note, n_ctx_train = kv_bytes_per_token(args.base_model, args.kv)
    except (urllib.error.URLError, OSError, KeyError) as exc:
        print(f"could not read {args.base_model} config: {exc}", file=sys.stderr)
        return 1

    repos = args.repo or (CATALOGUE["standard"] + CATALOGUE["uncensored"]
                          if args.variant == "all" else CATALOGUE[args.variant])
    proj_mib = 900 if args.vision else 0

    # Under --json nothing but JSON may reach stdout, or consumers cannot parse it.
    def say(*a, **k):
        if not args.json:
            print(*a, **k)

    say(f"budget      : {budget} MiB ({how})")
    say(f"kv          : {kv_note}")
    say(f"reserve     : {args.reserve_mib} MiB"
          + (f" + {proj_mib} MiB projector" if proj_mib else ""))
    if "devices" in how:
        say("note        : weights are split across cards, so the aggregate is "
            "spendable -- but a model pinned to ONE card is bounded by the "
            "smallest card, not by this budget.")
    if not args.ladder:
        say(f"target ctx  : {args.target_context:,} tokens "
            f"= {args.target_context*per_tok/2**20:,.0f} MiB of KV")
    if n_ctx_train:
        say(f"n_ctx_train : {n_ctx_train:,} (hard cap regardless of memory)")
    print()
    say("'max ctx' is an UPPER BOUND from arithmetic. Compute buffers grow with")
    say("context and some runtimes allocate outside their own budget, so the")
    say("figure must be confirmed with measure-ceiling.sh before it is configured.")
    say()

    rows = []
    for repo in repos:
        try:
            files, proj, rev = list_quants(repo)
        except (urllib.error.URLError, OSError) as exc:
            print(f"  ! {repo}: {exc}", file=sys.stderr)
            continue
        if args.vision and not proj:
            # A repo with no projector cannot serve the vision preset at all.
            print(f"  ! {repo}: no projector published; skipped for --vision",
                  file=sys.stderr)
            continue
        for name, gib in files:
            w_mib = gib * 1024
            free_for_kv = budget - w_mib - args.reserve_mib - proj_mib
            max_ctx = int(max(0, free_for_kv) * 2**20 / per_tok)
            # Memory is not the only ceiling: runtimes refuse a max_model_len
            # above the model's trained positions, so cap there.
            if n_ctx_train:
                max_ctx = min(max_ctx, n_ctx_train)
            rows.append({
                "repo": repo, "rev": rev, "file": name, "gib": gib,
                "rank": quant_rank(name),
                "max_ctx": max_ctx, "fits": max_ctx >= args.target_context,
                "proj": proj[0] if proj else None,
            })

    by_quality = lambda r: (-r["rank"][0], -r["rank"][1], -r["gib"])

    if args.json:
        print(json.dumps({
            "budget_mib": budget, "how": how, "kv": args.kv,
            "kv_bytes_per_token": per_tok, "n_ctx_train": n_ctx_train,
            "bandwidth_gbs": bandwidth or None,
            "candidates": [
                {k: v for k, v in r.items() if k != "rank"} |
                {"quant_major": r["rank"][0], "quant_sub": r["rank"][1],
                 "predicted_tps": round(bandwidth * 1e9 / (r["gib"] * 2**30), 2)
                                  if bandwidth else None}
                for r in sorted(rows, key=by_quality)
            ],
        }, indent=2))
        return 0
    def pred_tps(r) -> float:
        return (bandwidth * 1e9 / (r["gib"] * 2**30)) if bandwidth else 0.0

    def candidates_for(ctx: int):
        """Quants covering ctx, best quality first, honouring --min-tps."""
        ok = [r for r in rows if r["max_ctx"] >= ctx]
        if args.min_tps and bandwidth:
            ok = [r for r in ok if pred_tps(r) >= args.min_tps]
        return sorted(ok, key=by_quality)

    def best_for(ctx: int):
        c = candidates_for(ctx)
        return c[0] if c else None

    if args.ladder:
        tiers = []
        for spec in args.ladder.split(","):
            name, _, val = spec.partition("=")
            tiers.append((name.strip(), int(val or name)))
        tiers.sort(key=lambda x: x[1])

        if args.min_tps:
            print(f"speed floor : {args.min_tps} tok/s (predicted)\n")
        chosen = []
        for name, ctx in tiers:
            cands = candidates_for(ctx)
            if not cands:
                print(f"{name:10} {ctx:>9,}  — nothing fits at this context"
                      + (f" above {args.min_tps} tok/s" if args.min_tps else ""))
                continue
            r = cands[0]
            print(f"{name:10} {ctx:>9,}  {r['file'][:34]:34} {r['gib']:6.2f}GiB "
                  f"{pred_tps(r):5.1f} t/s  (bound {r['max_ctx']:,})")
            # Show the next two down so the quality/speed trade is visible rather
            # than hidden behind a single "recommended" line.
            for alt in cands[1:3]:
                print(f"{'':10} {'':>9}    alt: {alt['file'][:31]:31} "
                      f"{alt['gib']:6.2f}GiB {pred_tps(alt):5.1f} t/s")
            chosen.append((name, ctx, r))

        if not chosen:
            print("\nNo tier could be satisfied.", file=sys.stderr)
            return 1
        print("\npredicted t/s is bandwidth / weight bytes — an upper bound; "
              "expect 60-85%.")

        if args.emit_preset:
            print(f"\n; ladder for {how}, {args.kv} KV")
            print("; UNVERIFIED: confirm each c= with measure-ceiling.sh.")
            print("\nversion = 1\n\n[*]\nn-gpu-layers = 999\nflash-attn = auto")
            print(f"jinja = true\ncache-type-k = {args.kv}\ncache-type-v = {args.kv}")
            seen = set()
            for name, ctx, r in chosen:
                mid = f"{args.model_id_prefix}-{name}" if args.model_id_prefix else name
                if mid in seen:
                    continue
                seen.add(mid)
                print(f"\n; {r['repo']} @ {r['rev']} — {r['file']}")
                print(f"[{mid}]")
                print(f"model = /var/lib/qwen-gguf/models/{r['file']}")
                # Only when --vision was asked for: the projector costs ~885 MiB
                # and is only in the budget above when that flag is set. Emitting
                # it otherwise silently spends memory the arithmetic did not
                # reserve, which is how a preset "that should fit" OOMs.
                if args.vision and r["proj"]:
                    print(f"mmproj = /var/lib/qwen-gguf/models/{r['proj']}")
                    print("image-min-tokens = 1024")
                print(f"c = {ctx}")
        return 0

    fitting = sorted([r for r in rows if r["fits"]], key=by_quality)
    print(f"{'quant':22} {'repo':30} {'size':>9} {'max ctx':>9}  fits")
    for r in sorted(rows, key=by_quality)[:20]:
        tag = f"Q{r['rank'][0]}" if r["rank"][0] else "?"
        print(f"{r['file'][:22]:22} {r['repo'].split('/')[0][:30]:30} "
              f"{r['gib']:6.2f}GiB {r['max_ctx']:>9,}  {'yes' if r['fits'] else 'no'}")

    if not fitting:
        best = max(rows, key=lambda r: r["max_ctx"]) if rows else None
        print("\nNothing fits the target context.", file=sys.stderr)
        if best:
            print(f"Largest reachable context is {best['max_ctx']:,} with "
                  f"{best['file']} ({best['gib']:.2f} GiB).", file=sys.stderr)
            print("Lower --target-context, use --kv q4_0, or pick a smaller model.",
                  file=sys.stderr)
        return 1

    w = fitting[0]
    print(f"\nRECOMMENDED  {w['repo']} @ {w['rev']}")
    print(f"             {w['file']}  ({w['gib']:.2f} GiB)")
    print(f"             serves up to {w['max_ctx']:,} tokens at {args.kv} KV")

    if args.emit_preset:
        mid = re.sub(r"\.gguf$", "", w["file"]).lower()
        mid = re.sub(r"[^a-z0-9.-]+", "-", mid).strip("-")
        ctx = min(w["max_ctx"], args.target_context)
        print(f"\n; generated by select-quant.py from a {budget} MiB budget.")
        print(f"; UNVERIFIED: run measure-ceiling.sh before trusting c = {ctx}.")
        print(f"[{mid}]")
        print(f"model = /var/lib/qwen-gguf/models/{w['file']}")
        if w["proj"]:
            print(f"mmproj = /var/lib/qwen-gguf/models/{w['proj']}")
            print("image-min-tokens = 1024")
        print(f"c = {ctx}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
