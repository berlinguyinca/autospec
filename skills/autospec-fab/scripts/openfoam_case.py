#!/usr/bin/env python3
"""
openfoam_case.py — OpenFOAM case-directory builder for the autospec-fab CFD stage.

Writes a minimal OpenFOAM case directory with the standard layout:
  constant/  — geometry + transport properties
  system/    — blockMeshDict, controlDict, fvSchemes, fvSolution
  0/         — initial/boundary conditions

This is a simplified single-pass case for the solver integration; real
production cases would use snappyHexMesh on the actual STL.
"""

from __future__ import annotations

import os
import shutil


_DEFAULT_INLET_VELOCITY_M_S = 5.0


def _controlDict(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class dictionary; location \"system\"; object controlDict; }\n"
        "application     simpleFoam;\n"
        "startFrom       startTime;\n"
        "startTime       0;\n"
        "stopAt          endTime;\n"
        "endTime         100;\n"
        "deltaT          1;\n"
        "writeControl    timeStep;\n"
        "writeInterval   100;\n"
    )


def _blockMeshDict(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class dictionary; location \"system\"; object blockMeshDict; }\n"
        "scale   1;\n"
        "vertices ( (0 0 0) (1 0 0) (1 1 0) (0 1 0) (0 0 1) (1 0 1) (1 1 1) (0 1 1) );\n"
        "blocks ( hex (0 1 2 3 4 5 6 7) (10 10 10) simpleGrading (1 1 1) );\n"
        "edges ();\nboundary ();\nmergePatchPairs ();\n"
    )


def _fvSchemes(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class dictionary; location \"system\"; object fvSchemes; }\n"
        "ddtSchemes { default steadyState; }\n"
        "divSchemes { default none; div(phi,U) bounded Gauss linearUpwind grad(U); }\n"
        "gradSchemes { default Gauss linear; }\n"
        "laplacianSchemes { default Gauss linear corrected; }\n"
    )


def _fvSolution(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class dictionary; location \"system\"; object fvSolution; }\n"
        "solvers { p { solver GAMG; tolerance 1e-6; relTol 0.1; } }\n"
        "SIMPLE { nNonOrthogonalCorrectors 0; }\n"
    )


def _transportProperties(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class dictionary; location \"constant\"; object transportProperties; }\n"
        "nu              [ 0 2 -1 0 0 0 0 ] 1.5e-05;\n"
    )


def _U(inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class volVectorField; location \"0\"; object U; }\n"
        "dimensions [0 1 -1 0 0 0 0];\n"
        f"internalField   uniform ({inlet_velocity} 0 0);\n"
        "boundaryField { inlet { type fixedValue; value uniform (" + f"{inlet_velocity} 0 0); }}" + "\n"
    )


def _p(_inlet_velocity: float) -> str:
    return (
        "FoamFile { version 2.0; format ascii; class volScalarField; location \"0\"; object p; }\n"
        "dimensions [0 2 -2 0 0 0 0];\n"
        "internalField   uniform 0;\n"
        "boundaryField { outlet { type zeroGradient; } }\n"
    )


# Relative path within the case dir → template renderer (inlet_velocity → text).
_CASE_FILES = {
    os.path.join("system", "controlDict"): _controlDict,
    os.path.join("system", "blockMeshDict"): _blockMeshDict,
    os.path.join("system", "fvSchemes"): _fvSchemes,
    os.path.join("system", "fvSolution"): _fvSolution,
    os.path.join("constant", "transportProperties"): _transportProperties,
    os.path.join("0", "U"): _U,
    os.path.join("0", "p"): _p,
}


def build_openfoam_case(
    stl_path: str,
    model: dict,
    flow: dict,
    work_dir: str,
) -> str:
    """Write a minimal OpenFOAM case directory and return its path."""
    case_dir = os.path.join(work_dir, "cfd_case")
    os.makedirs(os.path.join(case_dir, "constant", "triSurface"), exist_ok=True)
    os.makedirs(os.path.join(case_dir, "system"), exist_ok=True)
    os.makedirs(os.path.join(case_dir, "0"), exist_ok=True)

    shutil.copy(stl_path, os.path.join(case_dir, "constant", "triSurface", "geometry.stl"))

    inlet_velocity = flow.get("inlet_velocity_m_s", _DEFAULT_INLET_VELOCITY_M_S)
    for rel_path, render in _CASE_FILES.items():
        with open(os.path.join(case_dir, rel_path), "w") as fh:
            fh.write(render(inlet_velocity))

    return case_dir
