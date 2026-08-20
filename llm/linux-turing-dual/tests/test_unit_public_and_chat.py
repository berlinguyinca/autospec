"""The public read surface, and the chat panel's request rules.

Two policies live here, both stated as pure functions so they can be asserted
without a socket.

**Anyone may read this node.** How to reach the models, what is being served, how
busy and how fast it is, and who is on the leaderboard. Signing in is required
only to mint a key and to use the chat panel. The projections are allow-lists, so
every test below asserts ABSENCE as well as presence: that is the only form the
guarantee takes, and the failure mode of a forgotten field must be a missing
value rather than a disclosure.

**A cookie must not be able to spend this node.** The chat panel is the one place
where a browser session buys GPU time, and a cookie travels automatically -- so
the same-origin rule is tested from both sides, including the case where neither
header is present.
"""
import importlib

import pytest

from nodescripts import load_script

pub = load_script("publicview")
chat = load_script("chat")


# --- the node's numbers -----------------------------------------------------

def test_the_public_stats_carry_the_load_and_the_catalog():
    out = pub.stats({"llama_up": True, "model": "qwen3.8-27b",
                     "catalog": [{"id": "qwen3.8-27b", "context": 102400}],
                     "queue": {"slots": 2}, "tokens_per_second": 41.5,
                     "kv_cache_usage_ratio": 0.3})
    assert out["llama_up"] is True
    assert out["model"] == "qwen3.8-27b"
    assert out["catalog"][0]["context"] == 102400
    assert out["queue"]["slots"] == 2
    assert out["tokens_per_second"] == 41.5


def test_the_operators_diagnostics_are_not_public():
    """config_health reports journal readability and recent evictions. That is
    what an operator needs, and not what a visitor asked for."""
    out = pub.stats({"llama_up": True, "config_health": {"journal_readable": True}})
    assert "config_health" not in out


def test_a_field_added_upstream_is_not_published_until_it_is_named():
    """The allow-list's whole point. If this ever fails, someone replaced the
    projection with a deny-list and the next new field will leak."""
    out = pub.stats({"llama_up": True, "internal_host": "some-box.example.org",
                     "operator_note": "/etc/qwen-turing/site.conf"})
    assert "internal_host" not in out
    assert "operator_note" not in out
    assert "some-box.example.org" not in repr(out)


def test_gpu_telemetry_is_projected_per_card():
    out = pub.stats({"gpus": [{"index": 0, "name": "NVIDIA RTX 4090",
                               "mem_used_mib": 20000, "mem_total_mib": 24564,
                               "util_pct": 97, "temp_c": 71, "power_w": 330,
                               "uuid": "GPU-deadbeef", "serial": "1234"}]})
    g = out["gpus"][0]
    assert g["name"] == "NVIDIA RTX 4090" and g["util_pct"] == 97
    assert "uuid" not in g and "serial" not in g


# --- the fleet --------------------------------------------------------------

FULL_FLEET = {
    "servers": [{"id": "local", "kind": "local", "state": "online",
                 "models": ["qwen3.8-27b"], "priority": 0, "pool_member": True,
                 "in_flight": 1, "idle_pipes": None, "route": "/u/local/v1",
                 "base_url": None, "owner": None, "owner_name": None,
                 "error": None, "problems": [], "needs_key": True},
                {"id": "bender", "kind": "tunnel", "state": "online",
                 "models": ["exotic-70b"], "priority": 5, "pool_member": True,
                 "in_flight": 0, "idle_pipes": 4, "route": "/u/bender/v1",
                 "base_url": "http://gpu-box.invalid:8080/v1",
                 "owner": "cognito-sub-abc", "owner_name": "Alice",
                 "error": "connect to gpu-box.invalid:8080 refused",
                 "problems": ["unauthenticated"], "needs_key": False}],
    "poll_seconds": 30, "default_route": "/v1", "auto_route": "/u/auto/v1",
    "balanced_paths": ["/v1/chat/completions"], "routing": {"fastest": 3},
    "you": {"sub": "cognito-sub-abc", "is_admin": True},
    "registry_configured": True,
}


def test_the_public_fleet_names_the_servers_and_their_capability():
    out = pub.servers(FULL_FLEET)
    ids = [s["id"] for s in out["servers"]]
    assert ids == ["local", "bender"]
    bender = out["servers"][1]
    assert bender["models"] == ["exotic-70b"]
    assert bender["idle_pipes"] == 4 and bender["priority"] == 5
    assert out["default_route"] == "/v1"
    assert out["routing"] == {"fastest": 3}


def test_the_public_fleet_never_says_where_a_server_lives():
    """This repository is public and the whole node is built so that no
    identifier of this site appears outside site.conf. An upstream error string
    is an address too -- it quotes the host it failed to reach."""
    out = pub.servers(FULL_FLEET)
    blob = repr(out)
    assert "gpu-box.invalid" not in blob
    assert "cognito-sub-abc" not in blob
    for row in out["servers"]:
        for forbidden in ("base_url", "owner", "owner_name", "error",
                          "problems", "needs_key"):
            assert forbidden not in row


def test_the_public_fleet_publishes_measured_speed_and_seats():
    """Capability as EVIDENCE. The first version of the allow-list named a
    `measured` key that does not exist -- the figures are flattened onto the row
    -- so the public panel silently reported no speed for any server.
    """
    out = pub.servers({"servers": [{
        "id": "bender", "state": "online", "prefill_rate": 3410.18,
        "mean_service": 1.49, "samples": 16, "slots": 2,
        "gpus": "one RTX 4090 (24 GB)", "note": "the workstation"}]})
    row = out["servers"][0]
    assert round(row["prefill_rate"]) == 3410
    assert row["samples"] == 16 and row["slots"] == 2
    assert row["gpus"] == "one RTX 4090 (24 GB)"


def test_the_agent_build_is_not_capability():
    """A version string is software inventory, which is the operator's business."""
    out = pub.servers({"servers": [{"id": "x", "agent_version": "1"}]})
    assert "agent_version" not in out["servers"][0]


def test_the_public_fleet_does_not_say_who_is_asking():
    out = pub.servers(FULL_FLEET)
    assert "you" not in out
    assert "registry_configured" not in out


def test_a_new_server_field_is_not_published_until_it_is_named():
    payload = {"servers": [{"id": "x", "state": "online",
                            "admin_token": "sekrit"}]}
    assert "sekrit" not in repr(pub.servers(payload))


# --- the leaderboard --------------------------------------------------------

def test_the_leaderboard_ranks_by_name_without_publishing_contact_details():
    rows = [{"sub": "sub-a", "display_name": "Alice",
             "email": "alice@example.org", "requests": 9,
             "prompt_tokens": 100, "completion_tokens": 200,
             "cached_tokens": 5, "total_tokens": 300}]
    out = pub.leaderboard(rows)
    assert out[0]["display_name"] == "Alice"
    assert out[0]["total_tokens"] == 300
    assert "email" not in out[0] and "sub" not in out[0]
    assert "alice@example.org" not in repr(out)


def test_a_ranked_row_whose_owner_never_signed_in_is_kept():
    """Keys can be minted for a subject the registry has not met. Dropping such a
    row would make the public totals disagree with the node's own figures."""
    out = pub.leaderboard([{"display_name": None, "total_tokens": 7,
                            "requests": 1}])
    assert len(out) == 1
    assert out[0]["display_name"] is None and out[0]["total_tokens"] == 7


def test_an_empty_leaderboard_projects_to_an_empty_list():
    assert pub.leaderboard(None) == []
    assert pub.leaderboard([]) == []


# --- a cookie must not be able to spend this node ---------------------------

def _headers(**kw):
    lookup = {k.replace("_", "-").lower(): v for k, v in kw.items()}
    return lambda name: lookup.get(name.lower())


def test_the_pages_own_fetch_is_accepted():
    assert chat.same_origin(_headers(Sec_Fetch_Site="same-origin"), "llm.example.org")


def test_a_cross_site_fetch_is_refused():
    for site in ("cross-site", "same-site", "none"):
        assert not chat.same_origin(_headers(Sec_Fetch_Site=site), "llm.example.org")


def test_an_older_client_falls_back_to_origin():
    assert chat.same_origin(_headers(Origin="https://llm.example.org"),
                            "llm.example.org")


def test_an_origin_from_somewhere_else_is_refused():
    assert not chat.same_origin(_headers(Origin="https://evil.example.net"),
                                "llm.example.org")


def test_a_request_with_neither_header_is_refused():
    """Not a browser fetch. Something that is not a browser should be presenting
    a key at the real endpoint instead of spending a cookie here."""
    assert not chat.same_origin(_headers(), "llm.example.org")


def test_sec_fetch_site_wins_over_a_forged_origin():
    assert not chat.same_origin(
        _headers(Sec_Fetch_Site="cross-site", Origin="https://llm.example.org"),
        "llm.example.org")


# --- the panel's request rules ----------------------------------------------

SERVED = {"qwen3.8-27b", "exotic-70b"}


def _msg(text="hello", role="user"):
    return {"role": role, "content": text}


def ok(payload, served=SERVED):
    import json
    body, why = chat.validate(json.dumps(payload).encode(), served)
    assert why is None, why
    return body


def bad(payload, served=SERVED):
    import json
    body, why = chat.validate(json.dumps(payload).encode(), served)
    assert body is None and why
    return why


def test_a_valid_turn_becomes_a_streaming_completion():
    body = ok({"model": "qwen3.8-27b", "messages": [_msg()]})
    assert body["model"] == "qwen3.8-27b"
    assert body["messages"] == [{"role": "user", "content": "hello"}]
    assert body["stream"] is True
    assert body["max_tokens"] == chat.DEFAULT_MAX_TOKENS


def test_a_model_this_node_does_not_serve_is_refused_here():
    """The substitution guard, at the panel's own door. llama.cpp answers for a
    model it does not have -- measured on this fleet -- so an unchecked id comes
    back as somebody else's weights with a 200 and no error."""
    assert "does not serve" in bad({"model": "gpt-9", "messages": [_msg()]})


def test_thinking_is_off_unless_it_is_asked_for():
    """Reasoning tokens run FIRST, so a modest budget with thinking on returns
    empty content -- measured: max_tokens 16 yields nothing at all."""
    off = ok({"model": "qwen3.8-27b", "messages": [_msg()]})
    assert off["chat_template_kwargs"] == {"enable_thinking": False}
    on = ok({"model": "qwen3.8-27b", "messages": [_msg()], "thinking": True})
    assert "chat_template_kwargs" not in on


def test_an_oversized_budget_is_clamped_rather_than_refused():
    body = ok({"model": "qwen3.8-27b", "messages": [_msg()],
               "max_tokens": 999999})
    assert body["max_tokens"] == chat.MAX_TOKENS


def test_a_sensible_budget_is_honoured():
    assert ok({"model": "qwen3.8-27b", "messages": [_msg()],
               "max_tokens": 64})["max_tokens"] == 64


@pytest.mark.parametrize("payload,expect", [
    ({}, "name a model"),
    ({"model": ""}, "name a model"),
    ({"model": "qwen3.8-27b"}, "at least one message"),
    ({"model": "qwen3.8-27b", "messages": []}, "at least one message"),
    ({"model": "qwen3.8-27b", "messages": "hello"}, "at least one message"),
    ({"model": "qwen3.8-27b", "messages": ["hi"]}, "must be an object"),
    ({"model": "qwen3.8-27b", "messages": [{"role": "root", "content": "x"}]},
     "role must be one of"),
    ({"model": "qwen3.8-27b", "messages": [{"role": "user", "content": 7}]},
     "must be a string"),
    ({"model": "qwen3.8-27b", "messages": [_msg()], "max_tokens": 0},
     "positive whole number"),
    ({"model": "qwen3.8-27b", "messages": [_msg()], "max_tokens": "many"},
     "positive whole number"),
])
def test_a_malformed_turn_says_what_is_wrong(payload, expect):
    assert expect in bad(payload)


def test_a_boolean_is_not_a_token_budget():
    """True is an int in Python. A budget of `true` is a mistake, not a 1."""
    assert "positive whole number" in bad(
        {"model": "qwen3.8-27b", "messages": [_msg()], "max_tokens": True})


def test_the_panel_holds_a_conversation_not_a_corpus():
    many = [_msg("x")] * (chat.MAX_MESSAGES + 1)
    assert "messages at most" in bad({"model": "qwen3.8-27b", "messages": many})
    big = [_msg("x" * (chat.MAX_CHARS + 1))]
    assert "characters at most" in bad({"model": "qwen3.8-27b", "messages": big})


def test_a_conversation_at_the_limit_is_accepted():
    """The refusals above are only meaningful if the boundary itself passes."""
    assert ok({"model": "qwen3.8-27b",
               "messages": [_msg("x")] * chat.MAX_MESSAGES})
    assert ok({"model": "qwen3.8-27b", "messages": [_msg("x" * chat.MAX_CHARS)]})


def test_a_body_that_is_not_json_is_refused_as_such():
    body, why = chat.validate(b"<html>", SERVED)
    assert body is None and "not JSON" in why
    body, why = chat.validate(b'"a string"', SERVED)
    assert body is None and "JSON object" in why


def test_an_unknown_model_list_defers_to_the_upstream():
    """`None` means the node could not enumerate its models. Refusing every model
    because discovery is momentarily down would fail closed on telemetry."""
    body, why = chat.validate(
        b'{"model": "anything", "messages": [{"role":"user","content":"x"}]}',
        None)
    assert why is None and body["model"] == "anything"


def test_the_accounting_identity_is_a_sentinel_not_a_credential():
    """It must never look like a mintable key: the key namespaces are compiled
    prefixes, and a sentinel that parsed as one could be presented as a key."""
    keys = load_script("keys")
    assert chat.USAGE_KEY_ID == "dashboard-chat"
    for prefix in (keys.PREFIX, keys.PREFIX_SERVER, keys.PREFIX_ENROL):
        assert not chat.USAGE_KEY_ID.startswith(prefix)
    assert keys.parse(chat.USAGE_KEY_ID, keys.PREFIX) is None
