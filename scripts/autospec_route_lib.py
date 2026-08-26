#!/usr/bin/env python3
"""Strict, dependency-light contracts for AutoSpec routing."""

from __future__ import annotations

import json
import hashlib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse
from urllib.error import HTTPError, URLError
from urllib.request import HTTPRedirectHandler, Request, build_opener


MAX_DOCUMENT_BYTES = 1_048_576
HARNESS_IDS = {"pi", "codex", "opencode", "claude"}
TRANSPORTS = {"json", "rpc", "cli"}
PROTOCOLS = {"openai-compatible", "native"}
MODALITIES = {"text", "image"}
VISION_NODE_CLASSES = {"mac", "rtx6000"}


class RoutingError(Exception):
    def __init__(self, reason: str, message: str, exit_code: int = 1):
        super().__init__(message)
        self.reason = reason
        self.message = message
        self.exit_code = exit_code


def _error(reason: str, message: str, exit_code: int = 1) -> None:
    raise RoutingError(reason, message, exit_code)


def _object(value: Any, name: str, reason: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        _error(reason, f"{name} must be an object")
    return value


def _keys(value: dict[str, Any], allowed: set[str], required: set[str], name: str, reason: str) -> None:
    unknown = set(value) - allowed
    missing = required - set(value)
    if unknown:
        _error(reason, f"{name} has unknown keys: {', '.join(sorted(unknown))}")
    if missing:
        _error(reason, f"{name} is missing keys: {', '.join(sorted(missing))}")


def _positive(value: Any, name: str, reason: str, *, allow_zero: bool = False) -> int:
    minimum = 0 if allow_zero else 1
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        _error(reason, f"{name} must be an integer >= {minimum}")
    return value


def _string_list(value: Any, name: str, reason: str) -> list[str]:
    if not isinstance(value, list) or not value:
        _error(reason, f"{name} must be a non-empty list")
    if any(not isinstance(item, str) or not item for item in value):
        _error(reason, f"{name} must contain non-empty strings")
    if len(value) != len(set(value)):
        _error(reason, f"{name} must not contain duplicates")
    return value


def _safe_url(value: Any, name: str, reason: str, allow_loopback_http: bool) -> str:
    if not isinstance(value, str) or not value:
        _error(reason, f"{name} must be a non-empty URL")
    parsed = urlparse(value)
    if parsed.scheme == "https" and parsed.netloc:
        return value
    loopback = parsed.hostname in {"127.0.0.1", "localhost", "::1"}
    if allow_loopback_http and parsed.scheme == "http" and parsed.netloc and loopback:
        return value
    _error(reason, f"{name} must use HTTPS or explicitly allowed loopback HTTP")


def _read_bounded(path: Path, reason: str) -> str:
    try:
        size = path.stat().st_size
    except FileNotFoundError:
        _error(reason, f"file not found: {path}", 3 if reason == "ROUTING_CONFIG_MISSING" else 1)
    if size > MAX_DOCUMENT_BYTES:
        _error(reason, f"document exceeds {MAX_DOCUMENT_BYTES} bytes")
    return path.read_text(encoding="utf-8")


def validate_routing_config_path(path: Path, *, from_environment: bool = False) -> Path:
    if from_environment and not path.is_absolute():
        _error("ROUTING_CONFIG_INVALID", "AUTOSPEC_ROUTING_CONFIG must be absolute")
    return path


def load_routing_config(path: Path) -> dict[str, Any]:
    validate_routing_config_path(path)
    if not path.exists():
        _error("ROUTING_CONFIG_MISSING", f"routing configuration not found: {path}", 3)
    text = _read_bounded(path, "ROUTING_CONFIG_INVALID")
    try:
        import yaml
    except ImportError:
        _error("ROUTING_CONFIG_INVALID", "PyYAML is required to load routing configuration", 2)
    try:
        data = yaml.safe_load(text)
    except Exception as exc:
        _error("ROUTING_CONFIG_INVALID", f"invalid routing YAML: {exc}")
    return validate_routing_config(data)


def validate_routing_config(data: Any) -> dict[str, Any]:
    reason = "ROUTING_CONFIG_INVALID"
    root = _object(data, "routing configuration", reason)
    _keys(root, {"version", "harnesses", "routes", "inference_classes", "inferweave", "fallback"}, {"version", "harnesses", "routes", "inference_classes", "inferweave", "fallback"}, "routing configuration", reason)
    if root["version"] != 1:
        _error(reason, "routing configuration version must be 1")

    harnesses = _object(root["harnesses"], "harnesses", reason)
    if not harnesses:
        _error(reason, "harnesses must not be empty")
    unknown_harnesses = set(harnesses) - HARNESS_IDS
    if unknown_harnesses:
        _error(reason, f"unsupported harnesses: {', '.join(sorted(unknown_harnesses))}")
    for harness_id, raw in harnesses.items():
        harness = _object(raw, f"harnesses.{harness_id}", reason)
        _keys(harness, {"command", "transport", "provider_protocols", "endpoint_env", "api_key_env"}, {"command", "transport", "provider_protocols"}, f"harnesses.{harness_id}", reason)
        _string_list(harness["command"], f"harnesses.{harness_id}.command", reason)
        if harness["transport"] not in TRANSPORTS:
            _error(reason, f"unsupported transport for {harness_id}")
        protocols = set(_string_list(harness["provider_protocols"], f"harnesses.{harness_id}.provider_protocols", reason))
        if not protocols <= PROTOCOLS:
            _error(reason, f"unsupported provider protocol for {harness_id}")

    classes = _object(root["inference_classes"], "inference_classes", reason)
    if not classes:
        _error(reason, "inference_classes must not be empty")
    for class_id, raw in classes.items():
        if not class_id:
            _error(reason, "inference class names must not be empty")
        item = _object(raw, f"inference_classes.{class_id}", reason)
        _keys(item, {"modalities", "max_input_tokens", "reserve_output_tokens", "max_queue_seconds", "eligible_node_classes"}, {"modalities", "max_input_tokens", "reserve_output_tokens", "max_queue_seconds"}, f"inference_classes.{class_id}", reason)
        modalities = set(_string_list(item["modalities"], f"inference_classes.{class_id}.modalities", reason))
        if not modalities <= MODALITIES:
            _error(reason, f"unsupported modality in {class_id}")
        _positive(item["max_input_tokens"], f"{class_id}.max_input_tokens", reason)
        _positive(item["reserve_output_tokens"], f"{class_id}.reserve_output_tokens", reason)
        _positive(item["max_queue_seconds"], f"{class_id}.max_queue_seconds", reason)
        nodes = item.get("eligible_node_classes")
        if "image" in modalities:
            if set(_string_list(nodes, f"{class_id}.eligible_node_classes", reason)) - VISION_NODE_CLASSES:
                _error(reason, f"{class_id} vision nodes must be mac or rtx6000")
        elif nodes is not None:
            _string_list(nodes, f"{class_id}.eligible_node_classes", reason)

    routes = _object(root["routes"], "routes", reason)
    if not routes or any(not name for name in routes):
        _error(reason, "route names must be non-empty")
    for route_id, raw in routes.items():
        route = _object(raw, f"routes.{route_id}", reason)
        _keys(route, {"harnesses", "inference_class", "independent_from", "minimum_strength", "allow_opportunistic"}, {"harnesses", "inference_class"}, f"routes.{route_id}", reason)
        selected = _string_list(route["harnesses"], f"routes.{route_id}.harnesses", reason)
        if not set(selected) <= set(harnesses):
            _error(reason, f"route {route_id} references an unknown harness")
        if route["inference_class"] not in classes:
            _error(reason, f"route {route_id} references an unknown inference class")
        for reference in ("independent_from", "minimum_strength"):
            if reference in route and route[reference] not in routes:
                _error(reason, f"route {route_id} {reference} references an unknown route")
        if "allow_opportunistic" in route and not isinstance(route["allow_opportunistic"], bool):
            _error(reason, f"routes.{route_id}.allow_opportunistic must be boolean")

    inferweave = _object(root["inferweave"], "inferweave", reason)
    _keys(inferweave, {"discovery_url", "timeout_seconds", "maximum_age_seconds", "local_only", "allow_loopback_http"}, {"discovery_url", "timeout_seconds", "maximum_age_seconds", "local_only"}, "inferweave", reason)
    _safe_url(inferweave["discovery_url"], "inferweave.discovery_url", reason, bool(inferweave.get("allow_loopback_http", False)))
    _positive(inferweave["timeout_seconds"], "inferweave.timeout_seconds", reason)
    _positive(inferweave["maximum_age_seconds"], "inferweave.maximum_age_seconds", reason)
    if not isinstance(inferweave["local_only"], bool):
        _error(reason, "inferweave.local_only must be boolean")

    fallback = _object(root["fallback"], "fallback", reason)
    _keys(fallback, {"mode"}, {"mode"}, "fallback", reason)
    if fallback["mode"] != "existing-routing":
        _error(reason, "fallback.mode must be existing-routing")
    return root


def _parse_rfc3339(value: Any) -> datetime:
    if not isinstance(value, str):
        _error("ROUTING_CAPABILITY_INVALID", "generated_at must be RFC 3339")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        _error("ROUTING_CAPABILITY_INVALID", "generated_at must be RFC 3339")
    if parsed.tzinfo is None:
        _error("ROUTING_CAPABILITY_INVALID", "generated_at must include a timezone")
    return parsed.astimezone(timezone.utc)


def load_capabilities(path: Path, *, now: datetime, maximum_age_seconds: int, allow_loopback_http: bool = False) -> dict[str, Any]:
    if not path.exists():
        _error("ROUTING_DISCOVERY_FAILED", f"capability document not found: {path}", 3)
    text = _read_bounded(path, "ROUTING_CAPABILITY_INVALID")
    try:
        data = json.loads(text)
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        _error("ROUTING_CAPABILITY_INVALID", f"invalid capability JSON: {exc}", 3)
    return validate_capabilities(data, now=now, maximum_age_seconds=maximum_age_seconds, allow_loopback_http=allow_loopback_http)


class _RejectRedirects(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def fetch_capabilities(url: str, *, timeout_seconds: int, now: datetime, maximum_age_seconds: int, allow_loopback_http: bool = False) -> dict[str, Any]:
    _safe_url(url, "inferweave.discovery_url", "ROUTING_DISCOVERY_FAILED", allow_loopback_http)
    opener = build_opener(_RejectRedirects)
    request = Request(url, headers={"Accept": "application/json", "User-Agent": "autospec-route/1"})
    try:
        with opener.open(request, timeout=timeout_seconds) as response:
            content_length = response.headers.get("Content-Length")
            if content_length and int(content_length) > MAX_DOCUMENT_BYTES:
                _error("ROUTING_DISCOVERY_FAILED", "capability response is oversized", 3)
            body = response.read(MAX_DOCUMENT_BYTES + 1)
    except (HTTPError, URLError, TimeoutError, OSError, ValueError) as exc:
        _error("ROUTING_DISCOVERY_FAILED", f"capability discovery failed: {exc}", 3)
    if len(body) > MAX_DOCUMENT_BYTES:
        _error("ROUTING_DISCOVERY_FAILED", "capability response is oversized", 3)
    try:
        data = json.loads(body.decode("utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        _error("ROUTING_CAPABILITY_INVALID", f"invalid capability JSON: {exc}", 3)
    return validate_capabilities(data, now=now, maximum_age_seconds=maximum_age_seconds, allow_loopback_http=allow_loopback_http)


def validate_capabilities(data: Any, *, now: datetime, maximum_age_seconds: int, allow_loopback_http: bool = False) -> dict[str, Any]:
    reason = "ROUTING_CAPABILITY_INVALID"
    root = _object(data, "capability document", reason)
    _keys(root, {"version", "generated_at", "routes"}, {"version", "generated_at", "routes"}, "capability document", reason)
    if root["version"] != 1:
        _error(reason, "capability document version must be 1")
    generated = _parse_rfc3339(root["generated_at"])
    if (now.astimezone(timezone.utc) - generated).total_seconds() > maximum_age_seconds:
        _error("ROUTING_CAPABILITY_STALE", "capability document is stale", 3)
    if not isinstance(root["routes"], list):
        _error(reason, "capability routes must be a list")
    seen: set[str] = set()
    for index, raw in enumerate(root["routes"]):
        item = _object(raw, f"routes[{index}]", reason)
        required = {"id", "endpoint", "protocol", "model", "node_class", "modalities", "context_window", "max_input_tokens", "strength", "queue_seconds", "available", "opportunistic", "local"}
        _keys(item, required, required, f"routes[{index}]", reason)
        for key in ("id", "model", "node_class"):
            if not isinstance(item[key], str) or not item[key]:
                _error(reason, f"routes[{index}].{key} must be non-empty")
        if item["id"] in seen:
            _error(reason, f"duplicate capability route id: {item['id']}")
        seen.add(item["id"])
        _safe_url(item["endpoint"], f"routes[{index}].endpoint", reason, allow_loopback_http)
        if item["protocol"] not in PROTOCOLS:
            _error(reason, f"routes[{index}] has unsupported protocol")
        modalities = set(_string_list(item["modalities"], f"routes[{index}].modalities", reason))
        if not modalities <= MODALITIES:
            _error(reason, f"routes[{index}] has unsupported modality")
        if "image" in modalities and item["node_class"] not in VISION_NODE_CLASSES:
            _error(reason, f"vision route {item['id']} must use mac or rtx6000")
        context = _positive(item["context_window"], f"routes[{index}].context_window", reason)
        maximum = _positive(item["max_input_tokens"], f"routes[{index}].max_input_tokens", reason)
        if maximum > context:
            _error(reason, f"route {item['id']} input exceeds context window")
        _positive(item["strength"], f"routes[{index}].strength", reason, allow_zero=True)
        _positive(item["queue_seconds"], f"routes[{index}].queue_seconds", reason, allow_zero=True)
        for key in ("available", "opportunistic", "local"):
            if not isinstance(item[key], bool):
                _error(reason, f"routes[{index}].{key} must be boolean")
    return root


def _canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def resolve_dispatch(
    config: dict[str, Any],
    capabilities: dict[str, Any],
    kind: str,
    proposer: dict[str, Any] | None = None,
    *,
    available_harnesses: set[str] | None = None,
) -> dict[str, Any]:
    """Resolve one deterministic harness/capability pair from validated inputs."""
    config = validate_routing_config(config)
    if kind not in config["routes"]:
        _error("ROUTING_CONFIG_INVALID", f"unknown dispatch kind: {kind}")
    route = config["routes"][kind]
    inference_class = config["inference_classes"][route["inference_class"]]
    required_modalities = set(inference_class["modalities"])
    allowed_nodes = set(inference_class.get("eligible_node_classes", []))
    allow_opportunistic = route.get("allow_opportunistic", True)

    configured_harnesses = route["harnesses"]
    if available_harnesses is None:
        available_harnesses = set(configured_harnesses)
    harness_ids = [item for item in configured_harnesses if item in available_harnesses]
    if not harness_ids:
        _error("ROUTING_HARNESS_UNAVAILABLE", f"no configured harness is available for {kind}", 3)

    independent = route.get("independent_from")
    if independent and not proposer:
        _error("ROUTING_INDEPENDENCE_UNSATISFIED", f"{kind} requires a proposer envelope", 3)
    proposer_harness = (proposer or {}).get("harness", {}).get("id")
    proposer_route = (proposer or {}).get("inference", {}).get("route_id")
    proposer_strength = (proposer or {}).get("inference", {}).get("strength", 0)
    if independent:
        harness_ids = [item for item in harness_ids if item != proposer_harness]
        if not harness_ids:
            _error("ROUTING_INDEPENDENCE_UNSATISFIED", f"{kind} has no independent harness", 3)

    candidates = []
    for candidate in capabilities["routes"]:
        if not candidate["available"]:
            continue
        if not required_modalities <= set(candidate["modalities"]):
            continue
        if candidate["max_input_tokens"] < inference_class["max_input_tokens"]:
            continue
        if candidate["context_window"] < (
            inference_class["max_input_tokens"]
            + inference_class["reserve_output_tokens"]
        ):
            continue
        if candidate["queue_seconds"] > inference_class["max_queue_seconds"]:
            continue
        if allowed_nodes and candidate["node_class"] not in allowed_nodes:
            continue
        if not allow_opportunistic and candidate["opportunistic"]:
            continue
        if config["inferweave"]["local_only"] and not candidate["local"]:
            continue
        if independent and candidate["id"] == proposer_route:
            continue
        if route.get("minimum_strength") and candidate["strength"] < proposer_strength:
            continue
        candidates.append(candidate)
    if not candidates:
        reason = "ROUTING_INDEPENDENCE_UNSATISFIED" if independent else "ROUTING_CAPABILITY_UNAVAILABLE"
        _error(reason, f"no {route['inference_class']} candidate satisfies {kind}", 3)

    candidates.sort(
        key=lambda item: (
            bool(item["opportunistic"]),
            item["queue_seconds"],
            item["max_input_tokens"],
            item["id"],
        )
    )
    selected_harness = None
    selected_candidate = None
    for harness_id in harness_ids:
        harness = config["harnesses"][harness_id]
        supported = set(harness["provider_protocols"])
        for candidate in candidates:
            if candidate["protocol"] in supported:
                selected_harness = (harness_id, harness)
                selected_candidate = candidate
                break
        if selected_harness:
            break
    if selected_harness is None or selected_candidate is None:
        _error("ROUTING_ADAPTER_UNSUPPORTED", f"no {kind} harness supports an eligible protocol", 3)

    harness_id, harness = selected_harness
    decision_inputs = {
        "config": config,
        "capabilities": capabilities,
        "kind": kind,
        "proposer": proposer,
        "available_harnesses": sorted(available_harnesses),
    }
    digest = hashlib.sha256(_canonical(decision_inputs)).hexdigest()
    harness_envelope = {
        "id": harness_id,
        "command": list(harness["command"]),
        "transport": harness["transport"],
    }
    for optional in ("endpoint_env", "api_key_env"):
        if optional in harness:
            harness_envelope[optional] = harness[optional]
    return {
        "version": 1,
        "dispatch_id": f"sha256:{digest}",
        "kind": kind,
        "harness": harness_envelope,
        "inference": {
            "source": "inferweave",
            "route_id": selected_candidate["id"],
            "endpoint": selected_candidate["endpoint"],
            "protocol": selected_candidate["protocol"],
            "model": selected_candidate["model"],
            "modalities": list(selected_candidate["modalities"]),
            "context_window": selected_candidate["context_window"],
            "max_input_tokens": selected_candidate["max_input_tokens"],
            "reserve_output_tokens": inference_class["reserve_output_tokens"],
            "node_class": selected_candidate["node_class"],
            "strength": selected_candidate["strength"],
            "opportunistic": selected_candidate["opportunistic"],
        },
        "policy": {
            "independent_from": independent,
            "fallback": config["fallback"]["mode"],
        },
        "explain": [
            f"selected harness {harness_id}: first available configured harness",
            f"selected route {selected_candidate['id']}: eligible {route['inference_class']} candidate",
        ],
    }
