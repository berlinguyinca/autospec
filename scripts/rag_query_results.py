"""Result aggregation helpers for routed RAG query variants."""
from __future__ import annotations

from typing import Dict, List, Mapping, Sequence

from rag_retrieve_core import retrieve


def aggregate_variant_results(index: Mapping, config: Mapping, variants: Sequence[str], filters: Mapping[str, str], mode: str, overrides: Mapping[str, int | float | None], final_n: int) -> List[Dict]:
    by_id: Dict[str, Dict] = {}
    scores: Dict[str, float] = {}
    for variant in variants:
        out = retrieve(index, config, variant, filters, mode=mode, overrides=overrides)
        collect_variant_scores(out["results"], by_id, scores)
    return ranked_rows(by_id, scores, final_n)


def collect_variant_scores(results: Sequence[Mapping], by_id: Dict[str, Dict], scores: Dict[str, float]) -> None:
    for result in results:
        cid = str(result["chunk_id"])
        score = 1.0 / (int(result["rank"]) + 1)
        if score > scores.get(cid, -1.0):
            scores[cid] = score
            by_id[cid] = dict(result)


def ranked_rows(by_id: Mapping[str, Mapping], scores: Mapping[str, float], final_n: int) -> List[Dict]:
    rows = []
    for rank, cid in enumerate(sorted(scores, key=lambda cid: (-scores[cid], cid))[:final_n], 1):
        row = dict(by_id[cid])
        row["rank"] = rank
        row["score"] = round(scores[cid], 6)
        rows.append(row)
    return rows
