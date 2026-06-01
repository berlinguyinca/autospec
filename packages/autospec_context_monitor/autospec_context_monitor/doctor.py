"""autospec_context_monitor.doctor — diagnostic command.

Usage::

    python -m autospec_context_monitor.doctor [--json] [--self-test]

Exits 0 when all checks pass; non-zero when any check reports a problem.
"""

from __future__ import annotations

import argparse
import collections
import datetime
import importlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Callable, List, Tuple

_MONITORS_DIR = Path.home() / ".autospec" / "monitors"
_TRANSCRIPTS_DIR = Path.home() / ".claude" / "projects"
_STATS_FILE = Path.home() / ".autospec" / "monitors" / "stats.jsonl"

# ---------------------------------------------------------------------------
# Check helpers
# ---------------------------------------------------------------------------

def _tmux_version() -> str:
    result = subprocess.run(["tmux", "-V"], capture_output=True, text=True)
    if result.returncode != 0:
        return "MISSING"
    return result.stdout.strip() or "MISSING"


def _binary(name: str) -> str:
    return shutil.which(name) or "MISSING"


_ADAPTER_MODULES = [
    "autospec_context_monitor.adapters.claude",
    "autospec_context_monitor.adapters.codex",
    "autospec_context_monitor.adapters.opencode",
]


def _transcripts_writable() -> list[str]:
    """Return per-adapter transcript path write-access status.

    Iterates each adapter's ``PATHS_TO_CHECK`` list and checks whether the
    path (or its parent, when the path itself does not yet exist) is writable.
    Each entry is labelled with the short adapter name so operators can
    immediately identify which harness has a misconfigured transcript directory.

    Adapters without a ``PATHS_TO_CHECK`` attribute are skipped gracefully.
    """
    results: list[str] = []
    for mod_name in _ADAPTER_MODULES:
        try:
            mod = importlib.import_module(mod_name)
        except Exception as exc:  # noqa: BLE001
            results.append(f"{mod_name}: IMPORT_ERROR({exc})")
            continue
        paths: list[Path] = getattr(mod, "PATHS_TO_CHECK", [])
        # Short adapter label, e.g. "claude", "codex", "opencode".
        label = mod_name.split(".")[-1]
        for path in paths:
            check_target = path if path.exists() else path.parent
            writable = os.access(check_target, os.W_OK)
            status = "writable" if writable else "NOT_WRITABLE"
            results.append(f"{label}:{path}:{status}")
    return results


def _import_all_adapters() -> str:
    """Try to import all known adapter modules; return 'ok' or error string."""
    errors: list[str] = []
    for mod in (
        "autospec_context_monitor.adapters.base",
        "autospec_context_monitor.adapters.claude",
        "autospec_context_monitor.adapters.codex",
        "autospec_context_monitor.adapters.opencode",
    ):
        try:
            importlib.import_module(mod)
        except Exception as exc:  # noqa: BLE001
            errors.append(f"{mod}: {exc}")
    return "ok" if not errors else "IMPORT_ERROR: " + "; ".join(errors)


def _list_pids() -> list[str]:
    """Return list of '<session>:<pid>' strings for running monitors."""
    if not _MONITORS_DIR.exists():
        return []
    pids: list[str] = []
    for pidf in _MONITORS_DIR.glob("*.pid"):
        try:
            pid = int(pidf.read_text().strip())
        except (ValueError, OSError):
            continue
        # Check if the process is alive
        try:
            os.kill(pid, 0)
            alive = True
        except OSError:
            alive = False
        session = pidf.stem
        pids.append(f"{session}:{pid}({'alive' if alive else 'dead'})")
    return pids


def _recent_events(n: int = 5) -> list[str]:
    """Return up to *n* recent log lines (newest-first) across all monitor logs."""
    if not _MONITORS_DIR.exists():
        return []
    all_lines: list[tuple[float, str]] = []
    for logf in _MONITORS_DIR.glob("*.log"):
        try:
            lines = logf.read_text().splitlines()
        except OSError:
            continue
        for line in lines[-50:]:  # read tail of each log
            try:
                rec = json.loads(line)
                ts_str = rec.get("ts", "")
                # convert ISO ts to float for sorting
                try:
                    ts = time.mktime(time.strptime(ts_str, "%Y-%m-%dT%H:%M:%SZ"))
                except Exception:  # noqa: BLE001
                    ts = 0.0
                all_lines.append((ts, line))
            except json.JSONDecodeError:
                all_lines.append((0.0, line))
    all_lines.sort(key=lambda x: x[0], reverse=True)
    return [line for _, line in all_lines[:n]]


# ---------------------------------------------------------------------------
# Checks table
# ---------------------------------------------------------------------------

def _build_checks() -> List[Tuple[str, Callable[[], Any]]]:
    return [
        ("tmux", _tmux_version),
        ("claude binary", lambda: _binary("claude")),
        ("codex binary", lambda: _binary("codex")),
        ("opencode binary", lambda: _binary("opencode")),
        ("transcripts writable", _transcripts_writable),
        ("adapters importable", _import_all_adapters),
        ("monitor PIDs", _list_pids),
        ("recent events", _recent_events),
    ]


def _safe_run(fn: Callable[[], Any]) -> Any:
    try:
        return fn()
    except Exception as exc:  # noqa: BLE001
        return f"ERROR: {exc}"


def _is_failing(value: Any) -> bool:
    if value is False:
        return True
    if isinstance(value, str) and value in ("MISSING", ""):
        return True
    if isinstance(value, str) and value.startswith("ERROR:"):
        return True
    if isinstance(value, str) and value.startswith("IMPORT_ERROR:"):
        return True
    if isinstance(value, list):
        return any(
            isinstance(item, str) and ("NOT_WRITABLE" in item or "IMPORT_ERROR" in item)
            for item in value
        )
    return False


# ---------------------------------------------------------------------------
# Output formatters
# ---------------------------------------------------------------------------

_GREEN = "\033[32m"
_RED = "\033[31m"
_RESET = "\033[0m"

_NO_COLOR = not sys.stdout.isatty() or os.environ.get("NO_COLOR")


def _color(text: str, code: str) -> str:
    if _NO_COLOR:
        return text
    return f"{code}{text}{_RESET}"


def _print_table(results: List[Tuple[str, Any]]) -> None:
    width = max(len(name) for name, _ in results)
    for name, value in results:
        failing = _is_failing(value)
        status = _color("FAIL", _RED) if failing else _color("OK  ", _GREEN)
        if isinstance(value, list):
            display = ", ".join(str(v) for v in value) if value else "(none)"
        else:
            display = str(value)
        print(f"  [{status}] {name:<{width}}  {display}")


# ---------------------------------------------------------------------------
# Stats aggregation
# ---------------------------------------------------------------------------

def _print_stats(filter_cwd: str | None = None) -> None:
    """Read stats.jsonl and print last-7-day counts per event type and harness."""
    if not _STATS_FILE.exists():
        print("No stats ledger found at", _STATS_FILE)
        return

    cutoff = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=7)
    by_event: collections.Counter[str] = collections.Counter()
    by_harness: collections.Counter[str] = collections.Counter()
    total = 0
    skipped = 0

    with _STATS_FILE.open() as fh:
        for lineno, raw in enumerate(fh, 1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                row = json.loads(raw)
            except json.JSONDecodeError:
                skipped += 1
                continue

            # Filter by cwd when requested
            if filter_cwd is not None and row.get("cwd") != filter_cwd:
                continue

            # Filter by 7-day window
            ts_str = row.get("ts", "")
            try:
                ts = datetime.datetime.strptime(ts_str, "%Y-%m-%dT%H:%M:%SZ").replace(
                    tzinfo=datetime.timezone.utc
                )
            except (ValueError, TypeError):
                ts = datetime.datetime.min.replace(tzinfo=datetime.timezone.utc)

            if ts < cutoff:
                continue

            total += 1
            event = row.get("event", "(unknown)")
            harness = row.get("harness", "(unknown)")
            by_event[event] += 1
            by_harness[harness] += 1

    print(f"autospec stats — last 7 days  (ledger: {_STATS_FILE})")
    if filter_cwd:
        print(f"  filtered to cwd: {filter_cwd}")
    print(f"  total events : {total}")
    if skipped:
        print(f"  malformed lines skipped: {skipped}")
    print()

    print("By event type:")
    if by_event:
        width = max(len(k) for k in by_event)
        for event, count in sorted(by_event.items(), key=lambda x: -x[1]):
            print(f"  {event:<{width}}  {count}")
    else:
        print("  (none)")

    print()
    print("By harness:")
    if by_harness:
        width = max(len(k) for k in by_harness)
        for harness, count in sorted(by_harness.items(), key=lambda x: -x[1]):
            print(f"  {harness:<{width}}  {count}")
    else:
        print("  (none)")


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    """Run the doctor checks and return an exit code."""
    parser = argparse.ArgumentParser(
        prog="python -m autospec_context_monitor.doctor",
        description="Diagnostic tool for the autospec context-monitor.",
    )
    parser.add_argument(
        "--json",
        dest="json_output",
        action="store_true",
        default=False,
        help="Emit results as a JSON array to stdout.",
    )
    parser.add_argument(
        "--self-test",
        dest="self_test",
        action="store_true",
        default=False,
        help="After checks, run pytest over the adapter and engine test suites.",
    )
    parser.add_argument(
        "--stats",
        action="store_true",
        default=False,
        help="Print last-7-day counts from the stats ledger (~/.autospec/monitors/stats.jsonl).",
    )
    parser.add_argument(
        "--cwd",
        default=None,
        help="When used with --stats, filter events to this working directory.",
    )
    args = parser.parse_args(argv)

    # Short-circuit: if --stats is requested, print and exit.
    if args.stats:
        _print_stats(filter_cwd=args.cwd)
        return 0

    checks = _build_checks()
    results: List[Tuple[str, Any]] = [(name, _safe_run(fn)) for name, fn in checks]
    fail = any(_is_failing(v) for _, v in results)

    if args.json_output:
        print(json.dumps([{"check": name, "result": value} for name, value in results], default=str))
    else:
        print("autospec-context-monitor doctor")
        print("=" * 40)
        _print_table(results)
        if fail:
            print("\nOne or more checks failed. Review above for details.")
        else:
            print("\nAll checks passed.")

    if args.self_test:
        import pytest  # type: ignore[import]
        rc = pytest.main(["-q", "tests/adapters", "tests/engine"])
        return int(rc) or (1 if fail else 0)

    return 1 if fail else 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
