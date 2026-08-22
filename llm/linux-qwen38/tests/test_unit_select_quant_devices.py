"""Multi-GPU device accounting in select-quant.py.

The tool read nvidia-smi's FIRST ROW, so on a two-card host it saw one card's
VRAM and concluded nothing fits. These tests pin both halves of the fix:
the aggregate is summed across devices, and the reserve -- which pays for
compute buffers and the CUDA context -- scales with the number of cards,
because those are paid on each card rather than shared.

Single-device behaviour must stay byte-identical: the 24 GiB node's numbers
were measured, and this fix must not quietly move them.
"""
import importlib.util
import pathlib

SRC = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "select-quant.py"


def load():
    spec = importlib.util.spec_from_file_location("select_quant", SRC)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# --- the bug itself ---------------------------------------------------------

def test_parses_every_device_not_just_the_first():
    assert load().parse_device_totals("11264\n11264\n") == [11264, 11264]


def test_single_device_still_parses():
    assert load().parse_device_totals("24564\n") == [24564]


def test_blank_and_whitespace_lines_ignored():
    assert load().parse_device_totals("11264\n\n  \n11264\n") == [11264, 11264]


def test_three_devices_are_all_counted():
    assert load().parse_device_totals("11264\n11264\n11264\n") == [11264] * 3


def test_aggregate_sums_rather_than_taking_the_head():
    m = load()
    assert m.aggregate_budget([11264, 11264]) == 22528


# --- per-card overhead, applied where the reserve already lives -------------

def test_reserve_scales_with_device_count():
    m = load()
    # compute buffers + CUDA context are paid on EACH card
    assert m.effective_reserve(1200, 2) == 2400
    assert m.effective_reserve(1200, 3) == 3600


def test_reserve_unchanged_on_a_single_card():
    """The 24 GiB node's measured numbers must not move."""
    assert load().effective_reserve(1200, 1) == 1200


def test_reserve_never_scales_below_itself():
    m = load()
    assert m.effective_reserve(1200, 0) == 1200


# --- a model pinned to one card is bounded by the smallest card -------------

def test_per_card_ceiling_uses_the_smallest_card():
    m = load()
    assert m.per_card_ceiling([11264, 11264], 1200) == 11264 - 1200


def test_per_card_ceiling_with_mismatched_cards():
    m = load()
    assert m.per_card_ceiling([24564, 11264], 1200) == 11264 - 1200


# --- degradation ------------------------------------------------------------

def test_no_devices_is_empty_not_an_exception():
    assert load().parse_device_totals("") == []


def test_aggregate_of_nothing_is_zero():
    assert load().aggregate_budget([]) == 0
