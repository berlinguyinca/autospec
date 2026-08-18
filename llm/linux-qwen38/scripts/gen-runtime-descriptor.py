#!/usr/bin/env python3
"""Generate runtime-descriptor.json from the conf files.

The descriptor is what AutoSpec's model router reads. Generating it rather than
hand-maintaining it means a profile's context size cannot say one thing to the
launcher and another to the router.

    gen-runtime-descriptor.py            # write runtime-descriptor.json
    gen-runtime-descriptor.py --check    # exit 1 if the checked-in file is stale
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESCRIPTOR = ROOT / "runtime-descriptor.json"

# Context classes AutoSpec can request. A profile serves a class if its verified
# maximum is at least the class's ceiling.
#
# These ceilings are the MEASURED capability of this node, not the spec's
# aspirational 32K/64K/128K/150K ladder. Publishing a class this hardware cannot
# serve would just move the failure from the router to the request. The spec's
# "extended" class (~150K) has no entry because nothing here serves it.
CONTEXT_CLASSES = {"small": 8192, "standard": 24576, "large": 71680}


def parse_conf(path: Path) -> dict[str, str]:
    conf: dict[str, str] = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        if key.startswith("QWEN38_"):
            conf[key] = val.strip().strip('"')
    return conf


def build() -> dict:
    common = parse_conf(ROOT / "config" / "common.conf")
    profiles = []

    for path in sorted((ROOT / "config" / "profiles.d").glob("*.conf")):
        p = {**common, **parse_conf(path)}
        name = p["QWEN38_PROFILE"]
        runtime = p.get("QWEN38_RUNTIME", "vllm")

        raw_ctx = p.get("QWEN38_MAX_MODEL_LEN", "")
        ctx = int(raw_ctx) if raw_ctx.isdigit() else None
        raw_seqs = p.get("QWEN38_MAX_SEQS", "")
        seqs = int(raw_seqs) if raw_seqs.isdigit() else None

        # A profile the router may actually select. The llama.cpp `quality`
        # profile is specified but not provisioned (the node holds Q4_K_M, not
        # the Q6_K it declares), so it is published as unavailable rather than
        # as a selectable worker -- and its vision capability is reported false,
        # because a capability the router cannot successfully start is a lie.
        available = runtime == "vllm"

        profiles.append({
            "name": name,
            "version": p["QWEN38_PROFILE_VERSION"],
            "runtime": runtime,
            "available": available,
            "endpoint": f"http://{p['QWEN38_HOST']}:{p['QWEN38_PORT']}/v1",
            "model": p["QWEN38_SERVED_NAME"],
            "max_context": ctx,
            "max_concurrent_requests": seqs,
            "serves_context_classes": (
                sorted([c for c, n in CONTEXT_CLASSES.items() if ctx and ctx >= n],
                       key=lambda c: CONTEXT_CLASSES[c]) if ctx else []
            ),
            "capabilities": {
                # A text-only worker must not advertise vision. AutoSpec routes
                # on this field, so getting it wrong sends images to a server
                # configured to reject them.
                "vision": available and p.get("QWEN38_MULTIMODAL", "off") == "on",
                "streaming": True,
                "speculative_decoding": p.get("QWEN38_MTP", "off") == "on",
            },
            "unit": (f"autospec-qwen38@{name}.service" if runtime == "vllm" else None),
            # null, not a command: qwen38ctl deliberately refuses to start the
            # unprovisioned profile, so publishing a command here would invite
            # the router to run something that changes the boot default.
            "startable_via": (["qwen38ctl", "start", name] if available else None),
        })

    return {
        "schema": "autospec.runtime-descriptor/1",
        "node": "linux-qwen38",
        "host_gpu": "NVIDIA GeForce RTX 4090 24 GiB",
        "model_family": "Qwen3.8-27B",
        "n_ctx_train": 262144,
        "exclusive": True,  # only one profile may hold the GPU at a time
        "context_classes": CONTEXT_CLASSES,
        "profiles": profiles,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    text = json.dumps(build(), indent=2) + "\n"
    if args.check:
        current = DESCRIPTOR.read_text() if DESCRIPTOR.exists() else ""
        if current != text:
            print("runtime-descriptor.json is stale; run gen-runtime-descriptor.py",
                  file=sys.stderr)
            return 1
        print("runtime-descriptor.json is up to date")
        return 0

    DESCRIPTOR.write_text(text)
    print(f"wrote {DESCRIPTOR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
