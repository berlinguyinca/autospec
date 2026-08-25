"""Which model, at what context, is on which card.

The panel drew utilisation per card and the model list separately, and left the
join to the reader -- so "what is on GPU 1?" had no answer on the page.

The case worth the most care is the one that is NOT idle: memory held with
nothing loaded. A defunct llama-server whose allocations the driver never
released sat on 8.9 GiB of this node with the runtime stopped, and it rendered
identically to an idle card. Those must never be the same string.
"""
from nodescripts import load_script

d = load_script("dashboard")


def snap(cards, catalog=None):
    return {"gpus": cards, "catalog": catalog or []}


def card(i, used):
    return {"index": i, "mem_used_mib": used, "mem_total_mib": 11264}


LOADED = [{"id": "qwen3.8-27b", "context": 102400, "slots": 2,
           "kind": "text", "resident": True}]
NOT_LOADED = [{"id": "qwen3.8-27b", "context": 102400, "slots": 2,
               "kind": "text", "resident": False}]


def test_a_split_model_is_attributed_to_both_cards():
    p = d._placement(snap([card(0, 7000), card(1, 8000)], LOADED))
    assert p["model"] == "qwen3.8-27b"
    assert p["cards"] == [0, 1]
    assert p["context"] == 102400 and p["slots"] == 2


def test_a_single_card_model_is_attributed_to_one():
    p = d._placement(snap([card(0, 7100), card(1, 12)], LOADED))
    assert p["cards"] == [0]


def test_driver_overhead_is_not_a_model():
    """An idle card still holds a CUDA context -- tens of MiB. Counting that
    would report a model on every card the driver has ever touched."""
    p = d._placement(snap([card(0, 7100), card(1, 300)], LOADED))
    assert p["cards"] == [0]


def test_memory_held_with_nothing_loaded_is_reported():
    # THE LEAK. Runtime stopped, allocations never released.
    p = d._placement(snap([card(0, 8894), card(1, 4)], NOT_LOADED))
    assert p["model"] is None
    assert p["unattributed_mib"] == 8894
    assert p["cards"] == [0]


def test_a_genuinely_idle_node_reports_nothing_held():
    p = d._placement(snap([card(0, 4), card(1, 4)], NOT_LOADED))
    assert p["model"] is None
    assert p["unattributed_mib"] == 0
    assert p["cards"] == []


def test_placement_is_labelled_derived():
    """llama.cpp does not report per-device placement. A derived number that
    looks measured is worse than no number."""
    p = d._placement(snap([card(0, 7000)], LOADED))
    assert p["derived"] is True


def test_no_cards_at_all_does_not_raise():
    p = d._placement(snap([], []))
    assert p["cards"] == [] and p["model"] is None
