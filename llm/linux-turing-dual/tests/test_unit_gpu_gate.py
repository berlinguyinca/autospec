"""The dashboard's GPU verdict -- the thing the gateway is allowed to act on.

Computed here, not in the gateway, because the gateway runs with
PrivateDevices=true and cannot see /dev/nvidia*. Fed by the FAST (1 s) tick, not
the 30 s journal timer: a card falling off the bus must be visible to the gate
within a second.
"""
import os

from nodescripts import load_script

d = load_script("dashboard")

TWO = [{"index": 0}, {"index": 1}]
BUS_STDERR = ("Unable to determine the device handle for GPU1: "
              "0000:44:00.0: Unknown Error\n")


def gate(snap, expect="2"):
    old = os.environ.get("QT_EXPECT_DEVICES")
    os.environ["QT_EXPECT_DEVICES"] = expect
    try:
        return d._gpu_gate(snap)
    finally:
        if old is None:
            os.environ.pop("QT_EXPECT_DEVICES", None)
        else:
            os.environ["QT_EXPECT_DEVICES"] = old


def test_healthy_pair_is_ok():
    assert gate({"gpus": TWO, "_smi_stderr": "", "_smi_failed": False})["ok"]


def test_card_off_the_bus_is_not_ok():
    g = gate({"gpus": [{"index": 0}], "_smi_stderr": BUS_STDERR,
              "_smi_failed": False})
    assert not g["ok"] and g["reason"]


def test_missing_card_without_stderr_is_not_ok():
    # nvidia-smi answers cleanly for one card and says nothing about the other;
    # counting is the only signal, which is why the count check exists alongside
    # the stderr detectors rather than instead of them.
    g = gate({"gpus": [{"index": 0}], "_smi_stderr": "", "_smi_failed": False})
    assert not g["ok"] and "1 of 2" in g["reason"]


def test_unrunnable_nvidia_smi_is_still_ok():
    # WARNING in health.py, and it must stay a warning here: losing the
    # monitoring binary must not take inference offline.
    assert gate({"gpus": [], "_smi_stderr": "", "_smi_failed": True})["ok"]


def test_expect_unset_disables_the_count_check():
    g = gate({"gpus": [{"index": 0}], "_smi_stderr": "", "_smi_failed": False},
             expect="0")
    assert g["ok"]
