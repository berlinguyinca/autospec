"""The registry of routable servers.

Validation is asserted from the REFUSING side as hard as the accepting side: a
registry that quietly drops a malformed entry produces a Servers panel where a
misconfigured box is indistinguishable from a box nobody configured.
"""
import pytest

from conftest import load_script

up = load_script("upstreams")

GOOD = """
upstreams:
  - id: bigbox
    base_url: http://example.invalid:8000/v1
    key_file: /etc/qwen-turing/bigbox.key
    note: the workstation
    gpus: one RTX 4090
"""


def test_a_good_entry_parses_and_is_usable():
    u = up.load(GOOD)[0]
    assert u.id == "bigbox"
    assert u.base_url == "http://example.invalid:8000/v1"
    assert u.enabled is True and u.usable is True
    assert u.problems == []


def test_the_public_view_omits_the_key_path():
    pub = up.load(GOOD)[0].public()
    assert "key_file" not in pub
    assert pub["needs_key"] is True
    assert pub["gpus"] == "one RTX 4090"


def test_an_entry_can_be_parked_without_deleting_it():
    u = up.load(GOOD.replace("id: bigbox", "id: bigbox\n    enabled: false"))[0]
    assert u.enabled is False and u.usable is False
    assert u.problems == []          # parked is not broken


@pytest.mark.parametrize("bad,why", [
    ("id: BigBox", "id must be lowercase letters, digits and dashes"),
    ("id: has spaces", "id must be lowercase letters, digits and dashes"),
    ("id: local", "'local' is reserved"),
    ("id: auto", "'auto' is reserved"),
])
def test_bad_ids_are_reported_not_dropped(bad, why):
    u = up.load(GOOD.replace("id: bigbox", bad))[0]
    assert why in u.problems
    assert u.usable is False


def test_a_placeholder_base_url_is_refused():
    u = up.load(GOOD.replace("http://example.invalid:8000/v1",
                             "http://<host>:<port>/v1"))[0]
    assert "base_url is still a placeholder" in u.problems


def test_a_non_http_base_url_is_refused():
    u = up.load(GOOD.replace("http://example.invalid:8000/v1", "example.invalid:8000"))[0]
    assert "base_url must start with http:// or https://" in u.problems


def test_a_missing_id_is_reported():
    assert "no id" in up.load("upstreams:\n  - base_url: http://example.invalid/v1\n")[0].problems


def test_duplicate_ids_are_reported_on_the_second():
    two = GOOD + """
  - id: bigbox
    base_url: http://other.invalid:8000/v1
"""
    a, b = up.load(two)
    assert a.problems == [] and "duplicate id" in b.problems


def test_one_bad_entry_does_not_hide_the_others():
    mixed = """
upstreams:
  - id: BAD
    base_url: http://a.invalid/v1
  - id: good
    base_url: http://b.invalid/v1
"""
    entries = up.load(mixed)
    assert len(entries) == 2
    assert entries[1].usable is True


def test_an_empty_registry_is_not_an_error():
    assert up.load("") == []
    assert up.load("upstreams:\n") == []


def test_a_list_is_required():
    with pytest.raises(ValueError):
        up.load("upstreams: not-a-list\n")
    # Positive control: a well-formed list still parses, so this cannot pass
    # because load() started raising unconditionally.
    assert len(up.load(GOOD)) == 1


# --- routing ---------------------------------------------------------------

def test_a_plain_path_routes_locally():
    ups = up.load(GOOD)
    assert up.route("/v1/chat/completions", ups) == (None, "/v1/chat/completions")


def test_a_prefixed_path_routes_to_that_server():
    ups = up.load(GOOD)
    u, path = up.route("/u/bigbox/v1/chat/completions", ups)
    assert u.id == "bigbox" and path == "/v1/chat/completions"


def test_an_unknown_server_is_refused_rather_than_guessed():
    assert up.route("/u/nosuch/v1/chat/completions", up.load(GOOD)) is None


def test_a_parked_or_broken_server_is_refused():
    parked = up.load(GOOD.replace("id: bigbox", "id: bigbox\n    enabled: false"))
    assert up.route("/u/bigbox/v1/chat/completions", parked) is None


def test_the_bare_server_path_still_routes():
    u, path = up.route("/u/bigbox", up.load(GOOD))
    assert u.id == "bigbox" and path == "/"


# --- target rewriting ------------------------------------------------------

def test_the_base_url_prefix_is_not_duplicated():
    """base_url ends in /v1 and the incoming path starts with /v1; naive
    concatenation produces /v1/v1 and a 404 that looks like a dead server."""
    u = up.load(GOOD)[0]
    scheme, host, port, path = up.target(u, "/v1/chat/completions")
    assert (scheme, host, port) == ("http", "example.invalid", 8000)
    assert path == "/v1/chat/completions"


def test_https_defaults_to_443_and_http_to_80():
    a = up.load(GOOD.replace("http://example.invalid:8000/v1", "https://a.invalid/v1"))[0]
    assert up.target(a, "/v1/models")[2] == 443
    b = up.load(GOOD.replace("http://example.invalid:8000/v1", "http://b.invalid/v1"))[0]
    assert up.target(b, "/v1/models")[2] == 80


# --- what may be balanced at all --------------------------------------------

def test_only_model_bearing_endpoints_are_balanced():
    for ok in ("/v1/chat/completions", "/v1/completions", "/v1/embeddings",
               "/v1/rerank", "/v1/reranking", "/v1/chat/completions/"):
        assert up.balanceable(ok) is True, ok


def test_this_machines_own_endpoints_are_never_balanced():
    """These describe THIS box. Answering /slots or /metrics from another
    machine would report someone else's GPUs as ours."""
    for no in ("/health", "/props", "/metrics", "/slots", "/completion",
               "/v1/models", "/tokenize", "/detokenize", "/apply-template",
               "/v1/audio/speech", "/", "/v1"):
        assert up.balanceable(no) is False, no


# --- pinning this node explicitly -------------------------------------------

def test_u_local_pins_this_node():
    """It used to REFUSE -- `local` was a reserved id and never a destination.
    Now that the plain path is balanced, this is the only way to insist."""
    assert up.route("/u/local/v1/chat/completions", up.load(TWO)) == (
        up.LOCAL, "/v1/chat/completions")
    assert up.route("/u/local", up.load(TWO)) == (up.LOCAL, "/")
    assert up.route("/u/local/", up.load(TWO)) == (up.LOCAL, "/")


def test_local_is_still_a_reserved_registry_id():
    u = up.load(TWO.replace("id: alpha", "id: local"))[0]
    assert "'local' is reserved" in u.problems and u.usable is False


# --- the virtual "auto" server ----------------------------------------------

TWO = """
upstreams:
  - id: alpha
    base_url: http://a.invalid:8000/v1
  - id: beta
    base_url: http://b.invalid:8000/v1
"""

M = "qwen3.8-27b"
# A model only this node has. The fleet is genuinely like this: seven ids here,
# one on the workstation.
LOCAL_ONLY = "qwen3.5-9b-vision"
LOCAL_MODELS = [M, LOCAL_ONLY]

ONLINE = {"alpha": {"state": "online", "models": [M]},
          "beta": {"state": "online", "models": [M]}}


def test_auto_is_a_reserved_id():
    u = up.load(TWO.replace("id: alpha", "id: auto"))[0]
    assert "'auto' is reserved" in u.problems
    assert u.usable is False


def _pick(*a, **kw):
    """pick_auto without the estimate, which these tests do not assert."""
    sid, why, _est = up.pick_auto(*a, **kw)
    return sid, why


def test_auto_prefers_the_server_you_used_last():
    """The reason this rule exists: a warm prefix cache is worth ~10x here, and
    it lives on whichever machine served you last."""
    ups = up.load(TWO)
    assert _pick(ups, ONLINE, "beta", model=M) == ("beta", "warm")
    assert _pick(ups, ONLINE, "alpha", model=M) == ("alpha", "warm")
    # Reported as `fastest`, not `warm`: this node already wins the tie by
    # registry order, so warmth did not decide it. The reason names the deciding
    # factor, and claiming `warm` here would misattribute the choice.
    assert _pick(ups, ONLINE, "local", model=M) == ("local", "fastest")


def test_affinity_never_outranks_being_able_to_serve_the_model():
    """The ordering that matters. You used beta last, but you are now asking for
    a model beta has not got -- and llama.cpp would answer with the wrong model
    rather than refuse, so beta must drop out entirely."""
    ups = up.load(TWO)
    assert _pick(ups, ONLINE, "beta", model=LOCAL_ONLY,
                 local_models=LOCAL_MODELS) == ("local", "only-server")


def test_auto_falls_back_when_your_last_server_went_offline():
    ups = up.load(TWO)
    state = {"alpha": {"state": "offline", "models": [M]},
             "beta": {"state": "online", "models": [M]}}
    # Affinity must not pin you to a dead box.
    assert _pick(ups, state, "alpha", model=M)[0] == "local"
    assert _pick(ups, state, "alpha", model=M,
                 local_online=False) == ("beta", "only-server")


def test_auto_prefers_this_node_when_nothing_is_remembered():
    """Nothing is measured yet, so every candidate estimates the same and the
    tie falls to registry order -- which puts this node first, and costs no
    network hop."""
    assert _pick(up.load(TWO), ONLINE, None, model=M) == ("local", "fastest")


def test_auto_uses_a_remote_when_this_node_is_down():
    assert _pick(up.load(TWO), ONLINE, None, model=M,
                 local_online=False) == ("alpha", "fastest")


def test_auto_uses_a_remote_when_only_the_remote_has_the_model():
    """Not a fallback -- the only correct answer. This node cannot serve it."""
    state = {"alpha": {"state": "online", "models": ["exotic-70b"]},
             "beta": {"state": "online", "models": [M]}}
    assert _pick(up.load(TWO), state, "local", model="exotic-70b",
                 local_models=LOCAL_MODELS) == ("alpha", "only-server")


def test_auto_ignores_servers_that_are_not_online():
    ups = up.load(TWO)
    unknown = {"alpha": {"state": "unknown", "models": [M]},
               "beta": {"state": "offline", "models": [M]}}
    assert _pick(ups, unknown, None, model=M,
                 local_online=False) == (None, "none-eligible")
    assert _pick(ups, unknown, "beta", model=M,
                 local_online=False) == (None, "none-eligible")


def test_auto_refuses_rather_than_guessing_when_nothing_is_up():
    assert _pick([], {}, None, model=M, local_online=False) == (None, "none-eligible")
    assert _pick(up.load(TWO), {}, "alpha", model=M,
                 local_online=False) == (None, "none-eligible")


def test_auto_ignores_a_parked_server_even_if_it_answers():
    parked = up.load(TWO.replace("id: beta", "id: beta\n    enabled: false"))
    assert _pick(parked, ONLINE, "beta", model=M)[0] == "local"


def test_measured_speed_decides_between_two_that_can_both_serve_it():
    """The amendment's requirement: a box that delivers 40 tok/s is ranked by the
    40, not by what it claims to be."""
    stats = {("alpha", M): {"prefill_rate": 2000.0, "mean_service": 5.0},
             ("beta", M): {"prefill_rate": 100.0, "mean_service": 5.0}}
    assert _pick(up.load(TWO), ONLINE, None, model=M, local_online=False,
                 stats=stats, prompt_tokens=50_000) == ("alpha", "fastest")


def test_load_is_taken_from_the_caller_and_can_beat_speed():
    stats = {("alpha", M): {"prefill_rate": 2000.0, "mean_service": 30.0},
             ("beta", M): {"prefill_rate": 500.0, "mean_service": 30.0}}
    assert _pick(up.load(TWO), ONLINE, None, model=M, local_online=False,
                 stats=stats, load={"alpha": 4}, prompt_tokens=10_000) == (
        "beta", "fastest")


def test_an_operator_tier_wins_and_says_so():
    tiered = up.load(TWO)
    for u in tiered:
        if u.id == "beta":
            u.priority = 2
    stats = {("alpha", M): {"prefill_rate": 5000.0, "mean_service": 1.0},
             ("beta", M): {"prefill_rate": 50.0, "mean_service": 60.0}}
    assert _pick(tiered, ONLINE, None, model=M, local_online=False,
                 stats=stats, prompt_tokens=10_000) == ("beta", "priority")


def test_a_model_statistic_beats_a_server_wide_one(monkeypatch):
    """A 9B and a 27B on one card differ by an order of magnitude, so the
    per-model figure must be preferred where it exists."""
    stats = {("alpha", None): {"prefill_rate": 5000.0, "mean_service": 1.0},
             ("alpha", M): {"prefill_rate": 10.0, "mean_service": 100.0},
             ("beta", None): {"prefill_rate": 400.0, "mean_service": 5.0}}
    assert _pick(up.load(TWO), ONLINE, None, model=M, local_online=False,
                 stats=stats, prompt_tokens=10_000) == ("beta", "fastest")


def test_a_server_with_no_idle_capacity_is_ranked_last_not_excluded():
    """The tunnelled case: no pipe free right now. The balanced route should go
    around it, but a pin must still be able to reach it."""
    assert _pick(up.load(TWO), ONLINE, None, model=M, local_online=False,
                 ready={"alpha": False}, prompt_tokens=1000) == ("beta", "fastest")
    assert _pick(up.load(TWO), ONLINE, None, model=M, local_online=False,
                 ready={"alpha": False, "beta": False})[0] == "alpha"


# --- eligibility, which is the safety property -------------------------------

def test_a_server_that_does_not_advertise_the_model_is_not_eligible():
    """The measured reason: asked for qwen3.5-9b-vision, the workstation
    answered as qwen3.8-27b with no error. Wrong, not slow."""
    ups = up.load(TWO)
    assert up.eligible(ups, ONLINE, LOCAL_ONLY, local_models=LOCAL_MODELS) == ["local"]


def test_a_server_with_no_polled_list_is_excluded_not_included():
    """Eligibility is a POSITIVE check, so a failed poll loses a server instead
    of silently winning one."""
    ups = up.load(TWO)
    blank = {"alpha": {"state": "online"}, "beta": {"state": "online", "models": []}}
    assert up.eligible(ups, blank, M, local_models=LOCAL_MODELS) == ["local"]


def test_an_unreadable_model_keeps_the_request_on_this_node():
    """The peek buffer ran out. A remote might answer with the wrong model, so
    only this node is eligible -- and the choice is labelled `blind` so a rising
    count of them is visible rather than a mystery."""
    ups = up.load(TWO)
    assert up.eligible(ups, ONLINE, None) == ["local"]
    assert up.pick_auto(ups, ONLINE, "beta", model=None) == ("local", "blind", None)


def test_an_unreadable_model_with_this_node_down_is_refused():
    assert up.pick_auto(up.load(TWO), ONLINE, None, model=None,
                        local_online=False) == (None, "none-eligible", None)


def test_this_node_is_eligible_when_its_own_list_is_unknown():
    """A broken local probe must not empty the fleet: this node is the model
    authority here, so an unknown list fails OPEN for it."""
    assert up.eligible(up.load(TWO), ONLINE, LOCAL_ONLY, local_models=None) == ["local"]
    assert up.eligible(up.load(TWO), ONLINE, LOCAL_ONLY, local_models=[]) == ["local"]


def test_this_node_drops_out_for_a_model_it_does_not_serve():
    state = {"alpha": {"state": "online", "models": ["exotic-70b"]},
             "beta": {"state": "online", "models": [M]}}
    assert up.eligible(up.load(TWO), state, "exotic-70b",
                       local_models=LOCAL_MODELS) == ["alpha"]


def test_a_server_outside_the_pool_is_not_eligible_for_the_default_route():
    """Attaching a box is self-service; putting a stranger's hardware into
    everyone's /v1 is not. A pin asks for it by name, which is a different
    question, so pool_only is off there."""
    ups = up.load(TWO)
    for u in ups:
        u.pool_member = False
    assert up.eligible(ups, ONLINE, M, local_models=LOCAL_MODELS) == ["local"]
    assert up.eligible(ups, ONLINE, M, local_models=LOCAL_MODELS,
                       pool_only=False) == ["local", "alpha", "beta"]


def test_a_file_entry_is_in_the_pool_by_default():
    """It was configured by whoever runs the node, and it was already balanced
    before any of this existed -- changing that would be a silent regression."""
    assert all(u.pool_member and u.kind == up.KIND_FILE for u in up.load(TWO))


def test_an_attached_server_starts_outside_the_pool():
    rows = [{"server_id": "t1", "kind": "tunnel", "sub": "sub-a",
             "pool_member": False, "priority": 0},
            {"server_id": "s1", "kind": "static", "sub": "sub-a",
             "base_url": "http://box.invalid:8000/v1", "pool_member": True,
             "priority": 2}]
    made = {u.id: u for u in up.from_records(rows)}
    assert made["t1"].pool_member is False and made["t1"].kind == "tunnel"
    assert made["t1"].direct is False and made["t1"].problems == []
    assert made["s1"].pool_member is True and made["s1"].priority == 2
    assert made["s1"].direct is True


def test_a_static_record_without_a_usable_address_says_so():
    made = up.from_records([{"server_id": "s1", "kind": "static", "base_url": ""}])[0]
    assert made.usable is False and made.problems == ["no usable base_url"]


def test_a_tunnelled_server_never_reports_an_address():
    """It has none: it dialled in. A field for one would invite filling it in,
    and then the node would dial a box that expects to dial out."""
    made = up.from_records([{"server_id": "t1", "kind": "tunnel",
                             "base_url": "http://leaked.invalid/v1"}])[0]
    assert made.public()["base_url"] is None


def test_where_a_model_lives_is_reportable_for_the_refusal():
    """So a refusal can say "it is on beta, which is not answering" instead of
    the dead end "nothing serves it"."""
    state = {"alpha": {"state": "offline", "models": ["exotic-70b"]},
             "beta": {"state": "online", "models": [M]}}
    ups = up.load(TWO)
    assert up.servers_for(ups, state, "exotic-70b") == ["alpha"]
    assert up.servers_for(ups, state, "nowhere-9b") == []
