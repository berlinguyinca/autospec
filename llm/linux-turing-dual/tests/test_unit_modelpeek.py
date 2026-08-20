"""Finding the requested model in a bounded prefix of a request body.

Two properties matter more than the happy path, and both are asserted here:

  * A prefix that ran out must report itself INCONCLUSIVE. The caller turns
    "conclusive, no model" into "route this locally, nothing to match" and
    "inconclusive" into the same thing -- but the distinction is what stops a
    future caller from treating a truncated read as a body with no model.
  * A `"model"` that is not a top-level KEY must never be returned. That is the
    whole reason this is a position-aware scanner rather than a regex: a chat
    request carries user text, and user text can say anything.
"""
import json

from conftest import load_script

mp = load_script("modelpeek")


def test_the_common_case_model_first():
    body = b'{"model":"qwen3.8-27b","messages":[{"role":"user","content":"hi"}]}'
    assert mp.peek_model(body) == ("qwen3.8-27b", True)


def test_whitespace_and_ordering_do_not_matter():
    body = b'{\n  "stream" : true ,\n  "model"  :  "qwen3.5-9b"\n}'
    assert mp.peek_model(body) == ("qwen3.5-9b", True)


def test_model_after_a_nested_object_is_still_found():
    body = (b'{"messages":[{"role":"user","content":"hi"}],'
            b'"tools":[{"function":{"name":"x"}}],"model":"qwen3.8-27b-vision"}')
    assert mp.peek_model(body) == ("qwen3.8-27b-vision", True)


def test_a_model_key_inside_a_message_is_not_the_routing_key():
    # The user is TALKING about a model. Routing on this would send the request
    # to a server chosen by whatever the user happened to type.
    body = json.dumps({
        "messages": [{"role": "user", "content": "what is this?",
                      "model": "qwen3.8-27b"}],
        "model": "qwen3.5-9b"}).encode()
    assert mp.peek_model(body) == ("qwen3.5-9b", True)


def test_the_string_model_used_as_a_value_is_not_a_key():
    assert mp.peek_model(b'{"user":"model","model":"real-one"}') == ("real-one", True)


def test_user_text_that_looks_like_the_field_is_ignored():
    body = json.dumps({
        "model": "qwen3.5-9b",
        "messages": [{"role": "user", "content": '{"model": "injected"}'}],
    }).encode()
    assert mp.peek_model(body) == ("qwen3.5-9b", True)


def test_no_model_field_at_all_is_conclusive():
    assert mp.peek_model(b'{"messages":[],"max_tokens":8}') == (None, True)


def test_a_body_that_is_not_a_json_object_is_conclusive():
    # multipart audio, form data, a bare array: there is nothing to wait for.
    assert mp.peek_model(b'------WebKitFormBoundary\r\n') == (None, True)
    assert mp.peek_model(b'[1,2,3]') == (None, True)


def test_a_truncated_prefix_is_inconclusive_not_empty():
    body = b'{"messages":[{"role":"user","content":"' + b'x' * 200
    assert mp.peek_model(body) == (None, False)


def test_a_prefix_cut_inside_the_model_value_is_inconclusive():
    assert mp.peek_model(b'{"model":"qwen3.8-2') == (None, False)


def test_a_prefix_cut_inside_the_model_key_is_inconclusive():
    assert mp.peek_model(b'{"mod') == (None, False)


def test_a_prefix_cut_before_the_colon_is_inconclusive():
    assert mp.peek_model(b'{"model"') == (None, False)


def test_a_truncated_number_does_not_advance_past_the_buffer():
    # "12" here may really be 1234: reading on would be reading a value that
    # does not exist yet.
    assert mp.peek_model(b'{"max_tokens":12') == (None, False)


def test_an_empty_prefix_is_inconclusive():
    assert mp.peek_model(b"") == (None, False)
    assert mp.peek_model(b"   ") == (None, False)


def test_escapes_in_the_value_decode():
    assert mp.peek_model(b'{"model":"a\\u002db"}') == ("a-b", True)


def test_an_escaped_quote_does_not_end_the_string_early():
    assert mp.peek_model(b'{"a":"x\\"y","model":"m1"}') == ("m1", True)


def test_an_empty_model_is_reported_as_absent():
    # An empty id routes nowhere; treating it as a name would filter every
    # server out and refuse the request.
    assert mp.peek_model(b'{"model":""}') == (None, True)


def test_a_non_string_model_is_not_returned():
    assert mp.peek_model(b'{"model":123,"stream":true}') == (None, True)


def test_malformed_json_stops_rather_than_guessing():
    assert mp.peek_model(b'{"model" "qwen3.8-27b"}') == (None, True)


def test_the_peek_budget_is_small_enough_to_stay_cheap():
    # Two concurrent slots must not cost more than a few tens of KB: the
    # measured pass-through cost of this gateway is 1.7 MB of RSS growth for a
    # 100k-token request, and this buffer must stay noise against that.
    assert mp.PEEK_BYTES <= 16384


def test_a_realistic_large_body_is_found_within_the_budget():
    body = json.dumps({
        "model": "qwen3.8-27b-100k",
        "messages": [{"role": "user", "content": "x" * 400000}],
    }).encode()
    assert mp.peek_model(body[:mp.PEEK_BYTES]) == ("qwen3.8-27b-100k", True)


def test_a_body_with_the_model_beyond_the_budget_is_inconclusive():
    body = json.dumps({
        "messages": [{"role": "user", "content": "x" * 400000}],
        "model": "qwen3.8-27b",
    }).encode()
    assert mp.peek_model(body[:mp.PEEK_BYTES]) == (None, False)
