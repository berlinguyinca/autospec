#!/usr/bin/env python3
"""Validate and resolve AutoSpec routing envelopes."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from datetime import datetime, timezone
from pathlib import Path

from autospec_route_lib import (
    HARNESS_IDS,
    RoutingError,
    load_capabilities,
    load_routing_config,
    fetch_capabilities,
    resolve_dispatch,
    validate_routing_config_path,
)


def _now(value: str | None) -> datetime:
    if value is None:
        return datetime.now(timezone.utc)
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise RoutingError("ROUTING_CONFIG_INVALID", f"invalid --now value: {exc}")
    if parsed.tzinfo is None:
        raise RoutingError("ROUTING_CONFIG_INVALID", "--now requires a timezone")
    return parsed.astimezone(timezone.utc)


def _config_path(argument: str | None) -> Path:
    if argument:
        return Path(argument)
    configured = os.environ.get("AUTOSPEC_ROUTING_CONFIG")
    if configured:
        return validate_routing_config_path(Path(configured), from_environment=True)
    return Path.home() / ".autospec" / "routing.yml"


def _available(config: dict, explicit: list[str] | None) -> set[str]:
    if explicit:
        return set(explicit)
    return {
        harness_id
        for harness_id, harness in config["harnesses"].items()
        if shutil.which(harness["command"][0])
    }


def _refusal(error: RoutingError) -> dict:
    return {
        "version": 1,
        "status": "fallback_required",
        "reason": error.reason,
        "fallback": "existing-routing",
        "explain": [error.message],
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    validate = sub.add_parser("validate")
    validate.add_argument("--config")
    for name in ("resolve", "explain"):
        command = sub.add_parser(name)
        command.add_argument("--config")
        command.add_argument("--capabilities")
        command.add_argument("--kind", required=True)
        command.add_argument("--proposer-envelope")
        command.add_argument("--available-harness", action="append", choices=sorted(HARNESS_IDS))
        command.add_argument("--now")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        config = load_routing_config(_config_path(args.config))
        if args.command == "validate":
            print(json.dumps({"version": 1, "status": "valid"}, sort_keys=True))
            return 0
        current_time = _now(args.now)
        capability_args = {
            "now": current_time,
            "maximum_age_seconds": config["inferweave"]["maximum_age_seconds"],
            "allow_loopback_http": bool(config["inferweave"].get("allow_loopback_http", False)),
        }
        if args.capabilities:
            capabilities = load_capabilities(Path(args.capabilities), **capability_args)
        else:
            capabilities = fetch_capabilities(
                config["inferweave"]["discovery_url"],
                timeout_seconds=config["inferweave"]["timeout_seconds"],
                **capability_args,
            )
        proposer = None
        if args.proposer_envelope:
            proposer = json.loads(Path(args.proposer_envelope).read_text(encoding="utf-8"))
        envelope = resolve_dispatch(
            config,
            capabilities,
            args.kind,
            proposer=proposer,
            available_harnesses=_available(config, args.available_harness),
        )
        if args.command == "resolve":
            print(json.dumps(envelope, sort_keys=True, separators=(",", ":")))
        else:
            print(f"dispatch: {envelope['dispatch_id']}")
            for line in envelope["explain"]:
                print(line)
        return 0
    except RoutingError as error:
        if error.exit_code == 3:
            print(json.dumps(_refusal(error), sort_keys=True, separators=(",", ":")))
        else:
            print(f"{error.reason}: {error.message}", file=sys.stderr)
        return error.exit_code
    except (OSError, json.JSONDecodeError) as error:
        print(f"ROUTING_CONFIG_INVALID: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
