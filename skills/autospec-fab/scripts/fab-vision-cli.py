#!/usr/bin/env python3
"""
fab-vision-cli.py — the default, runnable $AUTOSPEC_FAB_VISION_CMD consumer.

This is the drop-in vision command that stage_vision.py invokes (judge +
adversarial verify). It satisfies the stage's contract byte-for-byte:

    JUDGE pass:   fab-vision-cli.py <sheet> [<rules>]
                  -> stdout {"observations": [...]}, exit 0
    VERIFY pass:  fab-vision-cli.py --verify <sheet> [<rules>]
                  with one observation JSON on stdin
                  -> stdout {"confirmed": true|false}, exit 0

Contract guarantees (design spec §"The vision-cli contract (authoritative)"):
  * Exit 0 for every reachable/unreachable-backend outcome — the verdict lives
    in the JSON, never the exit code. The ONLY non-zero exit is 2, for a usage
    error (bad flags / no positional <sheet>).
  * Honest degradation, never a crash and never a fabricated finding: missing /
    unreadable sheet, zero PNGs on disk, or no usable backend all resolve to
    the empty judge ({"observations": []}) / negative verify
    ({"confirmed": false}), exit 0.

SCOPE of this iteration (child #1289): stdlib only. The backend is ALWAYS
"none" here — there is no real model call yet. The backend-resolution seam
(resolve_backend()) is left clean so child #1290 can add the real "api" backend
(Anthropic SDK, base64 PNG image blocks) behind $AUTOSPEC_FAB_VISION_BACKEND
WITHOUT touching the argument parsing, the contact-sheet→PNG resolver, or the
degradation contract.

Stdlib only; no third-party imports.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from html.parser import HTMLParser

# Default cap on the number of images handed to a backend, to bound token/cost
# once a real backend exists (child #1290). Env-overridable. In this iteration
# the cap only trims the resolved list the (none) backend ignores.
_DEFAULT_MAX_IMAGES = 16


class _ImgSrcParser(HTMLParser):
    """Collect every <img src="..."> value in document order."""

    def __init__(self) -> None:
        super().__init__()
        self.srcs: list = []

    def handle_starttag(self, tag, attrs):
        if tag.lower() != "img":
            return
        for name, value in attrs:
            if name.lower() == "src" and value:
                self.srcs.append(value)


def _max_images() -> int:
    """Resolve the image cap from $AUTOSPEC_FAB_VISION_MAX_IMAGES (env)."""
    raw = os.environ.get("AUTOSPEC_FAB_VISION_MAX_IMAGES")
    if raw is None:
        return _DEFAULT_MAX_IMAGES
    try:
        n = int(raw)
    except (ValueError, TypeError):
        return _DEFAULT_MAX_IMAGES
    return n if n > 0 else _DEFAULT_MAX_IMAGES


def resolve_images(sheet_path: str) -> list:
    """
    Resolve the contact sheet HTML into a list of on-disk PNG paths.

    Reads the sheet, extracts <img src="..."> basenames in document order (the
    render stage writes them in REQUIRED_VIEWS table order, so the order is
    deterministic), resolves each src relative to the sheet's own directory, and
    keeps only the ones that exist on disk. A missing / unreadable sheet, or a
    sheet referencing no existing PNG, yields an empty list (the caller then
    emits the empty-judge degradation). The list is capped at _max_images().

    This NEVER raises: any I/O or parse error degrades to an empty list.
    """
    try:
        with open(sheet_path, "r", encoding="utf-8", errors="replace") as f:
            html_text = f.read()
    except OSError:
        return []

    parser = _ImgSrcParser()
    try:
        parser.feed(html_text)
    except Exception:
        # An unparseable sheet has nothing we can trust — degrade to empty.
        return []

    sheet_dir = os.path.dirname(os.path.abspath(sheet_path))
    resolved = []
    for src in parser.srcs:
        # Keep the referenced basename, resolved relative to the sheet dir.
        candidate = os.path.normpath(os.path.join(sheet_dir, src))
        if os.path.isfile(candidate):
            resolved.append(candidate)

    cap = _max_images()
    if len(resolved) > cap:
        sys.stderr.write(
            f"fab-vision-cli: capping {len(resolved)} images to {cap}\n"
        )
        resolved = resolved[:cap]
    return resolved


def resolve_backend() -> str:
    """
    Resolve which vision backend to use. Seam for child #1290.

    First usable wins (design spec §"Backend resolution"):
      1. $AUTOSPEC_FAB_VISION_BACKEND explicit override (api | claude-cli | none)
      2. Anthropic API ("api")   — added in child #1290 (anthropic SDK + key)
      3. claude CLI ("claude-cli") — candidate, pending support confirmation
      4. "none"                   — nothing usable -> honest degradation

    In THIS iteration only "none" is implemented: there is no real backend, so
    we always return "none" (an explicit override is honoured only insofar as it
    cannot conjure a backend that does not exist yet). Child #1290 fills in the
    real branches here without touching the rest of the CLI.
    """
    override = os.environ.get("AUTOSPEC_FAB_VISION_BACKEND")
    if override == "none":
        return "none"
    # No real backend exists in this iteration; everything degrades to "none".
    # (Child #1290 will return "api" / "claude-cli" here when usable.)
    return "none"


def judge(sheet_path: str, rules_path: str | None) -> dict:
    """
    JUDGE pass: review the contact sheet against the rules.

    Returns {"observations": [...]}. With backend "none" (this iteration) — or
    whenever there is nothing to inspect — this is the empty list. Never raises.
    """
    backend = resolve_backend()
    if backend == "none":
        return {"observations": []}

    # Unreachable in this iteration. Once child #1290 adds a real backend, the
    # resolved images + rules text would be sent here. The seam:
    images = resolve_images(sheet_path)  # noqa: F841  (used by #1290)
    if not images:
        return {"observations": []}
    # Real backend dispatch lands in child #1290; until then, degrade honestly.
    return {"observations": []}


def verify(sheet_path: str, rules_path: str | None,
           observation: dict | None) -> dict:
    """
    VERIFY pass: confirm or reject one candidate observation.

    Returns {"confirmed": bool}. With backend "none" (this iteration), an
    unreadable candidate, or any backend error this is {"confirmed": false}
    (conservative: an unverified observation is dropped). Never raises.
    """
    backend = resolve_backend()
    if backend == "none" or observation is None:
        return {"confirmed": False}

    # Real backend dispatch lands in child #1290; until then, degrade honestly.
    return {"confirmed": False}


def _read_stdin_observation():
    """Read one observation JSON object from stdin; None if absent/malformed."""
    try:
        raw = sys.stdin.read()
    except OSError:
        return None
    if not raw or not raw.strip():
        return None
    try:
        value = json.loads(raw)
    except (ValueError, TypeError):
        return None
    return value if isinstance(value, dict) else None


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        prog="fab-vision-cli.py",
        description="autospec-fab vision CLI consumer "
                    "($AUTOSPEC_FAB_VISION_CMD target)",
    )
    parser.add_argument(
        "--verify", action="store_true",
        help="verify pass: confirm one candidate observation read from stdin",
    )
    parser.add_argument("sheet", help="contact-sheet HTML path")
    parser.add_argument("rules", nargs="?", default=None,
                        help="STL Modeling Rules file (optional)")

    # argparse exits 2 on a usage error (bad flags / missing <sheet>), which is
    # exactly the contract's usage-error exit code — no special handling needed.
    args = parser.parse_args(argv)

    if args.verify:
        observation = _read_stdin_observation()
        result = verify(args.sheet, args.rules, observation)
    else:
        result = judge(args.sheet, args.rules)

    json.dump(result, sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
