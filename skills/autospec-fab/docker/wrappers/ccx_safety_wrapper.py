#!/usr/bin/env python3
"""ccx_safety_wrapper.py — bridge real CalculiX output to stage_fea.py's parse contract.

`stage_fea.py` runs ``ccx <job>`` and then parses a single ``SAFETY_FACTOR <value>``
line out of ``<job>.dat`` (see ``stage_fea.py:_parse_safety_factor``). Real CalculiX
does **not** emit a safety factor — with the deck's ``*EL PRINT ... S`` request it
writes raw stress tensors under a ``stresses`` block in ``<job>.dat``. This wrapper
is the one place the real solver meets the stage's parse contract:

  1. run the real ``ccx`` binary (resolved via $CCX_REAL_BIN or the next ``ccx`` on
     PATH that is not this wrapper),
  2. read MAX von-Mises stress out of the ``.dat`` stress block,
  3. derive ``SAFETY_FACTOR = material_yield / max_mises`` and append a
     ``SAFETY_FACTOR <value>`` line to ``<job>.dat`` so the unchanged stage parses it.

Installed in the container as ``/usr/local/bin/ccx`` (ahead of the real ccx on PATH),
so ``shutil.which("ccx")`` resolves here and the stage is untouched.

Material yield strengths (MPa) keyed by the same material names stage_fea.py uses;
overridable via $CCX_MATERIAL_YIELD_MPA for one-off parts/materials.

The von-Mises derivation is exposed as importable pure functions (``von_mises``,
``parse_max_mises``, ``derive_safety_factor``, ``resolve_yield_mpa``) so it is unit
testable against a sample real-ccx ``.dat`` without building the image or running ccx.
"""

from __future__ import annotations

import math
import os
import re
import shutil
import subprocess
import sys

# Yield strength (MPa) per FDM material, keyed to stage_fea.py's material names.
# Conservative tensile-yield figures for typical FDM filament.
_MATERIAL_YIELD_MPA: dict[str, float] = {
    "PETG": 50.0,
    "PLA": 60.0,
    "ABS": 40.0,
    "ASA": 44.0,
    "PCTG": 47.0,
}
_DEFAULT_YIELD_MPA = 40.0


def von_mises(sxx: float, syy: float, szz: float,
              sxy: float, sxz: float, syz: float) -> float:
    """Von-Mises equivalent stress from the six Cauchy stress components."""
    return math.sqrt(
        0.5 * (
            (sxx - syy) ** 2 + (syy - szz) ** 2 + (szz - sxx) ** 2
        )
        + 3.0 * (sxy ** 2 + sxz ** 2 + syz ** 2)
    )


def parse_max_mises(dat_text: str) -> float | None:
    """Return the maximum von-Mises stress over every stress row in a ccx .dat.

    CalculiX (with ``*EL PRINT ... S``) writes a ``stresses`` block; each data row
    is ``<elem> <intpnt> sxx syy szz sxy sxz syz`` (6 stress components after two
    integer indices). Header / blank / non-numeric lines are skipped. Returns None
    when no parsable stress row is found.

    Some CalculiX builds already emit a von-Mises column; this derives mises from
    the six tensor components, which is build-independent.
    """
    max_m: float | None = None
    for raw in dat_text.splitlines():
        line = raw.strip()
        if not line:
            continue
        toks = line.split()
        # Need two integer indices + six float stress components.
        if len(toks) < 8:
            continue
        try:
            int(toks[0])
            int(toks[1])
        except ValueError:
            continue
        try:
            comps = [float(t) for t in toks[2:8]]
        except ValueError:
            continue
        m = von_mises(*comps)
        if max_m is None or m > max_m:
            max_m = m
    return max_m


def resolve_yield_mpa(material: str | None) -> float:
    """Material yield strength in MPa, with $CCX_MATERIAL_YIELD_MPA override."""
    override = os.environ.get("CCX_MATERIAL_YIELD_MPA")
    if override:
        try:
            return float(override)
        except ValueError:
            pass
    if material:
        return _MATERIAL_YIELD_MPA.get(material.upper(), _DEFAULT_YIELD_MPA)
    return _DEFAULT_YIELD_MPA


def derive_safety_factor(max_mises_pa: float, yield_mpa: float) -> float:
    """Safety factor = yield / peak von-Mises. ccx stress is in Pa; yield is MPa."""
    max_mpa = max_mises_pa
    if max_mpa <= 0.0:
        # No load / zero stress: effectively unbounded safety; clamp to a large
        # finite value so the stage's float parse and comparison stay well-defined.
        return 1.0e6
    yield_pa = yield_mpa * 1.0e6
    return yield_pa / max_mpa


def annotate_dat(dat_path: str, material: str | None) -> bool:
    """Append a SAFETY_FACTOR line derived from the .dat stress block.

    Returns True if a line was appended, False if no stress data was parsable
    (the stage then emits its own parse-failure verdict, unchanged).
    """
    if not os.path.exists(dat_path):
        return False
    with open(dat_path) as fh:
        dat_text = fh.read()
    max_mises = parse_max_mises(dat_text)
    if max_mises is None:
        return False
    yield_mpa = resolve_yield_mpa(material)
    sf = derive_safety_factor(max_mises, yield_mpa)
    with open(dat_path, "a") as fh:
        fh.write(f"SAFETY_FACTOR {sf:.6f}\n")
    return True


def _job_base_from_argv(argv: list[str]) -> str | None:
    """ccx is invoked as ``ccx <job>`` (last positional, no extension)."""
    positionals = [a for a in argv if not a.startswith("-")]
    if not positionals:
        return None
    return positionals[-1]


def _resolve_real_ccx() -> str | None:
    """Locate the real ccx binary, skipping this wrapper itself."""
    explicit = os.environ.get("CCX_REAL_BIN")
    if explicit and os.path.exists(explicit):
        return explicit
    self_path = os.path.realpath(sys.argv[0])
    for candidate in ("ccx", "ccx_2.21", "ccx_2.20", "ccx_2.19"):
        found = shutil.which(candidate)
        if found and os.path.realpath(found) != self_path:
            return found
    return None


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:]) if argv is None else argv
    real_ccx = _resolve_real_ccx()
    if real_ccx is None:
        sys.stderr.write("ccx_safety_wrapper: no real ccx binary found\n")
        return 127
    proc = subprocess.run([real_ccx, *argv])
    if proc.returncode != 0:
        return proc.returncode

    job_base = _job_base_from_argv(argv)
    if job_base is None:
        return 0
    dat_path = job_base + ".dat"
    material = os.environ.get("CCX_MATERIAL")
    annotate_dat(dat_path, material)
    return 0


if __name__ == "__main__":
    sys.exit(main())
