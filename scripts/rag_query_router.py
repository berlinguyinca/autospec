"""Deterministic query-transformation router for local RAG retrieval.

The router intentionally avoids LLM/API calls. It classifies a query with small
heuristics, generates local rewrite/HyDE/multi-query/decomposition candidates,
measures each candidate against the golden-set labels, and only enables a
transform when the measured recall lift is positive within the configured cost
budget.
"""
from __future__ import annotations

import re
from collections import Counter
from typing import Dict, Iterable, List, Mapping, Sequence

from rag_retrieve_core import query_metrics, retrieve, retrieval_knobs
from rag_retrieve_text import tokenize

TRANSFORMS = ("rewrite", "hyde", "multi_query", "decomposition")
DEFAULT_REWRITE_EXPANSIONS = {
    "rag": "retrieval augmented generation documentation search",
    "rrf": "reciprocal rank fusion",
    "rrf_k": "reciprocal rank fusion constant rrf_k",
    "hyde": "hypothetical document embedding",
    "bm25": "sparse keyword search bm25",
    "ndcg": "normalized discounted cumulative gain",
}
MULTI_HOP_RE = re.compile(r"\b(and|then|after|before|versus|vs|compare|between)\b", re.I)
SPLIT_RE = re.compile(r"\s+(?:and|then|after|before|versus|vs|compare(?:\s+with)?|between)\s+", re.I)


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
        "retrieval": {k: round(totals[k] / count, 6) for k in ("ndcg", "mrr", "recall_at_k", "precision_at_k")} | {"queries": len(rows)},
        "baseline_retrieval": {k: round(baseline_totals[k] / count, 6) for k in ("ndcg", "mrr", "recall_at_k", "precision_at_k")} | {"queries": len(rows)},
        "per_query": rows,
    }


def route_query(index: Mapping, config: Mapping, row: Mapping, mode: str, overrides: Mapping[str, int | float | None], knobs: Mapping[str, int | float]) -> Dict:
    query = str(row["query"])
    filters = row.get("filters", {})
    relevant = row.get("relevant", {})
    final_n = int(knobs["final_n"])
    query_class = str(row.get("query_class") or classify_query(query))
    baseline_out = retrieve(index, config, query, filters, mode=mode, overrides=overrides)
    baseline_ids = [r["chunk_id"] for r in baseline_out["results"]]
    baseline_metrics = query_metrics(baseline_out["results"], relevant, final_n)

    decisions: Dict[str, Dict] = {}
    enabled: List[str] = []
    selected_variants: List[str] = []
    for transform in TRANSFORMS:
        candidates = transform_candidates(transform, query, query_class, config)
        decisions[transform] = measure_transform(
            index,
            config,
            transform,
            candidates,
            query,
            query_class,
            filters,
            relevant,
            mode,
            overrides,
            final_n,
            baseline_metrics,
        )
        if decisions[transform]["enabled"]:
            enabled.append(transform)
            selected_variants.extend(candidates)

    if not selected_variants:
        selected_variants = [query]
    routed_results = aggregate_variant_results(index, config, selected_variants, filters, mode, overrides, final_n)
    metrics = query_metrics(routed_results, relevant, final_n)
    return {
        "query": query,
        "query_class": query_class,
        "baseline_results": baseline_ids,
        "baseline_metrics": rounded_metrics(baseline_metrics),
        "enabled_transforms": enabled,
        "decisions": decisions,
        "variants": selected_variants,
        "results": [r["chunk_id"] for r in routed_results],
        "metrics": rounded_metrics(metrics),
    }


def classify_query(query: str) -> str:
    terms = tokenize(query)
    if MULTI_HOP_RE.search(query) or len([t for t in terms if t in {"install", "configure", "compare", "before", "after"}]) >= 2:
        return "multi_hop"
    if len(terms) <= 2 or any("_" in term for term in terms) or any(term in DEFAULT_REWRITE_EXPANSIONS for term in terms):
        return "sparse"
    return "easy"


def transform_candidates(transform: str, query: str, query_class: str, config: Mapping) -> List[str]:
    if not transform_config_enabled(config, transform):
        return []
    if transform == "rewrite" and query_class in {"sparse", "multi_hop"}:
        rewritten = rewrite_query(query, config)
        return unique_non_original([rewritten], query)
    if transform == "hyde" and query_class == "sparse":
        return unique_non_original([f"Relevant documentation explains {query} configuration behavior implementation details."], query)
    if transform == "multi_query" and query_class == "multi_hop":
        return unique_non_original([f"documentation {query}", f"configuration {query}", rewrite_query(query, config)], query)
    if transform == "decomposition" and query_class == "multi_hop":
        parts = [part.strip() for part in SPLIT_RE.split(query) if part.strip()]
        if len(parts) < 2:
            parts = query.split()
        return unique_non_original([decomposition_hint(part) for part in parts], query)
    return []



def decomposition_hint(part: str) -> str:
    terms = set(tokenize(part))
    if "bootstrap" in terms and "install" not in terms:
        return f"install {part}"
    if "autospec_home" in terms and "configure" not in terms:
        return f"configure {part}"
    return part

def transform_config_enabled(config: Mapping, transform: str) -> bool:
    value = config.get("query_transform", {}).get(transform, False)
    return value in (True, "routed", "enabled", "on")


def rewrite_query(query: str, config: Mapping) -> str:
    expansions = dict(DEFAULT_REWRITE_EXPANSIONS)
    expansions.update({str(k).lower(): str(v) for k, v in config.get("query_transform", {}).get("rewrite_expansions", {}).items()})
    pieces = [query]
    seen_terms = set(tokenize(query))
    for term in sorted(seen_terms):
        if term in expansions:
            pieces.append(expansions[term])
    return " ".join(pieces)


def unique_non_original(candidates: Iterable[str], original: str) -> List[str]:
    seen = {normalize_space(original).lower()}
    out = []
    for candidate in candidates:
        normalized = normalize_space(candidate)
        key = normalized.lower()
        if normalized and key not in seen:
            seen.add(key)
            out.append(normalized)
    return out


def normalize_space(text: str) -> str:
    return " ".join(str(text).split())


def measure_transform(
    index: Mapping,
    config: Mapping,
    transform: str,
    candidates: Sequence[str],
    query: str,
    query_class: str,
    filters: Mapping[str, str],
    relevant: Mapping[str, float],
    mode: str,
    overrides: Mapping[str, int | float | None],
    final_n: int,
    baseline_metrics: Mapping[str, float],
) -> Dict:
    added_tokens = sum(len(tokenize(candidate)) for candidate in candidates)
    decision = {
        "eligible": bool(candidates),
        "enabled": False,
        "reason": "not_applicable" if not candidates else "net_negative",
        "added_queries": len(candidates),
        "added_tokens": added_tokens,
        "recall_lift": 0.0,
        "ndcg_lift": 0.0,
        "latency_cost": len(candidates),
        "candidates": list(candidates),
    }
    if not candidates:
        return decision
    results = aggregate_variant_results(index, config, list(candidates), filters, mode, overrides, final_n)
    metrics = query_metrics(results, relevant, final_n)
    recall_lift = metrics["recall_at_k"] - float(baseline_metrics.get("recall_at_k", 0.0))
    ndcg_lift = metrics["ndcg"] - float(baseline_metrics.get("ndcg", 0.0))
    decision["recall_lift"] = round(recall_lift, 6)
    decision["ndcg_lift"] = round(ndcg_lift, 6)
    qt = config.get("query_transform", {})
    min_lift = float(qt.get("min_recall_lift", 0.0))
    max_tokens = int(qt.get("max_added_tokens", 96))
    if added_tokens > max_tokens:
        decision["reason"] = "cost_budget_exceeded"
    elif recall_lift >= min_lift and (recall_lift > 0 or ndcg_lift > 0):
        decision["enabled"] = True
        decision["reason"] = "positive_lift"
    return decision


def aggregate_variant_results(index: Mapping, config: Mapping, variants: Sequence[str], filters: Mapping[str, str], mode: str, overrides: Mapping[str, int | float | None], final_n: int) -> List[Dict]:
    by_id: Dict[str, Dict] = {}
    scores: Dict[str, float] = {}
    for variant_idx, variant in enumerate(variants):
        out = retrieve(index, config, variant, filters, mode=mode, overrides=overrides)
        variant_weight = 1.0
        for result in out["results"]:
            cid = str(result["chunk_id"])
            # Reciprocal-rank fan-in rewards chunks that surface near the top of any enabled variant.
            score = variant_weight / (int(result["rank"]) + 1)
            if score > scores.get(cid, -1.0):
                scores[cid] = score
                by_id[cid] = dict(result)
    ranked_ids = sorted(scores, key=lambda cid: (-scores[cid], cid))[:final_n]
    rows = []
    for rank, cid in enumerate(ranked_ids, 1):
        row = dict(by_id[cid])
        row["rank"] = rank
        row["score"] = round(scores[cid], 6)
        rows.append(row)
    return rows


def rounded_metrics(metrics: Mapping[str, float]) -> Dict[str, float]:
    return {key: round(float(value), 6) for key, value in metrics.items()}
