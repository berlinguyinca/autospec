"""Query classification and deterministic transform generation."""
from __future__ import annotations

import re
from typing import List, Mapping

from rag_query_textops import DEFAULT_REWRITE_EXPANSIONS, rewrite_query, unique_non_original
from rag_retrieve_text import tokenize

TRANSFORMS = ("rewrite", "hyde", "multi_query", "decomposition")
MULTI_HOP_RE = re.compile(r"\b(and|then|after|before|versus|vs|compare|between)\b", re.I)
SPLIT_RE = re.compile(r"\s+(?:and|then|after|before|versus|vs|compare(?:\s+with)?|between)\s+", re.I)


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
        return unique_non_original([rewrite_query(query, config)], query)
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
