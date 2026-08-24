"""What the unauthenticated surfaces may say.

llama.cpp's own /v1/models publishes each child instance's full argv without a
key -- binary path, model paths, and the API key's FILE LOCATION. That is the
standard to avoid, not to copy, so these tests assert on the key SET rather than
on individual fields: a field added to a shared serialiser later must not be able
to leak by default.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "collect-stats.py"


def load():
    spec = importlib.util.spec_from_file_location("collect_stats", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


FULL = {
    "llama_up": True,
    "model": "qwen3.8-27b",
    "prompt_tokens_total": 53958,
    "generated_tokens_total": 36,
    "tokens_per_second": 28.7,
    "kv_cache_usage_ratio": 0.42,
    "gpu_count": 2,
    "gpu_total_mem_mib": 22528,
    "gpu_used_mem_mib": 18342,
    "gpus": [{"index": 0, "name": "NVIDIA GeForce RTX 2080 Ti", "temp_c": 48}],
    "queue": {"slots": 2, "processing": 1, "queued": 3, "fullness": 2.0,
              "est_wait_seconds": 12.0, "service_rate": 0.25,
              "mean_service_seconds": 4.0, "samples": 120, "completions": 30},
}


# --- the allow-list -------------------------------------------------------

def test_public_payload_matches_the_allow_list_exactly():
    m = load()
    assert set(m.public_payload(FULL).keys()) == set(m.PUBLIC_FIELDS)


def test_public_payload_drops_gpu_detail():
    """Card names and temperatures are node inventory, not load."""
    p = load().public_payload(FULL)
    assert "gpus" not in p
    assert "gpu_total_mem_mib" not in p


def test_public_payload_reports_model_loaded_as_a_boolean_not_a_name():
    p = load().public_payload(FULL)
    assert p["model_loaded"] is True
    assert "model" not in p
    assert "qwen3.8-27b" not in str(p)


def test_public_payload_has_no_path_separator_anywhere():
    """A filesystem path in a public payload is a leak regardless of its field."""
    assert "/" not in str(load().public_payload(FULL))


def test_public_payload_survives_a_new_field_added_upstream():
    m = load()
    dirty = dict(FULL, secret_path="/etc/qwen-turing.key",
                 argv=["--api-key-file", "/run/credentials/x/apikey"])
    p = m.public_payload(dirty)
    assert set(p.keys()) == set(m.PUBLIC_FIELDS)
    assert "qwen-turing.key" not in str(p)


def test_public_payload_when_nothing_is_loaded():
    m = load()
    p = m.public_payload({"llama_up": False, "queue": {}})
    assert p["model_loaded"] is False
    assert p["est_wait_seconds"] is None
    assert set(p.keys()) == set(m.PUBLIC_FIELDS)


# --- queue_state ----------------------------------------------------------

def test_queue_state_reads_processing_and_deferred():
    q = load().queue_state({"llamacpp:requests_processing": 1.0,
                            "llamacpp:requests_deferred": 3.0}, slots_total=2)
    assert q["processing"] == 1
    assert q["queued"] == 3
    assert q["slots"] == 2
    assert q["outstanding"] == 4


def test_queue_state_fullness_is_outstanding_over_slots():
    q = load().queue_state({"llamacpp:requests_processing": 1.0,
                            "llamacpp:requests_deferred": 3.0}, slots_total=2)
    assert q["fullness"] == 2.0          # may exceed 1.0, and that is the point


def test_queue_state_with_zero_slots_does_not_divide_by_zero():
    q = load().queue_state({"llamacpp:requests_processing": 0.0}, slots_total=0)
    assert q["fullness"] is None
    assert q["slots"] == 0


def test_queue_state_missing_metrics_are_zero_not_none():
    q = load().queue_state({}, slots_total=2)
    assert q["processing"] == 0 and q["queued"] == 0


# --- sanitise_models ------------------------------------------------------

UPSTREAM = {"data": [{
    "id": "qwen3.8-27b",
    "aliases": ["qwen3.8-27b-40k", "qwen3.8-27b-100k"],
    "object": "model",
    "owned_by": "llamacpp",
    "created": 1787173730,
    "status": {"value": "loaded",
               "args": ["/opt/qwen-turing/llama.cpp/current/llama-server",
                        "--api-key-file",
                        "/run/credentials/qwen-turing@router.service/apikey"]},
}]}


def test_sanitise_keeps_what_clients_need():
    e = load().sanitise_models(UPSTREAM)["data"][0]
    assert e["id"] == "qwen3.8-27b"
    assert e["aliases"] == ["qwen3.8-27b-40k", "qwen3.8-27b-100k"]
    assert e["object"] == "model"


def test_sanitise_drops_status_and_argv():
    assert "status" not in load().sanitise_models(UPSTREAM)["data"][0]


def test_sanitise_leaves_no_path_anywhere():
    """The whole reason this function exists."""
    d = load().sanitise_models(UPSTREAM)
    assert "/" not in str(d)
    assert "apikey" not in str(d)


def test_sanitise_of_garbage_is_an_empty_list_not_an_exception():
    m = load()
    assert m.sanitise_models({}) == {"object": "list", "data": []}
    assert m.sanitise_models({"data": "nonsense"}) == {"object": "list", "data": []}
