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

SCOPE (child #1290): the real "api" backend (Anthropic Claude vision) plus a
test seam.

  * BACKEND RESOLUTION ($AUTOSPEC_FAB_VISION_BACKEND, first usable wins):
    explicit override (api | claude-cli | none); else "api" when a usable
    backend exists — either the $AUTOSPEC_FAB_VISION_STUB seam is set, or the
    anthropic SDK imports AND a key resolves; else "none" (honest degradation).

  * api BACKEND: each resolved PNG -> a base64 image content block
    (media_type image/png); the rules text (if any) + a fixed anti-fabrication
    instruction -> a text block; model from $AUTOSPEC_FAB_VISION_MODEL (default
    "claude-opus-4-8"); structured JSON output ({"observations":[...]} for
    judge, {"confirmed": bool} for verify). The anti-fabrication prompt asks
    for an EMPTY list when nothing is notable.

  * STUB SEAM ($AUTOSPEC_FAB_VISION_STUB=<path>): when set, the model call is
    delegated to that executable instead of the real Anthropic API, so CI can
    exercise the full code path (image/rules pass-through, contract JSON) with
    NO real API call. The stub is invoked as:
        <stub> judge  <sheet> [<rules>]   < (image-manifest JSON on stdin)
        <stub> verify <sheet> [<rules>]   < (candidate-observation JSON on stdin)
    and must print the same JSON contract the api backend would
    ({"observations":[...]} / {"confirmed": bool}) to stdout.

  * GRACEFUL DEGRADATION: ANY backend error / offline / timeout / malformed
    JSON is caught and resolves to the empty judge ({"observations": []}) /
    negative verify ({"confirmed": false}), exit 0 — never crash, never
    fabricate a finding.

The anthropic import is OPTIONAL (lazy + guarded): the CLI and its stdlib tests
work without the SDK installed.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
from html.parser import HTMLParser

# Default Anthropic model for the api backend, env-overridable via
# $AUTOSPEC_FAB_VISION_MODEL. claude-opus-4-8 supports base64 image content
# blocks + structured JSON output.
_DEFAULT_MODEL = "claude-opus-4-8"

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


def _model() -> str:
    """Resolve the api-backend model id from $AUTOSPEC_FAB_VISION_MODEL."""
    raw = os.environ.get("AUTOSPEC_FAB_VISION_MODEL")
    if raw and raw.strip():
        return raw.strip()
    return _DEFAULT_MODEL


def _stub_path() -> str | None:
    """Resolve the $AUTOSPEC_FAB_VISION_STUB test seam, if any."""
    raw = os.environ.get("AUTOSPEC_FAB_VISION_STUB")
    if raw and raw.strip():
        return raw.strip()
    return None


def _resolve_api_key() -> str | None:
    """
    Resolve an Anthropic API key from the environment.

    The Anthropic SDK itself reads ANTHROPIC_API_KEY (and other sources), but we
    check here so backend auto-detection can fall through to "none" cleanly when
    no key is configured, rather than constructing a client that fails at call
    time.
    """
    for name in ("ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"):
        val = os.environ.get(name)
        if val and val.strip():
            return val.strip()
    return None


def _anthropic_available() -> bool:
    """True if the optional anthropic SDK can be imported (lazy, guarded)."""
    try:
        import anthropic  # noqa: F401
    except Exception:
        return False
    return True


def resolve_backend() -> str:
    """
    Resolve which vision backend to use.

    First usable wins (design spec §"Backend resolution"):
      1. $AUTOSPEC_FAB_VISION_BACKEND explicit override (api | claude-cli | none)
      2. Anthropic API ("api") — usable when the $AUTOSPEC_FAB_VISION_STUB test
         seam is set, OR the anthropic SDK imports AND a key resolves
      3. claude CLI ("claude-cli") — candidate, pending support confirmation
      4. "none" — nothing usable -> honest degradation

    An explicit override cannot conjure a backend that is not actually usable:
    e.g. AUTOSPEC_FAB_VISION_BACKEND=api with no stub, no SDK, and no key still
    degrades to "none" (the contract: never crash, never fabricate). The stub
    seam makes "api" usable in CI without any real Anthropic call.
    """
    override = os.environ.get("AUTOSPEC_FAB_VISION_BACKEND")
    if override == "none":
        return "none"
    if override == "claude-cli":
        # Candidate backend; not implemented yet -> honest degradation.
        return "none"

    # "api" (explicit or auto-detected) is usable iff the stub seam is set, or
    # the SDK imports and a key resolves.
    api_usable = _stub_path() is not None or (
        _anthropic_available() and _resolve_api_key() is not None
    )
    if override == "api":
        return "api" if api_usable else "none"
    if override:
        # Unknown override -> honest degradation.
        return "none"

    # Auto-detect: prefer the real backend when usable, else degrade.
    return "api" if api_usable else "none"


# --- api backend -----------------------------------------------------------

_JUDGE_INSTRUCTION = (
    "You are reviewing rendered views of a 3D model (a contact sheet of PNG "
    "images) for STL modeling-rule issues. Report ONLY clearly visible, "
    "notable problems. Do NOT speculate, infer hidden geometry, or invent "
    "findings: if nothing is notable, return an EMPTY observations list. "
    "Respond with JSON of the shape "
    '{"observations": [{"observation": str, "severity": "info"|"warn", '
    '"view": str (optional), "rule": str (optional)}]}.'
)

_VERIFY_INSTRUCTION = (
    "You are adversarially verifying ONE candidate observation about the "
    "rendered views below. Confirm it ONLY if it is clearly and directly "
    "supported by what is visible. When in doubt, reject. Do NOT fabricate "
    "support. Respond with JSON of the shape {\"confirmed\": true|false}."
)

_JUDGE_SCHEMA = {
    "type": "object",
    "properties": {
        "observations": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "observation": {"type": "string"},
                    "severity": {"type": "string", "enum": ["info", "warn"]},
                    "view": {"type": "string"},
                    "rule": {"type": "string"},
                },
                "required": ["observation"],
                "additionalProperties": False,
            },
        }
    },
    "required": ["observations"],
    "additionalProperties": False,
}

_VERIFY_SCHEMA = {
    "type": "object",
    "properties": {"confirmed": {"type": "boolean"}},
    "required": ["confirmed"],
    "additionalProperties": False,
}


def _read_rules(rules_path: str | None) -> str:
    """Read the STL Modeling Rules text, or '' on any error / absence."""
    if not rules_path:
        return ""
    try:
        with open(rules_path, "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def _image_blocks(images: list) -> list:
    """Build base64 image content blocks for the resolved PNG paths."""
    blocks = []
    for path in images:
        try:
            with open(path, "rb") as f:
                data = base64.standard_b64encode(f.read()).decode("ascii")
        except OSError:
            continue
        blocks.append({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": data,
            },
        })
    return blocks


def _stub_call(mode: str, sheet_path: str, rules_path: str | None,
               images: list, stdin_obj) -> dict | None:
    """
    Delegate the model call to the $AUTOSPEC_FAB_VISION_STUB executable.

    The stub stands in for the real Anthropic API so CI exercises the full path
    without a network call. It is invoked as `<stub> <mode> <sheet> [<rules>]`;
    the resolved image paths + the candidate observation (verify) are passed as
    a JSON object on stdin so the stub can assert images/rules passed through.

    Returns the parsed stdout dict, or None on any error (caller degrades).
    """
    stub = _stub_path()
    if not stub:
        return None
    argv = [stub, mode, sheet_path]
    if rules_path:
        argv.append(rules_path)
    payload = {
        "mode": mode,
        "sheet": sheet_path,
        "rules": rules_path,
        "model": _model(),
        "images": list(images),
    }
    if stdin_obj is not None:
        payload["observation"] = stdin_obj
    try:
        proc = subprocess.run(
            argv,
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            timeout=120,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if proc.returncode != 0:
        return None
    out = (proc.stdout or "").strip()
    if not out:
        return None
    try:
        value = json.loads(out)
    except (ValueError, TypeError):
        return None
    return value if isinstance(value, dict) else None


def _api_message(content_blocks: list, schema: dict) -> dict | None:
    """
    Call the real Anthropic API with the given content blocks + JSON schema.

    Returns the parsed structured JSON dict, or None on ANY error (the caller
    degrades). The anthropic import is lazy + guarded here too.
    """
    try:
        import anthropic
    except Exception:
        return None
    try:
        client = anthropic.Anthropic()
        response = client.messages.create(
            model=_model(),
            max_tokens=4096,
            output_config={"format": {"type": "json_schema", "schema": schema}},
            messages=[{"role": "user", "content": content_blocks}],
        )
    except Exception:
        return None
    try:
        text = next(
            (b.text for b in response.content if getattr(b, "type", None) == "text"),
            None,
        )
    except Exception:
        return None
    if not text:
        return None
    try:
        value = json.loads(text)
    except (ValueError, TypeError):
        return None
    return value if isinstance(value, dict) else None


def _normalize_judge(payload) -> dict:
    """Coerce a backend judge payload into {"observations": [list of dicts]}."""
    if not isinstance(payload, dict):
        return {"observations": []}
    raw = payload.get("observations")
    if not isinstance(raw, list):
        return {"observations": []}
    return {"observations": [ob for ob in raw if isinstance(ob, dict)]}


def judge(sheet_path: str, rules_path: str | None) -> dict:
    """
    JUDGE pass: review the contact sheet against the rules.

    Returns {"observations": [...]}. With backend "none", nothing to inspect, or
    ANY backend error this is the empty list — honest degradation, never raises,
    never fabricates.
    """
    backend = resolve_backend()
    if backend != "api":
        return {"observations": []}

    images = resolve_images(sheet_path)
    if not images:
        # No PNGs to inspect -> nothing to judge.
        return {"observations": []}

    rules = _read_rules(rules_path)

    # Stub seam first: lets CI exercise the full path with no real API call.
    if _stub_path() is not None:
        result = _stub_call("judge", sheet_path, rules_path, images, None)
        return _normalize_judge(result)

    # Real Anthropic vision call.
    instruction = _JUDGE_INSTRUCTION
    if rules.strip():
        instruction += "\n\nSTL Modeling Rules:\n" + rules
    content = _image_blocks(images) + [{"type": "text", "text": instruction}]
    result = _api_message(content, _JUDGE_SCHEMA)
    return _normalize_judge(result)


def verify(sheet_path: str, rules_path: str | None,
           observation: dict | None) -> dict:
    """
    VERIFY pass: confirm or reject one candidate observation.

    Returns {"confirmed": bool}. With backend "none", a missing candidate, or
    ANY backend error this is {"confirmed": false} (conservative: an unverified
    observation is dropped). Never raises, never fabricates a confirmation.
    """
    backend = resolve_backend()
    if backend != "api" or observation is None:
        return {"confirmed": False}

    images = resolve_images(sheet_path)
    if not images:
        return {"confirmed": False}

    rules = _read_rules(rules_path)

    # Stub seam first.
    if _stub_path() is not None:
        result = _stub_call("verify", sheet_path, rules_path, images,
                            observation)
        confirmed = (isinstance(result, dict)
                     and result.get("confirmed") is True)
        return {"confirmed": bool(confirmed)}

    # Real Anthropic vision call.
    instruction = _VERIFY_INSTRUCTION
    if rules.strip():
        instruction += "\n\nSTL Modeling Rules:\n" + rules
    instruction += ("\n\nCandidate observation (JSON):\n"
                    + json.dumps(observation))
    content = _image_blocks(images) + [{"type": "text", "text": instruction}]
    result = _api_message(content, _VERIFY_SCHEMA)
    confirmed = isinstance(result, dict) and result.get("confirmed") is True
    return {"confirmed": bool(confirmed)}


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
