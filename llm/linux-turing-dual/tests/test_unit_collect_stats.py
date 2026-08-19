"""The stats collector.

Deliberately small: llama.cpp already publishes tokens, throughput and KV-pool
usage on /metrics, and nvidia-smi reports per-card utilisation, so the whole
dashboard is one collector plus one page rather than Prometheus, Grafana and an
exporter. These tests pin the two parsers and, most importantly, that both
degrade to empty rather than raising -- a stats page that 500s while the model
is reloading is worse than one showing a gap.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


PROM = """\
# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.
# TYPE llamacpp:prompt_tokens_total counter
llamacpp:prompt_tokens_total 123456
llamacpp:tokens_predicted_total 7890
llamacpp:predicted_tokens_seconds 29.7
llamacpp:kv_cache_usage_ratio 0.42
llamacpp:requests_processing 1
llamacpp:requests_deferred 0
"""

CSV = ("0, NVIDIA GeForce RTX 2080 Ti, 47, 8192, 11264, 61, 142.50\n"
       "1, NVIDIA GeForce RTX 2080 Ti, 39, 7100, 11264, 58, 130.00\n")


# --- prometheus -------------------------------------------------------------

def test_parses_counters_and_gauges():
    d = load().parse_prometheus(PROM)
    assert d["llamacpp:prompt_tokens_total"] == 123456.0
    assert d["llamacpp:tokens_predicted_total"] == 7890.0
    assert d["llamacpp:predicted_tokens_seconds"] == 29.7
    assert d["llamacpp:kv_cache_usage_ratio"] == 0.42


def test_comments_are_not_metrics():
    assert not any(k.startswith("#") for k in load().parse_prometheus(PROM))


def test_help_and_type_lines_do_not_become_metrics():
    d = load().parse_prometheus(PROM)
    assert "HELP" not in " ".join(d.keys())
    assert len(d) == 6


def test_empty_metrics_is_empty_dict_not_an_exception():
    assert load().parse_prometheus("") == {}


def test_unparseable_value_is_skipped_not_fatal():
    d = load().parse_prometheus("good 1.5\nbad notanumber\n")
    assert d == {"good": 1.5}


# --- nvidia-smi -------------------------------------------------------------

def test_parses_every_card():
    gpus = load().parse_nvidia_csv(CSV)
    assert len(gpus) == 2
    assert gpus[0]["index"] == 0 and gpus[1]["index"] == 1


def test_reads_each_field():
    g = load().parse_nvidia_csv(CSV)[0]
    assert g["name"] == "NVIDIA GeForce RTX 2080 Ti"
    assert g["util_pct"] == 47
    assert g["mem_used_mib"] == 8192
    assert g["mem_total_mib"] == 11264
    assert g["temp_c"] == 61
    assert g["power_w"] == 142.5


def test_second_card_is_not_a_copy_of_the_first():
    """The whole family of bugs this node exposed was reading only card 0."""
    gpus = load().parse_nvidia_csv(CSV)
    assert gpus[1]["mem_used_mib"] == 7100
    assert gpus[0]["mem_used_mib"] != gpus[1]["mem_used_mib"]


def test_no_devices_is_empty_list():
    assert load().parse_nvidia_csv("") == []


def test_not_supported_fields_become_none_rather_than_crashing():
    """nvidia-smi prints [N/A] for unsupported sensors on some cards."""
    g = load().parse_nvidia_csv(
        "0, Some Card, [N/A], 100, 200, [N/A], [N/A]\n")[0]
    assert g["util_pct"] is None
    assert g["temp_c"] is None
    assert g["mem_used_mib"] == 100


def test_short_row_is_skipped():
    assert load().parse_nvidia_csv("0, incomplete\n") == []


# --- the joined payload ------------------------------------------------------

def test_summarise_totals_vram_across_cards():
    m = load()
    s = m.summarise(m.parse_prometheus(PROM), m.parse_nvidia_csv(CSV))
    assert s["gpu_total_mem_mib"] == 22528
    assert s["gpu_used_mem_mib"] == 8192 + 7100
    assert s["prompt_tokens_total"] == 123456
    assert s["generated_tokens_total"] == 7890
    assert s["tokens_per_second"] == 29.7
    assert s["kv_cache_usage_ratio"] == 0.42


def test_summarise_survives_a_dead_server():
    """Model reloading, or the unit down: report the GPUs, not a traceback."""
    m = load()
    s = m.summarise({}, m.parse_nvidia_csv(CSV))
    assert s["prompt_tokens_total"] == 0
    assert s["gpu_total_mem_mib"] == 22528
    assert s["llama_up"] is False


def test_summarise_marks_llama_up_when_metrics_present():
    m = load()
    assert m.summarise(m.parse_prometheus(PROM), [])["llama_up"] is True
