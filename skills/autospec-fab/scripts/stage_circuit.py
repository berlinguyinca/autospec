#!/usr/bin/env python3
"""
stage_circuit.py — vacuum circuit QA stage for autospec-fab.

Uniform stage CLI:
    stage_circuit.py --in <stl|dir> --circuit <circuit.json> --out <fragment.json>

The circuit graph is a SEPARATE input (--circuit) because
autospec-fab-model.schema.json is additionalProperties:false and the channel
graph does not belong in the per-model metadata sidecar.

Circuit JSON schema:
  {
    "nodes":           [<node_id>, ...],
    "edges":           [[<a>, <b>], ...],       // undirected channel segments
    "inlets":          [<node_id>, ...],         // vacuum inlet nodes
    "targets":         [<node_id>, ...],         // gaskets/cavities/ports each inlet must reach
    "isolated_groups": [[<node_id>, ...], ...],  // node-sets that must stay mutually unreachable
    "self_clamping":   true | false,             // true → shared-input branches allowed
    "pump_type":       "low_flow" | "standard",
    "relief_nodes":    [<node_id>, ...]          // relief valves / bleed ports / restrictors
  }

Checks performed:
  1. REACHABILITY   — every inlet must reach every target via BFS over the
                      undirected channel graph.
                      Fail: code_health disconnected_flow; detail names the pair.
  2. ISOLATION      — declared isolated_groups must remain mutually UNREACHABLE
                      (no path between any node in group A and any node in group B),
                      UNLESS self_clamping is true (shared-input branches waived).
                      Fail: circuit_isolation_violation.
  3. NO RELIEF ON   — if pump_type == "low_flow" and relief_nodes is non-empty,
     LOW-FLOW PUMPS   the stage rejects the model.
                      Fail: relief_on_low_flow.

Exit codes:
  0  — harness success (gate verdict is in fragment "status", not exit code).
  1  — harness/usage error (bad args, unreadable files, etc.).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import deque
from typing import Dict, List, Optional, Set, Tuple

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

STAGE_NAME = "vacuum-circuit"

# ---------------------------------------------------------------------------
# Types
# ---------------------------------------------------------------------------

AdjGraph = Dict[str, List[str]]
Finding = Dict[str, str]
Fragment = Dict


# ---------------------------------------------------------------------------
# Graph helpers (stdlib BFS — no external graph lib)
# ---------------------------------------------------------------------------

def _build_adjacency(nodes: List[str], edges: List[List[str]]) -> AdjGraph:
    """Build an undirected adjacency dict from nodes + edge list."""
    adj: AdjGraph = {n: [] for n in nodes}
    for edge in edges:
        if len(edge) < 2:
            continue
        a, b = edge[0], edge[1]
        # Add nodes implicitly if not declared (defensive)
        if a not in adj:
            adj[a] = []
        if b not in adj:
            adj[b] = []
        adj[a].append(b)
        adj[b].append(a)
    return adj


def _bfs_reachable(adj: AdjGraph, start: str) -> Set[str]:
    """Return the set of all nodes reachable from *start* via BFS."""
    if start not in adj:
        return {start}
    visited: Set[str] = {start}
    queue: deque[str] = deque([start])
    while queue:
        node = queue.popleft()
        for neighbour in adj.get(node, []):
            if neighbour not in visited:
                visited.add(neighbour)
                queue.append(neighbour)
    return visited


def _can_reach(adj: AdjGraph, src: str, dst: str) -> bool:
    """Return True if *dst* is reachable from *src*."""
    return dst in _bfs_reachable(adj, src)


# ---------------------------------------------------------------------------
# Check 1: reachability
# ---------------------------------------------------------------------------

def _check_reachability(
    adj: AdjGraph,
    inlets: List[str],
    targets: List[str],
) -> List[Finding]:
    """Every inlet must reach every target. Returns findings for broken pairs."""
    findings: List[Finding] = []
    for inlet in inlets:
        reachable = _bfs_reachable(adj, inlet)
        for target in targets:
            if target not in reachable:
                findings.append({
                    "code": "disconnected_flow",
                    "detail": (
                        f"inlet '{inlet}' cannot reach target '{target}': "
                        "no path in channel graph"
                    ),
                })
    return findings


# ---------------------------------------------------------------------------
# Check 2: isolation
# ---------------------------------------------------------------------------

def _check_isolation(
    adj: AdjGraph,
    isolated_groups: List[List[str]],
    self_clamping: bool,
) -> List[Finding]:
    """
    Declared isolated_groups must remain mutually unreachable unless
    self_clamping is True.

    Returns findings for any cross-group path detected.
    """
    if self_clamping:
        return []  # shared-input self-clamping branches are explicitly allowed

    findings: List[Finding] = []
    n = len(isolated_groups)
    for i in range(n):
        for j in range(i + 1, n):
            group_a = isolated_groups[i]
            group_b = isolated_groups[j]
            # Check whether ANY node in group_a can reach ANY node in group_b
            leaked_pair: Optional[Tuple[str, str]] = None
            for node_a in group_a:
                reachable_from_a = _bfs_reachable(adj, node_a)
                for node_b in group_b:
                    if node_b in reachable_from_a:
                        leaked_pair = (node_a, node_b)
                        break
                if leaked_pair:
                    break
            if leaked_pair:
                findings.append({
                    "code": "circuit_isolation_violation",
                    "detail": (
                        f"isolated groups {group_a} and {group_b} are not "
                        f"electrically/fluidically isolated: path found between "
                        f"'{leaked_pair[0]}' and '{leaked_pair[1]}'"
                    ),
                })
    return findings


# ---------------------------------------------------------------------------
# Check 3: no relief on low-flow pumps
# ---------------------------------------------------------------------------

def _check_relief(pump_type: str, relief_nodes: List[str]) -> List[Finding]:
    """
    Reject relief valves / bleed ports / intentional restrictors on low-flow
    vacuum pump models. The low-flow pump rule forbids these components.
    """
    if pump_type != "low_flow":
        return []
    if not relief_nodes:
        return []
    return [
        {
            "code": "relief_on_low_flow",
            "detail": (
                f"pump_type 'low_flow' forbids relief/bleed/restrictor nodes; "
                f"found: {relief_nodes}"
            ),
        }
    ]


# ---------------------------------------------------------------------------
# Stage entry point
# ---------------------------------------------------------------------------

def _load_circuit(circuit_path: Optional[str]) -> Tuple[Optional[Dict], Optional[Fragment]]:
    """
    Resolve the circuit input. Returns (circuit, early_fragment).

    When no circuit graph is supplied (the engine sequences stages with only
    --in/--model/--out and this model declares no vacuum circuit), the stage
    DEGRADES to a non-blocking skip rather than hard-failing — a missing
    optional extra input is not a gate failure. On a load error the early
    fragment is a fail; otherwise early_fragment is None and circuit is set.
    """
    if not circuit_path:
        return None, {
            "stage": STAGE_NAME,
            "status": "skip",
            "detail": ("no circuit graph supplied (--circuit absent); "
                       "vacuum-circuit QA skipped"),
            "findings": [],
        }
    try:
        with open(circuit_path) as fh:
            return json.load(fh), None
    except Exception as exc:
        return None, {
            "stage": STAGE_NAME,
            "status": "fail",
            "detail": f"failed to load circuit JSON: {exc}",
            "findings": [{"code": "disconnected_flow", "detail": str(exc)}],
        }


def _assemble_fragment(
    findings: List[Finding],
    inlet_count: int,
    target_count: int,
) -> Fragment:
    """Build the pass/fail fragment from accumulated findings."""
    if findings:
        detail = "; ".join(f["detail"] for f in findings)
        status = "fail"
    else:
        detail = (
            f"all {inlet_count} inlet(s) reach all {target_count} target(s); "
            "isolation intact; pump configuration valid"
        )
        status = "pass"
    return {
        "stage": STAGE_NAME,
        "status": status,
        "detail": detail,
        "findings": findings,
    }


def run(circuit_path: Optional[str]) -> Fragment:
    """
    Execute all circuit checks and return the stage fragment dict.

    This is the public API; main() wraps it for CLI use.
    """
    circuit, early = _load_circuit(circuit_path)
    if early is not None:
        return early
    assert circuit is not None

    nodes: List[str] = circuit.get("nodes", [])
    edges: List[List[str]] = circuit.get("edges", [])
    inlets: List[str] = circuit.get("inlets", [])
    targets: List[str] = circuit.get("targets", [])
    isolated_groups: List[List[str]] = circuit.get("isolated_groups", [])
    self_clamping: bool = bool(circuit.get("self_clamping", False))
    pump_type: str = circuit.get("pump_type", "standard")
    relief_nodes: List[str] = circuit.get("relief_nodes", [])

    adj = _build_adjacency(nodes, edges)

    all_findings: List[Finding] = []
    all_findings.extend(_check_reachability(adj, inlets, targets))
    all_findings.extend(_check_isolation(adj, isolated_groups, self_clamping))
    all_findings.extend(_check_relief(pump_type, relief_nodes))

    return _assemble_fragment(all_findings, len(inlets), len(targets))


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab vacuum circuit QA stage",
    )
    parser.add_argument(
        "--in", dest="stl", required=True,
        help="path to STL file or directory (required for uniform CLI; "
             "circuit verdict is driven by --circuit graph, not geometry)",
    )
    parser.add_argument(
        "--circuit", required=False, default=None,
        help="path to circuit graph JSON (optional; absent → stage skips, "
             "for models with no vacuum circuit)",
    )
    parser.add_argument(
        "--out", required=True,
        help="output path for stage fragment JSON",
    )
    parser.add_argument(
        "--model", required=False, default=None,
        help="per-model metadata sidecar (accepted for uniform stage CLI "
             "symmetry; circuit verdict is driven by --circuit, not metadata)",
    )
    args = parser.parse_args(argv)

    fragment = run(args.circuit)

    try:
        with open(args.out, "w") as fh:
            json.dump(fragment, fh, indent=2)
    except Exception as exc:
        print(f"stage_circuit: failed to write fragment: {exc}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
