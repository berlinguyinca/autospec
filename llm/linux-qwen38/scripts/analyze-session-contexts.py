#!/usr/bin/env python3
"""Size a context window from the work that was actually done, not from a guess.

    analyze-session-contexts.py [--roots DIR[,DIR]] [--min-turns 5] [--json]

Reads agent-session transcripts and reports how much context each turn really
carried. "How big a window do I need" is an empirical question and the answer is
sitting in the logs: a window that covers the 90th percentile turn is a very
different number from one that covers the largest turn ever recorded, and
choosing between them is a cost decision, not a technical one.

Context for one API call is everything the model was shown:

    input_tokens + cache_creation_input_tokens + cache_read_input_tokens

The cached parts are the whole conversation so far. Counting only input_tokens
reports a few hundred tokens for a turn that actually carried 120,000, because
prompt caching moved the bulk into cache_read.

Turns with an `iterations` array made several calls; each carried its own
context, so the turn is scored by its LARGEST call rather than their sum.
"""
from __future__ import annotations

import argparse
import json
import sqlite3
import statistics
import sys
from pathlib import Path

# The tiers a local node can realistically serve. Sessions are scored against
# these because the useful output is "what fraction of my work fits", not a mean.
TIERS = [16384, 32768, 40960, 65536, 81920, 131072, 163840, 262144]


def call_context(usage: dict) -> int:
    return (usage.get("input_tokens", 0)
            + usage.get("cache_creation_input_tokens", 0)
            + usage.get("cache_read_input_tokens", 0))


def turn_context(usage: dict) -> int:
    """The largest single call this turn made."""
    iters = usage.get("iterations")
    if isinstance(iters, list) and iters:
        return max(call_context(i) for i in iters if isinstance(i, dict))
    return call_context(usage)


def read_opencode_db(path: Path) -> list[list[int]]:
    """Context size per assistant turn, per session, from an OpenCode database.

    Worth reading separately rather than assuming it matches another client:
    the floor differs by a factor of nearly three between the two on this host,
    and the floor is what decides whether a small tier can start at all.
    """
    sessions: dict[str, list[int]] = {}
    try:
        con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    except sqlite3.Error:
        return []
    try:
        rows = con.execute("SELECT session_id, data FROM message "
                           "ORDER BY time_created")
        for sid, data in rows:
            try:
                rec = json.loads(data)
            except (json.JSONDecodeError, TypeError):
                continue
            if rec.get("role") != "assistant":
                continue
            tok = rec.get("tokens") or {}
            cache = tok.get("cache") or {}
            ctx = ((tok.get("input") or 0) + (cache.get("read") or 0)
                   + (cache.get("write") or 0))
            if ctx > 0:
                sessions.setdefault(sid, []).append(ctx)
    except sqlite3.Error:
        return []
    finally:
        con.close()
    return list(sessions.values())


def read_session(path: Path) -> list[int]:
    """Context size of every assistant turn in one transcript, in order."""
    out = []
    try:
        with path.open(errors="replace") as fh:
            for line in fh:
                if '"usage"' not in line:
                    continue
                try:
                    rec = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if rec.get("type") != "assistant":
                    continue
                usage = (rec.get("message") or {}).get("usage")
                if isinstance(usage, dict):
                    ctx = turn_context(usage)
                    if ctx > 0:
                        out.append(ctx)
    except OSError:
        return []
    return out


def pct(values: list[int], p: float) -> int:
    if not values:
        return 0
    s = sorted(values)
    k = min(len(s) - 1, int(round((p / 100.0) * (len(s) - 1))))
    return s[k]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--roots", default=str(Path.home() / ".claude" / "projects"),
                    help="comma-separated directories of .jsonl transcripts")
    ap.add_argument("--min-turns", type=int, default=5,
                    help="ignore sessions shorter than this; a 2-turn session "
                         "says nothing about how much context work needs")
    ap.add_argument("--project", help="only sessions whose path contains this")
    ap.add_argument("--opencode-db", type=Path,
                    default=Path.home() / ".local/share/opencode/opencode.db",
                    help="read OpenCode sessions from this database as well")
    ap.add_argument("--source", choices=("all", "transcripts", "opencode"),
                    default="all")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    per_session: list[list[int]] = []
    if args.source in ("all", "transcripts"):
        files: list[Path] = []
        for root in args.roots.split(","):
            files.extend(Path(root.strip()).rglob("*.jsonl"))
        if args.project:
            files = [f for f in files if args.project in str(f)]
        per_session.extend(read_session(f) for f in files)
    if args.source in ("all", "opencode") and args.opencode_db.is_file():
        per_session.extend(read_opencode_db(args.opencode_db))

    all_turns: list[int] = []
    peaks: list[int] = []
    floors: list[int] = []
    growth: list[float] = []
    sessions = 0
    for turns in per_session:
        if len(turns) < args.min_turns:
            continue
        sessions += 1
        all_turns.extend(turns)
        peaks.append(max(turns))
        # The floor is what a session costs before any work happens: system
        # prompt, project instructions, memory, skill descriptions. No tier can
        # be smaller than this and still start.
        floors.append(turns[0])
        # Growth per turn drives the housekeeping schedule. Measured on the
        # rising part only: a transcript that compacts shows a sawtooth, and
        # averaging across the drop understates how fast the window fills.
        rises = [b - a for a, b in zip(turns, turns[1:]) if b > a]
        if rises:
            growth.append(statistics.median(rises))

    if not all_turns:
        print("no sessions matched", file=sys.stderr)
        return 1

    report = {
        "sessions": sessions,
        "turns": len(all_turns),
        "turn_percentiles": {f"p{p}": pct(all_turns, p)
                             for p in (50, 75, 90, 95, 99, 100)},
        "session_peak_percentiles": {f"p{p}": pct(peaks, p)
                                     for p in (50, 75, 90, 95, 99, 100)},
        "median_turn": int(statistics.median(all_turns)),
        "floor_percentiles": {f"p{p}": pct(floors, p)
                              for p in (50, 75, 90, 95, 100)},
        "growth_per_turn_median": int(statistics.median(growth)) if growth else 0,
        "tiers": [
            {
                "tier": t,
                # Two different questions. Turn coverage says how often a
                # request would fit as-is; session coverage says how often a
                # session would finish without ever needing to compact.
                "turns_fitting_pct": 100.0 * sum(1 for v in all_turns if v <= t)
                                     / len(all_turns),
                "sessions_never_compacting_pct": 100.0 * sum(1 for v in peaks
                                                             if v <= t)
                                                 / len(peaks),
            }
            for t in TIERS
        ],
    }

    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(f"\n{sessions:,} sessions, {len(all_turns):,} assistant turns "
          f"(>= {args.min_turns} turns each)\n")
    print("context carried per turn")
    for p in (50, 75, 90, 95, 99, 100):
        print(f"  p{p:<4} {report['turn_percentiles'][f'p{p}']:>9,}")
    print("\npeak context reached per session")
    for p in (50, 75, 90, 95, 99, 100):
        print(f"  p{p:<4} {report['session_peak_percentiles'][f'p{p}']:>9,}")
    print("\nsession floor (context before any work is done)")
    for p in (50, 75, 90, 95, 100):
        print(f"  p{p:<4} {report['floor_percentiles'][f'p{p}']:>9,}")

    g = report["growth_per_turn_median"]
    floor = report["floor_percentiles"]["p50"]
    print(f"\nmedian growth while rising: {g:,} tokens/turn\n")
    print(f"{'tier':>9}  {'turns that fit':>15}  {'sessions never':>15}  "
          f"{'turns before':>13}")
    print(f"{'':>9}  {'':>15}  {'compacting':>15}  {'compaction':>13}")
    print("-" * 60)
    for row in report["tiers"]:
        # Usable window is smaller than the tier: a client compacts with
        # headroom to spare, and the floor is never reclaimable.
        usable = row["tier"] * 0.9 - floor
        turns = int(usable / g) if g > 0 and usable > 0 else 0
        print(f"{row['tier']:>9,}  {row['turns_fitting_pct']:>14.1f}%  "
              f"{row['sessions_never_compacting_pct']:>14.1f}%  "
              f"{(turns if turns > 0 else 0):>13}")
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
