"""Dense, BM25, RRF, and reranking primitives for local RAG retrieval."""
import math
from collections import Counter
from typing import Dict, Mapping, Sequence, Tuple

from rag_retrieve_text import chunk_text, count_overlap, dense_tokens, tokenize


def dense_rank(query: str, chunks: Sequence[Mapping], top_k: int):
    query_counts = Counter(dense_tokens(query))
    scored = [
        (str(chunk.get("chunk_id")), count_overlap(query_counts, Counter(dense_tokens(chunk_text(chunk)))))
        for chunk in chunks
    ] if query_counts else []
    return sorted([(cid, score) for cid, score in scored if score > 0], key=lambda item: (-item[1], item[0]))[:top_k]


def bm25_rank(query: str, chunks: Sequence[Mapping], top_k: int):
    query_terms = tokenize(query)
    docs = [tokenize(chunk_text(chunk)) for chunk in chunks]
    avgdl = sum(len(doc) for doc in docs) / max(len(docs), 1)
    df = Counter(term for term in set(query_terms) for doc in docs if term in set(doc))
    scored = [_bm25_chunk_score(chunk, doc, Counter(query_terms), len(docs), avgdl, df) for chunk, doc in zip(chunks, docs)]
    return sorted([(cid, score) for cid, score in scored if score > 0], key=lambda item: (-item[1], item[0]))[:top_k]


def _bm25_chunk_score(chunk: Mapping, doc, q_counts: Counter, doc_count: int, avgdl: float, df: Counter) -> Tuple[str, float]:
    counts, dl, score = Counter(doc), len(doc) or 1, 0.0
    for term, qtf in q_counts.items():
        tf = counts.get(term, 0)
        idf = math.log(1 + (doc_count - df[term] + 0.5) / (df[term] + 0.5)) if tf else 0.0
        score += idf * ((tf * 2.2) / (tf + 1.2 * (1 - 0.75 + 0.75 * dl / max(avgdl, 1)))) * qtf if tf else 0.0
    return str(chunk.get("chunk_id")), score


def rrf_fuse(dense: Sequence[Tuple[str, float]], bm25: Sequence[Tuple[str, float]], rrf_k: int, fusion_weight: float, mode: str) -> Dict[str, float]:
    dense_weight, sparse_weight = _mode_weights(mode, fusion_weight)
    scores: Dict[str, float] = {}
    for rank, (chunk_id, _score) in enumerate(dense, 1):
        scores[chunk_id] = scores.get(chunk_id, 0.0) + (dense_weight / (rrf_k + rank))
    for rank, (chunk_id, _score) in enumerate(bm25, 1):
        scores[chunk_id] = scores.get(chunk_id, 0.0) + (sparse_weight / (rrf_k + rank))
    return {chunk_id: score for chunk_id, score in scores.items() if score > 0}


def _mode_weights(mode: str, fusion_weight: float):
    dense_weight = max(0.0, min(1.0, fusion_weight))
    if mode == "dense":
        return 1.0, 0.0
    if mode == "bm25":
        return 0.0, 1.0
    return dense_weight, 1.0 - dense_weight


def rerank_score(query: str, chunk: Mapping) -> float:
    q = Counter(tokenize(query))
    terms = Counter(tokenize(chunk_text(chunk)))
    exact = count_overlap(q, terms) / max(sum(q.values()), 1) if q else 0.0
    heading = count_overlap(q, Counter(tokenize(chunk.get("heading", "")))) / max(sum(q.values()), 1) if q else 0.0
    phrase = 1.0 if str(query).lower() in chunk_text(chunk).lower() else 0.0
    return (0.75 * exact) + (0.15 * heading) + (0.10 * phrase)
