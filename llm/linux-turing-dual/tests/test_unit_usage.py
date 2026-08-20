"""Exact token accounting, against RESPONSES CAPTURED FROM THE RUNNING NODE.

The fixtures are real bytes, not hand-written. A fixture built from the parser's
own assumptions cannot catch a bug in those assumptions -- which is how a
transcript-slug bug once shipped green for months in this repo.

Known values in the fixtures: prompt=13, completion=12 tokens.
"""
import importlib.util
import json
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPTS = HERE.parent / "scripts"
FIX = HERE / "fixtures"


def _load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    # Registered BEFORE exec, exactly as a real import does. `from __future__
    # import annotations` makes @dataclass resolve its field types through
    # sys.modules[cls.__module__], so a module loaded by path alone raises
    # "NoneType has no attribute __dict__" at class-creation time.
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


usage = _load("usage")


def _stream(name):
    acc = usage.StreamAccountant()
    acc.feed((FIX / name).read_bytes())
    return acc.result()


def test_streaming_without_stream_options_still_yields_exact_counts():
    """The decisive property: llama.cpp emits `timings` unconditionally, so the
    proxy never has to rewrite a client's request body to add stream_options."""
    u = _stream("stream_timings_only.sse")
    assert u.prompt_tokens == 13
    assert u.completion_tokens == 12
    assert u.truncated is False
    assert u.prompt_ms and u.prompt_ms > 0
    assert u.predicted_ms and u.predicted_ms > 0


def test_streaming_with_usage_prefers_the_usage_object():
    u = _stream("stream_with_usage.sse")
    assert u.prompt_tokens == 13
    assert u.completion_tokens == 12
    assert u.cached_tokens == 0        # present and zero -- not None
    assert u.truncated is False


def test_non_streaming_reads_usage_and_cached_tokens():
    body = json.loads((FIX / "nonstream.json").read_text())
    u = usage.from_json_body(body)
    assert u.prompt_tokens == 13
    assert u.completion_tokens == 12
    assert u.cached_tokens == 9         # a real prefix-cache hit
    assert u.truncated is False


def test_a_truncated_stream_is_marked_unknown_and_never_zero():
    """A client that disconnects mid-stream never sends the terminal chunk, so
    the count is genuinely unknown. Reporting zero would silently under-bill;
    estimating from byte counts would invent a measurement."""
    u = _stream("stream_truncated.sse")
    assert u.truncated is True
    assert u.prompt_tokens is None
    assert u.completion_tokens is None


@pytest.mark.parametrize("size", [1, 3, 17, 64, 500, 4096])
def test_chunk_boundaries_do_not_change_the_result(size):
    """Network reads split wherever they like, including mid-JSON and mid-UTF-8.
    The result must not depend on where."""
    raw = (FIX / "stream_with_usage.sse").read_bytes()
    acc = usage.StreamAccountant()
    for i in range(0, len(raw), size):
        acc.feed(raw[i:i + size])
    u = acc.result()
    assert (u.prompt_tokens, u.completion_tokens) == (13, 12)


def test_the_accountant_does_not_retain_the_whole_stream():
    """A 100k-token exchange must not be accumulated in memory just to read its
    last chunk."""
    acc = usage.StreamAccountant()
    blob = b"data: " + json.dumps({"choices": [{"delta": {"content": "x" * 900}}]}).encode() + b"\n\n"
    for _ in range(300):
        acc.feed(blob)
    assert acc.buffered_bytes() <= usage.MAX_TAIL_BYTES


def test_an_empty_stream_is_truncated_not_zero():
    acc = usage.StreamAccountant()
    u = acc.result()
    assert u.truncated is True and u.prompt_tokens is None


def test_a_body_with_no_usage_at_all_is_unknown():
    u = usage.from_json_body({"choices": []})
    assert u.truncated is True and u.prompt_tokens is None


def test_garbage_does_not_raise():
    acc = usage.StreamAccountant()
    acc.feed(b"data: {not json\n\ndata: [DONE]\n\n")
    u = acc.result()
    assert u.truncated is True
