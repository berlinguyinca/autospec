#!/usr/bin/env python3
"""foam_cfd_reduce.py — reduce real OpenFOAM field output to stage_cfd.py's contract.

`stage_cfd.py` runs ``simpleFoam`` in a case dir, then parses a ``cfd_results`` file
(see ``stage_cfd.py:_parse_cfd_results``) of the form::

    PRESSURE_DROP <value_pa>
    MIN_VELOCITY  <value_m_s>
    STAGNATION    <0|1>

Real OpenFOAM emits no such file — it writes ``p`` and ``U`` volume fields under
numeric time directories (``0/``, ``100/``, …). This is the **real integration
work** called out in the design §5.2: read the converged (latest) time-dir fields
and reduce them to the three scalars the unchanged stage parses.

Reductions (kinematic units; ``simpleFoam`` ``p`` is p/rho in m^2/s^2 — multiply
by density to get Pa):
  - PRESSURE_DROP = (max(p) - min(p)) * rho      [Pa]
  - MIN_VELOCITY  = min over cells of |U|        [m/s]
  - STAGNATION    = 1 if MIN_VELOCITY < stagnation_eps else 0

Installed in the container so the case/post pipeline calls it after ``simpleFoam``;
the field parsers + reductions are importable pure functions so they are unit
testable against sample OpenFOAM field files without OpenFOAM installed.
"""

from __future__ import annotations

import math
import os
import re
import sys

# simpleFoam (incompressible) uses kinematic pressure p/rho. Default density of air
# at sea level; override via $FOAM_RHO_KG_M3 for water/other fluids.
_DEFAULT_RHO = 1.225
# |U| below this (m/s) counts as a stagnation zone.
_DEFAULT_STAGNATION_EPS = 0.05


def _strip_comments(text: str) -> str:
    """Drop C-style block and // line comments (OpenFOAM dict comment styles)."""
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.DOTALL)
    text = re.sub(r"//[^\n]*", " ", text)
    return text


def _extract_internal_block(text: str) -> str:
    """Return the substring after ``internalField`` (where the field data lives)."""
    idx = text.find("internalField")
    return text[idx:] if idx != -1 else text


def parse_scalar_field(text: str) -> list[float]:
    """Parse the cell values of an OpenFOAM scalarField (e.g. the ``p`` field).

    Handles ``internalField uniform <v>;`` and the
    ``internalField nonuniform List<scalar> N ( v0 v1 ... );`` long form.
    """
    text = _strip_comments(text)
    block = _extract_internal_block(text)
    m = re.search(r"internalField\s+uniform\s+([-\d.eE+]+)", block)
    if m:
        return [float(m.group(1))]
    # nonuniform list: N ( v0 v1 ... )
    m = re.search(r"List<scalar>\s*\d*\s*\((.*?)\)", block, flags=re.DOTALL)
    if not m:
        return []
    return [float(t) for t in m.group(1).split()]


def parse_vector_field(text: str) -> list[tuple[float, float, float]]:
    """Parse the cell vectors of an OpenFOAM vectorField (e.g. the ``U`` field).

    Handles ``internalField uniform (x y z);`` and
    ``internalField nonuniform List<vector> N ( (x y z) ... );``.
    """
    text = _strip_comments(text)
    block = _extract_internal_block(text)
    m = re.search(
        r"internalField\s+uniform\s+\(\s*([-\d.eE+]+)\s+([-\d.eE+]+)\s+([-\d.eE+]+)\s*\)",
        block,
    )
    if m:
        return [(float(m.group(1)), float(m.group(2)), float(m.group(3)))]
    m = re.search(r"List<vector>\s*\d*\s*\((.*)\)\s*;", block, flags=re.DOTALL)
    if not m:
        return []
    vectors: list[tuple[float, float, float]] = []
    for vm in re.finditer(r"\(\s*([-\d.eE+]+)\s+([-\d.eE+]+)\s+([-\d.eE+]+)\s*\)", m.group(1)):
        vectors.append((float(vm.group(1)), float(vm.group(2)), float(vm.group(3))))
    return vectors


def reduce_pressure_drop(p_values: list[float], rho: float) -> float | None:
    """(max p - min p) * rho → Pa. None if no values."""
    if not p_values:
        return None
    return (max(p_values) - min(p_values)) * rho


def reduce_min_velocity(u_vectors: list[tuple[float, float, float]]) -> float | None:
    """min over cells of |U|. None if no vectors."""
    if not u_vectors:
        return None
    return min(math.sqrt(x * x + y * y + z * z) for (x, y, z) in u_vectors)


def _latest_time_dir(case_dir: str) -> str | None:
    """Highest-numbered numeric time directory in a case (the converged solution)."""
    best: tuple[float, str] | None = None
    for entry in os.listdir(case_dir):
        full = os.path.join(case_dir, entry)
        if not os.path.isdir(full):
            continue
        try:
            t = float(entry)
        except ValueError:
            continue
        if best is None or t > best[0]:
            best = (t, full)
    return best[1] if best else None


def reduce_case(case_dir: str,
                rho: float | None = None,
                stagnation_eps: float | None = None) -> dict:
    """Read the latest time-dir p/U fields and reduce to the cfd_results scalars.

    Returns a dict with keys pressure_drop_pa, min_velocity_m_s, stagnation (bool).
    Missing fields yield None for that metric (the stage then sees a partial/empty
    parse and emits its own verdict, unchanged).
    """
    rho = _DEFAULT_RHO if rho is None else rho
    stagnation_eps = _DEFAULT_STAGNATION_EPS if stagnation_eps is None else stagnation_eps
    out: dict = {"pressure_drop_pa": None, "min_velocity_m_s": None, "stagnation": False}

    time_dir = _latest_time_dir(case_dir)
    if time_dir is None:
        return out

    p_path = os.path.join(time_dir, "p")
    if os.path.exists(p_path):
        with open(p_path) as fh:
            out["pressure_drop_pa"] = reduce_pressure_drop(parse_scalar_field(fh.read()), rho)

    u_path = os.path.join(time_dir, "U")
    if os.path.exists(u_path):
        with open(u_path) as fh:
            min_vel = reduce_min_velocity(parse_vector_field(fh.read()))
        out["min_velocity_m_s"] = min_vel
        if min_vel is not None:
            out["stagnation"] = min_vel < stagnation_eps
    return out


def format_cfd_results(reduced: dict) -> str:
    """Render the reduced scalars as the exact cfd_results line format the stage parses."""
    lines: list[str] = []
    dp = reduced.get("pressure_drop_pa")
    if dp is not None:
        lines.append(f"PRESSURE_DROP {dp:.6f}")
    vel = reduced.get("min_velocity_m_s")
    if vel is not None:
        lines.append(f"MIN_VELOCITY {vel:.6f}")
    lines.append(f"STAGNATION {1 if reduced.get('stagnation') else 0}")
    return "\n".join(lines) + "\n"


def write_cfd_results(case_dir: str,
                      rho: float | None = None,
                      stagnation_eps: float | None = None) -> str:
    """Reduce the case and write ``<case_dir>/cfd_results``; return its path."""
    reduced = reduce_case(case_dir, rho=rho, stagnation_eps=stagnation_eps)
    out_path = os.path.join(case_dir, "cfd_results")
    with open(out_path, "w") as fh:
        fh.write(format_cfd_results(reduced))
    return out_path


def _env_float(name: str, default: float | None) -> float | None:
    val = os.environ.get(name)
    if val is None:
        return default
    try:
        return float(val)
    except ValueError:
        return default


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:]) if argv is None else argv
    case_dir = argv[0] if argv else os.getcwd()
    if not os.path.isdir(case_dir):
        sys.stderr.write(f"foam_cfd_reduce: not a case dir: {case_dir}\n")
        return 1
    rho = _env_float("FOAM_RHO_KG_M3", _DEFAULT_RHO)
    eps = _env_float("FOAM_STAGNATION_EPS", _DEFAULT_STAGNATION_EPS)
    out_path = write_cfd_results(case_dir, rho=rho, stagnation_eps=eps)
    sys.stderr.write(f"foam_cfd_reduce: wrote {out_path}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
