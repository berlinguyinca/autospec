"""Deterministic query-transformation router for local RAG retrieval.

The router intentionally avoids LLM/API calls. It classifies a query with small
heuristics, generates local rewrite/HyDE/multi-query/decomposition candidates,
measures each candidate against the golden-set labels, and only enables a
transform when the measured recall lift is positive within the configured cost
budget.
"""
from __future__ import annotations

from collections import Counter
from typing import Dict, List, Mapping, Sequence

from rag_query_results import aggregate_variant_results
from rag_query_transforms import TRANSFORMS, classify_query, transform_candidates
from rag_retrieve_core import query_metrics, retrieve, retrieval_knobs
from rag_retrieve_text import tokenize


def route_evaluate(index: Mapping, config: Mapping, golden: Mapping, mode: str, overrides: Mapping[str, int | float | None]) -> Dict:
    rows: List[Dict] = []
    totals = Counter()
    baseline_totals = Counter()
    queries = golden.get("queries", golden if isinstance(golden, list) else [])
    knobs = retrieval_knobs(config, overrides)
    for row in queries:
        routed = route_query(index, config, row, mode, overrides, knobs)
        totals.update(routed["metrics"])
        baseline_totals.update(routed["baseline_metrics"])
        rows.append(routed)
    count = max(len(rows), 1)
    return {
        "mode": mode,
        "knobs": knobs,
        "retrieval": average_metrics(totals, count) | {"queries": len(rows)},
        "baseline_retrieval": average_metrics(baseline_totals, count) | {"queries": len(rows)},
        "per_query": rows,
    }


def route_query(index: Mapping, config: Mapping, row: Mapping, mode: str, overrides: Mapping[str, int | float | None], knobs: Mapping[str, int | float]) -> Dict:
    query = str(row["query"])
    filters = row.get("filters", {})
    relevant = row.get("relevant", {})
    final_n = int(knobs["final_n"])
    query_class = str(row.get("query_class") or classify_query(query))
    baseline_out = retrieve(index, config, query, filters, mode=mode, overrides=overrides)
    baseline_metrics = query_metrics(baseline_out["results"], relevant, final_n)
    decisions, selected_variants = route_transform_decisions(index, config, query, query_class, filters, relevant, mode, overrides, final_n, baseline_metrics)
    routed_results = aggregate_variant_results(index, config, selected_variants or [query], filters, mode, overrides, final_n)
    metrics = query_metrics(routed_results, relevant, final_n)
    return {
        "query": query,
        "query_class": query_class,
        "baseline_results": [r["chunk_id"] for r in baseline_out["results"]],
        "baseline_metrics": rounded_metrics(baseline_metrics),
        "enabled_transforms": [name for name, decision in decisions.items() if decision["enabled"]],
        "decisions": decisions,
        "variants": selected_variants or [query],
        "results": [r["chunk_id"] for r in routed_results],
        "metrics": rounded_metrics(metrics),
    }


def route_transform_decisions(index: Mapping, config: Mapping, query: str, query_class: str, filters: Mapping[str, str], relevant: Mapping[str, float], mode: str, overrides: Mapping[str, int | float | None], final_n: int, baseline_metrics: Mapping[str, float]):
    decisions: Dict[str, Dict] = {}
    selected_variants: List[str] = []
    for transform in TRANSFORMS:
        candidates = transform_candidates(transform, query, query_class, config)
        decisions[transform] = measure_transform(index, config, candidates, filters, relevant, mode, overrides, final_n, baseline_metrics)
        if decisions[transform]["enabled"]:
            selected_variants.extend(candidates)
    return decisions, selected_variants


def measure_transform(index: Mapping, config: Mapping, candidates: Sequence[str], filters: Mapping[str, str], relevant: Mapping[str, float], mode: str, overrides: Mapping[str, int | float | None], final_n: int, baseline_metrics: Mapping[str, float]) -> Dict:
    added_tokens = sum(len(tokenize(candidate)) for candidate in candidates)
    decision = empty_decision(candidates, added_tokens)
    if not candidates:
        return decision
    metrics = query_metrics(aggregate_variant_results(index, config, list(candidates), filters, mode, overrides, final_n), relevant, final_n)
    recall_lift = metrics["recall_at_k"] - float(baseline_metrics.get("recall_at_k", 0.0))
    ndcg_lift = metrics["ndcg"] - float(baseline_metrics.get("ndcg", 0.0))
    decision.update({"recall_lift": round(recall_lift, 6), "ndcg_lift": round(ndcg_lift, 6)})
    return gate_decision(decision, config, recall_lift, ndcg_lift, added_tokens)


def empty_decision(candidates: Sequence[str], added_tokens: int) -> Dict:
    return {
        "eligible": bool(candidates),
        "enabled": False,
        "reason": "net_negative" if candidates else "not_applicable",
        "added_queries": len(candidates),
        "added_tokens": added_tokens,
        "recall_lift": 0.0,
        "ndcg_lift": 0.0,
        "latency_cost": len(candidates),
        "candidates": list(candidates),
    }


def gate_decision(decision: Dict, config: Mapping, recall_lift: float, ndcg_lift: float, added_tokens: int) -> Dict:
    qt = config.get("query_transform", {})
    min_lift = float(qt.get("min_recall_lift", 0.0))
    max_tokens = int(qt.get("max_added_tokens", 96))
    if added_tokens > max_tokens:
        decision["reason"] = "cost_budget_exceeded"
    elif recall_lift >= min_lift and (recall_lift > 0 or ndcg_lift > 0):
        decision["enabled"] = True
        decision["reason"] = "positive_lift"
    return decision


def average_metrics(totals: Mapping[str, float], count: int) -> Dict[str, float]:
    return {k: round(totals[k] / count, 6) for k in ("ndcg", "mrr", "recall_at_k", "precision_at_k")}


def rounded_metrics(metrics: Mapping[str, float]) -> Dict[str, float]:
    return {key: round(float(value), 6) for key, value in metrics.items()}
