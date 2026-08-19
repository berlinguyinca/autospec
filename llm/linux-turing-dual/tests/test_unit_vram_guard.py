"""The multi-GPU free-VRAM guard.

The single-card version of this guard read nvidia-smi's FIRST ROW and compared
it against a floor sized for one big card. On two 11 GiB cards, card 0 reports
~11000 against a 20000 floor, so it refused to start a node that fits
comfortably -- and the error blamed VRAM, which sent the reader looking in the
wrong place entirely.
"""
import os
import pathlib
import stat
import subprocess

GUARD = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "vram-guard.sh"


def fake_smi(tmp_path, lines):
    """A stub nvidia-smi printing one free-MiB figure per line."""
    p = tmp_path / "nvidia-smi"
    p.write_text("#!/bin/sh\ncat <<'EOF'\n" + lines + "\nEOF\n")
    p.chmod(p.stat().st_mode | stat.S_IEXEC)
    return p


def run(tmp_path, lines, *args):
    env = dict(os.environ, NVIDIA_SMI=str(fake_smi(tmp_path, lines)))
    return subprocess.run(["bash", str(GUARD), *args],
                          capture_output=True, text=True, env=env)


# --- the regression this guard exists for ----------------------------------

def test_two_cards_sum_to_enough(tmp_path):
    """2 x 11000 clears a 20000 floor that card 0 alone fails."""
    r = run(tmp_path, "11000\n11000", "--min-total", "20000")
    assert r.returncode == 0, r.stderr
    assert "22000" in r.stdout


def test_three_cards_also_sum(tmp_path):
    r = run(tmp_path, "11000\n11000\n11000", "--min-total", "30000")
    assert r.returncode == 0, r.stderr


# --- refusals --------------------------------------------------------------

def test_single_card_below_floor_is_refused(tmp_path):
    r = run(tmp_path, "9000", "--min-total", "20000")
    assert r.returncode == 75
    assert "9000" in r.stderr


def test_per_card_floor_enforced_even_when_total_passes(tmp_path):
    """A model pinned to one card needs the room on ONE card.

    Four 5000 MiB cards clear a 20000 total but cannot host a model needing
    8000 on a single device. The aggregate is the wrong bound here, which is
    exactly the mistake that makes a 22 GiB two-card host look like it can
    hold anything a 22 GiB single card can.
    """
    r = run(tmp_path, "5000\n5000\n5000\n5000",
            "--min-total", "20000", "--min-per-card", "8000")
    assert r.returncode == 75
    assert "per-card" in r.stderr
    assert "5000" in r.stderr


def test_per_card_floor_satisfied_by_the_best_card(tmp_path):
    r = run(tmp_path, "9000\n1000", "--min-total", "9000", "--min-per-card", "8000")
    assert r.returncode == 0, r.stderr


# --- distinct failure modes ------------------------------------------------

def test_missing_nvidia_smi_is_not_a_vram_failure(tmp_path):
    """69 not 75: 'no driver' and 'not enough VRAM' are different problems."""
    env = dict(os.environ, NVIDIA_SMI=str(tmp_path / "does-not-exist"))
    r = subprocess.run(["bash", str(GUARD), "--min-total", "1"],
                       capture_output=True, text=True, env=env)
    assert r.returncode == 69


def test_no_devices_reported_is_ex_unavailable(tmp_path):
    r = run(tmp_path, "", "--min-total", "1")
    assert r.returncode == 69


def test_unknown_argument_is_ex_usage(tmp_path):
    r = run(tmp_path, "11000", "--nonsense")
    assert r.returncode == 64
