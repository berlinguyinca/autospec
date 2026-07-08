"""Text helper operations for deterministic RAG query transforms."""
from __future__ import annotations

from typing import Iterable, List, Mapping

from rag_retrieve_text import tokenize

DEFAULT_REWRITE_EXPANSIONS = {
    "rag": "retrieval augmented generation documentation search",
    "rrf": "reciprocal rank fusion",
    "rrf_k": "reciprocal rank fusion constant rrf_k",
    "hyde": "hypothetical document embedding",
    "bm25": "sparse keyword search bm25",
    "ndcg": "normalized discounted cumulative gain",
}


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
