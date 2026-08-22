"""Config-health signals.

Each is a fixable misconfiguration rather than a fault, so the point is to name
the remedy. The one hard rule: an unreadable journal must not look like a quiet
one -- "no evictions" is exactly the good news an operator would act on.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


JOURNAL = "\n".join([
    "[38805] 0.03.390.914 I srv    load_model: initializing, n_slots = 2",
    "3.00.332.657 I srv  ensure_model: evicting idle LRU name=qwen3.5-9b to make room for name=qwen3.8-27b",
    "3.00.332.658 I srv        unload: stopping model instance name=qwen3.5-9b",
    "[58527] 0.06.080.972 W srv    load_model: cache_reuse is not supported by this context, it will be disabled",
    "12.03.715.819 I srv    unload_all: stopping model instance name=qwen3.8-27b",
])


def test_parses_an_eviction_with_both_model_names():
    e = load().parse_journal_events(JOURNAL)["evictions"]
    assert len(e) == 1
    assert e[0]["from"] == "qwen3.5-9b"
    assert e[0]["to"] == "qwen3.8-27b"


def test_parses_unload_events():
    u = load().parse_journal_events(JOURNAL)["unloads"]
    assert "qwen3.5-9b" in u and "qwen3.8-27b" in u


def test_parses_silently_disabled_options():
    d = load().parse_journal_events(JOURNAL)["disabled"]
    assert any("cache_reuse" in x for x in d)


def test_ordinary_load_lines_are_not_events():
    ev = load().parse_journal_events("I srv load_model: initializing, n_slots = 2")
    assert ev == {"evictions": [], "unloads": [], "disabled": []}


def test_empty_journal_is_empty_events():
    assert load().parse_journal_events("") == {"evictions": [], "unloads": [], "disabled": []}


def test_disabled_options_are_deduplicated():
    """The warning is logged on every load; the panel should say it once."""
    text = "\n".join([
        "W srv load_model: cache_reuse is not supported by this context, it will be disabled",
        "W srv load_model: cache_reuse is not supported by this context, it will be disabled",
    ])
    assert load().parse_journal_events(text)["disabled"] == ["cache_reuse"]


# --- cache health ---------------------------------------------------------

def test_hit_rate_from_metrics():
    c = load().cache_health({"llamacpp:prompt_tokens_total": 20000.0,
                             "llamacpp:prompt_tokens_cached_total": 19425.0})
    assert c["prompt_tokens"] == 20000
    assert c["cached_tokens"] == 19425
    assert abs(c["hit_rate"] - 0.97125) < 1e-6


def test_hit_rate_is_none_before_any_prompt():
    """Zero prompts is not a zero hit rate: idle and thrashing must differ."""
    c = load().cache_health({})
    assert c["prompt_tokens"] == 0
    assert c["hit_rate"] is None


def test_hit_rate_zero_is_distinguishable_from_unknown():
    c = load().cache_health({"llamacpp:prompt_tokens_total": 5000.0,
                             "llamacpp:prompt_tokens_cached_total": 0.0})
    assert c["hit_rate"] == 0.0
