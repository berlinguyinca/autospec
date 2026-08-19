#!/usr/bin/env python3
"""Check the router presets against the invariants the tier scheme relies on.

    check_presets.py <router-presets.ini>

Each `-NNk` alias is a promise to a client about how much context it may use.
Those promises are only kept if the server can actually back them, and llama.cpp
will not tell you otherwise until four sessions die at once in production.
"""
from __future__ import annotations

import configparser
import re
import sys

TIER_RE = re.compile(r"-(\d+)k$")


def main() -> int:
    ini = configparser.ConfigParser(strict=False)
    ini.optionxform = str
    # llama.cpp allows top-level keys before the first section ("version = 1").
    ini.read_string("[__top__]\n" + open(sys.argv[1]).read())
    shared = dict(ini["*"]) if ini.has_section("*") else {}

    problems: list[str] = []
    for name in ini.sections():
        if name in ("*", "__top__"):
            continue
        sec = {**shared, **dict(ini[name])}
        pool = int(sec.get("c") or sec.get("ctx-size") or 0)
        parallel = int(sec.get("parallel") or 1)
        aliases = [a.strip() for a in sec.get("alias", "").split(",") if a.strip()]

        if not pool:
            problems.append(f"{name}: no context set")
            continue

        # Differently-sized tiers are meaningless against a hard partition:
        # without kv-unified every slot is capped at c/parallel no matter what
        # the client declares.
        if aliases and sec.get("kv-unified", "").lower() != "true":
            problems.append(f"{name}: offers tiers without kv-unified = true")

        for alias in aliases:
            m = TIER_RE.search(alias)
            if not m:
                continue
            limit = int(m.group(1)) * 1024
            if limit > pool:
                problems.append(
                    f"{alias}: declares {limit:,} against a {pool:,} pool")
            elif pool // limit > parallel:
                problems.append(
                    f"{alias}: pool funds {pool // limit} sessions but only "
                    f"{parallel} slots exist")

    for p in problems:
        print(f"        {p}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
