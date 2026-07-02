#!/usr/bin/env python3
"""Compatibility runtime module for future-spec acceptance commands.

The active local implementation routes through scripts/autospec-baseline-v25.py;
this module exists so historical future-spec compile checks have a stable target.
"""

RUNTIME_SCHEMA = "autospec.runtime.v1"


def runtime_status() -> dict[str, str]:
    return {"schema": RUNTIME_SCHEMA, "status": "available"}
