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


# --- the virtual "auto" server ----------------------------------------------

TWO = """
upstreams:
  - id: alpha
    base_url: http://a.invalid:8000/v1
  - id: beta
    base_url: http://b.invalid:8000/v1
"""

ONLINE = {"alpha": {"state": "online"}, "beta": {"state": "online"}}


def test_auto_is_a_reserved_id():
    u = up.load(TWO.replace("id: alpha", "id: auto"))[0]
    assert "'auto' is reserved" in u.problems
    assert u.usable is False


def test_auto_prefers_the_server_you_used_last():
    """The reason this rule exists: a warm prefix cache is worth ~10x here, and
    it lives on whichever machine served you last."""
    ups = up.load(TWO)
    assert up.pick_auto(ups, ONLINE, last_used="beta") == "beta"
    assert up.pick_auto(ups, ONLINE, last_used="alpha") == "alpha"
    assert up.pick_auto(ups, ONLINE, last_used="local") == "local"


def test_auto_falls_back_when_your_last_server_went_offline():
    ups = up.load(TWO)
    state = {"alpha": {"state": "offline"}, "beta": {"state": "online"}}
    # Affinity must not pin you to a dead box.
    assert up.pick_auto(ups, state, last_used="alpha") == "local"
    assert up.pick_auto(ups, state, last_used="alpha", local_online=False) == "beta"


def test_auto_prefers_this_node_when_nothing_is_remembered():
    assert up.pick_auto(up.load(TWO), ONLINE, last_used=None) == "local"


def test_auto_uses_a_remote_when_this_node_is_down():
    assert up.pick_auto(up.load(TWO), ONLINE, last_used=None,
                        local_online=False) == "alpha"


def test_auto_ignores_servers_that_are_not_online():
    ups = up.load(TWO)
    unknown = {"alpha": {"state": "unknown"}, "beta": {"state": "offline"}}
    assert up.pick_auto(ups, unknown, last_used=None, local_online=False) is None
    assert up.pick_auto(ups, unknown, last_used="beta", local_online=False) is None


def test_auto_refuses_rather_than_guessing_when_nothing_is_up():
    assert up.pick_auto([], {}, last_used=None, local_online=False) is None
    assert up.pick_auto(up.load(TWO), {}, last_used="alpha", local_online=False) is None


def test_auto_ignores_a_parked_server_even_if_it_answers():
    parked = up.load(TWO.replace("id: beta", "id: beta\n    enabled: false"))
    assert up.pick_auto(parked, ONLINE, last_used="beta") == "local"
