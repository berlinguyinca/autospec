#!/usr/bin/env python3
"""Deterministic local hybrid retrieval for the RAG workstream.

No external vector DB, embedding API, or third-party dependency is used.  The
"dense" lane is a hash/bucket token placeholder that gives stable semantic-ish
coverage for tuning contracts; the sparse lane is exact-token BM25.  Hybrid mode
fuses both lanes with Reciprocal Rank Fusion and runs a small deterministic
reranker over the over-retrieved candidate set.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from collections import Counter
from typing import Dict, Iterable, List, Mapping, Sequence, Tuple

TOKEN_RE = re.compile(r"[a-z0-9_]+")


def load_json(path: str):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def write_json(obj):
    print(json.dumps(obj, indent=2, sort_keys=True))


def tokenize(text: str) -> List[str]:
    return TOKEN_RE.findall(str(text).lower())


def dense_tokens(text: str) -> List[str]:
    """Stable local embedding placeholder tokens.

    Exact short identifiers and snake_case jargon are intentionally excluded so
    dense-only retrieval can miss acronym/config-key queries that BM25 catches.
    Longer natural-language tokens are bucketed by prefix and hash to model a
    coarse semantic lane without adding an embedding dependency.
    """
    buckets: List[str] = []
    for token in tokenize(text):
        if "_" in token or len(token) < 5:
            continue
        digest = hashlib.sha256(token.encode("utf-8")).hexdigest()[:2]
        buckets.append(f"{token[:4]}:{digest}")
    return buckets


def chunk_text(chunk: Mapping) -> str:
    return "\n".join(str(chunk.get(k, "")) for k in ("heading", "text", "source_path", "doc_id"))


def chunk_metadata(chunk: Mapping) -> Dict[str, str]:
    metadata = {str(k): str(v) for k, v in dict(chunk.get("metadata", {})).items()}
    for key in ("index_version", "doc_version", "section", "product_area"):
        if key in chunk and key not in metadata:
            metadata[key] = str(chunk[key])
    return metadata


def candidate_chunks(index: Mapping, filters: Mapping[str, str]) -> List[Mapping]:
    chunks = index.get("chunks", index if isinstance(index, list) else [])
    result = []
    for chunk in chunks:
        meta = chunk_metadata(chunk)
        if all(str(meta.get(key, "")) == str(value) for key, value in filters.items()):
            result.append(chunk)
    return result


def count_overlap(query_counts: Counter, doc_counts: Counter) -> float:
    return float(sum(min(count, doc_counts.get(tok, 0)) for tok, count in query_counts.items()))


def dense_rank(query: str, chunks: Sequence[Mapping], top_k: int) -> List[Tuple[str, float]]:
    query_counts = Counter(dense_tokens(query))
    if not query_counts:
        return []
    scored = []
    for chunk in chunks:
        doc_counts = Counter(dense_tokens(chunk_text(chunk)))
        overlap = count_overlap(query_counts, doc_counts)
        if overlap <= 0:
            continue
        denom = math.sqrt(sum(v * v for v in query_counts.values())) * math.sqrt(sum(v * v for v in doc_counts.values()))
        score = overlap / denom if denom else 0.0
        scored.append((str(chunk.get("chunk_id")), score))
    return sorted(scored, key=lambda item: (-item[1], item[0]))[:top_k]


def bm25_rank(query: str, chunks: Sequence[Mapping], top_k: int) -> List[Tuple[str, float]]:
    query_terms = tokenize(query)
    if not query_terms or not chunks:
        return []
    docs = [tokenize(chunk_text(chunk)) for chunk in chunks]
    avgdl = sum(len(doc) for doc in docs) / max(len(docs), 1)
    df = Counter(term for term in set(query_terms) for doc in docs if term in set(doc))
    q_counts = Counter(query_terms)
    k1, b = 1.2, 0.75
    scored = []
    for chunk, doc in zip(chunks, docs):
        counts = Counter(doc)
        dl = len(doc) or 1
        score = 0.0
        for term, qtf in q_counts.items():
            tf = counts.get(term, 0)
            if tf == 0:
                continue
            idf = math.log(1 + (len(docs) - df[term] + 0.5) / (df[term] + 0.5))
            score += idf * ((tf * (k1 + 1)) / (tf + k1 * (1 - b + b * dl / max(avgdl, 1)))) * qtf
        if score > 0:
            scored.append((str(chunk.get("chunk_id")), score))
    return sorted(scored, key=lambda item: (-item[1], item[0]))[:top_k]


def rrf_fuse(dense: Sequence[Tuple[str, float]], bm25: Sequence[Tuple[str, float]], rrf_k: int, fusion_weight: float, mode: str) -> Dict[str, float]:
    dense_weight = max(0.0, min(1.0, fusion_weight))
    sparse_weight = 1.0 - dense_weight
    if mode == "dense":
        dense_weight, sparse_weight = 1.0, 0.0
    elif mode == "bm25":
        dense_weight, sparse_weight = 0.0, 1.0
    scores: Dict[str, float] = {}
    for rank, (chunk_id, _score) in enumerate(dense, 1):
        if dense_weight:
            scores[chunk_id] = scores.get(chunk_id, 0.0) + dense_weight / (rrf_k + rank)
    for rank, (chunk_id, _score) in enumerate(bm25, 1):
        if sparse_weight:
            scores[chunk_id] = scores.get(chunk_id, 0.0) + sparse_weight / (rrf_k + rank)
    return scores


def rerank_score(query: str, chunk: Mapping) -> float:
    q = Counter(tokenize(query))
    text = chunk_text(chunk).lower()
    terms = tokenize(text)
    if not q or not terms:
        return 0.0
    exact = count_overlap(q, Counter(terms)) / max(sum(q.values()), 1)
    phrase = 1.0 if str(query).lower() in text else 0.0
    heading_terms = Counter(tokenize(chunk.get("heading", "")))
    heading = count_overlap(q, heading_terms) / max(sum(q.values()), 1)
    return (0.75 * exact) + (0.15 * heading) + (0.10 * phrase)


def retrieval_knobs(config: Mapping, overrides: Mapping[str, int | float | None]) -> Dict[str, int | float]:
    retrieval = config.get("retrieval", {})
    knobs = {
        "dense_top_k": int(retrieval.get("dense_top_k", 50)),
        "bm25_top_k": int(retrieval.get("bm25_top_k", 50)),
        "rrf_k": int(retrieval.get("rrf_k", 60)),
        "fusion_weight": float(retrieval.get("fusion_weight", 0.5)),
        "rerank_top_n": int(retrieval.get("rerank_top_n", 50)),
        "final_n": int(retrieval.get("final_n", 8)),
    }
    for key, value in overrides.items():
        if value is not None:
            knobs[key] = float(value) if key == "fusion_weight" else int(value)
    return knobs


def retrieve(index: Mapping, config: Mapping, query: str, filters: Mapping[str, str] | None = None, mode: str = "hybrid", overrides: Mapping[str, int | float | None] | None = None) -> Dict:
    filters = filters or {}
    knobs = retrieval_knobs(config, overrides or {})
    chunks = candidate_chunks(index, filters)
    by_id = {str(chunk.get("chunk_id")): chunk for chunk in chunks}
    dense = dense_rank(query, chunks, int(knobs["dense_top_k"]))
    bm25 = bm25_rank(query, chunks, int(knobs["bm25_top_k"]))
    fused = rrf_fuse(dense, bm25, int(knobs["rrf_k"]), float(knobs["fusion_weight"]), mode)
    over = sorted(fused.items(), key=lambda item: (-item[1], item[0]))[: int(knobs["rerank_top_n"])]
    reranked = []
    max_fused = max([score for _cid, score in over] or [1.0])
    for cid, score in over:
        chunk = by_id[cid]
        rerank = rerank_score(query, chunk)
        final_score = (0.65 * (score / max_fused if max_fused else score)) + (0.35 * rerank)
        reranked.append((cid, final_score, score, rerank))
    reranked.sort(key=lambda item: (-item[1], item[0]))
    results = []
    for rank, (cid, score, fused_score, reranker_score) in enumerate(reranked[: int(knobs["final_n"])], 1):
        chunk = by_id[cid]
        results.append({
            "rank": rank,
            "chunk_id": cid,
            "doc_id": chunk.get("doc_id"),
            "source_path": chunk.get("source_path", chunk.get("path")),
            "heading": chunk.get("heading", ""),
            "score": round(score, 6),
            "fused_score": round(fused_score, 6),
            "reranker_score": round(reranker_score, 6),
            "metadata": chunk_metadata(chunk),
        })
    return {
        "query": query,
        "mode": mode,
        "filters": dict(filters),
        "knobs": knobs,
        "stages": {"candidates": len(chunks), "dense": len(dense), "bm25": len(bm25), "fused": len(fused), "reranked": len(reranked)},
        "results": results,
    }


def dcg(gains: Sequence[float]) -> float:
    return sum((2**gain - 1) / math.log2(idx + 2) for idx, gain in enumerate(gains))


def query_metrics(results: Sequence[Mapping], relevant: Mapping[str, float], final_n: int) -> Dict[str, float]:
    ranked_ids = [str(r.get("chunk_id")) for r in results[:final_n]]
    gains = [float(relevant.get(cid, 0.0)) for cid in ranked_ids]
    ideal = sorted((float(v) for v in relevant.values()), reverse=True)[:final_n]
    ndcg = dcg(gains) / dcg(ideal) if dcg(ideal) else 0.0
    rr = 0.0
    for idx, cid in enumerate(ranked_ids, 1):
        if float(relevant.get(cid, 0.0)) > 0:
            rr = 1.0 / idx
            break
    recall = sum(1 for cid in set(ranked_ids) if float(relevant.get(cid, 0.0)) > 0) / max(len(relevant), 1)
    precision = sum(1 for cid in ranked_ids if float(relevant.get(cid, 0.0)) > 0) / max(len(ranked_ids), 1)
    return {"ndcg": ndcg, "mrr": rr, "recall_at_k": recall, "precision_at_k": precision}


def evaluate(index: Mapping, config: Mapping, golden: Mapping, mode: str, overrides: Mapping[str, int | float | None]) -> Dict:
    rows = []
    totals = Counter()
    queries = golden.get("queries", golden if isinstance(golden, list) else [])
    knobs = retrieval_knobs(config, overrides)
    for row in queries:
        out = retrieve(index, config, row["query"], row.get("filters", {}), mode=mode, overrides=overrides)
        metrics = query_metrics(out["results"], row.get("relevant", {}), int(knobs["final_n"]))
        for key, value in metrics.items():
            totals[key] += value
        rows.append({"query": row["query"], "filters": row.get("filters", {}), "results": [r["chunk_id"] for r in out["results"]], "metrics": {k: round(v, 6) for k, v in metrics.items()}})
    count = max(len(rows), 1)
    return {
        "mode": mode,
        "knobs": knobs,
        "retrieval": {
            "queries": len(rows),
            "ndcg": round(totals["ndcg"] / count, 6),
            "mrr": round(totals["mrr"] / count, 6),
            "recall_at_k": round(totals["recall_at_k"] / count, 6),
            "precision_at_k": round(totals["precision_at_k"] / count, 6),
        },
        "per_query": rows,
    }


def parse_filter(items: Iterable[str]) -> Dict[str, str]:
    filters = {}
    for item in items:
        if "=" not in item:
            raise SystemExit(f"--filter must be KEY=VALUE: {item}")
        key, value = item.split("=", 1)
        filters[key] = value
    return filters


def add_knobs(parser):
    parser.add_argument("--dense-top-k", type=int)
    parser.add_argument("--bm25-top-k", type=int)
    parser.add_argument("--rrf-k", type=int)
    parser.add_argument("--fusion-weight", type=float)
    parser.add_argument("--rerank-top-n", type=int)
    parser.add_argument("--final-n", type=int)


def overrides(args) -> Dict[str, int | float | None]:
    return {
        "dense_top_k": args.dense_top_k,
        "bm25_top_k": args.bm25_top_k,
        "rrf_k": args.rrf_k,
        "fusion_weight": args.fusion_weight,
        "rerank_top_n": args.rerank_top_n,
        "final_n": args.final_n,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(prog="rag_retrieve.py")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("retrieve")
    p.add_argument("--index", required=True)
    p.add_argument("--config", required=True)
    p.add_argument("--query", required=True)
    p.add_argument("--filter", action="append", default=[])
    p.add_argument("--mode", choices=["hybrid", "dense", "bm25"], default="hybrid")
    add_knobs(p)
    p = sub.add_parser("retrieve-eval")
    p.add_argument("--index", required=True)
    p.add_argument("--config", required=True)
    p.add_argument("--golden", required=True)
    p.add_argument("--mode", choices=["hybrid", "dense", "bm25"], default="hybrid")
    add_knobs(p)
    args = parser.parse_args(argv)
    index = load_json(args.index)
    config = load_json(args.config)
    if args.cmd == "retrieve":
        write_json(retrieve(index, config, args.query, parse_filter(args.filter), mode=args.mode, overrides=overrides(args)))
    else:
        write_json(evaluate(index, config, load_json(args.golden), args.mode, overrides(args)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
