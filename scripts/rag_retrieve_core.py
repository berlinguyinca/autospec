"""Core deterministic local retrieval/evaluation API."""
import math
from collections import Counter
from typing import Dict, Mapping, Sequence

from rag_retrieve_rank import bm25_rank, dense_rank, rerank_score, rrf_fuse
from rag_retrieve_text import candidate_chunks, chunk_metadata


def retrieval_knobs(config: Mapping, overrides: Mapping[str, int | float | None]) -> Dict[str, int | float]:
    retrieval = config.get("retrieval", {})
    knobs = {"dense_top_k": int(retrieval.get("dense_top_k", 50)), "bm25_top_k": int(retrieval.get("bm25_top_k", 50)), "rrf_k": int(retrieval.get("rrf_k", 60)), "fusion_weight": float(retrieval.get("fusion_weight", 0.5)), "rerank_top_n": int(retrieval.get("rerank_top_n", 50)), "final_n": int(retrieval.get("final_n", 8))}
    knobs.update({key: (float(value) if key == "fusion_weight" else int(value)) for key, value in overrides.items() if value is not None})
    return knobs


def retrieve(index: Mapping, config: Mapping, query: str, filters: Mapping[str, str] | None = None, mode: str = "hybrid", overrides: Mapping[str, int | float | None] | None = None) -> Dict:
    filters, knobs = filters or {}, retrieval_knobs(config, overrides or {})
    chunks = candidate_chunks(index, filters)
    by_id = {str(chunk.get("chunk_id")): chunk for chunk in chunks}
    dense = dense_rank(query, chunks, int(knobs["dense_top_k"]))
    bm25 = bm25_rank(query, chunks, int(knobs["bm25_top_k"]))
    fused = rrf_fuse(dense, bm25, int(knobs["rrf_k"]), float(knobs["fusion_weight"]), mode)
    reranked = _rerank(query, by_id, fused, int(knobs["rerank_top_n"]))
    return {"query": query, "mode": mode, "filters": dict(filters), "knobs": knobs, "stages": {"candidates": len(chunks), "dense": len(dense), "bm25": len(bm25), "fused": len(fused), "reranked": len(reranked)}, "results": _result_rows(by_id, reranked, int(knobs["final_n"]))}


def _rerank(query: str, by_id: Mapping[str, Mapping], fused: Mapping[str, float], top_n: int):
    over = sorted(fused.items(), key=lambda item: (-item[1], item[0]))[:top_n]
    max_fused = max([score for _cid, score in over] or [1.0])
    rows = [(cid, (0.65 * (score / max_fused if max_fused else score)) + (0.35 * rerank_score(query, by_id[cid])), score, rerank_score(query, by_id[cid])) for cid, score in over]
    return sorted(rows, key=lambda item: (-item[1], item[0]))


def _result_rows(by_id: Mapping[str, Mapping], reranked, final_n: int):
    return [{"rank": rank, "chunk_id": cid, "doc_id": by_id[cid].get("doc_id"), "source_path": by_id[cid].get("source_path", by_id[cid].get("path")), "heading": by_id[cid].get("heading", ""), "score": round(score, 6), "fused_score": round(fused_score, 6), "reranker_score": round(reranker_score, 6), "metadata": chunk_metadata(by_id[cid])} for rank, (cid, score, fused_score, reranker_score) in enumerate(reranked[:final_n], 1)]


def evaluate(index: Mapping, config: Mapping, golden: Mapping, mode: str, overrides: Mapping[str, int | float | None]) -> Dict:
    rows, totals = [], Counter()
    queries = golden.get("queries", golden if isinstance(golden, list) else [])
    knobs = retrieval_knobs(config, overrides)
    for row in queries:
        out = retrieve(index, config, row["query"], row.get("filters", {}), mode=mode, overrides=overrides)
        metrics = query_metrics(out["results"], row.get("relevant", {}), int(knobs["final_n"]))
        totals.update(metrics)
        rows.append({"query": row["query"], "filters": row.get("filters", {}), "results": [r["chunk_id"] for r in out["results"]], "metrics": {k: round(v, 6) for k, v in metrics.items()}})
    count = max(len(rows), 1)
    return {"mode": mode, "knobs": knobs, "retrieval": {k: round(totals[k] / count, 6) for k in ("ndcg", "mrr", "recall_at_k", "precision_at_k")} | {"queries": len(rows)}, "per_query": rows}


def dcg(gains: Sequence[float]) -> float:
    return sum((2**gain - 1) / math.log2(idx + 2) for idx, gain in enumerate(gains))


def query_metrics(results: Sequence[Mapping], relevant: Mapping[str, float], final_n: int) -> Dict[str, float]:
    ranked_ids = [str(r.get("chunk_id")) for r in results[:final_n]]
    gains = [float(relevant.get(cid, 0.0)) for cid in ranked_ids]
    ideal = sorted((float(v) for v in relevant.values()), reverse=True)[:final_n]
    rr = next((1.0 / idx for idx, cid in enumerate(ranked_ids, 1) if float(relevant.get(cid, 0.0)) > 0), 0.0)
    hits = sum(1 for cid in ranked_ids if float(relevant.get(cid, 0.0)) > 0)
    return {"ndcg": dcg(gains) / dcg(ideal) if dcg(ideal) else 0.0, "mrr": rr, "recall_at_k": hits / max(len(relevant), 1), "precision_at_k": hits / max(len(ranked_ids), 1)}
